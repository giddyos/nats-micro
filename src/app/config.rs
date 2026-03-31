use std::time::Duration;

use anyhow::Result;

use super::limits::DEFAULT_CONCURRENCY_LIMIT;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkerFailurePolicy {
    Ignore,
    #[default]
    ShutdownApp,
}

#[derive(Debug, Clone)]
pub struct NatsAppConfig {
    default_concurrency_limit: u64,
    shutdown_drain_timeout: Option<Duration>,
    worker_failure_policy: WorkerFailurePolicy,
}

impl Default for NatsAppConfig {
    fn default() -> Self {
        Self {
            default_concurrency_limit: DEFAULT_CONCURRENCY_LIMIT,
            shutdown_drain_timeout: None,
            worker_failure_policy: WorkerFailurePolicy::ShutdownApp,
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

    pub fn shutdown_drain_timeout(&self) -> Option<Duration> {
        self.shutdown_drain_timeout
    }

    pub fn worker_failure_policy(&self) -> WorkerFailurePolicy {
        self.worker_failure_policy
    }

    pub fn with_default_concurrency_limit(mut self, limit: u64) -> Self {
        self.default_concurrency_limit = limit;
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

    pub(crate) fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.default_concurrency_limit > 0,
            "NatsAppConfig.default_concurrency_limit must be greater than zero"
        );
        Ok(())
    }
}
