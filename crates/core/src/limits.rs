use std::sync::Arc;

use async_trait::async_trait;
use wasmtime::ResourceLimiterAsync;

/// An externally-supplied policy consulted on every memory growth attempt, in addition to
/// the store's static [`StoreLimitsAsync::max_memory_size`] ceiling.
///
/// This lets an embedder deny growth for reasons it alone knows about (e.g. the host process
/// as a whole is under memory pressure), without `StoreLimitsAsync` needing to know anything
/// about those reasons itself.
#[async_trait]
pub trait GrowthLimiter: Send + Sync {
    /// Ask to reserve memory for a potential growth.
    ///
    /// Returns whether the reservation is allowed. `current_total` is the store's total memory
    /// already consumed (summed across all of its memories); `desired_total` is the total that
    /// would result if this particular grow is allowed.
    async fn reserve(&self, current_total: u64, desired_total: u64) -> bool;

    /// Release a previously reserved amount of memory.
    fn release(&self, amount: usize);
}

/// Async implementation of wasmtime's `StoreLimits`: https://github.com/bytecodealliance/wasmtime/blob/main/crates/wasmtime/src/limits.rs
/// Used to limit the memory use and table size of each Instance
#[derive(Default)]
pub struct StoreLimitsAsync {
    max_memory_size: Option<usize>,
    max_table_elements: Option<usize>,
    memory_consumed: u64,
    growth_limiter: Option<Arc<dyn GrowthLimiter>>,
}

#[async_trait]
impl ResourceLimiterAsync for StoreLimitsAsync {
    async fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let within_configured_limit = if let Some(limit) = self.max_memory_size {
            desired <= limit
        } else {
            true
        };
        let current_total = self.memory_consumed;
        let desired_total =
            (current_total as i64 + (desired as i64 - current as i64)) as u64;
        let allowed_by_limiter = match &self.growth_limiter {
            Some(limiter) => limiter.reserve(current_total, desired_total).await,
            None => true,
        };
        let can_grow = within_configured_limit && allowed_by_limiter;
        if can_grow {
            self.memory_consumed = desired_total;
        } else {
            tracing::warn!(
                "error.type" = if within_configured_limit {
                    "growth_limiter_denied"
                } else {
                    "memory_limit_exceeded"
                },
                current,
                desired,
                maximum,
                max_memory_size = self.max_memory_size,
                current_total,
                desired_total,
                "instance memory growth denied",
            );
        }
        Ok(can_grow)
    }

    async fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let can_grow = if let Some(limit) = self.max_table_elements {
            desired <= limit
        } else {
            true
        };
        Ok(can_grow)
    }
}

impl StoreLimitsAsync {
    /// How much memory has been consumed in bytes
    pub fn memory_consumed(&self) -> u64 {
        self.memory_consumed
    }

    /// Sets the maximum memory allocation limit, leaving other settings untouched.
    pub fn set_max_memory_size(&mut self, max_memory_size: usize) {
        self.max_memory_size = Some(max_memory_size);
    }

    /// Registers a [`GrowthLimiter`] that is consulted (in addition to the static
    /// `max_memory_size` ceiling) on every memory growth attempt.
    pub fn set_growth_limiter(&mut self, growth_limiter: Arc<dyn GrowthLimiter>) {
        self.growth_limiter = Some(growth_limiter);
    }
}

impl Drop for StoreLimitsAsync {
    fn drop(&mut self) {
        if let Some(limiter) = &mut self.growth_limiter {
            limiter.release(self.memory_consumed as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_store_limits_memory() {
        let mut limits = StoreLimitsAsync {
            max_memory_size: Some(65536),
            growth_limiter: None,
            ..Default::default()
        };
        assert!(limits.memory_growing(0, 65536, None).await.unwrap());
        assert_eq!(limits.memory_consumed, 65536);
        assert!(!limits.memory_growing(65536, 131072, None).await.unwrap());
        assert_eq!(limits.memory_consumed, 65536);
    }

    #[tokio::test]
    async fn test_store_limits_table() {
        let mut limits = StoreLimitsAsync {
            max_table_elements: Some(10),
            growth_limiter: None,
            ..Default::default()
        };
        assert!(limits.table_growing(9, 10, None).await.unwrap());
        assert!(!limits.table_growing(10, 11, None).await.unwrap());
    }

    struct FlagGrowthLimiter {
        threshold: u64,
        deny: std::sync::atomic::AtomicBool,
    }

    #[async_trait]
    impl GrowthLimiter for FlagGrowthLimiter {
        async fn reserve(&self, current_total: u64, _desired_total: u64) -> bool {
            !(current_total >= self.threshold
                && self.deny.load(std::sync::atomic::Ordering::Relaxed))
        }

        fn release(&self, _amount: usize) {
            // No-op for this simple flag-based limiter
        }
    }

    #[tokio::test]
    async fn test_growth_limiter() {
        let limiter = Arc::new(FlagGrowthLimiter {
            threshold: 100,
            deny: std::sync::atomic::AtomicBool::new(false),
        });
        let mut limits = StoreLimitsAsync::default();
        limits.set_growth_limiter(limiter.clone());

        // Below the threshold: allowed regardless of the deny flag.
        assert!(limits.memory_growing(0, 50, None).await.unwrap());
        assert_eq!(limits.memory_consumed(), 50);

        // Crosses the threshold, but the deny flag isn't set yet: still allowed.
        assert!(limits.memory_growing(50, 150, None).await.unwrap());
        assert_eq!(limits.memory_consumed(), 150);

        // Now flip the flag: further growth while over threshold is denied.
        limiter
            .deny
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(!limits.memory_growing(150, 200, None).await.unwrap());
        assert_eq!(limits.memory_consumed(), 150);

        // Flip it back off: growth is allowed again immediately (no latching).
        limiter
            .deny
            .store(false, std::sync::atomic::Ordering::Relaxed);
        assert!(limits.memory_growing(150, 200, None).await.unwrap());
        assert_eq!(limits.memory_consumed(), 200);
    }

    #[tokio::test]
    async fn test_memory_consumed() {
        let engine = wasmtime::Engine::new(crate::Config::default().wasmtime_config()).unwrap();
        let linker = wasmtime::component::Linker::new(&engine);
        let component = wasmtime::component::Component::new(
            &engine,
            r#"
            (component
                (core module $m (memory 1))
                (core instance $a (instantiate $m))
            )
            "#,
        )
        .unwrap();
        let mut store = wasmtime::Store::new(&engine, StoreLimitsAsync::default());
        store.limiter_async(|s| s);
        let _ = linker
            .instantiate_async(&mut store, &component)
            .await
            .unwrap();
        assert_eq!(store.data().memory_consumed(), 1 << 16);
    }
}
