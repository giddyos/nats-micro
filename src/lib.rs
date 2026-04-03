use serde::{Deserialize, Serialize};

macro_rules! define_error_code_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $variant:ident => { code: $code:literal, status: $status:expr }
            ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        $vis enum $name {
            $(
                #[serde(rename = $code)]
                $variant,
            )+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[
                $(Self::$variant,)+
            ];

            pub const fn as_code(self) -> &'static str {
                match self {
                    $(Self::$variant => $code,)+
                }
            }

            pub const fn status_code(self) -> u16 {
                match self {
                    $(Self::$variant => $status,)+
                }
            }

            pub fn from_code(code: &str) -> Option<Self> {
                match code {
                    $($code => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_code())
            }
        }
    };
}

define_error_code_enum! {
    pub enum FrameworkError {
        BadJson => { code: "BAD_JSON", status: 400 },
        BadProtobuf => { code: "BAD_PROTOBUF", status: 400 },
        BadUtf8 => { code: "BAD_UTF8", status: 400 },
        DeserializationError => { code: "DESERIALIZATION_ERROR", status: 500 },
        Error => { code: "ERROR", status: 500 },
        Forbidden => { code: "FORBIDDEN", status: 403 },
        InitializationError => { code: "INITIALIZATION_ERROR", status: 500 },
        InternalError => { code: "INTERNAL_ERROR", status: 500 },
        InvalidResponse => { code: "INVALID_RESPONSE", status: 500 },
        ParamNameMissing => { code: "PARAM_NAME_MISSING", status: 500 },
        SerializationError => { code: "SERIALIZATION_ERROR", status: 500 },
        ShutdownSignalUnavailable => { code: "SHUTDOWN_SIGNAL_UNAVAILABLE", status: 500 },
        StateNotFound => { code: "STATE_NOT_FOUND", status: 500 },
        SubjectParamInvalid => { code: "SUBJECT_PARAM_INVALID", status: 400 },
        SubjectParamMissing => { code: "SUBJECT_PARAM_MISSING", status: 400 },
        SubjectTemplateMissing => { code: "SUBJECT_TEMPLATE_MISSING", status: 500 },
        Unauthorized => { code: "UNAUTHORIZED", status: 401 },
        DecryptError => { code: "DECRYPT_ERROR", status: 500 },
        DecryptFailed => { code: "DECRYPT_FAILED", status: 400 },
        EncryptFailed => { code: "ENCRYPT_FAILED", status: 500 },
        EncryptRequired => { code: "ENCRYPT_REQUIRED", status: 400 },
        MissingRecipient => { code: "MISSING_RECIPIENT", status: 400 },
        MissingRecipientPubkey => { code: "MISSING_RECIPIENT_PUBKEY", status: 400 },
        NoEncryptionKey => { code: "NO_ENCRYPTION_KEY", status: 500 },
        SignatureInvalid => { code: "SIGNATURE_INVALID", status: 400 },
        SignatureMissing => { code: "SIGNATURE_MISSING", status: 400 },
        AuthModeConflict => { code: "AUTH_MODE_CONFLICT", status: 400 },
        AuthUsernameRequired => { code: "AUTH_USERNAME_REQUIRED", status: 400 },
        AuthPasswordRequired => { code: "AUTH_PASSWORD_REQUIRED", status: 400 },
        AuthNkeyUnsupported => { code: "AUTH_NKEY_UNSUPPORTED", status: 400 },
        ClientCertKeyMismatch => { code: "CLIENT_CERT_KEY_MISMATCH", status: 400 },
        ClientConnectFailed => { code: "CLIENT_CONNECT_FAILED", status: 500 }
    }
}

define_error_code_enum! {
    pub enum TransportError {
        NatsRequestFailed => { code: "NATS_REQUEST_FAILED", status: 500 },
        StartupError => { code: "STARTUP_ERROR", status: 500 },
        TransportError => { code: "TRANSPORT_ERROR", status: 500 }
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameworkError, TransportError};

    #[test]
    fn framework_error_codes_round_trip() {
        assert_eq!(
            FrameworkError::MissingRecipientPubkey.as_code(),
            "MISSING_RECIPIENT_PUBKEY"
        );
        assert_eq!(FrameworkError::MissingRecipientPubkey.status_code(), 400);
        assert_eq!(
            FrameworkError::from_code("MISSING_RECIPIENT_PUBKEY"),
            Some(FrameworkError::MissingRecipientPubkey)
        );
    }

    #[test]
    fn transport_error_codes_round_trip() {
        assert_eq!(
            TransportError::NatsRequestFailed.as_code(),
            "NATS_REQUEST_FAILED"
        );
        assert_eq!(
            TransportError::from_code("NATS_REQUEST_FAILED"),
            Some(TransportError::NatsRequestFailed)
        );
    }
}
