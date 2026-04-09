use super::{
    redis::{self, Error, RedisParameter, RedisResult},
    Context, TestConfig,
};
use anyhow::{ensure, Result};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use wasmtime::{component::InstancePre, Engine};

/// Report of which Redis tests succeeded or failed
#[derive(Serialize, PartialEq, Eq, Debug)]
pub struct RedisReport {
    /// Result of the Redis `PUBLISH` test
    ///
    /// The guest module should expect a call according to [`super::InvocationStyle`] with
    /// \["redis-publish", "127.0.0.1", "foo", "bar"\] as arguments.  The module should call the
    /// host-implemented `redis::publish` function with the arguments \["127.0.0.1", "foo", "bar"\] and
    /// expect `ok(unit)` as the result.  The host will assert that said function is called exactly once with the
    /// specified arguments.
    pub publish: Result<(), String>,

    /// Result of the Redis `SET` test
    ///
    /// The guest module should expect a call according to [`super::InvocationStyle`] with \["redis-set",
    /// "127.0.0.1", "foo", "bar"\] as arguments.  The module should call the host-implemented
    /// `redis::set` function with the arguments \["127.0.0.1", "foo", "bar"\] and expect `ok(unit)` as
    /// the result.  The host will assert that said function is called exactly once with the specified arguments.
    pub set: Result<(), String>,

    /// Result of the Redis `GET` test
    ///
    /// The guest module should expect a call according to [`super::InvocationStyle`] with \["redis-get",
    /// "127.0.0.1", "foo"\] as arguments.  The module should call the host-implemented `redis::get`
    /// function with the arguments \["127.0.0.1", "foo"\] and expect `ok("bar")` (UTF-8-encoded) as the result.
    /// The host will assert that said function is called exactly once with the specified arguments.
    pub get: Result<(), String>,

    /// Result of the Redis `INCR` test
    ///
    /// The guest module should expect a call according to [`super::InvocationStyle`] with \["redis-incr",
    /// "127.0.0.1", "foo"\] as arguments.  The module should call the host-implemented `redis::incr`
    /// function with the arguments \["127.0.0.1", "foo"\] and expect `ok(42)` as the result.  The host will assert
    /// that said function is called exactly once with the specified arguments.
    pub incr: Result<(), String>,

    /// Result of the Redis `DEL` test
    ///
    /// The guest module should expect a call according to [`super::InvocationStyle`] with \["redis-del",
    /// "127.0.0.1", "foo"\] as arguments. The module should call the host-implemented `redis::del`
    /// function with the arguments \["127.0.0.1", \["foo"\]\] and expect `ok(0)` as the result.  The host will assert
    /// that said function is called exactly once with the specified arguments.
    pub del: Result<(), String>,

    /// Result of the Redis `SADD` test
    ///
    /// The guest module should expect a call according to [`super::InvocationStyle`] with \["redis-sadd",
    /// "127.0.0.1", "foo", "bar", "baz"\] as arguments. The module should call the host-implemented
    /// `redis::sadd` function with the arguments \["127.0.0.1", "foo", \["bar", "baz"\]\] and expect
    /// `ok(2)` as the result.  The host will assert that said function is called exactly once with the specified
    /// arguments.
    pub sadd: Result<(), String>,

    /// Result of the Redis `SREM` test
    ///
    /// The guest module should expect a call according to [`super::InvocationStyle`] with \["redis-srem",
    /// "127.0.0.1", "foo", "bar", "baz"\] as arguments. The module should call the host-implemented
    /// `redis::srem` function with the arguments \["127.0.0.1", "foo", \["bar", "baz"\]\] and expect
    /// `ok(2)` as the result.  The host will assert that said function is called exactly once with the specified
    /// arguments.
    pub srem: Result<(), String>,

    /// Result of the Redis `SMEMBERS` test
    ///
    /// The guest module should expect a call according to [`super::InvocationStyle`] with
    /// \["redis-smembers", "127.0.0.1", "foo"\] as arguments. The module should call the host-implemented
    /// `redis::smembers` function with the arguments \["127.0.0.1", "foo"\] and expect `ok(list("bar",
    /// "baz"))` as the result.  The host will assert that said function is called exactly once with the specified
    /// arguments.
    pub smembers: Result<(), String>,

    /// Result of the Redis `execute` test
    ///
    /// The guest module should expect a call according to [`super::InvocationStyle`] with
    /// \["redis-execute", "127.0.0.1", "append", "foo", "baz"\] as arguments. The module should call the
    /// host-implemented `redis::execute` function with the arguments \["127.0.0.1", "append", "foo",
    /// "baz"\] and expect `ok(list(value::int(3)))` as the result.  The host will assert that said function is
    /// called exactly once with the specified arguments.
    pub execute: Result<(), String>,
}

#[derive(Default)]
pub(crate) struct Redis {
    publish_set: HashSet<(String, String, Vec<u8>)>,
    set_set: HashSet<(String, String, Vec<u8>)>,
    get_map: HashMap<(String, String), Vec<u8>>,
    incr_map: HashMap<(String, String), i64>,
    del_map: HashMap<(String, Vec<String>), i64>,
    sadd_map: HashMap<(String, String, Vec<String>), i64>,
    srem_map: HashMap<(String, String, Vec<String>), i64>,
    smembers_map: HashMap<(String, String), Vec<String>>,
    #[allow(clippy::type_complexity)]
    execute_map: HashMap<(String, String, Vec<Vec<u8>>), Vec<RedisResult>>,
}

impl redis::Host for Redis {
    async fn publish(
        &mut self,
        address: String,
        channel: String,
        payload: Vec<u8>,
    ) -> wasmtime::Result<Result<(), Error>> {
        Ok(if self.publish_set.remove(&(address, channel, payload)) {
            Ok(())
        } else {
            Err(Error::Error)
        })
    }

    async fn get(
        &mut self,
        address: String,
        key: String,
    ) -> wasmtime::Result<Result<Vec<u8>, Error>> {
        Ok(self.get_map.remove(&(address, key)).ok_or(Error::Error))
    }

    async fn set(
        &mut self,
        address: String,
        key: String,
        value: Vec<u8>,
    ) -> wasmtime::Result<Result<(), Error>> {
        Ok(if self.set_set.remove(&(address, key, value)) {
            Ok(())
        } else {
            Err(Error::Error)
        })
    }

    async fn incr(&mut self, address: String, key: String) -> wasmtime::Result<Result<i64, Error>> {
        Ok(self
            .incr_map
            .remove(&(address, key))
            .map(|value| value + 1)
            .ok_or(Error::Error))
    }

    async fn del(
        &mut self,
        address: String,
        keys: Vec<String>,
    ) -> wasmtime::Result<Result<i64, Error>> {
        Ok(self.del_map.remove(&(address, keys)).ok_or(Error::Error))
    }

    async fn sadd(
        &mut self,
        address: String,
        key: String,
        values: Vec<String>,
    ) -> wasmtime::Result<Result<i64, Error>> {
        Ok(self
            .sadd_map
            .remove(&(address, key, values))
            .ok_or(Error::Error))
    }

    async fn srem(
        &mut self,
        address: String,
        key: String,
        values: Vec<String>,
    ) -> wasmtime::Result<Result<i64, Error>> {
        Ok(self
            .srem_map
            .remove(&(address, key, values))
            .ok_or(Error::Error))
    }

    async fn smembers(
        &mut self,
        address: String,
        key: String,
    ) -> wasmtime::Result<Result<Vec<String>, Error>> {
        Ok(self
            .smembers_map
            .remove(&(address, key))
            .ok_or(Error::Error))
    }

    async fn execute(
        &mut self,
        address: String,
        command: String,
        arguments: Vec<RedisParameter>,
    ) -> wasmtime::Result<Result<Vec<RedisResult>, Error>> {
        Ok(self
            .execute_map
            .remove(&(
                address,
                command,
                arguments
                    .into_iter()
                    .filter_map(|v| {
                        if let RedisParameter::Binary(bytes) = v {
                            Some(bytes)
                        } else {
                            None
                        }
                    })
                    .collect(),
            ))
            .ok_or(Error::Error))
    }
}

pub(crate) async fn test(
    engine: &Engine,
    test_config: TestConfig,
    pre: &InstancePre<Context>,
) -> Result<RedisReport> {
    Ok(RedisReport {
        publish: {
            let mut store =
                super::create_store_with_context(engine, test_config.clone(), |context| {
                    context.redis.publish_set.insert((
                        "127.0.0.1".into(),
                        "foo".into(),
                        "bar".as_bytes().to_vec(),
                    ));
                });
            super::run_command(
                &mut store,
                pre,
                &["redis-publish", "127.0.0.1", "foo", "bar"],
                |store| {
                    ensure!(
                        store.data().redis.publish_set.is_empty(),
                        "expected module to call `redis::publish` exactly once"
                    );

                    Ok(())
                },
            )
            .await
        },

        set: {
            let mut store =
                super::create_store_with_context(engine, test_config.clone(), |context| {
                    context.redis.set_set.insert((
                        "127.0.0.1".into(),
                        "foo".into(),
                        "bar".as_bytes().to_vec(),
                    ));
                });
            super::run_command(
                &mut store,
                pre,
                &["redis-set", "127.0.0.1", "foo", "bar"],
                |store| {
                    ensure!(
                        store.data().redis.set_set.is_empty(),
                        "expected module to call `redis::set` exactly once"
                    );

                    Ok(())
                },
            )
            .await
        },

        get: {
            let mut store =
                super::create_store_with_context(engine, test_config.clone(), |context| {
                    context.redis.get_map.insert(
                        ("127.0.0.1".into(), "foo".into()),
                        "bar".as_bytes().to_vec(),
                    );
                });
            super::run_command(
                &mut store,
                pre,
                &["redis-get", "127.0.0.1", "foo"],
                |store| {
                    ensure!(
                        store.data().redis.get_map.is_empty(),
                        "expected module to call `redis::get` exactly once"
                    );

                    Ok(())
                },
            )
            .await
        },

        incr: {
            let mut store =
                super::create_store_with_context(engine, test_config.clone(), |context| {
                    context
                        .redis
                        .incr_map
                        .insert(("127.0.0.1".into(), "foo".into()), 41);
                });

            super::run_command(
                &mut store,
                pre,
                &["redis-incr", "127.0.0.1", "foo"],
                |store| {
                    ensure!(
                        store.data().redis.incr_map.is_empty(),
                        "expected module to call `redis::incr` exactly once"
                    );

                    Ok(())
                },
            )
            .await
        },

        del: {
            let mut store =
                super::create_store_with_context(engine, test_config.clone(), |context| {
                    context
                        .redis
                        .del_map
                        .insert(("127.0.0.1".into(), vec!["foo".to_owned()]), 0);
                });
            super::run_command(
                &mut store,
                pre,
                &["redis-del", "127.0.0.1", "foo"],
                |store| {
                    ensure!(
                        store.data().redis.del_map.is_empty(),
                        "expected module to call `redis::del` exactly once"
                    );

                    Ok(())
                },
            )
            .await
        },

        sadd: {
            let mut store =
                super::create_store_with_context(engine, test_config.clone(), |context| {
                    context.redis.sadd_map.insert(
                        (
                            "127.0.0.1".into(),
                            "foo".to_owned(),
                            vec!["bar".to_owned(), "baz".to_owned()],
                        ),
                        0,
                    );
                });

            super::run_command(
                &mut store,
                pre,
                &["redis-sadd", "127.0.0.1", "foo", "bar", "baz"],
                |store| {
                    ensure!(
                        store.data().redis.sadd_map.is_empty(),
                        "expected module to call `redis::sadd` exactly once"
                    );

                    Ok(())
                },
            )
            .await
        },

        srem: {
            let mut store =
                super::create_store_with_context(engine, test_config.clone(), |context| {
                    context.redis.srem_map.insert(
                        (
                            "127.0.0.1".into(),
                            "foo".to_owned(),
                            vec!["bar".to_owned(), "baz".to_owned()],
                        ),
                        0,
                    );
                });

            super::run_command(
                &mut store,
                pre,
                &["redis-srem", "127.0.0.1", "foo", "bar", "baz"],
                |store| {
                    ensure!(
                        store.data().redis.srem_map.is_empty(),
                        "expected module to call `redis::srem` exactly once"
                    );

                    Ok(())
                },
            )
            .await
        },

        smembers: {
            let mut store =
                super::create_store_with_context(engine, test_config.clone(), |context| {
                    context.redis.smembers_map.insert(
                        ("127.0.0.1".into(), "foo".to_owned()),
                        vec!["bar".to_owned(), "baz".to_owned()],
                    );
                });

            super::run_command(
                &mut store,
                pre,
                &["redis-smembers", "127.0.0.1", "foo"],
                |store| {
                    ensure!(
                        store.data().redis.smembers_map.is_empty(),
                        "expected module to call `redis::smembers` exactly once"
                    );

                    Ok(())
                },
            )
            .await
        },

        execute: {
            let mut store =
                super::create_store_with_context(engine, test_config.clone(), |context| {
                    context.redis.execute_map.insert(
                        (
                            "127.0.0.1".into(),
                            "append".to_owned(),
                            vec![b"foo".to_vec(), b"baz".to_vec()],
                        ),
                        vec![RedisResult::Int64(3)],
                    );
                });

            super::run_command(
                &mut store,
                pre,
                &["redis-execute", "127.0.0.1", "append", "foo", "baz"],
                |store| {
                    ensure!(
                        store.data().redis.execute_map.is_empty(),
                        "expected module to call `redis::execute` exactly once"
                    );

                    Ok(())
                },
            )
            .await
        },
    })
}
