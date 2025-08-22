use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::Context;
use serde_json::json;
use spin_app::locked::LockedApp;
use spin_core::{AsState, Component, Config, Engine, State, Store, StoreBuilder, Trap};
use spin_factor_wasi::{DummyFilesMounter, WasiFactor};
use spin_factors::{App, AsInstanceState, RuntimeFactors};
use tokio::{fs, io::AsyncWrite};
use wasmtime_wasi::I32Exit;

#[tokio::test(flavor = "multi_thread")]
async fn test_max_memory_size_obeyed() {
    let max = 10_000_000;
    let alloc = max / 10;
    run_test(
        ["alloc", &format!("{alloc}")],
        |store_builder| {
            store_builder.max_memory_size(max);
        },
        |_| {},
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_max_memory_size_violated() {
    let max = 10_000_000;
    let alloc = max * 2;
    let err = run_test(
        ["alloc", &format!("{alloc}")],
        |store_builder| {
            store_builder.max_memory_size(max);
        },
        |_| {},
    )
    .await
    .unwrap_err();
    let trap = err
        .root_cause() // The error returned is a backtrace. We need the root cause.
        .downcast_ref::<I32Exit>()
        .expect("trap error was not an I32Exit");
    assert_eq!(trap.0, 1);
}

// FIXME: racy timing test
#[tokio::test(flavor = "multi_thread")]
async fn test_set_deadline_obeyed() {
    run_test(
        ["sleep", "20"],
        |_| {},
        |store| {
            store.set_deadline(Instant::now() + Duration::from_millis(10000));
        },
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_set_deadline_violated() {
    let err = run_test(
        ["sleep", "100"],
        |_| {},
        |store| {
            store.set_deadline(Instant::now() + Duration::from_millis(10));
        },
    )
    .await
    .unwrap_err();
    let trap = err.downcast::<Trap>().expect("trap");
    assert_eq!(trap, Trap::Interrupt);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_panic() {
    let err = run_test(["panic"], |_| {}, |_| {}).await.unwrap_err();
    let trap = err.downcast::<Trap>().expect("trap");
    assert_eq!(trap, Trap::UnreachableCodeReached);
}

#[derive(RuntimeFactors)]
struct TestFactors {
    wasi: WasiFactor,
}

struct TestState {
    core: State,
    factors: TestFactorsInstanceState,
}

impl AsState for TestState {
    fn as_state(&mut self) -> &mut State {
        &mut self.core
    }
}

impl AsInstanceState<TestFactorsInstanceState> for TestState {
    fn as_instance_state(&mut self) -> &mut TestFactorsInstanceState {
        &mut self.factors
    }
}

async fn run_test(
    args: impl IntoIterator<Item = &'_ str>,
    update_store_builder: impl FnOnce(&mut StoreBuilder),
    update_store: impl FnOnce(&mut Store<TestState>),
) -> anyhow::Result<()> {
    let mut factors = TestFactors {
        wasi: WasiFactor::new(DummyFilesMounter),
    };

    let mut config = Config::default();
    config
        .wasmtime_config()
        .wasm_backtrace_details(wasmtime::WasmBacktraceDetails::Enable);

    let mut builder = Engine::builder(&config).unwrap();
    factors.init(builder.linker())?;
    let engine = builder.build();

    let mut store_builder = engine.store_builder();
    update_store_builder(&mut store_builder);

    let locked: LockedApp = serde_json::from_value(json!({
        "spin_lock_version": 1,
        "triggers": [],
        "components": [{
            "id": "test-component",
            "source": {
                "content_type": "application/wasm",
                "content": {},
            },
        }]
    }))?;
    let app = App::new("test-app", locked);
    let configured_app = factors.configure_app(app, Default::default())?;
    let mut builders = factors.prepare(&configured_app, "test-component")?;
    builders.wasi().args(args);
    let instance_state = factors.build_instance_state(builders)?;
    let state = TestState {
        core: State::default(),
        factors: instance_state,
    };

    let mut store = store_builder.build(state)?;
    update_store(&mut store);

    let module_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/test-programs/core-wasi-test.wasm");
    let component = spin_componentize::componentize_command(&fs::read(module_path).await?)?;
    let component = Component::new(engine.as_ref(), &component)?;
    let instance_pre = engine.instantiate_pre(&component)?;
    let instance = instance_pre.instantiate_async(&mut store).await?;
    let func = {
        let func = instance
            .get_export_index(&mut store, None, "wasi:cli/run@0.2.0")
            .and_then(|i| instance.get_export_index(&mut store, Some(&i), "run"))
            .context("missing the expected 'wasi:cli/run@0.2.0/run' function")?;
        instance.get_typed_func::<(), (Result<(), ()>,)>(&mut store, &func)?
    };

    func.call_async(&mut store, ())
        .await?
        .0
        .map_err(|()| anyhow::anyhow!("command failed"))
}

// Write with `print!`, required for test output capture
struct TestWriter(tokio::io::Stdout);

impl std::io::Write for TestWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        print!("{}", String::from_utf8_lossy(buf));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl AsyncWrite for TestWriter {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        let this = self.get_mut();
        std::pin::Pin::new(&mut this.0).poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        let this = self.get_mut();
        std::pin::Pin::new(&mut this.0).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        let this = self.get_mut();
        std::pin::Pin::new(&mut this.0).poll_shutdown(cx)
    }
}
