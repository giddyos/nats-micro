#[cfg(feature = "telemetry")]
pub(crate) use tracing::{debug, error, info, warn};

#[cfg(not(feature = "telemetry"))]
macro_rules! debug {
    ($($tokens:tt)*) => {};
}

#[cfg(not(feature = "telemetry"))]
macro_rules! error {
    ($($tokens:tt)*) => {};
}

#[cfg(not(feature = "telemetry"))]
macro_rules! info {
    ($($tokens:tt)*) => {};
}

#[cfg(not(feature = "telemetry"))]
macro_rules! warn_noop {
    ($($tokens:tt)*) => {};
}

#[cfg(not(feature = "telemetry"))]
pub(crate) use {debug, error, info, warn_noop as warn};
