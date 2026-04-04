use std::{path::PathBuf, time::Duration};

use nats_micro_shared::FrameworkError;

use crate::error::NatsErrorResponse;

#[derive(Debug, Clone, Default)]
pub struct NapiAuthOptions {
    pub token: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub nkey: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct NapiConnectOptions {
    pub name: Option<String>,
    pub no_echo: Option<bool>,
    pub max_reconnects: Option<u32>,
    pub connection_timeout_ms: Option<u32>,
    pub auth: Option<NapiAuthOptions>,
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

pub struct ConnectedClient {
    pub client: async_nats::Client,
    pub subject_prefix: Option<String>,
    #[cfg(feature = "encryption")]
    pub recipient: Option<crate::ServiceRecipient>,
}

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

fn apply_auth(
    mut options: async_nats::ConnectOptions,
    auth: NapiAuthOptions,
) -> Result<async_nats::ConnectOptions, NatsErrorResponse> {
    let NapiAuthOptions {
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

/// Connects a JS client using the validated N-API options surface.
///
/// # Errors
///
/// Returns a framework error when the supplied connect options are invalid or
/// when the underlying NATS client cannot connect.
pub async fn connect(
    server: String,
    input: NapiConnectOptions,
) -> Result<ConnectedClient, NatsErrorResponse> {
    let NapiConnectOptions {
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
    } = input;

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
