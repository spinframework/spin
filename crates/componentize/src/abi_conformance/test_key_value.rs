use super::{
    key_value::{self, Error, Store as KvStore},
    Context, TestConfig,
};
use anyhow::{ensure, Result};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    iter,
};
use wasmtime::{component::InstancePre, Engine};

/// Report of which key-value functions a module successfully used, if any
#[derive(Serialize, PartialEq, Eq, Debug)]
pub struct KeyValueReport {
    pub open: Result<(), String>,
    pub get: Result<(), String>,
    pub set: Result<(), String>,
    pub delete: Result<(), String>,
    pub exists: Result<(), String>,
    pub get_keys: Result<(), String>,
    pub close: Result<(), String>,
}

#[derive(Default)]
pub(crate) struct KeyValue {
    open_map: HashMap<String, KvStore>,
    get_map: HashMap<(KvStore, String), Vec<u8>>,
    set_set: HashSet<(KvStore, String, Vec<u8>)>,
    delete_set: HashSet<(KvStore, String)>,
    exists_map: HashMap<(KvStore, String), bool>,
    get_keys_map: HashMap<KvStore, Vec<String>>,
    close_set: HashSet<KvStore>,
}

impl key_value::Host for KeyValue {
    async fn open(&mut self, name: String) -> wasmtime::Result<Result<KvStore, Error>> {
        Ok(self.open_map.remove(&name).ok_or_else(|| {
            Error::Io(format!(
                "expected {:?}, got {:?}",
                self.open_map.keys(),
                name
            ))
        }))
    }

    async fn get(
        &mut self,
        store: KvStore,
        name: String,
    ) -> wasmtime::Result<Result<Vec<u8>, Error>> {
        Ok(self
            .get_map
            .remove(&(store, name.to_owned()))
            .ok_or_else(|| {
                Error::Io(format!(
                    "expected {:?}, got {:?}",
                    self.get_map.keys(),
                    iter::once(&(store, name.to_owned()))
                ))
            }))
    }

    async fn set(
        &mut self,
        store: KvStore,
        name: String,
        value: Vec<u8>,
    ) -> wasmtime::Result<Result<(), Error>> {
        Ok(
            if self
                .set_set
                .remove(&(store, name.to_owned(), value.to_vec()))
            {
                Ok(())
            } else {
                Err(Error::Io(format!(
                    "expected {:?}, got {:?}",
                    self.set_set.iter(),
                    iter::once(&(store, name, value))
                )))
            },
        )
    }

    async fn delete(
        &mut self,
        store: KvStore,
        name: String,
    ) -> wasmtime::Result<Result<(), Error>> {
        Ok(if self.delete_set.remove(&(store, name.to_owned())) {
            Ok(())
        } else {
            Err(Error::Io(format!(
                "expected {:?}, got {:?}",
                self.delete_set.iter(),
                iter::once(&(store, name))
            )))
        })
    }

    async fn exists(
        &mut self,
        store: KvStore,
        name: String,
    ) -> wasmtime::Result<Result<bool, Error>> {
        Ok(self
            .exists_map
            .remove(&(store, name.to_owned()))
            .ok_or_else(|| {
                Error::Io(format!(
                    "expected {:?}, got {:?}",
                    self.exists_map.keys(),
                    iter::once(&(store, name))
                ))
            }))
    }

    async fn get_keys(&mut self, store: KvStore) -> wasmtime::Result<Result<Vec<String>, Error>> {
        Ok(self.get_keys_map.remove(&store).ok_or_else(|| {
            Error::Io(format!(
                "expected {:?}, got {:?}",
                self.open_map.keys(),
                iter::once(&store)
            ))
        }))
    }

    async fn close(&mut self, store: KvStore) -> wasmtime::Result<()> {
        if self.close_set.remove(&store) {
            Ok(())
        } else {
            Err(wasmtime::format_err!(
                "expected {:?}, got {:?}",
                self.close_set.iter(),
                iter::once(&store)
            ))
        }
    }
}

pub(crate) async fn test(
    engine: &Engine,
    test_config: TestConfig,
    pre: &InstancePre<Context>,
) -> Result<KeyValueReport> {
    Ok(KeyValueReport {
        open: {
            let mut store =
                super::create_store_with_context(engine, test_config.clone(), |context| {
                    context.key_value.open_map.insert("foo".into(), 42);
                });

            super::run_command(&mut store, pre, &["key-value-open", "foo"], |store| {
                ensure!(
                    store.data().key_value.open_map.is_empty(),
                    "expected module to call `key_value::open` exactly once"
                );

                Ok(())
            })
            .await
        },

        get: {
            let mut store =
                super::create_store_with_context(engine, test_config.clone(), |context| {
                    context
                        .key_value
                        .get_map
                        .insert((42, "foo".into()), b"bar".to_vec());
                });

            super::run_command(&mut store, pre, &["key-value-get", "42", "foo"], |store| {
                ensure!(
                    store.data().key_value.get_map.is_empty(),
                    "expected module to call `key_value::get` exactly once"
                );

                Ok(())
            })
            .await
        },

        set: {
            let mut store =
                super::create_store_with_context(engine, test_config.clone(), |context| {
                    context
                        .key_value
                        .set_set
                        .insert((42, "foo".into(), b"bar".to_vec()));
                });

            super::run_command(
                &mut store,
                pre,
                &["key-value-set", "42", "foo", "bar"],
                |store| {
                    ensure!(
                        store.data().key_value.set_set.is_empty(),
                        "expected module to call `key_value::set` exactly once"
                    );

                    Ok(())
                },
            )
            .await
        },

        delete: {
            let mut store =
                super::create_store_with_context(engine, test_config.clone(), |context| {
                    context.key_value.delete_set.insert((42, "foo".into()));
                });

            super::run_command(
                &mut store,
                pre,
                &["key-value-delete", "42", "foo"],
                |store| {
                    ensure!(
                        store.data().key_value.delete_set.is_empty(),
                        "expected module to call `key_value::delete` exactly once"
                    );

                    Ok(())
                },
            )
            .await
        },

        exists: {
            let mut store =
                super::create_store_with_context(engine, test_config.clone(), |context| {
                    context
                        .key_value
                        .exists_map
                        .insert((42, "foo".into()), true);
                });

            super::run_command(
                &mut store,
                pre,
                &["key-value-exists", "42", "foo"],
                |store| {
                    ensure!(
                        store.data().key_value.exists_map.is_empty(),
                        "expected module to call `key_value::exists` exactly once"
                    );

                    Ok(())
                },
            )
            .await
        },

        get_keys: {
            let mut store =
                super::create_store_with_context(engine, test_config.clone(), |context| {
                    context
                        .key_value
                        .get_keys_map
                        .insert(42, vec!["foo".into(), "bar".into()]);
                });

            super::run_command(&mut store, pre, &["key-value-get-keys", "42"], |store| {
                ensure!(
                    store.data().key_value.get_keys_map.is_empty(),
                    "expected module to call `key_value::get_keys` exactly once"
                );

                Ok(())
            })
            .await
        },

        close: {
            let mut store = super::create_store_with_context(engine, test_config, |context| {
                context.key_value.close_set.insert(42);
            });

            super::run_command(&mut store, pre, &["key-value-close", "42"], |store| {
                ensure!(
                    store.data().key_value.close_set.is_empty(),
                    "expected module to call `key_value::close` exactly once"
                );

                Ok(())
            })
            .await
        },
    })
}
