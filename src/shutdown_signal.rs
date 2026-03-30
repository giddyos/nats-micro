use std::time::Duration;

use tokio::{sync::watch, time::Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ShutdownState {
    pub(crate) requested: bool,
    pub(crate) drain_timeout: Option<Duration>,
    pub(crate) deadline: Option<Instant>,
}

impl ShutdownState {
    pub(crate) fn running(drain_timeout: Option<Duration>) -> Self {
        Self {
            requested: false,
            drain_timeout,
            deadline: None,
        }
    }

    pub(crate) fn requested(drain_timeout: Option<Duration>) -> Self {
        Self {
            requested: true,
            drain_timeout,
            deadline: None,
        }
    }

    pub(crate) fn draining(drain_timeout: Option<Duration>, deadline: Instant) -> Self {
        Self {
            requested: true,
            drain_timeout,
            deadline: Some(deadline),
        }
    }
}

#[derive(Clone)]
pub struct ShutdownSignal {
    state_rx: watch::Receiver<ShutdownState>,
}

impl Default for ShutdownSignal {
    fn default() -> Self {
        let (_, state_rx) = watch::channel(ShutdownState::running(None));
        Self { state_rx }
    }
}

impl ShutdownSignal {
    pub(crate) fn new(state_rx: watch::Receiver<ShutdownState>) -> Self {
        Self { state_rx }
    }

    fn state(&self) -> ShutdownState {
        *self.state_rx.borrow()
    }

    pub fn is_requested(&self) -> bool {
        self.state().requested
    }

    pub fn drain_timeout(&self) -> Option<Duration> {
        self.state().drain_timeout
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.state().deadline
    }

    pub fn remaining(&self) -> Option<Duration> {
        self.deadline()
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
    }

    pub async fn changed(&mut self) -> bool {
        self.state_rx.changed().await.is_ok()
    }

    pub async fn wait_for_shutdown(&mut self) {
        while !self.is_requested() {
            if !self.changed().await {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::{sync::watch, time::Instant};

    use super::{ShutdownSignal, ShutdownState};

    #[tokio::test]
    async fn wait_for_shutdown_observes_requested_state() {
        let drain_timeout = Some(Duration::from_secs(2));
        let (tx, rx) = watch::channel(ShutdownState::running(drain_timeout));
        let mut signal = ShutdownSignal::new(rx);

        assert!(!signal.is_requested());
        assert_eq!(signal.drain_timeout(), drain_timeout);
        assert_eq!(signal.deadline(), None);

        tx.send(ShutdownState::requested(drain_timeout)).unwrap();
        signal.wait_for_shutdown().await;

        assert!(signal.is_requested());
        assert_eq!(signal.drain_timeout(), drain_timeout);
        assert_eq!(signal.deadline(), None);
    }

    #[tokio::test]
    async fn changed_observes_drain_deadline_updates() {
        let drain_timeout = Some(Duration::from_secs(3));
        let (tx, rx) = watch::channel(ShutdownState::requested(drain_timeout));
        let mut signal = ShutdownSignal::new(rx);
        let deadline = Instant::now() + Duration::from_secs(3);

        tx.send(ShutdownState::draining(drain_timeout, deadline))
            .unwrap();

        assert!(signal.changed().await);
        assert_eq!(signal.deadline(), Some(deadline));
        assert_eq!(signal.drain_timeout(), drain_timeout);
        assert!(signal.remaining().is_some());
    }
}