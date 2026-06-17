use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::anyhow;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};
use tokio::time;

/// A semaphore paired with its configured permit limit, so utilization can be computed.
///
/// The two are bound together because a semaphore's `available_permits()` is meaningless for
/// utilization without knowing the limit it was created with. Construct this where the semaphore
/// is created so the pair can never drift.
#[derive(Clone, Debug)]
pub struct LimitedSemaphore {
    sem: Arc<Semaphore>,
    limit: usize,
}

impl LimitedSemaphore {
    /// Creates a semaphore with `limit` permits, paired with that limit. Constructing the
    /// semaphore here (rather than accepting a pre-built one) guarantees the stored limit always
    /// matches the semaphore's actual capacity.
    pub fn new(limit: usize) -> Self {
        Self {
            sem: Arc::new(Semaphore::new(limit)),
            limit,
        }
    }

    /// The configured permit limit this semaphore was created with.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Returns a handle to the underlying semaphore so tests can inspect `available_permits()`
    /// or acquire permits out-of-band to drive contention.
    #[cfg(test)]
    pub(crate) fn semaphore(&self) -> Arc<Semaphore> {
        self.sem.clone()
    }
}

/// Wraps an optional global and an optional factor-specific semaphore.
#[derive(Clone, Debug)]
pub struct ConnectionSemaphore {
    /// Optional semaphore shared across factors.
    /// When configured, this limits the total number of concurrent connections across all factors that
    /// share this global instance.
    global: Option<LimitedSemaphore>,
    /// Optional semaphore specific to this factor.
    ///
    /// When configured, this limits the number of concurrent connections of this specific factor,
    /// independent of the global limit.
    factor_specific: Option<LimitedSemaphore>,
    /// Label for this factor, used in emitted telemetry to differentiate factors sharing a global pool.
    factor: &'static str,
    /// Optional duration to wait for a permit before giving up and returning an error.
    ///
    /// When `None`, `acquire()` will wait indefinitely until a permit is available.
    wait_timeout: Option<Duration>,
    /// Identifier of the app this semaphore is scoped to.
    ///
    /// Used as a structured field on rejection tracing events so operators can attribute breaches to a tenant
    /// without putting `app_id` on any metric label (which would explode cardinality).
    app_id: Arc<str>,
    /// Edge-trigger guard for the rejection warning.
    ///
    /// Set to `true` once a rejection has been logged, and reset to `false` on the next successful acquire.
    rejecting: Arc<AtomicBool>,
}

impl ConnectionSemaphore {
    /// Creates a new `ConnectionSemaphore`.
    ///
    /// `global` is an optional [`LimitedSemaphore`] shared across factors; `factor_specific` is an
    /// optional [`LimitedSemaphore`] for this specific factor. If either is `None`, that level of
    /// limiting is disabled. `factor` is a label used in emitted telemetry, `app_id` identifies
    /// the owning app for tenant-attribution in tracing events, and `wait_timeout` is an optional
    /// duration to wait for a permit before giving up and returning an error.
    pub fn new(
        global: Option<LimitedSemaphore>,
        factor_specific: Option<LimitedSemaphore>,
        factor: &'static str,
        app_id: Arc<str>,
        wait_timeout: Option<Duration>,
    ) -> Self {
        Self {
            global,
            factor_specific,
            factor,
            wait_timeout,
            app_id,
            rejecting: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Records that the semaphore just served a successful acquire, re-arming the rejection
    /// warning so the next rejection is logged again.
    fn mark_serving(&self) {
        self.rejecting.store(false, Ordering::Relaxed);
    }

    /// Acquire both configured semaphore slots, returning a permit that holds
    /// them until dropped.
    ///
    /// When both a global and a factor-specific semaphore are configured, this
    /// method acquires factor-specific first, then global, ensuring the global
    /// permit is never held while blocking on a factor-specific backlog.
    ///
    /// If `wait_timeout` is configured and the permits cannot be acquired within
    /// that duration, an error is returned.
    pub async fn acquire(&self) -> anyhow::Result<ConnectionPermit> {
        // Fast path: all required permits are already available
        if let Ok(permit) = self.try_acquire_permits() {
            spin_telemetry::monotonic_counter!(
                outbound_connection_permits_acquired = 1,
                kind = self.factor,
                waited = false
            );
            self.mark_serving();
            self.emit_utilization();
            return Ok(permit);
        }

        match self.wait_timeout {
            Some(timeout) => time::timeout(timeout, self.acquire_inner())
                .await
                .map_err(|_| {
                    // Log a warning on the first rejection to make it easier for operators
                    // to notice when limits are being hit, but avoid spamming.
                    if !self.rejecting.swap(true, Ordering::Relaxed) {
                        tracing::warn!(
                            kind = self.factor,
                            app_id = %self.app_id,
                            "connection permit rejected: timeout waiting for permit"
                        );
                    }
                    anyhow!("connection semaphore timed out after {timeout:?}")
                })?,
            None => self.acquire_inner().await,
        }
    }

    /// Inner logic for [`Self::acquire`], separated so the caller can apply a timeout.
    async fn acquire_inner(&self) -> anyhow::Result<ConnectionPermit> {
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

        // Acquire factor-specific first, then global. This ensures we never hold
        // the global permit while blocking on factor-specific backlog.
        let factor_specific = match &self.factor_specific {
            Some(f) => Some(acquire_one(&f.sem, &mut waited, "factor").await?),
            None => None,
        };
        // It's fine to hold the factor-specific permit while waiting for the global slot, since
        // other consumers of the factor-specific would also end up waiting for the same global slot.
        let global = match &self.global {
            Some(g) => Some(acquire_one(&g.sem, &mut waited, "global").await?),
            None => None,
        };

        let factor = self.factor;
        if waited {
            spin_telemetry::histogram!(
                outbound_connection_permit_wait_duration_ms = start.elapsed().as_millis() as f64,
                kind = factor
            );
        }
        spin_telemetry::monotonic_counter!(
            outbound_connection_permits_acquired = 1,
            kind = factor,
            waited = waited
        );
        self.mark_serving();
        self.emit_utilization();

        Ok(ConnectionPermit {
            global_permit: global,
            factor_specific_permit: factor_specific,
            semaphore: self.clone(),
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
                    kind = self.factor,
                    waited = false
                );
                self.mark_serving();
                self.emit_utilization();
                Some(permit)
            }
            Err(limit) => {
                spin_telemetry::monotonic_counter!(
                    outbound_connection_permits_rejected = 1,
                    kind = self.factor,
                    limit = limit
                );
                // Log a warning on the first rejection to make it easier for operators
                // to notice when limits are being hit, but avoid spamming.
                if !self.rejecting.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        kind = self.factor,
                        app_id = %self.app_id,
                        limit = limit,
                        "connection permit rejected: limit exhausted"
                    );
                }
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
            Some(s) => match s.sem.clone().try_acquire_owned() {
                Ok(p) => Some(p),
                Err(_) => return Err("global"),
            },
            None => None,
        };
        // Now attempt the factor-specific permit.
        // On failure, `global` is dropped here, releasing the global slot.
        let factor_specific = match &self.factor_specific {
            Some(s) => match s.sem.clone().try_acquire_owned() {
                Ok(p) => Some(p),
                Err(_) => return Err("factor"),
            },
            None => None,
        };
        Ok(ConnectionPermit {
            global_permit: global,
            factor_specific_permit: factor_specific,
            semaphore: self.clone(),
        })
    }

    /// Emits one sample each of factor-specific and global utilization (0.0..=1.0) as histograms,
    /// for whichever limits are configured.
    ///
    /// Factor-specific utilization is labeled with `kind` (one series per factor). Global
    /// utilization carries no `kind` label: the global pool is shared across factors, so its
    /// utilization is a single value — labeling by factor would emit redundant series all
    /// reporting the same number.
    fn emit_utilization(&self) {
        if let Some(util) = utilization(self.factor_specific.as_ref()) {
            spin_telemetry::histogram!(
                outbound_connection_factor_utilization = util,
                kind = self.factor
            );
        }
        if let Some(util) = utilization(self.global.as_ref()) {
            spin_telemetry::histogram!(outbound_connection_global_utilization = util);
        }
    }
}

/// Custom histogram bucket boundaries for the utilization metrics this crate emits.
///
/// Both `outbound_connection_factor_utilization` and `outbound_connection_global_utilization` are
/// recorded on a 0.0..=1.0 scale, so the OTel default boundaries (tuned for millisecond durations,
/// topping out at 10000) would collapse every sample into the lowest bucket. These boundaries sit
/// near typical alerting cutoffs (75%, 90%, 95%, 99%). Pass the result to `spin_telemetry::init`.
///
/// The metric names here must match the identifiers used in the `histogram!` calls above.
pub fn metric_histogram_buckets() -> Vec<spin_telemetry::HistogramBuckets> {
    let boundaries = vec![0.25, 0.5, 0.75, 0.9, 0.95, 0.99];
    [
        "outbound_connection_factor_utilization",
        "outbound_connection_global_utilization",
    ]
    .into_iter()
    .map(|metric_name| spin_telemetry::HistogramBuckets {
        metric_name,
        boundaries: boundaries.clone(),
    })
    .collect()
}

/// Computes utilization (0.0..=1.0) for a limited semaphore, or `None` when no limit is
/// configured (or the limit is zero, which would make utilization undefined).
fn utilization(limited: Option<&LimitedSemaphore>) -> Option<f64> {
    let limited = limited?;
    if limited.limit == 0 {
        return None;
    }
    let in_flight = limited
        .limit
        .saturating_sub(limited.sem.available_permits());
    Some(in_flight as f64 / limited.limit as f64)
}

/// Holds up to two semaphore permits (global + factor-specific).
/// Both permits are released when this value is dropped.
/// All-`None` permit fields are valid and represent the no-limits case.
#[derive(Debug)]
pub struct ConnectionPermit {
    global_permit: Option<OwnedSemaphorePermit>,
    factor_specific_permit: Option<OwnedSemaphorePermit>,
    /// The issuing semaphore, retained so `Drop` can re-sample utilization *after* the inner
    /// permits are released (its `LimitedSemaphore`s share the same `Arc`s these permits came from).
    semaphore: ConnectionSemaphore,
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        // Explicitly release the inner permits before sampling so the histogram
        // reflects post-release state. Without this, the implicit field drop would
        // happen after our body — meaning we'd read the *pre-release* available count.
        self.global_permit.take();
        self.factor_specific_permit.take();
        self.semaphore.emit_utilization();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn no_limits_acquire_always_succeeds() {
        let sem = ConnectionSemaphore::new(None, None, "test", Arc::from("test-app"), None);
        let permit = sem.acquire().await.expect("should succeed");
        drop(permit);
        let _permit2 = sem.acquire().await.expect("should succeed again");
    }

    #[test]
    fn no_limits_try_acquire_always_succeeds() {
        let sem = ConnectionSemaphore::new(None, None, "test", Arc::from("test-app"), None);
        let permit = sem.try_acquire().expect("should succeed");
        drop(permit);
        let _permit2 = sem.try_acquire().expect("should succeed again");
    }

    #[test]
    fn global_limit_only_exhausted() {
        let global = LimitedSemaphore::new(1);
        let global_sem = global.semaphore();
        let sem = ConnectionSemaphore::new(Some(global), None, "test", Arc::from("test-app"), None);
        let permit1 = sem.try_acquire().expect("first should succeed");
        assert!(
            sem.try_acquire().is_none(),
            "second should fail: global exhausted"
        );
        drop(permit1);
        assert_eq!(global_sem.available_permits(), 1);
        let _permit3 = sem.try_acquire().expect("after release should succeed");
    }

    #[test]
    fn factor_limit_only_exhausted() {
        let sem = ConnectionSemaphore::new(
            None,
            Some(LimitedSemaphore::new(1)),
            "test",
            Arc::from("test-app"),
            None,
        );
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
        let global = LimitedSemaphore::new(1);
        let factor = LimitedSemaphore::new(2);
        let factor_sem = factor.semaphore();
        let sem = ConnectionSemaphore::new(
            Some(global),
            Some(factor),
            "test",
            Arc::from("test-app"),
            None,
        );

        let permit1 = sem.try_acquire().expect("first should succeed");
        // After permit1: global=0, factor=1
        let factor_before = factor_sem.available_permits();

        // Second try_acquire should fail because global is exhausted.
        assert!(sem.try_acquire().is_none(), "should fail: global exhausted");
        // Factor must NOT have been consumed by the failed attempt.
        assert_eq!(
            factor_sem.available_permits(),
            factor_before,
            "factor permits should not be consumed when global is exhausted"
        );
        drop(permit1);
    }

    #[test]
    fn both_limits_factor_exhausted_global_released() {
        let global = LimitedSemaphore::new(2);
        let factor = LimitedSemaphore::new(1);
        let global_sem = global.semaphore();
        let sem = ConnectionSemaphore::new(
            Some(global),
            Some(factor),
            "test",
            Arc::from("test-app"),
            None,
        );

        let permit1 = sem.try_acquire().expect("first should succeed");
        // Global still has 1, factor exhausted
        let result = sem.try_acquire();
        assert!(result.is_none(), "should fail: factor exhausted");
        // Global slot must have been released (back to 1)
        assert_eq!(global_sem.available_permits(), 1);
        drop(permit1);
        assert_eq!(global_sem.available_permits(), 2);
    }

    #[tokio::test]
    async fn acquire_waits_for_release() {
        let sem = ConnectionSemaphore::new(
            Some(LimitedSemaphore::new(1)),
            None,
            "test",
            Arc::from("test-app"),
            None,
        );

        let permit = sem.try_acquire().expect("first should succeed");

        let sem2 = sem.clone();
        let handle = tokio::spawn(async move {
            let _p = sem2.acquire().await.expect("should eventually acquire");
        });

        drop(permit); // release so the spawned task can proceed
        handle.await.expect("task should complete");
    }

    /// Verifies that when factor-specific is exhausted, acquire() doesn't hold
    /// a global permit while waiting — so other connection types aren't blocked.
    #[tokio::test]
    async fn acquire_releases_global_while_waiting_for_factor() {
        let global = LimitedSemaphore::new(1);
        let factor = LimitedSemaphore::new(1);
        let global_sem = global.semaphore();
        let factor_sem = factor.semaphore();
        let sem = ConnectionSemaphore::new(
            Some(global),
            Some(factor),
            "test",
            Arc::from("test-app"),
            None,
        );

        // Exhaust factor-specific from outside.
        let _factor_hold = factor_sem.acquire_owned().await.unwrap();

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
            global_sem.available_permits(),
            1,
            "global should be free while acquire() waits for factor-specific"
        );

        drop(_factor_hold);
        handle.await.expect("task should complete");
    }

    #[tokio::test]
    async fn acquire_times_out_when_semaphore_exhausted() {
        let sem = ConnectionSemaphore::new(
            Some(LimitedSemaphore::new(1)),
            None,
            "test",
            Arc::from("test-app"),
            Some(Duration::from_millis(10)),
        );

        let _permit = sem.try_acquire().expect("first should succeed");

        let err = sem.acquire().await.expect_err("should time out");
        assert!(
            err.to_string().contains("timed out"),
            "error message should mention timed out: {err}"
        );
    }

    /// Captures the f64 values logged for a given tracing field name, regardless of
    /// surrounding span context. Used to inspect the histogram samples that
    /// `spin_telemetry::histogram!` emits as `tracing::trace!` events.
    mod capture {
        use std::sync::{Arc, Mutex};
        use tracing::field::{Field, Visit};
        use tracing_subscriber::layer::{Context, Layer};

        #[derive(Clone, Default)]
        pub(super) struct CapturedValues(pub Arc<Mutex<Vec<f64>>>);

        impl CapturedValues {
            pub fn snapshot(&self) -> Vec<f64> {
                self.0.lock().unwrap().clone()
            }
        }

        pub(super) struct CaptureLayer {
            pub field_name: &'static str,
            pub sink: CapturedValues,
        }

        impl<S: tracing::Subscriber> Layer<S> for CaptureLayer {
            fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
                let mut v = FieldVisitor {
                    target: self.field_name,
                    found: None,
                };
                event.record(&mut v);
                if let Some(val) = v.found {
                    self.sink.0.lock().unwrap().push(val);
                }
            }
        }

        struct FieldVisitor {
            target: &'static str,
            found: Option<f64>,
        }

        impl Visit for FieldVisitor {
            fn record_f64(&mut self, field: &Field, value: f64) {
                if field.name() == self.target {
                    self.found = Some(value);
                }
            }
            fn record_i64(&mut self, _: &Field, _: i64) {}
            fn record_u64(&mut self, _: &Field, _: u64) {}
            fn record_bool(&mut self, _: &Field, _: bool) {}
            fn record_str(&mut self, _: &Field, _: &str) {}
            fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
        }
    }

    fn with_capture<R>(field_name: &'static str, f: impl FnOnce() -> R) -> (R, Vec<f64>) {
        use tracing_subscriber::layer::SubscriberExt;
        let sink = capture::CapturedValues::default();
        let layer = capture::CaptureLayer {
            field_name,
            sink: sink.clone(),
        };
        let subscriber = tracing_subscriber::registry().with(layer);
        let result = tracing::subscriber::with_default(subscriber, f);
        (result, sink.snapshot())
    }

    const UTIL_FIELD: &str = "histogram.outbound_connection_factor_utilization";
    const GLOBAL_UTIL_FIELD: &str = "histogram.outbound_connection_global_utilization";

    #[test]
    fn utilization_emitted_on_acquire_and_drop_reflects_post_release_state() {
        let (_, samples) = with_capture(UTIL_FIELD, || {
            let sem = ConnectionSemaphore::new(
                None,
                Some(LimitedSemaphore::new(2)),
                "test",
                Arc::from("test-app"),
                None,
            );
            let p1 = sem.try_acquire().expect("first acquire");
            // After acquire: 1/2 in use
            let p2 = sem.try_acquire().expect("second acquire");
            // After acquire: 2/2 in use
            drop(p1);
            // After release: 1/2 in use
            drop(p2);
            // After release: 0/2 in use
        });
        assert_eq!(
            samples,
            vec![0.5, 1.0, 0.5, 0.0],
            "expected acquire/drop transitions at 0.5, 1.0, 0.5, 0.0; got {samples:?}"
        );
    }

    #[test]
    fn no_utilization_emitted_when_factor_limit_is_none() {
        let (_, samples) = with_capture(UTIL_FIELD, || {
            let sem = ConnectionSemaphore::new(None, None, "test", Arc::from("test-app"), None);
            let p = sem.try_acquire().expect("acquire");
            drop(p);
        });
        assert!(
            samples.is_empty(),
            "expected no utilization samples when factor limit unset; got {samples:?}"
        );
    }

    #[test]
    fn global_utilization_emitted_on_acquire_and_drop_reflects_post_release_state() {
        let (_, samples) = with_capture(GLOBAL_UTIL_FIELD, || {
            let sem = ConnectionSemaphore::new(
                Some(LimitedSemaphore::new(2)),
                None,
                "test",
                Arc::from("test-app"),
                None,
            );
            let p1 = sem.try_acquire().expect("first acquire");
            // After acquire: 1/2 in use
            let p2 = sem.try_acquire().expect("second acquire");
            // After acquire: 2/2 in use
            drop(p1);
            // After release: 1/2 in use
            drop(p2);
            // After release: 0/2 in use
        });
        assert_eq!(
            samples,
            vec![0.5, 1.0, 0.5, 0.0],
            "expected acquire/drop transitions at 0.5, 1.0, 0.5, 0.0; got {samples:?}"
        );
    }

    #[test]
    fn no_global_utilization_emitted_when_global_limit_is_none() {
        let (_, samples) = with_capture(GLOBAL_UTIL_FIELD, || {
            let sem = ConnectionSemaphore::new(
                None,
                Some(LimitedSemaphore::new(2)),
                "test",
                Arc::from("test-app"),
                None,
            );
            let p = sem.try_acquire().expect("acquire");
            drop(p);
        });
        assert!(
            samples.is_empty(),
            "expected no global utilization samples when global limit unset; got {samples:?}"
        );
    }

    #[tokio::test]
    async fn utilization_emitted_on_slow_path_acquire() {
        use tracing_subscriber::layer::SubscriberExt;
        let sink = capture::CapturedValues::default();
        let layer = capture::CaptureLayer {
            field_name: UTIL_FIELD,
            sink: sink.clone(),
        };
        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        let factor = LimitedSemaphore::new(1);
        let factor_sem = factor.semaphore();
        let sem = ConnectionSemaphore::new(
            None,
            Some(factor),
            "test",
            Arc::from("test-app"),
            Some(Duration::from_secs(1)),
        );
        let blocker = factor_sem.acquire_owned().await.unwrap();
        let sem_clone = sem.clone();
        let handle = tokio::spawn(async move { sem_clone.acquire().await });
        tokio::task::yield_now().await;
        drop(blocker);
        let permit = handle.await.expect("task").expect("acquire");
        drop(permit);

        let samples = sink.snapshot();
        // Slow-path acquire fills the only slot (1.0), then drop releases it (0.0).
        assert_eq!(
            samples,
            vec![1.0, 0.0],
            "expected slow-path acquire and drop samples; got {samples:?}"
        );
    }
}
