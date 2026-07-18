use bytes::Bytes;

/// Borrowed request data used by the static v2 dispatcher.
pub mod borrowed {
    use std::sync::OnceLock;

    use async_nats::HeaderMap;

    pub const REQUEST_ID_HEADER: &str = "x-request-id";
    const MAX_REQUEST_ID_LEN: usize = 128;

    #[inline]
    fn is_valid_request_id(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= MAX_REQUEST_ID_LEN
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/' | b'@')
            })
    }

    /// A copyable view over incoming NATS headers.
    #[derive(Debug, Clone, Copy)]
    pub struct Headers<'a> {
        inner: Option<&'a HeaderMap>,
    }

    impl<'a> Headers<'a> {
        #[inline]
        #[must_use]
        pub const fn new(inner: Option<&'a HeaderMap>) -> Self {
            Self { inner }
        }

        #[inline]
        #[must_use]
        pub fn get(self, name: &str) -> Option<&'a str> {
            self.inner
                .and_then(|headers| headers.get(name))
                .map(async_nats::HeaderValue::as_str)
        }

        #[inline]
        #[must_use]
        pub const fn raw(self) -> Option<&'a HeaderMap> {
            self.inner
        }
    }

    /// A borrowed request ID that is generated only when it is needed.
    #[derive(Debug)]
    pub struct RequestId<'a> {
        existing: Option<&'a str>,
        generated: OnceLock<String>,
    }

    impl<'a> RequestId<'a> {
        #[inline]
        #[must_use]
        pub fn new(headers: Headers<'a>) -> Self {
            Self {
                existing: headers
                    .get(REQUEST_ID_HEADER)
                    .filter(|value| is_valid_request_id(value)),
                generated: OnceLock::new(),
            }
        }

        #[inline]
        #[must_use]
        pub const fn existing(&self) -> Option<&'a str> {
            self.existing
        }

        #[inline]
        #[must_use]
        pub fn get_or_generate(&self) -> &str {
            self.existing.unwrap_or_else(|| {
                self.generated
                    .get_or_init(|| uuid::Uuid::now_v7().to_string())
                    .as_str()
            })
        }
    }

    /// A request view that borrows directly from the incoming transport frame.
    #[derive(Debug)]
    #[allow(clippy::struct_field_names)]
    pub struct Request<'a> {
        subject: &'a str,
        reply: Option<&'a str>,
        payload: &'a [u8],
        headers: Headers<'a>,
        request_id: RequestId<'a>,
    }

    impl<'a> Request<'a> {
        #[inline]
        #[must_use]
        pub fn new(
            subject: &'a str,
            reply: Option<&'a str>,
            payload: &'a [u8],
            headers: Option<&'a HeaderMap>,
        ) -> Self {
            let headers = Headers::new(headers);
            Self {
                subject,
                reply,
                payload,
                headers,
                request_id: RequestId::new(headers),
            }
        }

        #[inline]
        #[must_use]
        pub const fn subject(&self) -> &'a str {
            self.subject
        }

        #[inline]
        #[must_use]
        pub const fn reply(&self) -> Option<&'a str> {
            self.reply
        }

        #[inline]
        #[must_use]
        pub const fn body(&self) -> &'a [u8] {
            self.payload
        }

        #[inline]
        #[must_use]
        pub const fn headers(&self) -> Headers<'a> {
            self.headers
        }

        #[inline]
        #[must_use]
        pub const fn request_id(&self) -> &RequestId<'a> {
            &self.request_id
        }

        #[inline]
        #[must_use]
        pub const fn meta(&self) -> RequestMeta<'_> {
            RequestMeta {
                subject: self.subject,
                reply: self.reply,
                headers: self.headers,
                request_id: &self.request_id,
            }
        }
    }

    #[derive(Debug, Clone, Copy)]
    pub struct Body<'a>(pub &'a [u8]);

    #[derive(Debug, Clone, Copy)]
    pub struct Text<'a>(pub &'a str);

    #[derive(Debug, Clone, Copy)]
    pub struct RequestMeta<'a> {
        pub subject: &'a str,
        pub reply: Option<&'a str>,
        pub headers: Headers<'a>,
        pub request_id: &'a RequestId<'a>,
    }

    #[cfg(test)]
    mod tests {
        use async_nats::HeaderMap;

        use super::Request;

        #[test]
        fn borrows_frame_fields_and_existing_request_id() {
            let mut headers = HeaderMap::new();
            headers.insert("x-request-id", "request-42");
            let payload = b"hello";
            let request = Request::new(
                "users.v1.lookup",
                Some("_INBOX.reply"),
                payload,
                Some(&headers),
            );

            assert_eq!(request.subject(), "users.v1.lookup");
            assert_eq!(request.reply(), Some("_INBOX.reply"));
            assert_eq!(request.body(), payload);
            assert_eq!(request.request_id().existing(), Some("request-42"));
            assert_eq!(request.request_id().get_or_generate(), "request-42");
        }

        #[test]
        fn request_id_is_generated_lazily_for_missing_or_invalid_values() {
            let mut headers = HeaderMap::new();
            headers.insert("x-request-id", "");
            let request = Request::new("users.v1.lookup", None, b"", Some(&headers));

            assert_eq!(request.request_id().existing(), None);
            let generated = request.request_id().get_or_generate();
            assert_eq!(generated.len(), 36);
            assert_eq!(request.request_id().get_or_generate(), generated);
        }
    }
}

pub use borrowed::{Body, Headers as BorrowedHeaders, Request, RequestId, RequestMeta, Text};

#[derive(Debug, Clone)]
pub struct Header {
    pub key: String,
    pub value: String,
    #[cfg(feature = "encryption")]
    pub was_encrypted: bool,
}

impl Header {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

#[derive(Debug, Clone, Default)]
pub struct Headers(Vec<Header>);

impl Headers {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Header> {
        self.0
            .iter()
            .rfind(|header| header.key.eq_ignore_ascii_case(key))
    }

    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.insert_with_encryption(key, value, false);
    }

    pub fn append(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.append_with_encryption(key, value, false);
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Header> {
        self.0.iter()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[cfg(feature = "encryption")]
    pub fn insert_encrypted(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.insert_with_encryption(key, value, true);
    }

    fn insert_with_encryption(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
        was_encrypted: bool,
    ) {
        #[cfg(not(feature = "encryption"))]
        let _ = was_encrypted;

        let key = key.into();
        self.0
            .retain(|header| !header.key.eq_ignore_ascii_case(&key));
        self.0.push(Header {
            key,
            value: value.into(),
            #[cfg(feature = "encryption")]
            was_encrypted,
        });
    }

    fn append_with_encryption(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
        was_encrypted: bool,
    ) {
        #[cfg(not(feature = "encryption"))]
        let _ = was_encrypted;

        self.0.push(Header {
            key: key.into(),
            value: value.into(),
            #[cfg(feature = "encryption")]
            was_encrypted,
        });
    }
}

impl std::ops::Deref for Headers {
    type Target = Vec<Header>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> IntoIterator for &'a Headers {
    type Item = &'a Header;
    type IntoIter = std::slice::Iter<'a, Header>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl From<crate::NatsHeaderMap> for Headers {
    fn from(headers: crate::NatsHeaderMap) -> Self {
        let mut out = Self::new();

        for (name, values) in headers.iter() {
            let name_str: &str = name.as_ref();
            for value in values {
                out.append(name_str, value.as_str());
            }
        }

        out
    }
}

#[derive(Debug, Clone)]
pub struct NatsRequest {
    pub subject: String,
    pub payload: Bytes,
    pub headers: Headers,
    pub reply: Option<String>,
    pub request_id: String,
}

impl NatsRequest {
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    #[must_use]
    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }
}
