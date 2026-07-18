use std::time::Duration;

use super::{HandlerPanicPolicy, WorkerFailurePolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Development,
    Test,
    Production,
    MaximumThroughput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TelemetryConfig {
    pub json_logs: bool,
    pub opentelemetry: bool,
    pub verbose_startup: bool,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub default_concurrency: usize,
    pub max_concurrency: usize,
    pub shutdown_drain_timeout: Duration,
    pub startup_timeout: Duration,
    pub worker_failure: WorkerFailurePolicy,
    pub handler_panic: HandlerPanicPolicy,
    pub generate_missing_request_ids: bool,
    pub telemetry: TelemetryConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self::for_profile(Profile::Development)
    }
}

impl AppConfig {
    #[must_use]
    pub fn for_profile(profile: Profile) -> Self {
        match profile {
            Profile::Development => Self {
                default_concurrency: 64,
                max_concurrency: 1_024,
                shutdown_drain_timeout: Duration::from_secs(10),
                startup_timeout: Duration::from_secs(15),
                worker_failure: WorkerFailurePolicy::ShutdownApp,
                handler_panic: HandlerPanicPolicy::LogAndContinue,
                generate_missing_request_ids: true,
                telemetry: TelemetryConfig {
                    verbose_startup: true,
                    ..TelemetryConfig::default()
                },
            },
            Profile::Test => Self {
                default_concurrency: 4,
                max_concurrency: 32,
                shutdown_drain_timeout: Duration::from_secs(2),
                startup_timeout: Duration::from_secs(5),
                worker_failure: WorkerFailurePolicy::ShutdownApp,
                handler_panic: HandlerPanicPolicy::FailWorker,
                generate_missing_request_ids: false,
                telemetry: TelemetryConfig::default(),
            },
            Profile::Production => Self {
                default_concurrency: 256,
                max_concurrency: 4_096,
                shutdown_drain_timeout: Duration::from_secs(30),
                startup_timeout: Duration::from_secs(30),
                worker_failure: WorkerFailurePolicy::Restart {
                    initial_backoff: Duration::from_millis(100),
                    max_backoff: Duration::from_secs(5),
                    max_restarts: 8,
                    window: Duration::from_mins(1),
                },
                handler_panic: HandlerPanicPolicy::LogAndContinue,
                generate_missing_request_ids: false,
                telemetry: TelemetryConfig {
                    json_logs: true,
                    opentelemetry: true,
                    verbose_startup: false,
                },
            },
            Profile::MaximumThroughput => Self {
                default_concurrency: 512,
                max_concurrency: 8_192,
                shutdown_drain_timeout: Duration::from_secs(15),
                startup_timeout: Duration::from_secs(30),
                worker_failure: WorkerFailurePolicy::ShutdownApp,
                handler_panic: HandlerPanicPolicy::FailWorker,
                generate_missing_request_ids: false,
                telemetry: TelemetryConfig::default(),
            },
        }
    }

    pub fn apply_profile(&mut self, profile: Profile) {
        *self = Self::for_profile(profile);
    }

    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.default_concurrency > 0,
            "default concurrency must be greater than zero"
        );
        anyhow::ensure!(
            self.max_concurrency >= self.default_concurrency,
            "maximum concurrency cannot be lower than default concurrency"
        );
        anyhow::ensure!(
            !self.startup_timeout.is_zero(),
            "startup timeout must be greater than zero"
        );
        anyhow::ensure!(
            !self.shutdown_drain_timeout.is_zero(),
            "shutdown drain timeout must be greater than zero"
        );
        if let WorkerFailurePolicy::Restart {
            initial_backoff,
            max_backoff,
            max_restarts,
            window,
        } = self.worker_failure
        {
            anyhow::ensure!(
                !initial_backoff.is_zero(),
                "restart initial backoff must be greater than zero"
            );
            anyhow::ensure!(
                max_backoff >= initial_backoff,
                "restart maximum backoff cannot be lower than the initial backoff"
            );
            anyhow::ensure!(
                max_restarts > 0,
                "restart maximum attempts must be greater than zero"
            );
            anyhow::ensure!(
                !window.is_zero(),
                "restart window must be greater than zero"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    pub server: String,
    pub options: Option<crate::ConnectOptions>,
}

impl ConnectionConfig {
    #[must_use]
    pub fn new(server: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            options: None,
        }
    }

    #[must_use]
    pub fn options(mut self, options: crate::ConnectOptions) -> Self {
        self.options = Some(options);
        self
    }

    #[must_use]
    pub fn from_env() -> Self {
        Self::new(std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned()))
    }
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{AppConfig, Profile};
    use crate::WorkerFailurePolicy;

    #[test]
    fn restart_configuration_is_validated_before_connecting() {
        let mut config = AppConfig::for_profile(Profile::Production);
        config.worker_failure = WorkerFailurePolicy::Restart {
            initial_backoff: Duration::from_secs(2),
            max_backoff: Duration::from_secs(1),
            max_restarts: 1,
            window: Duration::from_secs(30),
        };

        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("maximum backoff")
        );
    }
}
