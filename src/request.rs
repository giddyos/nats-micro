use bytes::Bytes;

#[derive(Debug, Clone)]
pub struct Header {
    pub key: String,
    pub value: String,
    #[cfg(feature = "encryption")]
    pub was_encrypted: bool,
}

impl Header {
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

#[derive(Debug, Clone, Default)]
pub struct Headers(Vec<Header>);

impl Headers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, key: &str) -> Option<&Header> {
        self.0
            .iter()
            .rev()
            .find(|header| header.key.eq_ignore_ascii_case(key))
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

impl From<async_nats::HeaderMap> for Headers {
    fn from(headers: async_nats::HeaderMap) -> Self {
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
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }
}
