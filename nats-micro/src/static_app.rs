use std::future::Future;
use std::sync::Arc;

use anyhow::Context;
use tokio::sync::watch;

use crate::{NatsClient, RunnableService, ShutdownState};

/// A statically dispatched application containing one generated service.
///
/// Phase 3 extends this bootstrap into the complete typed service-list API.
pub struct App<S, Services = NoService> {
    state: S,
    services: Services,
    client: Option<NatsClient>,
}

#[doc(hidden)]
pub struct NoService;

impl App<(), NoService> {
    #[must_use]
    pub const fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> App<S, NoService> {
    #[must_use]
    pub const fn new(state: S) -> Self {
        Self {
            state,
            services: NoService,
            client: None,
        }
    }

    #[must_use]
    pub fn serve<Service>(self, service: Service) -> App<S, Service>
    where
        Service: RunnableService<S>,
    {
        App {
            state: self.state,
            services: service,
            client: self.client,
        }
    }
}

impl<S, Services> App<S, Services> {
    /// Uses an already-connected client instead of the `NATS_URL` environment
    /// variable. This is also the deterministic entry point for live tests.
    #[must_use]
    pub fn with_client(mut self, client: NatsClient) -> Self {
        self.client = Some(client);
        self
    }
}

impl<S, Services> App<S, Services>
where
    S: Send + Sync + 'static,
    Services: RunnableService<S> + Send + 'static,
{
    pub async fn run(self) -> anyhow::Result<()> {
        self.run_until(async {
            tokio::signal::ctrl_c()
                .await
                .context("failed to listen for the shutdown signal")
        })
        .await
    }

    pub async fn run_until<F>(self, shutdown_signal: F) -> anyhow::Result<()>
    where
        F: Future<Output = anyhow::Result<()>>,
    {
        let client = if let Some(client) = self.client {
            client
        } else {
            let server = std::env::var("NATS_URL").unwrap_or_else(|_| "127.0.0.1:4222".to_owned());
            crate::async_nats::connect(server)
                .await
                .context("failed to connect to NATS")?
        };
        let (shutdown_tx, shutdown_rx) = watch::channel(ShutdownState::running(None));
        let service = self
            .services
            .run_requests(Arc::new(self.state), client, shutdown_rx);
        tokio::pin!(service);
        tokio::pin!(shutdown_signal);

        tokio::select! {
            result = &mut service => result,
            signal = &mut shutdown_signal => {
                signal?;
                let _ = shutdown_tx.send(ShutdownState::requested(None));
                service.await
            }
        }
    }
}
