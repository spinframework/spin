use super::{
    Context, TestConfig,
    redis_types::{Error, Payload},
};
use anyhow::anyhow;
use wasmtime::{Engine, component::InstancePre};

pub(crate) async fn test(
    engine: &Engine,
    test_config: TestConfig,
    pre: &InstancePre<Context>,
) -> Result<(), String> {
    super::run(async {
        let mut store = super::create_store(engine, test_config);
        let instance = pre.instantiate_async(&mut store).await?;

        let func = instance
            .get_export_index(&mut store, None, "fermyon:spin/inbound-redis")
            .and_then(|i| instance.get_export_index(&mut store, Some(&i), "handle-message"))
            .ok_or_else(|| {
                anyhow!("no fermyon:spin/inbound-redis/handle-message function was found")
            })?;
        let func =
            instance.get_typed_func::<(Payload,), (Result<(), Error>,)>(&mut store, &func)?;

        match func
            .call_async(store, (b"Hello, SpinRedis!".to_vec(),))
            .await?
        {
            (Ok(()) | Err(Error::Success),) => Ok(()),
            (Err(e),) => Err(e.into()),
        }
    })
    .await
}
