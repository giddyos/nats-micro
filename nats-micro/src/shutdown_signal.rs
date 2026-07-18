use std::time::Duration;

use tokio::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownState {
    requested: bool,
    drain_timeout: Option<Duration>,
    deadline: Option<Instant>,
}

impl ShutdownState {
    #[must_use]
    pub const fn running(drain_timeout: Option<Duration>) -> Self {
        Self {
            requested: false,
            drain_timeout,
            deadline: None,
        }
    }

    #[must_use]
    pub const fn requested(drain_timeout: Option<Duration>) -> Self {
        Self {
            requested: true,
            drain_timeout,
            deadline: None,
        }
    }

    #[must_use]
    pub const fn draining(drain_timeout: Option<Duration>, deadline: Instant) -> Self {
        Self {
            requested: true,
            drain_timeout,
            deadline: Some(deadline),
        }
    }

    #[must_use]
    pub const fn is_requested(self) -> bool {
        self.requested
    }

    #[must_use]
    pub const fn drain_timeout(self) -> Option<Duration> {
        self.drain_timeout
    }

    #[must_use]
    pub const fn deadline(self) -> Option<Instant> {
        self.deadline
    }
}
