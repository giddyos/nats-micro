use std::{collections::BTreeMap, path::Path, time::Duration};

use super::{Cons, Nil};
use crate::{App, NatsTransport, Service, ServiceSet};

const SKIP_PREFIX: &str = "NATS_MICRO_LIVE_TEST_SKIPPED:";

pub trait LiveServiceSet<S>: Send + Sync + 'static {
    fn collect_streams(&self, streams: &mut BTreeMap<&'static str, Vec<&'static str>>);
}

impl<S> LiveServiceSet<S> for Nil
where
    S: Send + Sync + 'static,
{
    fn collect_streams(&self, _streams: &mut BTreeMap<&'static str, Vec<&'static str>>) {}
}

impl<S, Head, Tail> LiveServiceSet<S> for Cons<Head, Tail>
where
    S: Send + Sync + 'static,
    Head: Service<S>,
    Tail: LiveServiceSet<S>,
{
    fn collect_streams(&self, streams: &mut BTreeMap<&'static str, Vec<&'static str>>) {
        for consumer in Head::SPEC.consumers {
            let subjects = streams.entry(consumer.stream).or_default();
            if !subjects.contains(&consumer.filter_subject) {
                subjects.push(consumer.filter_subject);
            }
        }
        self.tail.collect_streams(streams);
    }
}

pub struct LiveTestApp<S = (), Services = Nil> {
    state: S,
    services: Services,
    jetstream: bool,
    config: crate::AppConfig,
    #[cfg(feature = "encryption")]
    encryption: Option<crate::ServiceKeyPair>,
    #[cfg(feature = "telemetry")]
    telemetry_layer: Option<std::sync::Arc<dyn crate::TelemetryLayer>>,
}

impl LiveTestApp<(), Nil> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: (),
            services: Nil,
            jetstream: false,
            config: crate::AppConfig::for_profile(crate::Profile::Test),
            #[cfg(feature = "encryption")]
            encryption: None,
            #[cfg(feature = "telemetry")]
            telemetry_layer: None,
        }
    }
}

impl Default for LiveTestApp<(), Nil> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> LiveTestApp<S, Nil>
where
    S: Send + Sync + 'static,
{
    #[must_use]
    pub fn state<NewState>(self, state: NewState) -> LiveTestApp<NewState, Nil>
    where
        NewState: Send + Sync + 'static,
    {
        LiveTestApp {
            state,
            services: Nil,
            jetstream: self.jetstream,
            config: self.config,
            #[cfg(feature = "encryption")]
            encryption: self.encryption,
            #[cfg(feature = "telemetry")]
            telemetry_layer: self.telemetry_layer,
        }
    }
}

impl<S, Services> LiveTestApp<S, Services>
where
    S: Send + Sync + 'static,
{
    #[must_use]
    pub fn jetstream(mut self) -> Self {
        self.jetstream = true;
        self
    }

    #[must_use]
    pub fn config(mut self, config: crate::AppConfig) -> Self {
        self.config = config;
        self
    }

    #[cfg(feature = "telemetry")]
    #[must_use]
    pub fn telemetry_layer<L>(mut self, layer: L) -> Self
    where
        L: crate::TelemetryLayer,
    {
        self.telemetry_layer = Some(std::sync::Arc::new(layer));
        self
    }

    #[cfg(feature = "encryption")]
    #[must_use]
    pub fn encryption(mut self, keypair: crate::ServiceKeyPair) -> Self {
        self.encryption = Some(keypair);
        self
    }

    #[must_use]
    pub fn serve<New>(self, service: New) -> LiveTestApp<S, Cons<New, Services>>
    where
        New: Service<S>,
    {
        LiveTestApp {
            state: self.state,
            services: Cons {
                head: service,
                tail: self.services,
            },
            jetstream: self.jetstream,
            config: self.config,
            #[cfg(feature = "encryption")]
            encryption: self.encryption,
            #[cfg(feature = "telemetry")]
            telemetry_layer: self.telemetry_layer,
        }
    }

    pub async fn start(self) -> crate::Result<LiveTestHarness<S, Services>>
    where
        Services: ServiceSet<S> + LiveServiceSet<S>,
    {
        let mut streams = BTreeMap::new();
        self.services.collect_streams(&mut streams);
        let with_jetstream = self.jetstream || !streams.is_empty();
        let keep_logs = keep_test_logs();
        let server = start_server(with_jetstream, keep_logs).await?;
        let server_url = server.client_url();
        let client = async_nats::connect(server_url.clone()).await?;

        if with_jetstream {
            let jetstream = async_nats::jetstream::new(client.clone());
            for (name, subjects) in streams {
                jetstream
                    .get_or_create_stream(async_nats::jetstream::stream::Config {
                        name: name.to_owned(),
                        subjects: subjects.into_iter().map(str::to_owned).collect(),
                        ..Default::default()
                    })
                    .await?;
            }
        }

        let app = App::from_service_set(self.state, self.services)
            .config(self.config)
            .connection(crate::ConnectionConfig::new(server_url.clone()));
        #[cfg(feature = "telemetry")]
        let app = if let Some(layer) = self.telemetry_layer {
            app.telemetry_layer_arc(layer)
        } else {
            app
        };
        #[cfg(feature = "encryption")]
        let (app, recipient) = if let Some(keypair) = self.encryption {
            let recipient = crate::ServiceRecipient::from_bytes(keypair.public_key_bytes());
            (app.encryption(keypair), Some(recipient))
        } else {
            (app, None)
        };
        let running = app.start().await?;
        running.ready().await?;

        Ok(LiveTestHarness {
            transport: NatsTransport::new(client.clone()),
            client,
            running: Some(running),
            server: Some(server),
            server_url,
            state: std::marker::PhantomData,
            services: std::marker::PhantomData,
            #[cfg(feature = "encryption")]
            recipient,
        })
    }
}

pub struct LiveTestHarness<S, Services> {
    transport: NatsTransport,
    client: crate::NatsClient,
    running: Option<crate::RunningApp>,
    server: Option<nats_server::Server>,
    server_url: String,
    state: std::marker::PhantomData<fn() -> S>,
    services: std::marker::PhantomData<fn() -> Services>,
    #[cfg(feature = "encryption")]
    recipient: Option<crate::ServiceRecipient>,
}

impl<S, Services> LiveTestHarness<S, Services>
where
    S: Send + Sync + 'static,
{
    #[must_use]
    pub fn client<ServiceType>(&self) -> ServiceType::Client<NatsTransport>
    where
        ServiceType: Service<S>,
    {
        ServiceType::client(self.transport.clone())
    }

    #[cfg(feature = "encryption")]
    #[must_use]
    pub fn encrypted_client<ServiceType>(&self) -> ServiceType::Client<NatsTransport>
    where
        ServiceType: crate::EncryptedService<S>,
    {
        ServiceType::encrypted_client(
            self.transport.clone(),
            self.recipient
                .clone()
                .expect("encrypted live client requires a LiveTestApp encryption key"),
        )
    }

    #[must_use]
    pub const fn nats_client(&self) -> &crate::NatsClient {
        &self.client
    }

    #[must_use]
    pub fn server_url(&self) -> &str {
        &self.server_url
    }

    #[must_use]
    pub fn server(&self) -> &nats_server::Server {
        self.server.as_ref().expect("live server is present")
    }

    pub fn server_mut(&mut self) -> &mut nats_server::Server {
        self.server.as_mut().expect("live server is present")
    }

    #[must_use]
    pub fn server_log_path(&self) -> &Path {
        self.server().log_path()
    }

    pub async fn shutdown(mut self) -> crate::Result<()> {
        let result = if let Some(running) = self.running.take() {
            running.shutdown().await
        } else {
            Ok(())
        };
        self.server.take();
        result
    }
}

impl<S, Services> Drop for LiveTestHarness<S, Services> {
    fn drop(&mut self) {
        let Some(running) = self.running.take() else {
            return;
        };
        running.request_shutdown();
        let server = self.server.take();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread {
                tokio::task::block_in_place(|| {
                    let _ = handle.block_on(running.shutdown());
                });
                drop(server);
            } else {
                handle.spawn(async move {
                    let _ = running.shutdown().await;
                    drop(server);
                });
            }
        } else {
            drop(server);
        }
    }
}

async fn start_server(jetstream: bool, keep_logs: bool) -> crate::Result<nats_server::Server> {
    if !nats_server::is_server_available() {
        return missing_server("`nats-server` is not available on PATH");
    }

    let server = tokio::task::spawn_blocking(move || {
        let mut server = if jetstream {
            nats_server::try_run_server_with_jetstream()
        } else {
            nats_server::try_run_basic_server()
        }?;
        server.preserve_logs(keep_logs);
        Ok::<_, std::io::Error>(server)
    })
    .await
    .map_err(|error| anyhow::anyhow!("live server startup task failed: {error}"))?;

    match server {
        Ok(server) => Ok(server),
        Err(error) => missing_server(&format!("failed to start `nats-server`: {error}")),
    }
}

fn missing_server<T>(message: &str) -> crate::Result<T> {
    missing_server_for_requirement(message, require_nats_server())
}

fn missing_server_for_requirement<T>(message: &str, required: bool) -> crate::Result<T> {
    if required {
        anyhow::bail!("`nats-server` is required by NATS_MICRO_REQUIRE_NATS_SERVER=1: {message}");
    }
    anyhow::bail!("{SKIP_PREFIX} {message}");
}

#[must_use]
pub fn is_live_test_skip(error: &anyhow::Error) -> bool {
    error.to_string().starts_with(SKIP_PREFIX)
}

pub fn init_live_test_diagnostics() {
    super::init_test_tracing();
}

#[must_use]
pub fn live_test_timeout() -> Duration {
    std::env::var("NATS_MICRO_LIVE_TEST_TIMEOUT")
        .ok()
        .as_deref()
        .and_then(parse_duration)
        .unwrap_or(Duration::from_secs(30))
}

fn parse_duration(value: &str) -> Option<Duration> {
    if let Some(value) = value.strip_suffix("ms") {
        return value.parse().ok().map(Duration::from_millis);
    }
    if let Some(value) = value.strip_suffix('s') {
        return value.parse().ok().map(Duration::from_secs);
    }
    if let Some(value) = value.strip_suffix('m') {
        return value.parse().ok().map(Duration::from_mins);
    }
    value.parse().ok().map(Duration::from_secs)
}

fn require_nats_server() -> bool {
    env_flag(
        std::env::var("NATS_MICRO_REQUIRE_NATS_SERVER")
            .ok()
            .as_deref(),
    )
}

fn keep_test_logs() -> bool {
    env_flag(std::env::var("NATS_MICRO_KEEP_TEST_LOGS").ok().as_deref())
}

fn env_flag(value: Option<&str>) -> bool {
    matches!(value, Some("1"))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{env_flag, is_live_test_skip, missing_server_for_requirement, parse_duration};

    #[test]
    fn live_timeout_accepts_documented_units() {
        assert_eq!(parse_duration("250ms"), Some(Duration::from_millis(250)));
        assert_eq!(parse_duration("30s"), Some(Duration::from_secs(30)));
        assert_eq!(parse_duration("2m"), Some(Duration::from_mins(2)));
        assert_eq!(parse_duration("7"), Some(Duration::from_secs(7)));
        assert_eq!(parse_duration("invalid"), None);
    }

    #[test]
    fn live_environment_flags_require_exactly_one() {
        assert!(env_flag(Some("1")));
        assert!(!env_flag(None));
        assert!(!env_flag(Some("0")));
        assert!(!env_flag(Some("true")));
    }

    #[test]
    fn missing_server_only_skips_when_not_required() {
        let skipped = missing_server_for_requirement::<()>("not installed", false).unwrap_err();
        assert!(is_live_test_skip(&skipped));

        let required = missing_server_for_requirement::<()>("not installed", true).unwrap_err();
        assert!(!is_live_test_skip(&required));
        assert!(
            required
                .to_string()
                .contains("NATS_MICRO_REQUIRE_NATS_SERVER=1")
        );
    }
}
