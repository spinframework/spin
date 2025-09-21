use spin_factor_wasi::{DummyFilesMounter, WasiFactor};
use spin_factors::{anyhow, RuntimeFactors};
use spin_factors_test::{toml, TestEnvironment};
use wasmtime_wasi::p2::bindings::cli::environment::Host;

#[derive(RuntimeFactors)]
struct TestFactors {
    wasi: WasiFactor,
}

#[tokio::test]
async fn environment_works() -> anyhow::Result<()> {
    let factors = TestFactors {
        wasi: WasiFactor::new(DummyFilesMounter),
    };
    let env = TestEnvironment::new(factors).extend_manifest(toml! {
        [component.test-component]
        source = "does-not-exist.wasm"
        environment = { FOO = "bar" }
    });
    let mut state = env.build_instance_state().await?;
    let mut cli = WasiFactor::get_cli_impl(&mut state).unwrap();

    let val = cli
        .get_environment()?
        .into_iter()
        .find_map(|(key, val)| (key == "FOO").then_some(val));
    assert_eq!(val.as_deref(), Some("bar"));
    Ok(())
}
