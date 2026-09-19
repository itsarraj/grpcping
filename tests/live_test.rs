//! Live end-to-end test: spins up a real `tonic` gRPC server (with
//! real server reflection enabled) on localhost, then drives it
//! entirely through `grpcping`'s own client code — the same
//! `reflection::list_services`, `reflection::resolve_service`, and
//! `call::unary_call` functions the CLI binary calls. No generated
//! client stub is used to cross-check the response; the whole point
//! of this crate is that none exists for an arbitrary target service,
//! so the only honest test is exercising the dynamic path for real.

use std::time::Duration;

use prost_types::Timestamp;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

mod greeter_fixture {
    tonic::include_proto!("greeter");
    pub const DESCRIPTOR: &[u8] =
        include_bytes!(concat!(env!("OUT_DIR"), "/greeter_descriptor.bin"));
}

use greeter_fixture::greeter_server::{Greeter, GreeterServer};
use greeter_fixture::{HelloReply, HelloRequest};

#[derive(Default)]
struct MyGreeter;

#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        Ok(Response::new(HelloReply {
            message: format!("Hello, {}!", request.into_inner().name),
            created_at: Some(Timestamp {
                seconds: 1_700_000_000,
                nanos: 0,
            }),
        }))
    }

    async fn say_goodbye(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        Ok(Response::new(HelloReply {
            message: format!("Goodbye, {}!", request.into_inner().name),
            created_at: None,
        }))
    }
}

/// Binds an OS-assigned ephemeral port (`:0`) rather than a fixed one,
/// specifically so these tests are safe to run in parallel — the
/// default `cargo test` mode — instead of needing `--test-threads=1`
/// to avoid `AddrInUse` racing between tests, which a fixed port hit
/// immediately the first time this suite ran with default threading.
async fn spawn_test_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(greeter_fixture::DESCRIPTOR)
        .build_v1()
        .expect("building reflection service");

    tokio::spawn(async move {
        Server::builder()
            .add_service(GreeterServer::new(MyGreeter))
            .add_service(reflection_service)
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .expect("test server crashed");
    });

    // Give the server a moment to start accepting connections before
    // the test starts dialing it.
    tokio::time::sleep(Duration::from_millis(100)).await;
    addr
}

#[tokio::test]
async fn lists_the_real_greeter_service_via_reflection() {
    let addr = spawn_test_server().await;

    let channel = grpcping::reflection::connect(&addr).await.unwrap();
    let services = grpcping::reflection::list_services(channel).await.unwrap();

    assert!(
        services.iter().any(|s| s == "greeter.Greeter"),
        "expected greeter.Greeter in {services:?}"
    );
    // The reflection service itself is always registered too.
    assert!(services.iter().any(|s| s.contains("ServerReflection")));
}

#[tokio::test]
async fn resolves_the_service_and_its_two_methods_via_reflection() {
    let addr = spawn_test_server().await;

    let channel = grpcping::reflection::connect(&addr).await.unwrap();
    let (_pool, service) = grpcping::reflection::resolve_service(channel, "greeter.Greeter")
        .await
        .unwrap();

    let method_names: Vec<String> = service.methods().map(|m| m.name().to_string()).collect();
    assert!(method_names.contains(&"SayHello".to_string()));
    assert!(method_names.contains(&"SayGoodbye".to_string()));
}

#[tokio::test]
async fn resolves_a_field_of_an_imported_well_known_type() {
    // HelloReply.created_at is a google.protobuf.Timestamp — a type
    // declared in a *different* file than greeter.proto, imported by
    // it. Reflection only ever answers with the one file that directly
    // declares whatever was asked for (see reflection.rs's doc
    // comment), so successfully resolving this field's type at all is
    // proof the transitive dependency walk actually fetched
    // google/protobuf/timestamp.proto over a second reflection round
    // trip, not just the entries the first response.
    let addr = spawn_test_server().await;

    let channel = grpcping::reflection::connect(&addr).await.unwrap();
    let (_pool, service) = grpcping::reflection::resolve_service(channel, "greeter.Greeter")
        .await
        .unwrap();

    let say_hello = service.methods().find(|m| m.name() == "SayHello").unwrap();
    let created_at_field = say_hello
        .output()
        .get_field_by_name("created_at")
        .expect("created_at field should resolve");
    assert_eq!(
        created_at_field.kind().as_message().unwrap().full_name(),
        "google.protobuf.Timestamp"
    );
}

#[tokio::test]
async fn makes_a_real_dynamic_unary_call_and_gets_a_real_reply() {
    let addr = spawn_test_server().await;

    let channel = grpcping::reflection::connect(&addr).await.unwrap();
    let (_pool, service) =
        grpcping::reflection::resolve_service(channel.clone(), "greeter.Greeter")
            .await
            .unwrap();
    let method = service.methods().find(|m| m.name() == "SayHello").unwrap();

    let response = grpcping::call::unary_call(channel, &method, r#"{"name": "World"}"#)
        .await
        .unwrap();

    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["message"], "Hello, World!");
}

#[tokio::test]
async fn a_malformed_request_payload_is_a_clean_error_not_a_panic() {
    let addr = spawn_test_server().await;

    let channel = grpcping::reflection::connect(&addr).await.unwrap();
    let (_pool, service) =
        grpcping::reflection::resolve_service(channel.clone(), "greeter.Greeter")
            .await
            .unwrap();
    let method = service.methods().find(|m| m.name() == "SayHello").unwrap();

    // "name" should be a string; giving it a number should fail
    // cleanly during JSON-to-message conversion, before anything is
    // ever sent to the server.
    let result = grpcping::call::unary_call(channel, &method, r#"{"name": 12345}"#).await;
    assert!(result.is_err());
}
