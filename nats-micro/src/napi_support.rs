use nats_micro_shared::FrameworkError;

use crate::error::NatsErrorResponse;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NapiClientHeaderValue {
    pub name: String,
    pub value: String,
    #[cfg(feature = "encryption")]
    pub encrypted: bool,
}

impl NapiClientHeaderValue {
    #[cfg(feature = "encryption")]
    #[must_use]
    fn into_parts(self) -> (String, String, bool) {
        (self.name, self.value, self.encrypted)
    }

    #[cfg(not(feature = "encryption"))]
    #[must_use]
    fn into_parts(self) -> (String, String, bool) {
        (self.name, self.value, false)
    }
}

fn framework_error(error: FrameworkError, message: impl Into<String>) -> NatsErrorResponse {
    NatsErrorResponse::framework(error, message)
}

/// Converts JS-visible header input into client call options.
///
/// # Errors
///
/// Returns a framework error when encrypted headers are provided without a
/// configured recipient key, or when encryption support is unavailable.
pub fn client_call_options_from_headers<I>(
    headers: I,
    has_recipient: bool,
) -> Result<crate::ClientCallOptions, NatsErrorResponse>
where
    I: IntoIterator<Item = NapiClientHeaderValue>,
{
    let mut options = crate::ClientCallOptions::new();

    for header in headers {
        let (name, value, encrypted) = header.into_parts();

        if encrypted {
            #[cfg(feature = "encryption")]
            {
                if !has_recipient {
                    return Err(framework_error(
                        FrameworkError::MissingRecipientPubkey,
                        "Encrypted headers require recipientPublicKey in the N-API client connect options.",
                    ));
                }

                options = options.encrypted_header(name, value);
                continue;
            }

            #[cfg(not(feature = "encryption"))]
            {
                return Err(framework_error(
                    FrameworkError::EncryptRequired,
                    format!(
                        "Header `{name}` is marked as encrypted, but this nats-micro build does not enable the `encryption` feature."
                    ),
                ));
            }
        }

        options = options.header(name, value);
    }

    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::{NapiClientHeaderValue, client_call_options_from_headers};
    use nats_micro_shared::FrameworkError;

    #[test]
    fn plaintext_headers_build_client_call_options() {
        let options = client_call_options_from_headers(
            [NapiClientHeaderValue {
                name: "x-request-id".to_string(),
                value: "req-1".to_string(),
                #[cfg(feature = "encryption")]
                encrypted: false,
            }],
            false,
        )
        .unwrap();

        let header = options
            .plaintext_headers
            .get("x-request-id")
            .expect("plaintext header should be preserved");

        assert_eq!(header.as_str(), "req-1");

        #[cfg(feature = "encryption")]
        assert!(options.encrypted_headers.is_empty());
    }

    #[cfg(feature = "encryption")]
    #[test]
    fn encrypted_headers_build_client_call_options() {
        let options = client_call_options_from_headers(
            [NapiClientHeaderValue {
                name: "x-tenant-id".to_string(),
                value: "tenant-42".to_string(),
                encrypted: true,
            }],
            true,
        )
        .unwrap();

        assert!(options.plaintext_headers.is_empty());
        assert_eq!(
            options.encrypted_headers,
            vec![("x-tenant-id".to_string(), "tenant-42".to_string())]
        );
    }

    #[cfg(feature = "encryption")]
    #[test]
    fn encrypted_headers_require_recipient_public_key() {
        let Err(error) = client_call_options_from_headers(
            [NapiClientHeaderValue {
                name: "x-tenant-id".to_string(),
                value: "tenant-42".to_string(),
                encrypted: true,
            }],
            false,
        ) else {
            panic!("encrypted headers without recipient should fail");
        };

        assert_eq!(error.kind, FrameworkError::MissingRecipientPubkey.as_code());
        assert!(error.message.contains("recipientPublicKey"));
    }
}
