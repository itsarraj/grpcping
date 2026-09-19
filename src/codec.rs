use prost::bytes::Buf;
use prost::Message;
use prost_reflect::{DynamicMessage, MessageDescriptor};
use tonic::codec::{BufferSettings, Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::Status;

/// A `tonic` [`Codec`] for `DynamicMessage`, the piece that makes a
/// gRPC call possible without generated `T: Message + Default` stubs.
/// Encoding needs no extra state — a `DynamicMessage` already carries
/// its own descriptor internally. Decoding does need the *response*
/// descriptor supplied up front, though: `DynamicMessage` has no
/// `Default` impl (there's no such thing as a message with no
/// descriptor at all), so `Codec::decoder()` bakes `response_desc` in
/// instead of relying on the `T::default()` trick generated codecs use.
#[derive(Clone)]
pub struct DynamicCodec {
    response_desc: MessageDescriptor,
}

impl DynamicCodec {
    pub fn new(response_desc: MessageDescriptor) -> Self {
        Self { response_desc }
    }
}

impl Codec for DynamicCodec {
    type Encode = DynamicMessage;
    type Decode = DynamicMessage;
    type Encoder = DynamicEncoder;
    type Decoder = DynamicDecoder;

    fn encoder(&mut self) -> Self::Encoder {
        DynamicEncoder
    }

    fn decoder(&mut self) -> Self::Decoder {
        DynamicDecoder {
            desc: self.response_desc.clone(),
        }
    }
}

#[derive(Clone)]
pub struct DynamicEncoder;

impl Encoder for DynamicEncoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, buf: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        item.encode(buf)
            .map_err(|e| Status::internal(format!("encoding request: {e}")))
    }

    fn buffer_settings(&self) -> BufferSettings {
        BufferSettings::default()
    }
}

#[derive(Clone)]
pub struct DynamicDecoder {
    desc: MessageDescriptor,
}

impl Decoder for DynamicDecoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn decode(&mut self, buf: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        if !buf.has_remaining() {
            return Ok(None);
        }
        let msg = DynamicMessage::decode(self.desc.clone(), buf)
            .map_err(|e| Status::internal(format!("decoding response: {e}")))?;
        Ok(Some(msg))
    }

    fn buffer_settings(&self) -> BufferSettings {
        BufferSettings::default()
    }
}
