use std::{fmt::Display, path::PathBuf, time::Duration};

use async_nats::HeaderMap;
use bytes::Bytes;
use nats_micro_shared::{FrameworkError, TransportError as SharedTransportError};

use crate::error::NatsErrorResponse;

pub const X_CLIENT_VERSION_HEADER: &str = "x-client-version";

fn request_failed(subject: &str, error: impl Display) -> NatsErrorResponse {
    NatsErrorResponse::transport(
        SharedTransportError::NatsRequestFailed,
        format!("failed to send a NATS request to subject `{subject}`: {error}"),
    )
}

fn framework_error(error: FrameworkError, message: impl Into<String>) -> NatsErrorResponse {
    NatsErrorResponse::framework(error, message)
}

#[derive(Debug, Clone, Default)]
pub struct AuthOptions {
    pub token: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub nkey: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ConnectOptions {
    pub name: Option<String>,
    pub no_echo: Option<bool>,
    pub max_reconnects: Option<u32>,
    pub connection_timeout_ms: Option<u32>,
    pub auth: Option<AuthOptions>,
    pub tls_required: Option<bool>,
    pub tls_first: Option<bool>,
    pub certificates: Option<Vec<String>>,
    pub client_cert: Option<String>,
    pub client_key: Option<String>,
    pub ping_interval_ms: Option<u32>,
    pub subscription_capacity: Option<u32>,
    pub sender_capacity: Option<u32>,
    pub inbox_prefix: Option<String>,
    pub request_timeout_ms: Option<u32>,
    pub retry_on_initial_connect: Option<bool>,
    pub ignore_discovered_servers: Option<bool>,
    pub retain_servers_order: Option<bool>,
    pub read_buffer_capacity: Option<u16>,
    #[cfg(debug_assertions)]
    pub subject_prefix: Option<String>,
    #[cfg(feature = "encryption")]
    pub recipient_public_key: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct ConnectedClient {
    pub client: async_nats::Client,
    pub subject_prefix: Option<String>,
    #[cfg(feature = "encryption")]
    pub recipient: Option<crate::ServiceRecipient>,
}

fn apply_auth(
    mut options: async_nats::ConnectOptions,
    auth: AuthOptions,
) -> Result<async_nats::ConnectOptions, NatsErrorResponse> {
    let AuthOptions {
        token,
        username,
        password,
        nkey,
    } = auth;

    let has_token = token.is_some();
    let has_userpass = username.is_some() || password.is_some();
    let has_nkey = nkey.is_some();

    let modes = u8::from(has_token) + u8::from(has_userpass) + u8::from(has_nkey);
    if modes > 1 {
        return Err(framework_error(
            FrameworkError::AuthModeConflict,
            "Choose exactly one authentication mode: token, username/password, or nkey.",
        ));
    }

    if let Some(token) = token {
        options = options.token(token);
    }

    if has_userpass {
        let username = username.ok_or_else(|| {
            framework_error(
                FrameworkError::AuthUsernameRequired,
                "username is required when password is provided.",
            )
        })?;
        let password = password.ok_or_else(|| {
            framework_error(
                FrameworkError::AuthPasswordRequired,
                "password is required when username is provided.",
            )
        })?;
        options = options.user_and_password(username, password);
    }

    if nkey.is_some() {
        return Err(framework_error(
            FrameworkError::AuthNkeyUnsupported,
            "NKey authentication is not enabled in this nats-micro build.",
        ));
    }

    Ok(options)
}

#[cfg(feature = "encryption")]
fn parse_recipient_public_key(
    recipient_public_key: Option<Vec<u8>>,
) -> Result<Option<crate::ServiceRecipient>, NatsErrorResponse> {
    let Some(recipient_public_key) = recipient_public_key else {
        return Ok(None);
    };

    let actual_len = recipient_public_key.len();
    let public_key: [u8; 32] = recipient_public_key.try_into().map_err(|_| {
        framework_error(
            FrameworkError::MissingRecipientPubkey,
            format!(
                "recipient_public_key must contain exactly 32 bytes, but received {actual_len}."
            ),
        )
    })?;

    Ok(Some(crate::ServiceRecipient::from_bytes(public_key)))
}

fn capacity_from_u32(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

fn apply_basic_connect_options(
    mut options: async_nats::ConnectOptions,
    name: Option<String>,
    no_echo: Option<bool>,
    max_reconnects: Option<u32>,
    connection_timeout_ms: Option<u32>,
) -> async_nats::ConnectOptions {
    if let Some(name) = name {
        options = options.name(name);
    }

    if no_echo.unwrap_or(false) {
        options = options.no_echo();
    }

    if let Some(max_reconnects) = max_reconnects {
        options = if max_reconnects == 0 {
            options.max_reconnects(None)
        } else {
            options.max_reconnects(Some(capacity_from_u32(max_reconnects)))
        };
    }

    if let Some(connection_timeout_ms) = connection_timeout_ms {
        options =
            options.connection_timeout(Duration::from_millis(u64::from(connection_timeout_ms)));
    }

    options
}

fn apply_tls_connect_options(
    mut options: async_nats::ConnectOptions,
    tls_required: Option<bool>,
    tls_first: Option<bool>,
    certificates: Option<Vec<String>>,
    client_cert: Option<String>,
    client_key: Option<String>,
) -> Result<async_nats::ConnectOptions, NatsErrorResponse> {
    if tls_required.unwrap_or(false) {
        options = options.require_tls(true);
    }

    if tls_first.unwrap_or(false) {
        options = options.tls_first();
    }

    if let Some(certificates) = certificates {
        for certificate in certificates {
            options = options.add_root_certificates(PathBuf::from(certificate));
        }
    }

    match (client_cert, client_key) {
        (Some(client_cert), Some(client_key)) => {
            options = options
                .add_client_certificate(PathBuf::from(client_cert), PathBuf::from(client_key));
        }
        (None, None) => {}
        _ => {
            return Err(framework_error(
                FrameworkError::ClientCertKeyMismatch,
                "TLS client authentication requires both client_cert and client_key.",
            ));
        }
    }

    Ok(options)
}

#[allow(clippy::too_many_arguments)]
fn apply_runtime_connect_options(
    mut options: async_nats::ConnectOptions,
    ping_interval_ms: Option<u32>,
    subscription_capacity: Option<u32>,
    sender_capacity: Option<u32>,
    inbox_prefix: Option<String>,
    request_timeout_ms: Option<u32>,
    retry_on_initial_connect: Option<bool>,
    ignore_discovered_servers: Option<bool>,
    retain_servers_order: Option<bool>,
    read_buffer_capacity: Option<u16>,
) -> async_nats::ConnectOptions {
    if let Some(ping_interval_ms) = ping_interval_ms {
        options = options.ping_interval(Duration::from_millis(u64::from(ping_interval_ms)));
    }

    if let Some(subscription_capacity) = subscription_capacity {
        options = options.subscription_capacity(capacity_from_u32(subscription_capacity));
    }

    if let Some(sender_capacity) = sender_capacity {
        options = options.client_capacity(capacity_from_u32(sender_capacity));
    }

    if let Some(inbox_prefix) = inbox_prefix {
        options = options.custom_inbox_prefix(inbox_prefix);
    }

    if let Some(request_timeout_ms) = request_timeout_ms {
        options =
            options.request_timeout(Some(Duration::from_millis(u64::from(request_timeout_ms))));
    }

    if retry_on_initial_connect.unwrap_or(false) {
        options = options.retry_on_initial_connect();
    }

    if ignore_discovered_servers.unwrap_or(false) {
        options = options.ignore_discovered_servers();
    }

    if retain_servers_order.unwrap_or(false) {
        options = options.retain_servers_order();
    }

    if let Some(read_buffer_capacity) = read_buffer_capacity {
        options = options.read_buffer_capacity(read_buffer_capacity);
    }

    options
}

pub async fn connect(
    server: impl Into<String>,
    input: Option<ConnectOptions>,
) -> Result<ConnectedClient, NatsErrorResponse> {
    let server = server.into();
    let ConnectOptions {
        name,
        no_echo,
        max_reconnects,
        connection_timeout_ms,
        auth,
        tls_required,
        tls_first,
        certificates,
        client_cert,
        client_key,
        ping_interval_ms,
        subscription_capacity,
        sender_capacity,
        inbox_prefix,
        request_timeout_ms,
        retry_on_initial_connect,
        ignore_discovered_servers,
        retain_servers_order,
        read_buffer_capacity,
        subject_prefix,
        #[cfg(feature = "encryption")]
        recipient_public_key,
    } = input.unwrap_or_default();

    #[cfg(feature = "encryption")]
    let recipient = parse_recipient_public_key(recipient_public_key)?;

    let mut options = apply_basic_connect_options(
        async_nats::ConnectOptions::new(),
        name,
        no_echo,
        max_reconnects,
        connection_timeout_ms,
    );

    if let Some(auth) = auth {
        options = apply_auth(options, auth)?;
    }

    options = apply_tls_connect_options(
        options,
        tls_required,
        tls_first,
        certificates,
        client_cert,
        client_key,
    )?;

    options = apply_runtime_connect_options(
        options,
        ping_interval_ms,
        subscription_capacity,
        sender_capacity,
        inbox_prefix,
        request_timeout_ms,
        retry_on_initial_connect,
        ignore_discovered_servers,
        retain_servers_order,
        read_buffer_capacity,
    );

    let client = options.connect(server.clone()).await.map_err(|error| {
        framework_error(
            FrameworkError::ClientConnectFailed,
            format!("failed to connect to NATS server `{server}`: {error}"),
        )
    })?;

    Ok(ConnectedClient {
        client,
        subject_prefix,
        #[cfg(feature = "encryption")]
        recipient,
    })
}

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

    pub fn bearer_token(mut self, token: impl Into<String>) -> Self {
        let token = token.into();

        let header_name = "authorization";
        let header_value = format!("Bearer {token}")
            .parse::<async_nats::HeaderValue>()
            .expect("generated client headers must be valid HTTP header values");

        if cfg!(feature = "encryption") {
            self.encrypted_headers
                .push((header_name.to_string(), header_value.to_string()));
        } else {
            self.plaintext_headers.insert(header_name, header_value);
        }

        self
    }

    #[cfg(feature = "encryption")]
    pub fn encrypted_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.encrypted_headers.push((key.into(), value.into()));
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

    #[doc(hidden)]
    pub fn with_required_header(mut self, key: &'static str, value: impl Into<String>) -> Self {
        let header_value = value
            .into()
            .parse::<async_nats::HeaderValue>()
            .expect("generated client headers must be valid HTTP header values");
        self.plaintext_headers.insert(key, header_value);
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
                NatsErrorResponse::framework(
                    FrameworkError::MissingRecipientPubkey,
                    "Encrypted headers require a recipient public key. Configure the generated client with a default recipient public key.",
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

            let request_subject = subject.clone();
            let (msg, _) = builder
                .nats_request(subject)
                .await
                .map_err(|e| request_failed(&request_subject, e))?;
            return Ok(msg);
        }

        if self.plaintext_headers.is_empty() {
            let request_subject = subject.clone();
            client
                .request(subject, payload)
                .await
                .map_err(|e| request_failed(&request_subject, e))
        } else {
            let request_subject = subject.clone();
            client
                .request_with_headers(subject, self.plaintext_headers, payload)
                .await
                .map_err(|e| request_failed(&request_subject, e))
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
                NatsErrorResponse::framework(
                    FrameworkError::MissingRecipientPubkey,
                    "Encrypted headers require a recipient public key. Configure the generated client with a recipient public key.",
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

            let request_subject = subject.clone();
            let (msg, eph_ctx) = builder
                .nats_request(subject)
                .await
                .map_err(|e| request_failed(&request_subject, e))?;
            return Ok((msg, Some(eph_ctx)));
        }

        let msg = if self.plaintext_headers.is_empty() {
            let request_subject = subject.clone();
            client
                .request(subject, payload)
                .await
                .map_err(|e| request_failed(&request_subject, e))?
        } else {
            let request_subject = subject.clone();
            client
                .request_with_headers(subject, self.plaintext_headers, payload)
                .await
                .map_err(|e| request_failed(&request_subject, e))?
        };

        Ok((msg, None))
    }

    #[cfg(feature = "encryption")]
    #[doc(hidden)]
    pub async fn into_encrypted_request(
        self,
        client: &async_nats::Client,
        subject: String,
        payload: Vec<u8>,
    ) -> Result<(async_nats::Message, crate::encryption::EphemeralContext), NatsErrorResponse> {
        let recipient = self.recipient.ok_or_else(|| {
            NatsErrorResponse::framework(
                FrameworkError::MissingRecipientPubkey,
                "This endpoint expects an encrypted payload. Encrypted payloads require a recipient public key. Configure the generated client with a recipient public key.",
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

        let request_subject = subject.clone();
        let (msg, eph_ctx) = builder
            .nats_request(subject)
            .await
            .map_err(|e| request_failed(&request_subject, e))?;
        Ok((msg, eph_ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::{AuthOptions, ConnectOptions, connect};
    use nats_micro_shared::FrameworkError;

    #[tokio::test]
    async fn connect_rejects_conflicting_auth_modes_before_dialing() {
        let Err(error) = connect(
            "nats://127.0.0.1:4222",
            Some(ConnectOptions {
                auth: Some(AuthOptions {
                    token: Some("token".to_string()),
                    username: Some("user".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        )
        .await
        else {
            panic!("connect should reject conflicting auth modes");
        };

        assert_eq!(error.kind, FrameworkError::AuthModeConflict.as_code());
        assert!(
            error
                .message
                .contains("Choose exactly one authentication mode")
        );
    }

    #[tokio::test]
    async fn connect_requires_password_with_username_auth() {
        let Err(error) = connect(
            "nats://127.0.0.1:4222",
            Some(ConnectOptions {
                auth: Some(AuthOptions {
                    username: Some("user".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        )
        .await
        else {
            panic!("connect should require a password with username auth");
        };

        assert_eq!(error.kind, FrameworkError::AuthPasswordRequired.as_code());
        assert!(error.message.contains("password is required"));
    }

    #[cfg(feature = "encryption")]
    #[tokio::test]
    async fn connect_rejects_invalid_recipient_public_key_before_dialing() {
        let Err(error) = connect(
            "nats://127.0.0.1:4222",
            Some(ConnectOptions {
                recipient_public_key: Some(vec![1, 2, 3]),
                ..Default::default()
            }),
        )
        .await
        else {
            panic!("connect should reject invalid recipient keys");
        };

        assert_eq!(error.kind, FrameworkError::MissingRecipientPubkey.as_code());
        assert!(error.message.contains("exactly 32 bytes"));
    }
}
