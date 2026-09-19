use std::collections::HashMap;

use prost::Message;
use prost_reflect::{DescriptorPool, ServiceDescriptor};
use prost_types::FileDescriptorProto;
use tonic::transport::Channel;
use tonic_reflection::pb::v1::server_reflection_client::ServerReflectionClient;
use tonic_reflection::pb::v1::server_reflection_request::MessageRequest;
use tonic_reflection::pb::v1::server_reflection_response::MessageResponse;
use tonic_reflection::pb::v1::ServerReflectionRequest;

/// Sends exactly one reflection request over its own short-lived
/// bidirectional stream and reads back the one response. The
/// reflection RPC is spec'd as streaming so a client *can* pipeline
/// many requests over one connection, but a fresh stream per request
/// is far simpler to get right — no request/response ordering to
/// track — and every server this was tested against (tonic-reflection
/// itself, which only ever answers one file per request anyway — see
/// `resolve_service`) tolerates it fine.
async fn reflect_once(
    client: &mut ServerReflectionClient<Channel>,
    message_request: MessageRequest,
) -> Result<MessageResponse, String> {
    let request = ServerReflectionRequest {
        host: String::new(),
        message_request: Some(message_request),
    };
    let stream = tokio_stream::once(request);
    let mut responses = client
        .server_reflection_info(stream)
        .await
        .map_err(|e| format!("reflection request failed: {e}"))?
        .into_inner();

    let response = responses
        .message()
        .await
        .map_err(|e| format!("reading reflection response: {e}"))?
        .ok_or_else(|| "server closed the reflection stream with no response".to_string())?;

    let message_response = response
        .message_response
        .ok_or_else(|| "reflection response had no message_response set".to_string())?;

    if let MessageResponse::ErrorResponse(err) = &message_response {
        return Err(format!(
            "server returned reflection error {}: {}",
            err.error_code, err.error_message
        ));
    }
    Ok(message_response)
}

pub async fn connect(addr: &str) -> Result<Channel, String> {
    let uri = if addr.contains("://") {
        addr.to_string()
    } else {
        format!("http://{addr}")
    };
    Channel::from_shared(uri.clone())
        .map_err(|e| format!("invalid address '{addr}': {e}"))?
        .connect()
        .await
        .map_err(|e| format!("connecting to '{addr}': {e}"))
}

pub async fn list_services(channel: Channel) -> Result<Vec<String>, String> {
    let mut client = ServerReflectionClient::new(channel);
    let response = reflect_once(&mut client, MessageRequest::ListServices(String::new())).await?;
    match response {
        MessageResponse::ListServicesResponse(list) => {
            let mut names: Vec<String> = list.service.into_iter().map(|s| s.name).collect();
            names.sort();
            Ok(names)
        }
        other => Err(format!("unexpected reflection response: {other:?}")),
    }
}

fn decode_file_descriptor_proto(bytes: &[u8]) -> Result<FileDescriptorProto, String> {
    FileDescriptorProto::decode(bytes).map_err(|e| format!("decoding FileDescriptorProto: {e}"))
}

/// Resolves the full transitive set of `.proto` files needed to
/// understand `service_full_name`, and returns a ready-to-use
/// `DescriptorPool` plus that service's descriptor.
///
/// The reflection protocol only ever answers with the *one* file that
/// directly declares whatever was asked for (confirmed by reading
/// `tonic-reflection`'s own server implementation, which always
/// responds with `file_descriptor_proto: vec![fd]` — a single entry,
/// never the transitive closure some documentation implies servers
/// might send). So this walks each file's `dependency` list and
/// issues additional `file_by_filename` requests for anything not yet
/// known, breadth-first, until nothing new is discovered — the same
/// thing a generated `protoc` invocation with `--include_imports`
/// would have produced in one shot, reconstructed here one reflection
/// round trip at a time.
pub async fn resolve_service(
    channel: Channel,
    service_full_name: &str,
) -> Result<(DescriptorPool, ServiceDescriptor), String> {
    let mut client = ServerReflectionClient::new(channel);
    let mut files: HashMap<String, FileDescriptorProto> = HashMap::new();

    let response = reflect_once(
        &mut client,
        MessageRequest::FileContainingSymbol(service_full_name.to_string()),
    )
    .await?;
    let root_files = match response {
        MessageResponse::FileDescriptorResponse(r) => r.file_descriptor_proto,
        other => return Err(format!("unexpected reflection response: {other:?}")),
    };

    let mut queue: Vec<String> = Vec::new();
    for raw in root_files {
        let fd = decode_file_descriptor_proto(&raw)?;
        queue.extend(fd.dependency.clone());
        files.insert(fd.name.clone().unwrap_or_default(), fd);
    }

    while let Some(dep_name) = queue.pop() {
        if files.contains_key(&dep_name) {
            continue;
        }
        let response = reflect_once(
            &mut client,
            MessageRequest::FileByFilename(dep_name.clone()),
        )
        .await?;
        let raw_files = match response {
            MessageResponse::FileDescriptorResponse(r) => r.file_descriptor_proto,
            other => return Err(format!("unexpected reflection response: {other:?}")),
        };
        for raw in raw_files {
            let fd = decode_file_descriptor_proto(&raw)?;
            for dep in &fd.dependency {
                if !files.contains_key(dep) {
                    queue.push(dep.clone());
                }
            }
            files.insert(fd.name.clone().unwrap_or_default(), fd);
        }
    }

    let mut pool = DescriptorPool::new();
    pool.add_file_descriptor_protos(files.into_values())
        .map_err(|e| format!("building descriptor pool: {e}"))?;

    let service = pool
        .get_service_by_name(service_full_name)
        .ok_or_else(|| format!("service '{service_full_name}' not found after reflection"))?;

    Ok((pool, service))
}
