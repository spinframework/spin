use std::sync::Arc;

use anyhow::anyhow;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};

/// Wraps an optional global and an optional factor-specific semaphore.
#[derive(Clone)]
pub struct ConnectionSemaphore {
    global: Option<Arc<Semaphore>>,
    factor_specific: Option<Arc<Semaphore>>,
    factor: &'static str,
}

impl ConnectionSemaphore {
    /// Creates a new `ConnectionSemaphore`.
    pub fn new(
        global: Option<Arc<Semaphore>>,
        factor_specific_limit: Option<usize>,
        factor: &'static str,
    ) -> Self {
        Self {
            global,
            factor_specific: factor_specific_limit.map(|n| Arc::new(Semaphore::new(n))),
            factor,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_raw(
        global: Option<Arc<Semaphore>>,
        factor_specific: Option<Arc<Semaphore>>,
        factor: &'static str,
    ) -> Self {
        Self {
            global,
            factor_specific,
            factor,
        }
    }

    /// Acquire both configured semaphore slots, returning a permit that holds
    /// them until dropped.
    ///
    /// When both a global and a factor-specific semaphore are configured, this
    /// method never holds one permit while blocking on the other, preventing global
    /// permits from being tied up while waiting on a factor-specific backlog.
    pub async fn acquire(&self) -> anyhow::Result<ConnectionPermit> {
        /// Acquires a single permit from `sem`, trying non-blocking first.
        ///
        /// Sets `*waited = true` if a blocking wait was required.
        async fn acquire_one(
            sem: &Arc<Semaphore>,
            waited: &mut bool,
            label: &str,
        ) -> anyhow::Result<OwnedSemaphorePermit> {
            match sem.clone().try_acquire_owned() {
                Ok(p) => Ok(p),
                Err(TryAcquireError::NoPermits) => {
                    *waited = true;
                    sem.clone()
                        .acquire_owned()
                        .await
                        .map_err(|_| anyhow!("{label} connection semaphore closed"))
                }
                Err(_) => Err(anyhow!("{label} connection semaphore closed")),
            }
        }
        let mut waited = false;
        let start = std::time::Instant::now();

        let (global, factor_specific) = match (&self.global, &self.factor_specific) {
            (None, None) => (None, None),
            (Some(g), None) => (Some(acquire_one(g, &mut waited, "global").await?), None),
            (None, Some(f)) => (None, Some(acquire_one(f, &mut waited, "factor").await?)),
            // Loop until we acquire both. We have to be careful to avoid holding one permit while waiting for the other.
            (Some(g), Some(f)) => loop {
                let global = acquire_one(g, &mut waited, "global").await?;
                match f.clone().try_acquire_owned() {
                    Ok(factor) => break (Some(global), Some(factor)),
                    Err(TryAcquireError::NoPermits) => {}
                    Err(_) => anyhow::bail!("factor connection semaphore closed"),
                }
                // Factor specific has no free permits: release global so other connection types aren't blocked,
                // then wait for factor-specific before trying global again.
                drop(global);
                waited = true;
                let factor = acquire_one(f, &mut waited, "factor").await?;
                match g.clone().try_acquire_owned() {
                    Ok(global) => break (Some(global), Some(factor)),
                    Err(TryAcquireError::NoPermits) => {}
                    Err(_) => anyhow::bail!("global connection semaphore closed"),
                }
                // Global has no free permits: release factor specific and retry from the top of the loop.
                drop(factor);
            },
        };

        let factor = self.factor;
        if waited {
            spin_telemetry::histogram!(
                outbound_connection_permit_wait_duration_ms = start.elapsed().as_millis() as f64,
                factor = factor
            );
        }
        spin_telemetry::monotonic_counter!(
            outbound_connection_permits_acquired = 1,
            factor = factor,
            waited = waited
        );

        Ok(ConnectionPermit {
            _global: global,
            _factor_specific: factor_specific,
        })
    }

    /// Attempt to acquire both configured slots without waiting.
    /// Returns `None` if either semaphore is exhausted.
    ///
    /// If the global permit is acquired but the factor-specific permit is not
    /// available, the global permit is released before returning `None`.
    pub fn try_acquire(&self) -> Option<ConnectionPermit> {
        match self.try_acquire_permits() {
            Ok(permit) => {
                spin_telemetry::monotonic_counter!(
                    outbound_connection_permits_acquired = 1,
                    factor = self.factor,
                    waited = false
                );
                Some(permit)
            }
            Err(limit) => {
                spin_telemetry::monotonic_counter!(
                    outbound_connection_permits_rejected = 1,
                    factor = self.factor,
                    limit = limit
                );
                None
            }
        }
    }

    /// Inner logic for [`Self::try_acquire`], separated so the caller can emit
    /// telemetry based on whether a permit was obtained.
    ///
    /// Returns `Err("global")` or `Err("factor")` to indicate which limit was
    /// exhausted, so the caller can tag the rejection metric accordingly.
    fn try_acquire_permits(&self) -> Result<ConnectionPermit, &'static str> {
        // Acquire global first. If it fails, nothing is consumed.
        let global = match &self.global {
            Some(s) => match s.clone().try_acquire_owned() {
                Ok(p) => Some(p),
                Err(_) => return Err("global"),
            },
            None => None,
        };
        // Now attempt the factor-specific permit.
        // On failure, `global` is dropped here, releasing the global slot.
        let factor_specific = match &self.factor_specific {
            Some(s) => match s.clone().try_acquire_owned() {
                Ok(p) => Some(p),
                Err(_) => return Err("factor"),
            },
            None => None,
        };
        Ok(ConnectionPermit {
            _global: global,
            _factor_specific: factor_specific,
        })
    }
}

/// Holds up to two semaphore permits (global + factor-specific).
/// Both permits are released when this value is dropped.
/// All-`None` fields are valid and represent the no-limits case.
///
/// Fields are intentionally prefixed with `_` — they exist solely to be dropped.
pub struct ConnectionPermit {
    _global: Option<OwnedSemaphorePermit>,
    _factor_specific: Option<OwnedSemaphorePermit>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn no_limits_acquire_always_succeeds() {
        let sem = ConnectionSemaphore::new(None, None, "test");
        let permit = sem.acquire().await.expect("should succeed");
        drop(permit);
        let _permit2 = sem.acquire().await.expect("should succeed again");
    }

    #[test]
    fn no_limits_try_acquire_always_succeeds() {
        let sem = ConnectionSemaphore::new(None, None, "test");
        let permit = sem.try_acquire().expect("should succeed");
        drop(permit);
        let _permit2 = sem.try_acquire().expect("should succeed again");
    }

    #[test]
    fn global_limit_only_exhausted() {
        let global = Arc::new(Semaphore::new(1));
        let sem = ConnectionSemaphore::new(Some(global.clone()), None, "test");
        let permit1 = sem.try_acquire().expect("first should succeed");
        assert!(
            sem.try_acquire().is_none(),
            "second should fail: global exhausted"
        );
        drop(permit1);
        assert_eq!(global.available_permits(), 1);
        let _permit3 = sem.try_acquire().expect("after release should succeed");
    }

    #[test]
    fn factor_limit_only_exhausted() {
        let sem = ConnectionSemaphore::new(None, Some(1), "test");
        let permit1 = sem.try_acquire().expect("first should succeed");
        assert!(
            sem.try_acquire().is_none(),
            "second should fail: factor exhausted"
        );
        drop(permit1);
        let _permit3 = sem.try_acquire().expect("after release should succeed");
    }

    #[test]
    fn both_limits_global_exhausted_first() {
        let global = Arc::new(Semaphore::new(1));
        let factor = Arc::new(Semaphore::new(2));
        let sem = ConnectionSemaphore::from_raw(Some(global.clone()), Some(factor.clone()), "test");

        let permit1 = sem.try_acquire().expect("first should succeed");
        // After permit1: global=0, factor=1
        let factor_before = factor.available_permits();

        // Second try_acquire should fail because global is exhausted.
        assert!(sem.try_acquire().is_none(), "should fail: global exhausted");
        // Factor must NOT have been consumed by the failed attempt.
        assert_eq!(
            factor.available_permits(),
            factor_before,
            "factor permits should not be consumed when global is exhausted"
        );
        drop(permit1);
    }

    #[test]
    fn both_limits_factor_exhausted_global_released() {
        let global = Arc::new(Semaphore::new(2));
        let factor = Arc::new(Semaphore::new(1));
        let sem = ConnectionSemaphore::from_raw(Some(global.clone()), Some(factor.clone()), "test");

        let permit1 = sem.try_acquire().expect("first should succeed");
        // Global still has 1, factor exhausted
        let result = sem.try_acquire();
        assert!(result.is_none(), "should fail: factor exhausted");
        // Global slot must have been released (back to 1)
        assert_eq!(global.available_permits(), 1);
        drop(permit1);
        assert_eq!(global.available_permits(), 2);
    }

    #[tokio::test]
    async fn acquire_waits_for_release() {
        let global = Arc::new(Semaphore::new(1));
        let sem = ConnectionSemaphore::new(Some(global.clone()), None, "test");

        let permit = sem.try_acquire().expect("first should succeed");

        let sem2 = sem.clone();
        let handle = tokio::spawn(async move {
            let _p = sem2.acquire().await.expect("should eventually acquire");
        });

        drop(permit); // release so the spawned task can proceed
        handle.await.expect("task should complete");
    }

    /// Verifies that when factor-specific is exhausted, acquire() releases
    /// the global permit while waiting — so other connection types aren't blocked.
    #[tokio::test]
    async fn acquire_releases_global_while_waiting_for_factor() {
        let global = Arc::new(Semaphore::new(1));
        let factor = Arc::new(Semaphore::new(1));
        let sem = ConnectionSemaphore::from_raw(Some(global.clone()), Some(factor.clone()), "test");

        // Exhaust factor-specific from outside.
        let _factor_hold = factor.clone().acquire_owned().await.unwrap();

        let global_clone = global.clone();
        let sem_clone = sem.clone();
        let handle = tokio::spawn(async move {
            sem_clone
                .acquire()
                .await
                .expect("should succeed after factor is released")
        });

        // Yield twice: first to let the spawned task run until it blocks waiting
        // for factor-specific; second to confirm it has released the global permit.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        assert_eq!(
            global_clone.available_permits(),
            1,
            "global should be free while acquire() waits for factor-specific"
        );

        drop(_factor_hold);
        handle.await.expect("task should complete");
    }
}
