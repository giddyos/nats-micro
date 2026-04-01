use async_nats::HeaderMap;
use bytes::Bytes;
#[cfg(feature = "encryption")]
use x25519_dalek::PublicKey;

use crate::error::NatsErrorResponse;

#[derive(Default)]
pub struct ClientCallOptions {
    pub plaintext_headers: HeaderMap,
    #[cfg(feature = "encryption")]
    pub encrypted_headers: Vec<(String, String)>,
    #[cfg(feature = "encryption")]
    recipient: Option<crate::encryption::ServiceRecipient>,
}

impl ClientCallOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let key = key.into();
        let value = value.into();
        if let Ok(val) = value.parse::<async_nats::HeaderValue>() {
            self.plaintext_headers.insert(key.as_str(), val);
        }
        self
    }

    #[cfg(feature = "encryption")]
    pub fn encrypted_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.encrypted_headers.push((key.into(), value.into()));
        self
    }

    #[cfg(feature = "encryption")]
    pub fn recipient(mut self, pub_key: [u8; 32]) -> Self {
        self.recipient = Some(crate::encryption::ServiceRecipient::from_bytes(pub_key));
        self
    }

    #[cfg(feature = "encryption")]
    #[doc(hidden)]
    pub fn with_default_recipient(
        mut self,
        recipient: crate::encryption::ServiceRecipient,
    ) -> Self {
        if self.recipient.is_none() {
            self.recipient = Some(recipient);
        }
        self
    }

    pub async fn into_request(
        self,
        client: &async_nats::Client,
        subject: String,
        payload: Bytes,
    ) -> Result<async_nats::Message, NatsErrorResponse> {
        #[cfg(feature = "encryption")]
        if self.recipient.is_some() || !self.encrypted_headers.is_empty() {
            let recipient = self.recipient.ok_or_else(|| {
                NatsErrorResponse::internal(
                    "MISSING_RECIPIENT",
                    "encrypted headers require a recipient",
                )
            })?;
            let recipient = recipient.with_client(client.clone());
            let mut builder = recipient.request_builder();
            for (name, values) in self.plaintext_headers.iter() {
                let name_str: &str = name.as_ref();
                for val in values {
                    builder = builder.header(name_str, val.as_str());
                }
            }
            for (k, v) in self.encrypted_headers {
                builder = builder.encrypted_header(k, v);
            }
            builder = builder.payload(payload.to_vec());

            let (msg, _) = builder
                .nats_request(subject)
                .await
                .map_err(|e| NatsErrorResponse::internal("NATS_REQUEST_FAILED", e.to_string()))?;
            return Ok(msg);
        }

        if self.plaintext_headers.is_empty() {
            client
                .request(subject, payload)
                .await
                .map_err(|e| NatsErrorResponse::internal("NATS_REQUEST_FAILED", e.to_string()))
        } else {
            client
                .request_with_headers(subject, self.plaintext_headers, payload)
                .await
                .map_err(|e| NatsErrorResponse::internal("NATS_REQUEST_FAILED", e.to_string()))
        }
    }

    #[cfg(feature = "encryption")]
    #[doc(hidden)]
    pub async fn into_request_with_context(
        self,
        client: &async_nats::Client,
        subject: String,
        payload: Bytes,
    ) -> Result<
        (
            async_nats::Message,
            Option<crate::encryption::EphemeralContext>,
        ),
        NatsErrorResponse,
    > {
        if self.recipient.is_some() || !self.encrypted_headers.is_empty() {
            let recipient = self.recipient.ok_or_else(|| {
                NatsErrorResponse::internal(
                    "MISSING_RECIPIENT",
                    "encrypted headers require a recipient",
                )
            })?;
            let recipient = recipient.with_client(client.clone());
            let mut builder = recipient.request_builder();
            for (name, values) in self.plaintext_headers.iter() {
                let name_str: &str = name.as_ref();
                for val in values {
                    builder = builder.header(name_str, val.as_str());
                }
            }
            for (k, v) in self.encrypted_headers {
                builder = builder.encrypted_header(k, v);
            }
            builder = builder.payload(payload.to_vec());

            let (msg, eph_ctx) = builder
                .nats_request(subject)
                .await
                .map_err(|e| NatsErrorResponse::internal("NATS_REQUEST_FAILED", e.to_string()))?;
            return Ok((msg, Some(eph_ctx)));
        }

        let msg = if self.plaintext_headers.is_empty() {
            client
                .request(subject, payload)
                .await
                .map_err(|e| NatsErrorResponse::internal("NATS_REQUEST_FAILED", e.to_string()))?
        } else {
            client
                .request_with_headers(subject, self.plaintext_headers, payload)
                .await
                .map_err(|e| NatsErrorResponse::internal("NATS_REQUEST_FAILED", e.to_string()))?
        };

        Ok((msg, None))
    }

    #[cfg(feature = "encryption")]
    pub async fn into_encrypted_request(
        self,
        client: &async_nats::Client,
        subject: String,
        payload: Vec<u8>,
    ) -> Result<(async_nats::Message, crate::encryption::EphemeralContext), NatsErrorResponse> {
        let recipient = self.recipient.ok_or_else(|| {
            NatsErrorResponse::internal(
                "MISSING_RECIPIENT",
                "encrypted payloads require a recipient",
            )
        })?;
        let recipient = recipient.with_client(client.clone());
        let mut builder = recipient.request_builder();
        for (name, values) in self.plaintext_headers.iter() {
            let name_str: &str = name.as_ref();
            for val in values {
                builder = builder.header(name_str, val.as_str());
            }
        }
        for (k, v) in self.encrypted_headers {
            builder = builder.encrypted_header(k, v);
        }
        builder = builder.encrypted_payload(payload);

        let (msg, eph_ctx) = builder
            .nats_request(subject)
            .await
            .map_err(|e| NatsErrorResponse::internal("NATS_REQUEST_FAILED", e.to_string()))?;
        Ok((msg, eph_ctx))
    }
}
