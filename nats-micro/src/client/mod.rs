use std::{fmt::Display, path::PathBuf, str::FromStr, time::Duration};

mod transport;
mod v2;

pub use transport::{ClientRequest, ClientResponse, ClientTransport, NatsTransport, Subject};
pub use v2::{
    BytesDecoder, ClientBuildError, ClientResponseDecoder, EmptyDecoder, JsonDecoder,
    OptionalBytesDecoder, OptionalJsonDecoder, OptionalProtoDecoder, OptionalTextDecoder,
    OptionalVecDecoder, ProtoDecoder, PublishCall, RequestCall, ResponseDecoder, TextDecoder,
    VecDecoder, merge_headers,
};
#[cfg(feature = "encryption")]
pub use v2::{EncryptedPublishCall, EncryptedRequestCall};

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
    pub subject_prefix: Option<String>,
    #[cfg(feature = "encryption")]
    pub recipient_public_key: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct ConnectedClient {
    pub client: crate::NatsClient,
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
    } = auth;

    let has_token = token.is_some();
    let has_userpass = username.is_some() || password.is_some();

    let modes = u8::from(has_token) + u8::from(has_userpass);
    if modes > 1 {
        return Err(framework_error(
            FrameworkError::AuthModeConflict,
            "Choose exactly one authentication mode: token or username/password.",
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
    connect_inner(
        server,
        input,
        #[cfg(feature = "telemetry")]
        None,
    )
    .await
}

#[cfg(feature = "telemetry")]
pub(crate) async fn connect_with_telemetry(
    server: impl Into<String>,
    input: Option<ConnectOptions>,
    telemetry: Option<crate::telemetry::Telemetry>,
) -> Result<ConnectedClient, NatsErrorResponse> {
    connect_inner(server, input, telemetry).await
}

async fn connect_inner(
    server: impl Into<String>,
    input: Option<ConnectOptions>,
    #[cfg(feature = "telemetry")] telemetry: Option<crate::telemetry::Telemetry>,
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

    #[cfg(feature = "telemetry")]
    if let Some(telemetry) = telemetry {
        let connected_once = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        options = options.event_callback(move |event| {
            let telemetry = telemetry.clone();
            let connected_once = std::sync::Arc::clone(&connected_once);
            async move {
                if event == async_nats::Event::Connected
                    && connected_once.swap(true, std::sync::atomic::Ordering::Relaxed)
                {
                    telemetry.application(crate::MetricName::Reconnects, None);
                }
            }
        });
    }

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
    pub plaintext_headers: crate::NatsHeaderMap,
    #[cfg(feature = "encryption")]
    pub encrypted_headers: Vec<(String, String)>,
    #[cfg(feature = "encryption")]
    recipient: Option<crate::encryption::ServiceRecipient>,
}

async fn send_plain_request(
    client: &crate::NatsClient,
    subject: String,
    headers: crate::NatsHeaderMap,
    payload: Bytes,
) -> Result<crate::NatsMessage, NatsErrorResponse> {
    let request_subject = subject.clone();
    if headers.is_empty() {
        client
            .request(subject, payload)
            .await
            .map_err(|e| request_failed(&request_subject, e))
    } else {
        client
            .request_with_headers(subject, headers, payload)
            .await
            .map_err(|e| request_failed(&request_subject, e))
    }
}

#[cfg(feature = "encryption")]
fn apply_headers_to_builder(
    mut builder: crate::encryption::RequestBuilder,
    plaintext_headers: &crate::NatsHeaderMap,
    encrypted_headers: Vec<(String, String)>,
) -> Result<crate::encryption::RequestBuilder, NatsErrorResponse> {
    for (name, values) in plaintext_headers.iter() {
        let name_str: &str = name.as_ref();
        for val in values {
            builder = builder.header(name_str, val.as_str());
        }
    }
    for (key, value) in encrypted_headers {
        builder = builder
            .try_encrypted_header(key, value)
            .map_err(|error| encryption_error_to_invalid_header(&error))?;
    }
    Ok(builder)
}

#[cfg(feature = "encryption")]
fn encryption_error_to_invalid_header(error: &crate::EncryptionError) -> NatsErrorResponse {
    framework_error(FrameworkError::InvalidHeader, error.to_string())
}

impl ClientCallOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn try_from_request_context(
        ctx: &crate::RequestContext,
    ) -> Result<Self, NatsErrorResponse> {
        let mut options = Self::new();

        if !ctx.request.request_id.is_empty() {
            options = options.try_header("x-request-id", ctx.request.request_id.clone())?;
        }

        for header in &ctx.request.headers {
            if !matches!(
                header.key.to_ascii_lowercase().as_str(),
                "traceparent" | "tracestate" | "baggage" | "x-client-name"
            ) {
                continue;
            }

            #[cfg(feature = "encryption")]
            {
                if header.was_encrypted {
                    options =
                        options.try_encrypted_header(header.key.clone(), header.value.clone())?;
                    continue;
                }
            }

            options = options.try_header(header.key.clone(), header.value.clone())?;
        }

        Ok(options)
    }

    pub fn from_request_context(ctx: &crate::RequestContext) -> Self {
        Self::try_from_request_context(ctx).expect("invalid propagated request context header")
    }

    pub fn try_header(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, NatsErrorResponse> {
        let key = key.into();
        let value = value.into();
        let name = crate::NatsHeaderName::from_str(&key).map_err(|error| {
            framework_error(
                FrameworkError::InvalidHeader,
                format!("invalid client header name `{key}`: {error}"),
            )
        })?;
        let val = value.parse::<crate::NatsHeaderValue>().map_err(|error| {
            framework_error(
                FrameworkError::InvalidHeader,
                format!("invalid client header value for `{key}`: {error}"),
            )
        })?;
        self.plaintext_headers.insert(name, val);
        Ok(self)
    }

    pub fn header(self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.try_header(key, value)
            .expect("invalid client header name or value")
    }

    pub fn try_bearer_token(mut self, token: impl Into<String>) -> Result<Self, NatsErrorResponse> {
        let token = token.into();

        let header_name = "authorization";
        let header_value = format!("Bearer {token}");
        let header_value = header_value
            .parse::<crate::NatsHeaderValue>()
            .map_err(|error| {
                framework_error(
                    FrameworkError::InvalidHeader,
                    format!("invalid client bearer token header value: {error}"),
                )
            })?;

        #[cfg(feature = "encryption")]
        {
            self = self.try_encrypted_header(header_name, header_value.as_str())?;
        }

        #[cfg(not(feature = "encryption"))]
        {
            self.plaintext_headers.insert(header_name, header_value);
        }

        Ok(self)
    }

    pub fn bearer_token(self, token: impl Into<String>) -> Self {
        self.try_bearer_token(token)
            .expect("invalid client bearer token header value")
    }

    #[cfg(feature = "encryption")]
    pub fn try_encrypted_header(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, NatsErrorResponse> {
        let key = key.into();
        let value = value.into();
        let _ = crate::NatsHeaderName::from_str(&key).map_err(|error| {
            framework_error(
                FrameworkError::InvalidHeader,
                format!("invalid encrypted client header name `{key}`: {error}"),
            )
        })?;
        let _ = value.parse::<crate::NatsHeaderValue>().map_err(|error| {
            framework_error(
                FrameworkError::InvalidHeader,
                format!("invalid encrypted client header value for `{key}`: {error}"),
            )
        })?;
        if crate::encryption::is_reserved_encryption_header_name(&key) {
            return Err(framework_error(
                FrameworkError::InvalidHeader,
                format!(
                    "encrypted client header `{key}` is reserved for nats-micro encryption transport metadata"
                ),
            ));
        }

        self.encrypted_headers.push((key, value));
        Ok(self)
    }

    #[cfg(feature = "encryption")]
    pub fn encrypted_header(self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.try_encrypted_header(key, value)
            .expect("invalid encrypted client header name or value")
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
            .parse::<crate::NatsHeaderValue>()
            .expect("generated client headers must be valid HTTP header values");
        self.plaintext_headers.insert(key, header_value);
        self
    }

    pub async fn into_request(
        self,
        client: &crate::NatsClient,
        subject: String,
        payload: Bytes,
    ) -> Result<crate::NatsMessage, NatsErrorResponse> {
        #[cfg(feature = "encryption")]
        if self.recipient.is_some() || !self.encrypted_headers.is_empty() {
            let recipient = self.recipient.ok_or_else(|| {
                NatsErrorResponse::framework(
                    FrameworkError::MissingRecipientPubkey,
                    "Encrypted headers require a recipient public key. Configure the generated client with a default recipient public key.",
                )
            })?;
            let recipient = recipient.with_client(client.clone());
            let builder = apply_headers_to_builder(
                recipient.request_builder(),
                &self.plaintext_headers,
                self.encrypted_headers,
            )?
            .payload(payload.to_vec());

            let request_subject = subject.clone();
            let (msg, _) = builder
                .nats_request(subject)
                .await
                .map_err(|e| request_failed(&request_subject, e))?;
            return Ok(msg);
        }

        send_plain_request(client, subject, self.plaintext_headers, payload).await
    }

    #[cfg(feature = "encryption")]
    #[doc(hidden)]
    pub async fn into_request_with_context(
        self,
        client: &crate::NatsClient,
        subject: String,
        payload: Bytes,
    ) -> Result<
        (
            crate::NatsMessage,
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
            let builder = apply_headers_to_builder(
                recipient.request_builder(),
                &self.plaintext_headers,
                self.encrypted_headers,
            )?
            .payload(payload.to_vec());

            let request_subject = subject.clone();
            let (msg, eph_ctx) = builder
                .nats_request(subject)
                .await
                .map_err(|e| request_failed(&request_subject, e))?;
            return Ok((msg, Some(eph_ctx)));
        }

        let msg = send_plain_request(client, subject, self.plaintext_headers, payload).await?;

        Ok((msg, None))
    }

    #[cfg(feature = "encryption")]
    #[doc(hidden)]
    pub async fn into_encrypted_request(
        self,
        client: &crate::NatsClient,
        subject: String,
        payload: Vec<u8>,
    ) -> Result<(crate::NatsMessage, crate::encryption::EphemeralContext), NatsErrorResponse> {
        let recipient = self.recipient.ok_or_else(|| {
            NatsErrorResponse::framework(
                FrameworkError::MissingRecipientPubkey,
                "This endpoint expects an encrypted payload. Encrypted payloads require a recipient public key. Configure the generated client with a recipient public key.",
            )
        })?;
        let recipient = recipient.with_client(client.clone());
        let builder = apply_headers_to_builder(
            recipient.request_builder(),
            &self.plaintext_headers,
            self.encrypted_headers,
        )?
        .encrypted_payload(payload);

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
    use super::{AuthOptions, ClientCallOptions, ConnectOptions, connect};
    use crate::{NatsRequest, RequestContext, StateMap, request::Headers};
    use bytes::Bytes;
    use nats_micro_shared::FrameworkError;

    #[test]
    fn try_header_rejects_invalid_header_values() {
        let Err(err) = ClientCallOptions::new().try_header("x-trace", "bad\r\nvalue") else {
            panic!("invalid header value should be rejected");
        };

        assert_eq!(err.kind, FrameworkError::InvalidHeader.as_code());
        assert!(err.message.contains("invalid client header value"));
    }

    #[test]
    #[should_panic(expected = "invalid client header name or value")]
    fn header_panics_for_invalid_header_values() {
        let _ = ClientCallOptions::new().header("x-trace", "bad\r\nvalue");
    }

    #[test]
    fn try_bearer_token_rejects_invalid_header_values() {
        let Err(err) = ClientCallOptions::new().try_bearer_token("bad\r\nvalue") else {
            panic!("invalid bearer token should be rejected");
        };

        assert_eq!(err.kind, FrameworkError::InvalidHeader.as_code());
        assert!(err.message.contains("invalid client bearer token"));
    }

    #[test]
    #[should_panic(expected = "invalid client bearer token header value")]
    fn bearer_token_panics_for_invalid_header_values() {
        let _ = ClientCallOptions::new().bearer_token("bad\r\nvalue");
    }

    #[test]
    fn try_from_request_context_rejects_invalid_propagated_header_values() {
        let mut headers = Headers::new();
        headers.insert("traceparent", "bad\r\nvalue");
        let ctx = RequestContext::new(
            NatsRequest {
                subject: "demo.subject".to_string(),
                payload: Bytes::new(),
                headers,
                reply: None,
                request_id: "req-1".to_string(),
            },
            StateMap::new(),
            None,
        );

        let Err(err) = ClientCallOptions::try_from_request_context(&ctx) else {
            panic!("invalid propagated header should be rejected");
        };

        assert_eq!(err.kind, FrameworkError::InvalidHeader.as_code());
        assert!(err.message.contains("invalid client header value"));
    }

    #[cfg(feature = "encryption")]
    #[test]
    fn try_encrypted_header_rejects_reserved_transport_names() {
        let Err(err) =
            ClientCallOptions::new().try_encrypted_header("x-signature", "not-user-metadata")
        else {
            panic!("reserved encrypted header name should be rejected");
        };

        assert_eq!(err.kind, FrameworkError::InvalidHeader.as_code());
        assert!(err.message.contains("reserved"));
    }

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
