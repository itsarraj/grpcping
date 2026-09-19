//! A tiny real gRPC server with reflection enabled, for trying
//! `grpcping` against something real without needing your own service
//! handy. Run with `cargo run --example greeter_server`, then in
//! another terminal:
//!
//! ```bash
//! grpcping list 127.0.0.1:50077
//! grpcping describe 127.0.0.1:50077 greeter.Greeter
//! grpcping call 127.0.0.1:50077 greeter.Greeter/SayHello -d '{"name": "World"}'
//! ```

use prost_types::Timestamp;
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "127.0.0.1:50077".parse()?;
    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(greeter_fixture::DESCRIPTOR)
        .build_v1()?;

    println!("greeter example server listening on {addr}");
    Server::builder()
        .add_service(GreeterServer::new(MyGreeter))
        .add_service(reflection_service)
        .serve(addr)
        .await?;
    Ok(())
}
