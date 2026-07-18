#![cfg_attr(not(feature = "telemetry"), allow(unused_variables))]

use std::{future::Future, sync::Arc};

use anyhow::Context;
use tokio::{sync::watch, task::JoinHandle};

use super::{
    AppConfig, ConnectionConfig, Cons, Nil, Profile, Runtime, Service, ServiceSet,
    typed_runtime::ShutdownHook, validate_service_set,
};
use crate::ShutdownState;

pub struct App<S, Services = Nil, Hook = NoStartupHook> {
    state: Arc<S>,
    services: Services,
    startup_hook: Hook,
    config: AppConfig,
    connection: ConnectionConfig,
    connected_client: Option<crate::NatsClient>,
    shutdown_hook: Option<ShutdownHook>,
    #[cfg(feature = "encryption")]
    encryption: Option<Arc<crate::ServiceKeyPair>>,
    #[cfg(feature = "telemetry")]
    telemetry_layer: Option<Arc<dyn crate::TelemetryLayer>>,
}

#[doc(hidden)]
pub struct NoStartupHook;

#[doc(hidden)]
pub trait RunStartupHook<S>: Send + 'static {
    fn run(self, state: Arc<S>) -> impl Future<Output = crate::Result<()>> + Send;
}

impl<S> RunStartupHook<S> for NoStartupHook
where
    S: Send + Sync + 'static,
{
    async fn run(self, _state: Arc<S>) -> crate::Result<()> {
        Ok(())
    }
}

impl<S, F, Fut> RunStartupHook<S> for F
where
    F: FnOnce(Arc<S>) -> Fut + Send + 'static,
    Fut: Future<Output = crate::Result<()>> + Send,
{
    fn run(self, state: Arc<S>) -> impl Future<Output = crate::Result<()>> + Send {
        self(state)
    }
}

impl App<(), Nil> {
    #[must_use]
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> App<S, Nil>
where
    S: Send + Sync + 'static,
{
    #[must_use]
    pub fn new(state: S) -> Self {
        Self {
            state: Arc::new(state),
            services: Nil,
            startup_hook: NoStartupHook,
            config: AppConfig::default(),
            connection: ConnectionConfig::from_env(),
            connected_client: None,
            shutdown_hook: None,
            #[cfg(feature = "encryption")]
            encryption: None,
            #[cfg(feature = "telemetry")]
            telemetry_layer: None,
        }
    }
}

#[cfg(feature = "live-test")]
impl<S, Services> App<S, Services, NoStartupHook>
where
    S: Send + Sync + 'static,
{
    pub(crate) fn from_service_set(state: S, services: Services) -> Self {
        Self {
            state: Arc::new(state),
            services,
            startup_hook: NoStartupHook,
            config: AppConfig::default(),
            connection: ConnectionConfig::from_env(),
            connected_client: None,
            shutdown_hook: None,
            #[cfg(feature = "encryption")]
            encryption: None,
            #[cfg(feature = "telemetry")]
            telemetry_layer: None,
        }
    }
}

impl<S, Services, Hook> App<S, Services, Hook>
where
    S: Send + Sync + 'static,
{
    #[must_use]
    pub fn serve<New>(self, service: New) -> App<S, Cons<New, Services>, Hook>
    where
        New: Service<S>,
    {
        App {
            state: self.state,
            services: Cons {
                head: service,
                tail: self.services,
            },
            startup_hook: self.startup_hook,
            config: self.config,
            connection: self.connection,
            connected_client: self.connected_client,
            shutdown_hook: self.shutdown_hook,
            #[cfg(feature = "encryption")]
            encryption: self.encryption,
            #[cfg(feature = "telemetry")]
            telemetry_layer: self.telemetry_layer,
        }
    }

    #[must_use]
    pub fn profile(mut self, profile: Profile) -> Self {
        self.config.apply_profile(profile);
        self
    }

    #[must_use]
    pub fn config(mut self, config: AppConfig) -> Self {
        self.config = config;
        self
    }

    #[must_use]
    pub fn connection(mut self, connection: ConnectionConfig) -> Self {
        self.connection = connection;
        self.connected_client = None;
        self
    }

    #[must_use]
    pub fn with_client(mut self, client: crate::NatsClient) -> Self {
        self.connected_client = Some(client);
        self
    }

    #[cfg(feature = "encryption")]
    #[must_use]
    pub fn encryption(mut self, keypair: crate::ServiceKeyPair) -> Self {
        self.encryption = Some(Arc::new(keypair));
        self
    }

    /// Installs the optional metrics/tracing layer used by static workers.
    #[cfg(feature = "telemetry")]
    #[must_use]
    pub fn telemetry_layer<L>(mut self, layer: L) -> Self
    where
        L: crate::TelemetryLayer,
    {
        self.telemetry_layer = Some(Arc::new(layer));
        self
    }

    #[cfg(all(feature = "telemetry", feature = "live-test"))]
    pub(crate) fn telemetry_layer_arc(mut self, layer: Arc<dyn crate::TelemetryLayer>) -> Self {
        self.telemetry_layer = Some(layer);
        self
    }

    #[must_use]
    pub fn startup_hook<NewHook>(self, hook: NewHook) -> App<S, Services, NewHook>
    where
        NewHook: Send + 'static,
    {
        App {
            state: self.state,
            services: self.services,
            startup_hook: hook,
            config: self.config,
            connection: self.connection,
            connected_client: self.connected_client,
            shutdown_hook: self.shutdown_hook,
            #[cfg(feature = "encryption")]
            encryption: self.encryption,
            #[cfg(feature = "telemetry")]
            telemetry_layer: self.telemetry_layer,
        }
    }

    #[must_use]
    pub fn shutdown_hook<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = crate::Result<()>> + Send + 'static,
    {
        self.shutdown_hook = Some(Arc::new(move || Box::pin(hook())));
        self
    }
}

impl<S, Services, Hook> App<S, Services, Hook>
where
    S: Send + Sync + 'static,
    Services: ServiceSet<S>,
    Hook: RunStartupHook<S>,
{
    pub async fn start(self) -> crate::Result<RunningApp> {
        self.config.validate()?;
        validate_service_set::<S, Services>(&self.services)?;

        let client = if let Some(client) = self.connected_client {
            client
        } else {
            #[cfg(feature = "telemetry")]
            let connected = crate::client::connect_with_telemetry(
                self.connection.server,
                self.connection.options,
                self.telemetry_layer.as_ref().map(|layer| {
                    crate::telemetry::Telemetry::new(Arc::clone(layer), self.config.telemetry)
                }),
            )
            .await?;
            #[cfg(not(feature = "telemetry"))]
            let connected = crate::connect(self.connection.server, self.connection.options).await?;
            connected.client
        };
        let mut runtime = Runtime::new(client, Arc::clone(&self.state), self.config.clone());
        #[cfg(feature = "encryption")]
        runtime.set_encryption_key(self.encryption);
        #[cfg(feature = "telemetry")]
        runtime.set_telemetry_layer(self.telemetry_layer);
        runtime.set_shutdown_hook(self.shutdown_hook);
        let startup_hook = tokio::time::timeout(
            self.config.startup_timeout,
            self.startup_hook.run(Arc::clone(&self.state)),
        )
        .await
        .context("application startup hook timed out")
        .and_then(std::convert::identity);
        if let Err(error) = startup_hook {
            if let Err(cleanup_error) = runtime.shutdown_after_failed_start().await {
                crate::trace::error!(%cleanup_error, "failed to clean up after startup hook failure");
            }
            return Err(error);
        }

        let service_start = tokio::time::timeout(
            self.config.startup_timeout,
            self.services.start_all(&mut runtime),
        )
        .await
        .context("application startup timed out")
        .and_then(std::convert::identity);
        if let Err(error) = service_start {
            if let Err(cleanup_error) = runtime.shutdown_after_failed_start().await {
                crate::trace::error!(%cleanup_error, "failed to clean up after service startup failure");
            }
            return Err(error);
        }

        let shutdown = runtime.shutdown_sender();
        let (ready_tx, ready) = watch::channel(false);
        let done = tokio::spawn(runtime.run());
        let _ = ready_tx.send(true);

        Ok(RunningApp {
            shutdown,
            ready,
            done,
            drain_timeout: self.config.shutdown_drain_timeout,
        })
    }

    pub async fn run(self) -> crate::Result<()> {
        self.run_until(async {
            tokio::signal::ctrl_c()
                .await
                .context("failed to listen for the shutdown signal")
        })
        .await
    }

    pub async fn run_until<F>(self, shutdown_signal: F) -> crate::Result<()>
    where
        F: Future<Output = crate::Result<()>>,
    {
        let running = self.start().await?;
        running.ready().await?;
        running.run_until(shutdown_signal).await
    }
}

pub struct RunningApp {
    shutdown: watch::Sender<ShutdownState>,
    ready: watch::Receiver<bool>,
    done: JoinHandle<crate::Result<()>>,
    drain_timeout: std::time::Duration,
}

impl RunningApp {
    #[cfg(feature = "live-test")]
    pub(crate) fn request_shutdown(&self) {
        let _ = self
            .shutdown
            .send(ShutdownState::requested(Some(self.drain_timeout)));
    }

    pub async fn ready(&self) -> crate::Result<()> {
        let mut ready = self.ready.clone();
        while !*ready.borrow() {
            ready
                .changed()
                .await
                .context("application stopped before becoming ready")?;
        }
        Ok(())
    }

    pub async fn shutdown(self) -> crate::Result<()> {
        let _ = self
            .shutdown
            .send(ShutdownState::requested(Some(self.drain_timeout)));
        self.done
            .await
            .context("application supervisor task failed")?
    }

    async fn run_until<F>(mut self, shutdown_signal: F) -> crate::Result<()>
    where
        F: Future<Output = crate::Result<()>>,
    {
        tokio::pin!(shutdown_signal);
        tokio::select! {
            result = &mut self.done => {
                result.context("application supervisor task failed")?
            }
            signal = &mut shutdown_signal => {
                signal?;
                let _ = self.shutdown.send(ShutdownState::requested(Some(
                    self.drain_timeout,
                )));
                self.done
                    .await
                    .context("application supervisor task failed")?
            }
        }
    }
}
