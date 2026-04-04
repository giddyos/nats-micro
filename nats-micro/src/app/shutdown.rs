use std::{any::Any, future::Future, panic::AssertUnwindSafe, pin::Pin, sync::Arc, time::Duration};

use anyhow::Result;
use futures::FutureExt;
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
    time::Instant,
};
use tracing::{error, warn};

use crate::shutdown_signal::ShutdownState;

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
    pub(super) async fn shutdown(self, deadline: Option<Instant>) -> Result<()> {
        wait_for_workers(self.workers, deadline).await
    }
}

pub(super) fn shutdown_requested(
    changed: &Result<(), watch::error::RecvError>,
    shutdown_rx: &watch::Receiver<ShutdownState>,
) -> bool {
    changed.is_err() || shutdown_rx.borrow().requested
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
        let exit = match AssertUnwindSafe(worker).catch_unwind().await {
            Ok(Ok(())) => WorkerExit::completed(label),
            Ok(Err(err)) => WorkerExit::failed(label, err.to_string()),
            Err(panic) => WorkerExit::failed(
                label,
                format!("task panicked: {}", panic_payload(panic.as_ref())),
            ),
        };
        let _ = worker_events.send(exit);
    }));
}

pub(super) fn shutdown_deadline(drain_timeout: Option<Duration>) -> Option<Instant> {
    drain_timeout.map(|timeout| Instant::now() + timeout)
}

pub(super) async fn wait_for_workers(
    workers: Vec<JoinHandle<()>>,
    deadline: Option<Instant>,
) -> Result<()> {
    let mut timed_out_workers = 0usize;

    for mut worker in workers {
        match deadline {
            Some(deadline) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    worker.abort();
                    let _ = worker.await;
                    timed_out_workers += 1;
                    continue;
                }

                match tokio::time::timeout(remaining, &mut worker).await {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => {
                        error!(error = %err, "worker task failed during shutdown");
                    }
                    Err(_) => {
                        worker.abort();
                        let _ = worker.await;
                        timed_out_workers += 1;
                    }
                }
            }
            None => {
                if let Err(err) = worker.await {
                    error!(error = %err, "worker task failed during shutdown");
                }
            }
        }
    }

    if timed_out_workers > 0 {
        warn!(
            timed_out_workers = timed_out_workers,
            "timed out waiting for workers to drain; aborting remaining tasks"
        );
        anyhow::bail!("timed out draining {timed_out_workers} worker(s)");
    }

    Ok(())
}

fn panic_payload(panic: &(dyn Any + Send)) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

pub(super) async fn run_shutdown_hook(shutdown_hook: Option<&ShutdownHook>) -> Result<()> {
    let Some(shutdown_hook) = shutdown_hook else {
        return Ok(());
    };

    (shutdown_hook)().await
}
