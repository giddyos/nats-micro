use std::{future::Future, pin::Pin, sync::Arc};

use anyhow::Result;
use tokio::{sync::watch, task::JoinHandle};
use tracing::error;

pub(super) type ShutdownHookFuture = Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>;
pub(super) type ShutdownHook = Arc<dyn Fn() -> ShutdownHookFuture + Send + Sync>;

pub(super) struct LiveService {
    pub(super) _service: async_nats::service::Service,
    pub(super) workers: Vec<JoinHandle<()>>,
}

impl LiveService {
    pub(super) async fn shutdown(self) {
        for worker in self.workers {
            if let Err(err) = worker.await {
                error!(error = %err, "worker task failed during shutdown");
            }
        }
    }
}

pub(super) fn shutdown_requested(
    changed: Result<(), watch::error::RecvError>,
    shutdown_rx: &watch::Receiver<bool>,
) -> bool {
    changed.is_err() || *shutdown_rx.borrow()
}

pub(super) async fn run_shutdown_hook(shutdown_hook: Option<&ShutdownHook>) -> Result<()> {
    let Some(shutdown_hook) = shutdown_hook else {
        return Ok(());
    };

    (shutdown_hook)().await
}
