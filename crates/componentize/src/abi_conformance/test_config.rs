use super::{config, Context, TestConfig};
use anyhow::{ensure, Result};
use std::collections::HashMap;
use wasmtime::{component::InstancePre, Engine};

#[derive(Default)]
pub(super) struct Config {
    map: HashMap<String, String>,
}

impl config::Host for Config {
    async fn get_config(&mut self, key: String) -> wasmtime::Result<Result<String, config::Error>> {
        Ok(self
            .map
            .remove(&key)
            .ok_or_else(|| config::Error::InvalidKey(key.to_owned())))
    }
}

pub(crate) async fn test(
    engine: &Engine,
    test_config: TestConfig,
    pre: &InstancePre<Context>,
) -> Result<(), String> {
    let mut store = super::create_store_with_context(engine, test_config, |context| {
        context.config.map.insert("foo".into(), "bar".into());
    });

    super::run_command(&mut store, pre, &["config", "foo"], |store| {
        ensure!(
            store.data().config.map.is_empty(),
            "expected module to call `spin-config::get-config` exactly once"
        );

        Ok(())
    })
    .await
}
