use std::{path::PathBuf, time::Duration};

mod transport;
mod v2;

pub use transport::{ClientRequest, ClientResponse, ClientTransport, NatsTransport, Subject};
pub use v2::{
    BytesDecoder, ClientBuildError, ClientResponseDecoder, EmptyDecoder, OptionalBytesDecoder,
    OptionalTextDecoder, OptionalVecDecoder, PublishCall, RequestCall, ResponseDecoder,
    TextDecoder, VecDecoder, merge_headers,
};
#[cfg(feature = "encryption")]
pub use v2::{EncryptedPublishCall, EncryptedRequestCall};
#[cfg(feature = "json")]
pub use v2::{JsonDecoder, OptionalJsonDecoder};
#[cfg(feature = "protobuf")]
pub use v2::{OptionalProtoDecoder, ProtoDecoder};

use nats_micro_shared::FrameworkError;

use crate::error::NatsErrorResponse;

pub const X_CLIENT_VERSION_HEADER: &str = "x-client-version";

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

    if has_token && has_userpass {
        return Err(framework_error(
            FrameworkError::AuthModeConflict,
            "choose either token or username/password authentication",
        ));
    }
    if let Some(token) = token {
        options = options.token(token);
    }
    if has_userpass {
        let username = username.ok_or_else(|| {
            framework_error(
                FrameworkError::AuthUsernameRequired,
                "username is required when password is provided",
            )
        })?;
        let password = password.ok_or_else(|| {
            framework_error(
                FrameworkError::AuthPasswordRequired,
                "password is required when username is provided",
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
            format!("recipient_public_key must contain 32 bytes, received {actual_len}"),
        )
    })?;
    Ok(Some(crate::ServiceRecipient::from_bytes(public_key)))
}

fn capacity(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
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

#[allow(clippy::too_many_lines)]
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

    let mut options = async_nats::ConnectOptions::new();
    if let Some(name) = name {
        options = options.name(name);
    }
    if no_echo.unwrap_or(false) {
        options = options.no_echo();
    }
    if let Some(max_reconnects) = max_reconnects {
        options = options.max_reconnects((max_reconnects != 0).then(|| capacity(max_reconnects)));
    }
    if let Some(timeout) = connection_timeout_ms {
        options = options.connection_timeout(Duration::from_millis(u64::from(timeout)));
    }
    if let Some(auth) = auth {
        options = apply_auth(options, auth)?;
    }
    if tls_required.unwrap_or(false) {
        options = options.require_tls(true);
    }
    if tls_first.unwrap_or(false) {
        options = options.tls_first();
    }
    for certificate in certificates.unwrap_or_default() {
        options = options.add_root_certificates(PathBuf::from(certificate));
    }
    match (client_cert, client_key) {
        (Some(cert), Some(key)) => {
            options = options.add_client_certificate(PathBuf::from(cert), PathBuf::from(key));
        }
        (None, None) => {}
        _ => {
            return Err(framework_error(
                FrameworkError::ClientCertKeyMismatch,
                "TLS client authentication requires both client_cert and client_key",
            ));
        }
    }
    if let Some(interval) = ping_interval_ms {
        options = options.ping_interval(Duration::from_millis(u64::from(interval)));
    }
    if let Some(value) = subscription_capacity {
        options = options.subscription_capacity(capacity(value));
    }
    if let Some(value) = sender_capacity {
        options = options.client_capacity(capacity(value));
    }
    if let Some(prefix) = inbox_prefix {
        options = options.custom_inbox_prefix(prefix);
    }
    if let Some(timeout) = request_timeout_ms {
        options = options.request_timeout(Some(Duration::from_millis(u64::from(timeout))));
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
    if let Some(value) = read_buffer_capacity {
        options = options.read_buffer_capacity(value);
    }

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
