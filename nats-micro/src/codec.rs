#[cfg(any(feature = "json", feature = "protobuf"))]
use bytes::Bytes;
#[cfg(feature = "json")]
use serde::{Deserialize, Serialize};

use crate::{ErrorReply, FrameworkError, Request};

#[cfg(feature = "json")]
#[derive(Default)]
struct JsonSize(usize);

#[cfg(feature = "json")]
impl std::io::Write for JsonSize {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(feature = "json")]
#[inline]
pub fn decode_json<'de, T>(request: &'de Request<'de>) -> Result<T, ErrorReply>
where
    T: Deserialize<'de>,
{
    serde_json::from_slice(request.body()).map_err(|error| {
        ErrorReply::framework(
            FrameworkError::BadJson,
            format!("failed to decode JSON request: {error}"),
            request.request_id().existing(),
        )
    })
}

#[inline]
pub fn decode_text<'a>(request: &'a Request<'a>) -> Result<&'a str, ErrorReply> {
    std::str::from_utf8(request.body()).map_err(|error| {
        ErrorReply::framework(
            FrameworkError::BadUtf8,
            format!("failed to decode UTF-8 request: {error}"),
            request.request_id().existing(),
        )
    })
}

#[cfg(feature = "json")]
#[inline]
pub fn encode_json<T>(value: &T) -> Result<Bytes, ErrorReply>
where
    T: Serialize + ?Sized,
{
    let mut size = JsonSize::default();
    serde_json::to_writer(&mut size, value).map_err(|error| {
        ErrorReply::framework(
            FrameworkError::SerializationError,
            format!("failed to size JSON response: {error}"),
            None,
        )
    })?;
    let mut output = Vec::with_capacity(size.0);
    serde_json::to_writer(&mut output, value).map_err(|error| {
        ErrorReply::framework(
            FrameworkError::SerializationError,
            format!("failed to encode JSON response: {error}"),
            None,
        )
    })?;
    Ok(Bytes::from(output))
}

#[cfg(feature = "protobuf")]
#[inline]
pub fn decode_proto<T>(request: &Request<'_>) -> Result<T, ErrorReply>
where
    T: prost::Message + Default,
{
    T::decode(request.body()).map_err(|error| {
        ErrorReply::framework(
            FrameworkError::BadProtobuf,
            format!("failed to decode protobuf request: {error}"),
            request.request_id().existing(),
        )
    })
}

#[cfg(feature = "protobuf")]
#[inline]
pub fn encode_proto<T>(value: &T) -> Result<Bytes, ErrorReply>
where
    T: prost::Message,
{
    let mut output = Vec::with_capacity(value.encoded_len());
    value.encode(&mut output).map_err(|error| {
        ErrorReply::framework(
            FrameworkError::SerializationError,
            format!("failed to encode protobuf response: {error}"),
            None,
        )
    })?;
    Ok(Bytes::from(output))
}

#[cfg(all(test, feature = "json"))]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::{decode_json, decode_text, encode_json};
    use crate::Request;

    #[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
    struct Borrowed<'a> {
        name: &'a str,
    }

    #[test]
    fn json_decode_can_borrow_from_the_request_frame() {
        let request = Request::new("users.v1.create", None, br#"{"name":"Ada"}"#, None);
        let decoded: Borrowed<'_> = decode_json(&request).unwrap();

        assert_eq!(decoded, Borrowed { name: "Ada" });
    }

    #[test]
    fn text_decode_borrows_and_rejects_invalid_utf8() {
        let request = Request::new("users.v1.echo", None, b"hello", None);
        assert_eq!(decode_text(&request).unwrap(), "hello");

        let invalid = Request::new("users.v1.echo", None, &[0xff], None);
        assert_eq!(decode_text(&invalid).unwrap_err().kind, "BAD_UTF8");
    }

    #[test]
    fn json_encode_writes_directly_into_bytes() {
        let encoded = encode_json(&Borrowed { name: "Ada" }).unwrap();
        assert_eq!(encoded, br#"{"name":"Ada"}"#.as_slice());
    }
}
