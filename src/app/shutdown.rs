use std::{future::Future, pin::Pin, sync::Arc};

use anyhow::Result;
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};
use tracing::error;

pub(super) type ShutdownHookFuture = Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>;
pub(super) type ShutdownHook = Arc<dyn Fn() -> ShutdownHookFuture + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkerExit {
    pub(super) label: String,
    pub(super) error: Option<String>,
}

impl WorkerExit {
    fn completed(label: String) -> Self {
        Self { label, error: None }
    }

    fn failed(label: String, error: String) -> Self {
        Self {
            label,
            error: Some(error),
        }
    }
}

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

pub(super) fn spawn_supervised_worker<Fut>(
    workers: &mut Vec<JoinHandle<()>>,
    worker_events: mpsc::UnboundedSender<WorkerExit>,
    label: impl Into<String>,
    worker: Fut,
) where
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    let label = label.into();
    workers.push(tokio::spawn(async move {
        let exit = match tokio::spawn(worker).await {
            Ok(Ok(())) => WorkerExit::completed(label),
            Ok(Err(err)) => WorkerExit::failed(label, err.to_string()),
            Err(err) => WorkerExit::failed(label, format!("task panicked: {err}")),
        };
        let _ = worker_events.send(exit);
    }));
}

pub(super) async fn run_shutdown_hook(shutdown_hook: Option<&ShutdownHook>) -> Result<()> {
    let Some(shutdown_hook) = shutdown_hook else {
        return Ok(());
    };

    (shutdown_hook)().await
}
