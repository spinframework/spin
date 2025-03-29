//! Spin core execution engine
//!
//! This crate provides low-level Wasm functionality required by Spin. Most of
//! this functionality consists of wrappers around [`wasmtime`] that narrow the
//! flexibility of `wasmtime` to the set of features used by Spin (such as only
//! supporting `wasmtime`'s async calling style).

#![deny(missing_docs)]

mod limits;
mod store;

use std::sync::OnceLock;
use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use tracing::instrument;
use wasmtime::{InstanceAllocationStrategy, PoolingAllocationConfig};

pub use async_trait::async_trait;
pub use wasmtime::Engine as WasmtimeEngine;
pub use wasmtime::{
    self,
    component::{Component, Instance, InstancePre, Linker},
    Instance as ModuleInstance, Module, Trap,
};

pub use store::{AsState, Store, StoreBuilder};

/// The default [`EngineBuilder::epoch_tick_interval`].
pub const DEFAULT_EPOCH_TICK_INTERVAL: Duration = Duration::from_millis(10);

const MB: u64 = 1 << 20;
const GB: usize = 1 << 30;

/// Global configuration for `EngineBuilder`.
///
/// This is currently only used for advanced (undocumented) use cases.
pub struct Config {
    inner: wasmtime::Config,
}

impl Config {
    /// Borrow the inner wasmtime::Config mutably.
    /// WARNING: This is inherently unstable and may break at any time!
    #[doc(hidden)]
    pub fn wasmtime_config(&mut self) -> &mut wasmtime::Config {
        &mut self.inner
    }

    /// Enable the Wasmtime compilation cache. If `path` is given it will override
    /// the system default path.
    ///
    /// For more information, see the [Wasmtime cache config documentation][docs].
    ///
    /// [docs]: https://docs.wasmtime.dev/cli-cache.html
    pub fn enable_cache(&mut self, config_path: &Option<PathBuf>) -> Result<()> {
        match config_path {
            Some(p) => self.inner.cache_config_load(p)?,
            None => self.inner.cache_config_load_default()?,
        };

        Ok(())
    }

    /// Disable the pooling instance allocator.
    pub fn disable_pooling(&mut self) -> &mut Self {
        self.inner
            .allocation_strategy(wasmtime::InstanceAllocationStrategy::OnDemand);
        self
    }
}

impl Default for Config {
    fn default() -> Self {
        let mut inner = wasmtime::Config::new();
        inner.async_support(true);
        inner.epoch_interruption(true);
        inner.wasm_component_model(true);
        // If targeting musl, disable native unwind to address this issue:
        // https://github.com/fermyon/spin/issues/2889
        // TODO: remove this when wasmtime is updated to >= v27.0.0
        #[cfg(all(target_os = "linux", target_env = "musl"))]
        inner.native_unwind_info(false);

        if use_pooling_allocator_by_default() {
            // By default enable the pooling instance allocator in Wasmtime. This
            // drastically reduces syscall/kernel overhead for wasm execution,
            // especially in async contexts where async stacks must be allocated.
            // The general goal here is that the default settings here rarely, if
            // ever, need to be modified. As a result there aren't fine-grained
            // knobs for each of these settings just yet and instead they're
            // generally set to defaults. Environment-variable-based fallbacks are
            // supported though as an escape valve for if this is a problem.
            let mut pooling_config = PoolingAllocationConfig::default();
            pooling_config
                .total_component_instances(env("SPIN_WASMTIME_INSTANCE_COUNT", 1_000))
                // This number accounts for internal data structures that Wasmtime allocates for each instance.
                // Instance allocation is proportional to the number of "things" in a wasm module like functions,
                // globals, memories, etc. Instance allocations are relatively small and are largely inconsequential
                // compared to other runtime state, but a number needs to be chosen here so a relatively large threshold
                // of 10MB is arbitrarily chosen. It should be unlikely that any reasonably-sized module hits this limit.
                .max_component_instance_size(env("SPIN_WASMTIME_INSTANCE_SIZE", 10 * MB) as usize)
                .max_core_instance_size(env("SPIN_WASMTIME_CORE_INSTANCE_SIZE", 10 * MB) as usize)
                .max_core_instances_per_component(env("SPIN_WASMTIME_CORE_INSTANCE_COUNT", 200))
                .max_tables_per_component(env("SPIN_WASMTIME_INSTANCE_TABLES", 20))
                .table_elements(env("SPIN_WASMTIME_INSTANCE_TABLE_ELEMENTS", 100_000))
                // The number of memories an instance can have effectively limits the number of inner components
                // a composed component can have (since each inner component has its own memory). We default to 32 for now, and
                // we'll see how often this limit gets reached.
                .max_memories_per_component(env("SPIN_WASMTIME_INSTANCE_MEMORIES", 32))
                .total_memories(env("SPIN_WASMTIME_TOTAL_MEMORIES", 1_000))
                .total_tables(env("SPIN_WASMTIME_TOTAL_TABLES", 2_000))
                // Nothing is lost from allowing the maximum size of memory for
                // all instance as it's still limited through other the normal
                // `StoreLimitsAsync` accounting method too.
                .max_memory_size(4 * GB)
                // These numbers are completely arbitrary at something above 0.
                .linear_memory_keep_resident(env(
                    "SPIN_WASMTIME_LINEAR_MEMORY_KEEP_RESIDENT",
                    2 * MB,
                ) as usize)
                .table_keep_resident(env("SPIN_WASMTIME_TABLE_KEEP_RESIDENT", MB / 2) as usize);
            inner.allocation_strategy(InstanceAllocationStrategy::Pooling(pooling_config));
        }

        return Self { inner };

        fn env<T>(name: &str, default: T) -> T
        where
            T: std::str::FromStr,
            T::Err: std::fmt::Display,
        {
            match std::env::var(name) {
                Ok(val) => val
                    .parse()
                    .unwrap_or_else(|e| panic!("failed to parse env var `{name}={val}`: {e}")),
                Err(_) => default,
            }
        }
    }
}

/// The pooling allocator is tailor made for the `spin up` use case, so
/// try to use it when we can. The main cost of the pooling allocator, however,
/// is the virtual memory required to run it. Not all systems support the same
/// amount of virtual memory, for example some aarch64 and riscv64 configuration
/// only support 39 bits of virtual address space.
///
/// The pooling allocator, by default, will request 1000 linear memories each
/// sized at 6G per linear memory. This is 6T of virtual memory which ends up
/// being about 42 bits of the address space. This exceeds the 39 bit limit of
/// some systems, so there the pooling allocator will fail by default.
///
/// This function attempts to dynamically determine the hint for the pooling
/// allocator. This returns `true` if the pooling allocator should be used
/// by default, or `false` otherwise.
///
/// The method for testing this is to allocate a 0-sized 64-bit linear memory
/// with a maximum size that's N bits large where we force all memories to be
/// static. This should attempt to acquire N bits of the virtual address space.
/// If successful that should mean that the pooling allocator is OK to use, but
/// if it fails then the pooling allocator is not used and the normal mmap-based
/// implementation is used instead.
fn use_pooling_allocator_by_default() -> bool {
    static USE_POOLING: OnceLock<bool> = OnceLock::new();
    const BITS_TO_TEST: u32 = 42;

    *USE_POOLING.get_or_init(|| {
        // Enable manual control through env vars as an escape hatch
        match std::env::var("SPIN_WASMTIME_POOLING") {
            Ok(s) if s == "1" => return true,
            Ok(s) if s == "0" => return false,
            Ok(s) => panic!("SPIN_WASMTIME_POOLING={s} not supported, only 1/0 supported"),
            Err(_) => {}
        }

        // If the env var isn't set then perform the dynamic runtime probe
        let mut config = wasmtime::Config::new();
        config.wasm_memory64(true);
        config.memory_reservation(1 << BITS_TO_TEST);

        match wasmtime::Engine::new(&config) {
            Ok(engine) => {
                let mut store = wasmtime::Store::new(&engine, ());
                // NB: the maximum size is in wasm pages so take out the 16-bits
                // of wasm page size here from the maximum size.
                let ty = wasmtime::MemoryType::new64(0, Some(1 << (BITS_TO_TEST - 16)));
                wasmtime::Memory::new(&mut store, ty).is_ok()
            }
            Err(_) => {
                tracing::debug!(
                    "unable to create an engine to test the pooling \
                     allocator, disabling pooling allocation"
                );
                false
            }
        }
    })
}

/// Host state data associated with individual [Store]s and [Instance]s.
#[derive(Default)]
pub struct State {
    store_limits: limits::StoreLimitsAsync,
}

impl State {
    /// Get the amount of memory in bytes consumed by instances in the store
    pub fn memory_consumed(&self) -> u64 {
        self.store_limits.memory_consumed()
    }
}

/// A builder interface for configuring a new [`Engine`].
///
/// A new [`EngineBuilder`] can be obtained with [`Engine::builder`].
pub struct EngineBuilder<T> {
    engine: wasmtime::Engine,
    linker: Linker<T>,
    epoch_tick_interval: Duration,
    epoch_ticker_thread: bool,
}

impl<T> EngineBuilder<T> {
    fn new(config: &Config) -> Result<Self> {
        let engine = wasmtime::Engine::new(&config.inner)?;
        let linker: Linker<T> = Linker::new(&engine);
        Ok(Self {
            engine,
            linker,
            epoch_tick_interval: DEFAULT_EPOCH_TICK_INTERVAL,
            epoch_ticker_thread: true,
        })
    }

    /// Returns a reference to the [`Linker`] for this [`Engine`].
    pub fn linker(&mut self) -> &mut Linker<T> {
        &mut self.linker
    }

    /// Sets the epoch tick internal for the built [`Engine`].
    ///
    /// This is used by [`Store::set_deadline`] to calculate the number of
    /// "ticks" for epoch interruption, and by the default epoch ticker thread.
    /// The default is [`DEFAULT_EPOCH_TICK_INTERVAL`].
    ///
    /// See [`EngineBuilder::epoch_ticker_thread`] and
    /// [`wasmtime::Config::epoch_interruption`](https://docs.rs/wasmtime/latest/wasmtime/struct.Config.html#method.epoch_interruption).
    pub fn epoch_tick_interval(&mut self, interval: Duration) {
        self.epoch_tick_interval = interval;
    }

    /// Configures whether the epoch ticker thread will be spawned when this
    /// [`Engine`] is built.
    ///
    /// Enabled by default; if disabled, the user must arrange to call
    /// `engine.as_ref().increment_epoch()` every `epoch_tick_interval` or
    /// interrupt-based features like `Store::set_deadline` will not work.
    pub fn epoch_ticker_thread(&mut self, enable: bool) {
        self.epoch_ticker_thread = enable;
    }

    fn maybe_spawn_epoch_ticker(&self) {
        if !self.epoch_ticker_thread {
            return;
        }
        let engine_weak = self.engine.weak();
        let interval = self.epoch_tick_interval;
        std::thread::spawn(move || loop {
            std::thread::sleep(interval);
            let Some(engine) = engine_weak.upgrade() else {
                break;
            };
            engine.increment_epoch();
        });
    }

    /// Builds an [`Engine`] from this builder.
    pub fn build(self) -> Engine<T> {
        self.maybe_spawn_epoch_ticker();
        Engine {
            inner: self.engine,
            linker: self.linker,
            epoch_tick_interval: self.epoch_tick_interval,
        }
    }
}

/// An `Engine` is a global context for the initialization and execution of
/// Spin components.
pub struct Engine<T> {
    inner: wasmtime::Engine,
    linker: Linker<T>,
    epoch_tick_interval: Duration,
}

impl<T> Engine<T> {
    /// Creates a new [`EngineBuilder`] with the given [`Config`].
    pub fn builder(config: &Config) -> Result<EngineBuilder<T>> {
        EngineBuilder::new(config)
    }

    /// Creates a new [`StoreBuilder`].
    pub fn store_builder(&self) -> StoreBuilder {
        StoreBuilder::new(self.inner.clone(), self.epoch_tick_interval)
    }

    /// Creates a new [`InstancePre`] for the given [`Component`].
    #[instrument(skip_all, level = "debug")]
    pub fn instantiate_pre(&self, component: &Component) -> Result<InstancePre<T>> {
        self.linker.instantiate_pre(component)
    }
}

impl<T> AsRef<wasmtime::Engine> for Engine<T> {
    fn as_ref(&self) -> &wasmtime::Engine {
        &self.inner
    }
}
