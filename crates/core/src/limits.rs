use anyhow::Result;
use async_trait::async_trait;
use wasmtime::ResourceLimiterAsync;

/// Async implementation of wasmtime's `StoreLimits`: https://github.com/bytecodealliance/wasmtime/blob/main/crates/wasmtime/src/limits.rs
/// Used to limit the memory use and table size of each Instance
#[derive(Default)]
pub struct StoreLimitsAsync {
    max_memory_size: Option<usize>,
    max_table_elements: Option<usize>,
    memory_consumed: u64,
}

#[async_trait]
impl ResourceLimiterAsync for StoreLimitsAsync {
    async fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool> {
        let can_grow = if let Some(limit) = self.max_memory_size {
            desired <= limit
        } else {
            true
        };
        if can_grow {
            self.memory_consumed =
                (self.memory_consumed as i64 + (desired as i64 - current as i64)) as u64;
        } else {
            tracing::warn!(
                "error.type" = "memory_limit_exceeded",
                current,
                desired,
                maximum,
                max_memory_size = self.max_memory_size,
                "instance memory limit exceeded",
            );
        }
        Ok(can_grow)
    }

    async fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool> {
        let can_grow = if let Some(limit) = self.max_table_elements {
            desired <= limit
        } else {
            true
        };
        Ok(can_grow)
    }
}

impl StoreLimitsAsync {
    pub fn new(max_memory_size: Option<usize>, max_table_elements: Option<usize>) -> Self {
        Self {
            max_memory_size,
            max_table_elements,
            memory_consumed: 0,
        }
    }

    /// How much memory has been consumed in bytes
    pub fn memory_consumed(&self) -> u64 {
        self.memory_consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_store_limits_memory() {
        let mut limits = StoreLimitsAsync {
            max_memory_size: Some(65536),
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
            ..Default::default()
        };
        assert!(limits.table_growing(9, 10, None).await.unwrap());
        assert!(!limits.table_growing(10, 11, None).await.unwrap());
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
