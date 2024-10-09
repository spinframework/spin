// use anyhow::bail;
// use spin_core::async_trait;
// use spin_factor_blobstore::{BlobStoreFactor, RuntimeConfig, Store, StoreManager};
// use spin_factors::RuntimeFactors;
// use spin_factors_test::{toml, TestEnvironment};
// use spin_world::wasi::blobstore::types::Error;
// use std::{collections::HashSet, sync::Arc};

// #[derive(RuntimeFactors)]
// struct TestFactors {
//     blobstore: BlobStoreFactor,
// }

// impl From<RuntimeConfig> for TestFactorsRuntimeConfig {
//     fn from(value: RuntimeConfig) -> Self {
//         Self {
//             blobstore: Some(value),
//         }
//     }
// }

// #[tokio::test]
// async fn works_when_allowed_store_is_defined() -> anyhow::Result<()> {
//     todo!("this test")
//     // let mut runtime_config = RuntimeConfig::default();
//     // runtime_config.add_store_manager("default".into(), mock_store_manager());
//     // let factors = TestFactors {
//     //     key_value: KeyValueFactor::new(),
//     // };
//     // let env = TestEnvironment::new(factors).extend_manifest(toml! {
//     //     [component.test-component]
//     //     source = "does-not-exist.wasm"
//     //     key_value_stores = ["default"]
//     // });
//     // let mut state = env
//     //     .runtime_config(runtime_config)?
//     //     .build_instance_state()
//     //     .await?;

//     // assert_eq!(
//     //     state.key_value.allowed_stores(),
//     //     &["default".into()].into_iter().collect::<HashSet<_>>()
//     // );

//     // assert!(state.key_value.open("default".to_owned()).await?.is_ok());
//     // Ok(())
// }

// #[tokio::test]
// async fn errors_when_store_is_not_defined() -> anyhow::Result<()> {
//     todo!("this test")
//     // let runtime_config = RuntimeConfig::default();
//     // let factors = TestFactors {
//     //     key_value: KeyValueFactor::new(),
//     // };
//     // let env = TestEnvironment::new(factors).extend_manifest(toml! {
//     //     [component.test-component]
//     //     source = "does-not-exist.wasm"
//     //     key_value_stores = ["default"]
//     // });
//     // let Err(err) = env
//     //     .runtime_config(runtime_config)?
//     //     .build_instance_state()
//     //     .await
//     // else {
//     //     bail!("expected instance build to fail but it didn't");
//     // };

//     // assert!(err
//     //     .to_string()
//     //     .contains(r#"unknown key_value_stores label "default""#));

//     // Ok(())
// }

// #[tokio::test]
// async fn errors_when_store_is_not_allowed() -> anyhow::Result<()> {
//     todo!("this test")
//     // let mut runtime_config = RuntimeConfig::default();
//     // runtime_config.add_store_manager("default".into(), mock_store_manager());
//     // let factors = TestFactors {
//     //     key_value: KeyValueFactor::new(),
//     // };
//     // let env = TestEnvironment::new(factors).extend_manifest(toml! {
//     //     [component.test-component]
//     //     source = "does-not-exist.wasm"
//     //     key_value_stores = []
//     // });
//     // let mut state = env
//     //     .runtime_config(runtime_config)?
//     //     .build_instance_state()
//     //     .await?;

//     // assert_eq!(state.key_value.allowed_stores(), &HashSet::new());

//     // assert!(matches!(
//     //     state.key_value.open("default".to_owned()).await?,
//     //     Err(Error::AccessDenied)
//     // ));

//     // Ok(())
// }

// fn mock_store_manager() -> Arc<dyn StoreManager> {
//     Arc::new(MockStoreManager)
// }

// struct MockStoreManager;

// #[async_trait]
// impl StoreManager for MockStoreManager {
//     async fn get(&self, name: &str) -> Result<Arc<dyn Store>, Error> {
//         let _ = name;
//         Ok(Arc::new(MockStore))
//     }

//     fn is_defined(&self, store_name: &str) -> bool {
//         let _ = store_name;
//         todo!()
//     }
// }

// struct MockStore;

// #[async_trait]
// impl Store for MockStore {
//     async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, Error> {
//         let _ = key;
//         todo!()
//     }
//     async fn set(&self, key: &str, value: &[u8]) -> Result<(), Error> {
//         let _ = (key, value);
//         todo!()
//     }
//     async fn delete(&self, key: &str) -> Result<(), Error> {
//         let _ = key;
//         todo!()
//     }
//     async fn exists(&self, key: &str) -> Result<bool, Error> {
//         let _ = key;
//         todo!()
//     }
//     async fn get_keys(&self) -> Result<Vec<String>, Error> {
//         todo!()
//     }
// }
