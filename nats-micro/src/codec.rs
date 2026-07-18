use bytes::{BufMut, Bytes, BytesMut};
use serde::{Deserialize, Serialize};

use crate::{ErrorReply, FrameworkError, Request};

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

#[inline]
pub fn encode_json<T>(value: &T) -> Result<Bytes, ErrorReply>
where
    T: Serialize + ?Sized,
{
    let mut output = BytesMut::with_capacity(256);
    {
        let mut writer = (&mut output).writer();
        serde_json::to_writer(&mut writer, value).map_err(|error| {
            ErrorReply::framework(
                FrameworkError::SerializationError,
                format!("failed to encode JSON response: {error}"),
                None,
            )
        })?;
    }
    Ok(output.freeze())
}

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

#[inline]
pub fn encode_proto<T>(value: &T) -> Result<Bytes, ErrorReply>
where
    T: prost::Message,
{
    let mut output = BytesMut::with_capacity(value.encoded_len());
    value.encode(&mut output).map_err(|error| {
        ErrorReply::framework(
            FrameworkError::SerializationError,
            format!("failed to encode protobuf response: {error}"),
            None,
        )
    })?;
    Ok(output.freeze())
}

#[cfg(test)]
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
