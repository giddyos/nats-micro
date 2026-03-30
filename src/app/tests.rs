use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tokio::sync::mpsc;

use super::{
    limits::{
        DEFAULT_CONCURRENCY_LIMIT, ResolvedConsumerConcurrencyLimit,
        resolve_consumer_concurrency_limit, resolve_endpoint_concurrency_limit,
        validate_consumer_concurrency_limit,
    },
    shutdown::{ShutdownHookFuture, spawn_supervised_worker, run_shutdown_hook},
};

#[test]
fn endpoint_concurrency_limit_defaults_to_constant() {
    assert_eq!(
        resolve_endpoint_concurrency_limit(None),
        DEFAULT_CONCURRENCY_LIMIT
    );
    assert_eq!(resolve_endpoint_concurrency_limit(Some(42)), 42);
}

#[test]
fn consumer_concurrency_limit_rejects_positive_max_ack_pending_violations() {
    assert!(validate_consumer_concurrency_limit(11, 10).is_err());
    assert!(validate_consumer_concurrency_limit(10, 10).is_ok());
    assert!(validate_consumer_concurrency_limit(100, 0).is_ok());
}

#[test]
fn consumer_default_concurrency_limit_promotes_to_server_max_ack_pending() {
    assert_eq!(
        resolve_consumer_concurrency_limit(None, 25_000),
        ResolvedConsumerConcurrencyLimit {
            value: 25_000,
            promoted_from_default: true,
        }
    );
    assert_eq!(
        resolve_consumer_concurrency_limit(None, 5_000),
        ResolvedConsumerConcurrencyLimit {
            value: DEFAULT_CONCURRENCY_LIMIT,
            promoted_from_default: false,
        }
    );
}

#[test]
fn explicit_consumer_concurrency_limit_is_preserved() {
    assert_eq!(
        resolve_consumer_concurrency_limit(Some(64), 100_000),
        ResolvedConsumerConcurrencyLimit {
            value: 64,
            promoted_from_default: false,
        }
    );
}

#[tokio::test]
async fn shutdown_hook_runs_when_present() {
    let counter = Arc::new(AtomicUsize::new(0));
    let hook_counter = counter.clone();
    let hook: Arc<dyn Fn() -> ShutdownHookFuture + Send + Sync> = Arc::new(move || {
        let hook_counter = hook_counter.clone();
        Box::pin(async move {
            hook_counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    });

    run_shutdown_hook(Some(&hook)).await.unwrap();
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn supervised_workers_report_success_failures_and_panics() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut workers = Vec::new();

    spawn_supervised_worker(&mut workers, tx.clone(), "ok-worker", async { Ok(()) });
    spawn_supervised_worker(&mut workers, tx.clone(), "err-worker", async {
        anyhow::bail!("boom")
    });
    spawn_supervised_worker(&mut workers, tx, "panic-worker", async {
        panic!("crash");
        #[allow(unreachable_code)]
        Ok(())
    });

    let mut exits = Vec::new();
    for _ in 0..3 {
        exits.push(rx.recv().await.expect("worker exit should be reported"));
    }

    for worker in workers {
        worker.await.unwrap();
    }

    exits.sort_by(|left, right| left.label.cmp(&right.label));

    assert_eq!(
        exits[0],
        super::shutdown::WorkerExit {
            label: "err-worker".to_string(),
            error: Some("boom".to_string()),
        }
    );
    assert_eq!(
        exits[1],
        super::shutdown::WorkerExit {
            label: "ok-worker".to_string(),
            error: None,
        }
    );
    assert_eq!(exits[2].label, "panic-worker");
    assert!(exits[2]
        .error
        .as_deref()
        .expect("panic should surface")
        .contains("task panicked"));
}
