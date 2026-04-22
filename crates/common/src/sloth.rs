//! Warn on slow operations

use tokio::task::JoinHandle;
use tokio::time::{Duration, sleep};

/// Print a warning message after the given duration unless the returned
/// [`SlothGuard`] is dropped first.
pub fn warn_if_slothful(warn_after_ms: u64, message: impl Into<String>) -> SlothGuard {
    let message = message.into();
    let warning = tokio::spawn(async move {
        sleep(Duration::from_millis(warn_after_ms)).await;
        eprintln!("{message}");
    });
    SlothGuard { warning }
}

/// Returned by [`warn_if_slothful`]; cancels the warning when dropped.
#[must_use]
pub struct SlothGuard {
    warning: JoinHandle<()>,
}

impl Drop for SlothGuard {
    fn drop(&mut self) {
        self.warning.abort()
    }
}
