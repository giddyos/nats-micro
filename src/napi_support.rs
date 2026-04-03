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

fn framework_error(error: FrameworkError, message: impl Into<String>) -> NatsErrorResponse {
    NatsErrorResponse::framework(error, message)
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

    let modes = has_token as u8 + has_userpass as u8 + has_nkey as u8;
    if modes > 1 {
        return Err(framework_error(
            FrameworkError::AuthModeConflict,
            "choose only one auth mode: token, username/password, or nkey",
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

    if nkey.is_some() {
        return Err(framework_error(
            FrameworkError::AuthNkeyUnsupported,
            "nkey auth is not enabled for this nats-micro build",
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

    let public_key: [u8; 32] = recipient_public_key.try_into().map_err(|_| {
        framework_error(
            FrameworkError::MissingRecipientPubkey,
            "recipient_public_key must contain exactly 32 bytes",
        )
    })?;

    Ok(Some(crate::ServiceRecipient::from_bytes(public_key)))
}

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

    let mut options = async_nats::ConnectOptions::new();

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
            options.max_reconnects(Some(max_reconnects as usize))
        };
    }

    if let Some(connection_timeout_ms) = connection_timeout_ms {
        options = options.connection_timeout(Duration::from_millis(connection_timeout_ms as u64));
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
                "client_cert and client_key must be provided together",
            ));
        }
    }

    if let Some(ping_interval_ms) = ping_interval_ms {
        options = options.ping_interval(Duration::from_millis(ping_interval_ms as u64));
    }

    if let Some(subscription_capacity) = subscription_capacity {
        options = options.subscription_capacity(subscription_capacity as usize);
    }

    if let Some(sender_capacity) = sender_capacity {
        options = options.client_capacity(sender_capacity as usize);
    }

    if let Some(inbox_prefix) = inbox_prefix {
        options = options.custom_inbox_prefix(inbox_prefix);
    }

    if let Some(request_timeout_ms) = request_timeout_ms {
        options = options.request_timeout(Some(Duration::from_millis(request_timeout_ms as u64)));
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

    let client = options
        .connect(server)
        .await
        .map_err(|error| framework_error(FrameworkError::ClientConnectFailed, error.to_string()))?;

    Ok(ConnectedClient {
        client,
        subject_prefix,
        #[cfg(feature = "encryption")]
        recipient,
    })
}
