use prost_reflect::{DynamicMessage, MethodDescriptor};
use tonic::transport::Channel;

use crate::codec::DynamicCodec;

/// Makes one dynamic unary gRPC call: `json_payload` is deserialized
/// against `method`'s real input message shape (so a typo'd field name
/// or wrong type fails before anything goes over the wire), sent using
/// the low-level `tonic::client::Grpc` API with `DynamicCodec` in
/// place of a generated codec, and the response comes back as a
/// `DynamicMessage` ready to serialize straight to JSON.
pub async fn unary_call(
    channel: Channel,
    method: &MethodDescriptor,
    json_payload: &str,
) -> Result<DynamicMessage, String> {
    let input_desc = method.input();
    let request_msg = DynamicMessage::deserialize(
        input_desc,
        &mut serde_json::Deserializer::from_str(json_payload),
    )
    .map_err(|e| {
        format!(
            "request JSON doesn't match {}: {e}",
            method.input().full_name()
        )
    })?;

    let path = tonic::codegen::http::uri::PathAndQuery::try_from(format!(
        "/{}/{}",
        method.parent_service().full_name(),
        method.name()
    ))
    .map_err(|e| format!("building call path: {e}"))?;

    let mut grpc = tonic::client::Grpc::new(channel);
    grpc.ready()
        .await
        .map_err(|e| format!("channel not ready: {e}"))?;

    let codec = DynamicCodec::new(method.output());
    let response = grpc
        .unary(tonic::Request::new(request_msg), path, codec)
        .await
        .map_err(|status| format!("call failed: {status}"))?;

    Ok(response.into_inner())
}
