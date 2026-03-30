use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use tokio::sync::mpsc;

use super::{
    config::{NatsAppConfig, WorkerFailurePolicy},
    limits::{
        DEFAULT_CONCURRENCY_LIMIT, ResolvedConsumerConcurrencyLimit,
        resolve_consumer_concurrency_limit, resolve_endpoint_concurrency_limit,
        validate_consumer_concurrency_limit,
    },
    shutdown::{
        ShutdownHookFuture, run_shutdown_hook, shutdown_deadline, spawn_supervised_worker,
        wait_for_workers,
    },
};

#[test]
fn endpoint_concurrency_limit_defaults_to_constant() {
    assert_eq!(
        resolve_endpoint_concurrency_limit(None, DEFAULT_CONCURRENCY_LIMIT),
        DEFAULT_CONCURRENCY_LIMIT
    );
    assert_eq!(
        resolve_endpoint_concurrency_limit(Some(42), DEFAULT_CONCURRENCY_LIMIT),
        42
    );
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
        resolve_consumer_concurrency_limit(None, 25_000, DEFAULT_CONCURRENCY_LIMIT),
        ResolvedConsumerConcurrencyLimit {
            value: 25_000,
            promoted_from_default: true,
        }
    );
    assert_eq!(
        resolve_consumer_concurrency_limit(None, 5_000, DEFAULT_CONCURRENCY_LIMIT),
        ResolvedConsumerConcurrencyLimit {
            value: DEFAULT_CONCURRENCY_LIMIT,
            promoted_from_default: false,
        }
    );
}

#[test]
fn explicit_consumer_concurrency_limit_is_preserved() {
    assert_eq!(
        resolve_consumer_concurrency_limit(Some(64), 100_000, DEFAULT_CONCURRENCY_LIMIT),
        ResolvedConsumerConcurrencyLimit {
            value: 64,
            promoted_from_default: false,
        }
    );
}

#[test]
fn app_config_defaults_are_explicit() {
    let config = NatsAppConfig::default();

    assert_eq!(config.default_concurrency_limit(), DEFAULT_CONCURRENCY_LIMIT);
    assert_eq!(config.shutdown_drain_timeout(), None);
    assert_eq!(
        config.worker_failure_policy(),
        WorkerFailurePolicy::ShutdownApp
    );
}

#[test]
fn app_config_builders_override_runtime_knobs() {
    let config = NatsAppConfig::new()
        .with_default_concurrency_limit(64)
        .with_shutdown_drain_timeout(Duration::from_secs(3))
        .with_worker_failure_policy(WorkerFailurePolicy::Ignore);

    assert_eq!(config.default_concurrency_limit(), 64);
    assert_eq!(
        config.shutdown_drain_timeout(),
        Some(Duration::from_secs(3))
    );
    assert_eq!(config.worker_failure_policy(), WorkerFailurePolicy::Ignore);
}

#[test]
fn app_config_rejects_zero_default_concurrency_limit() {
    let err = NatsAppConfig::new()
        .with_default_concurrency_limit(0)
        .validate()
        .unwrap_err();

    assert!(err.to_string().contains("greater than zero"));
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
    assert!(
        exits[2]
            .error
            .as_deref()
            .expect("panic should surface")
            .contains("task panicked")
    );
}

#[tokio::test]
async fn worker_drain_timeout_aborts_slow_workers() {
    let workers = vec![tokio::spawn(async {
        tokio::time::sleep(Duration::from_secs(30)).await;
    })];

    let err = wait_for_workers(workers, shutdown_deadline(Some(Duration::from_millis(10))))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("timed out draining"));
}
