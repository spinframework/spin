use anyhow::Result;
use std::time::{Duration, Instant};

use crate::{limits::StoreLimitsAsync, State, WasmtimeEngine};

#[cfg(doc)]
use crate::EngineBuilder;

/// A `Store` holds the runtime state of a Spin instance.
///
/// In general, a `Store` is expected to live only for the lifetime of a single
/// Spin trigger invocation.
///
/// A `Store` can be built with a [`StoreBuilder`].
pub struct Store<T: 'static> {
    inner: wasmtime::Store<T>,
    epoch_tick_interval: Duration,
}

impl<T: 'static> Store<T> {
    /// Sets the execution deadline.
    ///
    /// This is a rough deadline; an instance will trap some time after this
    /// deadline, determined by [`EngineBuilder::epoch_tick_interval`] and
    /// details of the system's thread scheduler.
    ///
    /// See [`wasmtime::Store::set_epoch_deadline`](https://docs.rs/wasmtime/latest/wasmtime/struct.Store.html#method.set_epoch_deadline).
    pub fn set_deadline(&mut self, deadline: Instant) {
        let now = Instant::now();
        let duration = deadline - now;
        let ticks = if duration.is_zero() {
            tracing::warn!("Execution deadline set in past: {deadline:?} < {now:?}");
            0
        } else {
            let ticks = duration.as_micros() / self.epoch_tick_interval.as_micros();
            let ticks = ticks.min(u64::MAX as u128) as u64;
            ticks + 1 // Add one to allow for current partially-completed tick
        };
        self.inner.set_epoch_deadline(ticks);
    }

    /// Provides access to the inner [`wasmtime::Store`]'s data.
    pub fn data(&self) -> &T {
        self.inner.data()
    }

    /// Provides access to the inner [`wasmtime::Store`]'s data.
    pub fn data_mut(&mut self) -> &mut T {
        self.inner.data_mut()
    }
}

impl<T: 'static> AsRef<wasmtime::Store<T>> for Store<T> {
    fn as_ref(&self) -> &wasmtime::Store<T> {
        &self.inner
    }
}

impl<T: 'static> AsMut<wasmtime::Store<T>> for Store<T> {
    fn as_mut(&mut self) -> &mut wasmtime::Store<T> {
        &mut self.inner
    }
}

impl<T: 'static> wasmtime::AsContext for Store<T> {
    type Data = T;

    fn as_context(&self) -> wasmtime::StoreContext<'_, Self::Data> {
        self.inner.as_context()
    }
}

impl<T: 'static> wasmtime::AsContextMut for Store<T> {
    fn as_context_mut(&mut self) -> wasmtime::StoreContextMut<'_, Self::Data> {
        self.inner.as_context_mut()
    }
}

/// A builder interface for configuring a new [`Store`].
///
/// A new [`StoreBuilder`] can be obtained with [`crate::Engine::store_builder`].
pub struct StoreBuilder {
    engine: WasmtimeEngine,
    epoch_tick_interval: Duration,
    store_limits: StoreLimitsAsync,
}

impl StoreBuilder {
    // Called by Engine::store_builder.
    pub(crate) fn new(engine: WasmtimeEngine, epoch_tick_interval: Duration) -> Self {
        Self {
            engine,
            epoch_tick_interval,
            store_limits: StoreLimitsAsync::default(),
        }
    }

    /// Sets a maximum memory allocation limit.
    ///
    /// See [`wasmtime::ResourceLimiter::memory_growing`] (`maximum`) for
    /// details on how this limit is enforced.
    pub fn max_memory_size(&mut self, max_memory_size: usize) {
        self.store_limits = StoreLimitsAsync::new(Some(max_memory_size), None);
    }

    /// Builds a [`Store`] from this builder with given host state data.
    ///
    /// The `T` parameter must provide access to a [`State`] via `impl
    /// AsMut<State>`.
    pub fn build<T: AsState>(self, mut data: T) -> Result<Store<T>> {
        data.as_state().store_limits = self.store_limits;

        let mut inner = wasmtime::Store::new(&self.engine, data);
        inner.limiter_async(|data| &mut data.as_state().store_limits);

        // With epoch interruption enabled, there must be _some_ deadline set
        // or execution will trap immediately. Since this is a delta, we need
        // to avoid overflow so we'll use 2^63 which is still "practically
        // forever" for any plausible tick interval.
        inner.set_epoch_deadline(u64::MAX / 2);

        Ok(Store {
            inner,
            epoch_tick_interval: self.epoch_tick_interval,
        })
    }
}

/// For consumers that need to use a type other than [`State`] as the [`Store`]
/// `data`, this trait must be implemented for that type.
pub trait AsState {
    /// Gives access to the inner [`State`].
    fn as_state(&mut self) -> &mut State;
}

impl AsState for State {
    fn as_state(&mut self) -> &mut State {
        self
    }
}
