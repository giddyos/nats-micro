use std::time::Duration;

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
    Propagate,
}
