use std::time::Duration;

use anyhow::Result;

use super::limits::{DEFAULT_CONCURRENCY_LIMIT, DEFAULT_MAX_CONCURRENCY_LIMIT};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkerFailurePolicy {
    Ignore,
    #[default]
    ShutdownApp,
    Restart {
        initial_backoff: Duration,
        max_backoff: Duration,
        max_restarts: u32,
        window: Duration,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HandlerPanicPolicy {
    #[default]
    FailWorker,
    LogAndContinue,
}

#[derive(Debug, Clone)]
pub struct NatsAppConfig {
    default_concurrency_limit: u64,
    max_concurrency_limit: u64,
    unbounded_server_ack_pending_promotion: bool,
    shutdown_drain_timeout: Option<Duration>,
    worker_failure_policy: WorkerFailurePolicy,
    handler_panic_policy: HandlerPanicPolicy,
}

impl Default for NatsAppConfig {
    fn default() -> Self {
        Self {
            default_concurrency_limit: DEFAULT_CONCURRENCY_LIMIT,
            max_concurrency_limit: DEFAULT_MAX_CONCURRENCY_LIMIT,
            unbounded_server_ack_pending_promotion: false,
            shutdown_drain_timeout: None,
            worker_failure_policy: WorkerFailurePolicy::ShutdownApp,
            handler_panic_policy: HandlerPanicPolicy::FailWorker,
        }
    }
}

impl NatsAppConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn default_concurrency_limit(&self) -> u64 {
        self.default_concurrency_limit
    }

    pub fn max_concurrency_limit(&self) -> u64 {
        self.max_concurrency_limit
    }

    pub fn unbounded_server_ack_pending_promotion(&self) -> bool {
        self.unbounded_server_ack_pending_promotion
    }

    pub fn shutdown_drain_timeout(&self) -> Option<Duration> {
        self.shutdown_drain_timeout
    }

    pub fn worker_failure_policy(&self) -> WorkerFailurePolicy {
        self.worker_failure_policy
    }

    pub fn handler_panic_policy(&self) -> HandlerPanicPolicy {
        self.handler_panic_policy
    }

    pub fn with_default_concurrency_limit(mut self, limit: u64) -> Self {
        self.default_concurrency_limit = limit;
        self
    }

    pub fn with_max_concurrency_limit(mut self, limit: u64) -> Self {
        self.max_concurrency_limit = limit;
        self
    }

    pub fn with_unbounded_server_ack_pending_promotion(mut self, enabled: bool) -> Self {
        self.unbounded_server_ack_pending_promotion = enabled;
        self
    }

    pub fn with_shutdown_drain_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_drain_timeout = Some(timeout);
        self
    }

    pub fn without_shutdown_drain_timeout(mut self) -> Self {
        self.shutdown_drain_timeout = None;
        self
    }

    pub fn with_worker_failure_policy(mut self, policy: WorkerFailurePolicy) -> Self {
        self.worker_failure_policy = policy;
        self
    }

    pub fn with_handler_panic_policy(mut self, policy: HandlerPanicPolicy) -> Self {
        self.handler_panic_policy = policy;
        self
    }

    pub(crate) fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.default_concurrency_limit > 0,
            "NatsAppConfig.default_concurrency_limit must be greater than 0"
        );
        anyhow::ensure!(
            self.max_concurrency_limit > 0,
            "NatsAppConfig.max_concurrency_limit must be greater than 0"
        );
        anyhow::ensure!(
            self.default_concurrency_limit <= self.max_concurrency_limit,
            "NatsAppConfig.default_concurrency_limit cannot exceed max_concurrency_limit"
        );
        Ok(())
    }
}
