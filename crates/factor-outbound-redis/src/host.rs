use std::net::SocketAddr;

use anyhow::Result;
use redis::io::AsyncDNSResolver;
use redis::AsyncConnectionConfig;
use redis::{aio::MultiplexedConnection, AsyncCommands, FromRedisValue, Value};
use spin_core::wasmtime::component::{Accessor, Resource};
use spin_factor_otel::OtelFactorState;
use spin_factor_outbound_networking::config::blocked_networks::BlockedNetworks;
use spin_world::spin::redis::redis as v3;
use spin_world::v1::{redis as v1, redis_types};
use spin_world::v2::redis as v2;
use spin_world::MAX_HOST_BUFFERED_BYTES;
use tracing::field::Empty;
use tracing::{instrument, Level};

use crate::allowed_hosts::AllowedHostChecker;

pub struct InstanceState {
    pub(crate) allowed_host_checker: AllowedHostChecker,
    pub blocked_networks: BlockedNetworks,
    pub connections: spin_resource_table::Table<MultiplexedConnection>,
    pub otel: OtelFactorState,
}

impl InstanceState {
    async fn is_address_allowed(&self, address: &str) -> Result<bool> {
        self.allowed_host_checker.is_address_allowed(address).await
    }

    async fn establish_connection(
        &mut self,
        address: String,
    ) -> Result<Resource<v2::Connection>, v2::Error> {
        let config = AsyncConnectionConfig::new()
            .set_dns_resolver(SpinDnsResolver(self.blocked_networks.clone()));
        let conn = redis::Client::open(address.as_str())
            .map_err(|_| v2::Error::InvalidAddress)?
            .get_multiplexed_async_connection_with_config(&config)
            .await
            .map_err(other_error_v2)?;
        self.connections
            .push(conn)
            .map(Resource::new_own)
            .map_err(|_| v2::Error::TooManyConnections)
    }

    async fn get_conn(
        &mut self,
        connection: Resource<v2::Connection>,
    ) -> Result<&mut MultiplexedConnection, v2::Error> {
        self.connections
            .get_mut(connection.rep())
            .ok_or(v2::Error::Other(
                "could not find connection for resource".into(),
            ))
    }

    fn get_conn_v3(
        &mut self,
        connection: Resource<v3::Connection>,
    ) -> Result<MultiplexedConnection, v3::Error> {
        self.connections
            .get(connection.rep())
            .cloned()
            .ok_or(v3::Error::Other(
                "could not find connection for resource".into(),
            ))
    }
}

mod operations {
    use super::*;

    pub async fn publish(
        conn: &mut MultiplexedConnection,
        channel: String,
        payload: v3::Payload,
    ) -> Result<(), v3::Error> {
        // The `let () =` syntax is needed to suppress a warning when the result type is inferred.
        // You can read more about the issue here: <https://github.com/redis-rs/redis-rs/issues/1228>
        let () = conn
            .publish(&channel, &payload)
            .await
            .map_err(other_error_v3)?;
        Ok(())
    }

    pub async fn get(
        conn: &mut MultiplexedConnection,
        key: String,
    ) -> Result<Option<Vec<u8>>, v3::Error> {
        let value = conn
            .get::<_, Option<Vec<u8>>>(&key)
            .await
            .map_err(other_error_v3)?;

        // Currently there's no way to stream a `GET` result using the `redis`
        // crate without buffering, so the damage (in terms of host memory
        // usage) is already done, but we can still enforce the limit:
        if std::mem::size_of::<Option<Vec<u8>>>() + value.as_ref().map(|v| v.len()).unwrap_or(0)
            > MAX_HOST_BUFFERED_BYTES
        {
            Err(v3::Error::Other(format!(
                "query result exceeds limit of {MAX_HOST_BUFFERED_BYTES} bytes"
            )))
        } else {
            Ok(value)
        }
    }

    pub async fn set(
        conn: &mut MultiplexedConnection,
        key: String,
        value: Vec<u8>,
    ) -> Result<(), v3::Error> {
        // The `let () =` syntax is needed to suppress a warning when the result type is inferred.
        // You can read more about the issue here: <https://github.com/redis-rs/redis-rs/issues/1228>
        let () = conn.set(&key, &value).await.map_err(other_error_v3)?;
        Ok(())
    }

    pub async fn incr(conn: &mut MultiplexedConnection, key: String) -> Result<i64, v3::Error> {
        conn.incr(&key, 1).await.map_err(other_error_v3)
    }

    pub async fn del(
        conn: &mut MultiplexedConnection,
        keys: Vec<String>,
    ) -> Result<u32, v3::Error> {
        conn.del(&keys).await.map_err(other_error_v3)
    }

    pub async fn sadd(
        conn: &mut MultiplexedConnection,
        key: String,
        values: Vec<String>,
    ) -> Result<u32, v3::Error> {
        let value = conn.sadd(&key, &values).await.map_err(|e| {
            if e.kind() == redis::ErrorKind::TypeError {
                v3::Error::TypeError
            } else {
                v3::Error::Other(e.to_string())
            }
        })?;
        Ok(value)
    }

    pub async fn smembers(
        conn: &mut MultiplexedConnection,
        key: String,
    ) -> Result<Vec<String>, v3::Error> {
        conn.smembers(&key).await.map_err(other_error_v3)
    }

    pub async fn srem(
        conn: &mut MultiplexedConnection,
        key: String,
        values: Vec<String>,
    ) -> Result<u32, v3::Error> {
        conn.srem(&key, &values).await.map_err(other_error_v3)
    }

    pub async fn execute(
        conn: &mut MultiplexedConnection,
        command: String,
        arguments: impl IntoIterator<Item = v3::RedisParameter>,
    ) -> Result<RedisResults, v3::Error> {
        let mut cmd = redis::cmd(&command);
        arguments.into_iter().for_each(|value| match value {
            v3::RedisParameter::Int64(v) => {
                cmd.arg(v);
            }
            v3::RedisParameter::Binary(v) => {
                cmd.arg(v);
            }
        });

        let results = cmd
            .query_async::<RedisResults>(conn)
            .await
            .map_err(other_error_v3)?;

        // Currently there's no way to stream results using the `redis`
        // crate without buffering, so the damage (in terms of host memory
        // usage) is already done, but we can still enforce the limit:
        if std::mem::size_of::<Vec<v3::RedisResult>>()
            + results.0.iter().map(memory_size).sum::<usize>()
            > MAX_HOST_BUFFERED_BYTES
        {
            Err(v3::Error::Other(format!(
                "query result exceeds limit of {MAX_HOST_BUFFERED_BYTES} bytes"
            )))
        } else {
            Ok(results)
        }
    }
}

impl v3::Host for crate::InstanceState {
    fn convert_error(&mut self, error: v3::Error) -> anyhow::Result<v3::Error> {
        Ok(error)
    }
}

impl v3::HostConnection for crate::InstanceState {
    async fn drop(&mut self, connection: Resource<v3::Connection>) -> anyhow::Result<()> {
        self.connections.remove(connection.rep());
        Ok(())
    }
}

impl crate::RedisFactorData {
    fn get_conn<T: Send>(
        accessor: &Accessor<T, Self>,
        connection: Resource<v3::Connection>,
    ) -> Result<MultiplexedConnection, v3::Error> {
        accessor.with(|mut access| {
            let host = access.get();
            host.otel.reparent_tracing_span();
            host.get_conn_v3(connection)
        })
    }
}

impl v3::HostConnectionWithStore for crate::RedisFactorData {
    #[instrument(name = "spin_outbound_redis.open_connection", skip(accessor, address), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", db.address = Empty, server.port = Empty, db.namespace = Empty))]
    async fn open<T: Send>(
        accessor: &Accessor<T, Self>,
        address: String,
    ) -> Result<Resource<v3::Connection>, v3::Error> {
        let (allowed_host_checker, blocked_networks) = accessor.with(|mut access| {
            let host = access.get();
            host.otel.reparent_tracing_span();
            (
                host.allowed_host_checker.clone(),
                host.blocked_networks.clone(),
            )
        });

        if !allowed_host_checker
            .is_address_allowed(&address)
            .await
            .map_err(|e| v3::Error::Other(e.to_string()))?
        {
            return Err(v3::Error::InvalidAddress);
        }

        let config =
            AsyncConnectionConfig::new().set_dns_resolver(SpinDnsResolver(blocked_networks));
        let conn = redis::Client::open(address.as_str())
            .map_err(|_| v3::Error::InvalidAddress)?
            .get_multiplexed_async_connection_with_config(&config)
            .await
            .map_err(other_error_v3)?;

        let rsrc = accessor.with(|mut access| {
            let host = access.get();
            host.connections
                .push(conn)
                .map(Resource::new_own)
                .map_err(|_| v3::Error::TooManyConnections)
        });

        rsrc
    }

    #[instrument(name = "spin_outbound_redis.publish", skip(accessor, connection, payload), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", otel.name = format!("PUBLISH {}", channel)))]
    async fn publish<T: Send>(
        accessor: &Accessor<T, Self>,
        connection: Resource<v3::Connection>,
        channel: String,
        payload: v3::Payload,
    ) -> Result<(), v3::Error> {
        let mut conn = Self::get_conn(accessor, connection)?;
        operations::publish(&mut conn, channel, payload).await
    }

    #[instrument(name = "spin_outbound_redis.get", skip(accessor, connection), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", otel.name = format!("GET {}", key)))]
    async fn get<T: Send>(
        accessor: &Accessor<T, Self>,
        connection: Resource<v3::Connection>,
        key: String,
    ) -> Result<Option<v3::Payload>, v3::Error> {
        let mut conn = Self::get_conn(accessor, connection)?;
        operations::get(&mut conn, key).await
    }

    #[instrument(name = "spin_outbound_redis.set", skip(accessor, connection, value), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", otel.name = format!("SET {}", key)))]
    async fn set<T: Send>(
        accessor: &Accessor<T, Self>,
        connection: Resource<v3::Connection>,
        key: String,
        value: v3::Payload,
    ) -> Result<(), v3::Error> {
        let mut conn = Self::get_conn(accessor, connection)?;
        operations::set(&mut conn, key, value).await
    }

    #[instrument(name = "spin_outbound_redis.incr", skip(accessor, connection), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", otel.name = format!("INCRBY {} 1", key)))]
    async fn incr<T: Send>(
        accessor: &Accessor<T, Self>,
        connection: Resource<v3::Connection>,
        key: String,
    ) -> Result<i64, v3::Error> {
        let mut conn = Self::get_conn(accessor, connection)?;
        operations::incr(&mut conn, key).await
    }

    #[instrument(name = "spin_outbound_redis.del", skip(accessor, connection), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", otel.name = format!("DEL {}", keys.join(" "))))]
    async fn del<T: Send>(
        accessor: &Accessor<T, Self>,
        connection: Resource<v3::Connection>,
        keys: Vec<String>,
    ) -> Result<u32, v3::Error> {
        let mut conn = Self::get_conn(accessor, connection)?;
        operations::del(&mut conn, keys).await
    }

    #[instrument(name = "spin_outbound_redis.sadd", skip(accessor, connection, values), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", otel.name = format!("SADD {} {}", key, values.join(" "))))]
    async fn sadd<T: Send>(
        accessor: &Accessor<T, Self>,
        connection: Resource<v3::Connection>,
        key: String,
        values: Vec<String>,
    ) -> Result<u32, v3::Error> {
        let mut conn = Self::get_conn(accessor, connection)?;
        operations::sadd(&mut conn, key, values).await
    }

    #[instrument(name = "spin_outbound_redis.smembers", skip(accessor, connection), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", otel.name = format!("SMEMBERS {}", key)))]
    async fn smembers<T: Send>(
        accessor: &Accessor<T, Self>,
        connection: Resource<v3::Connection>,
        key: String,
    ) -> Result<Vec<String>, v3::Error> {
        let mut conn = Self::get_conn(accessor, connection)?;
        operations::smembers(&mut conn, key).await
    }

    #[instrument(name = "spin_outbound_redis.srem", skip(accessor, connection, values), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", otel.name = format!("SREM {} {}", key, values.join(" "))))]
    async fn srem<T: Send>(
        accessor: &Accessor<T, Self>,
        connection: Resource<v3::Connection>,
        key: String,
        values: Vec<String>,
    ) -> Result<u32, v3::Error> {
        let mut conn = Self::get_conn(accessor, connection)?;
        operations::srem(&mut conn, key, values).await
    }

    #[instrument(name = "spin_outbound_redis.execute", skip(accessor, connection), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", otel.name = format!("{}", command)))]
    async fn execute<T: Send>(
        accessor: &Accessor<T, Self>,
        connection: Resource<v3::Connection>,
        command: String,
        arguments: Vec<v3::RedisParameter>,
    ) -> Result<Vec<v3::RedisResult>, v3::Error> {
        let mut conn = Self::get_conn(accessor, connection)?;
        Ok(operations::execute(&mut conn, command, arguments)
            .await?
            .into_v3())
    }
}

impl v2::Host for crate::InstanceState {
    fn convert_error(&mut self, error: v2::Error) -> Result<v2::Error> {
        Ok(error)
    }
}

impl v2::HostConnection for crate::InstanceState {
    #[instrument(name = "spin_outbound_redis.open_connection", skip(self, address), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", db.address = Empty, server.port = Empty, db.namespace = Empty))]
    async fn open(&mut self, address: String) -> Result<Resource<v2::Connection>, v2::Error> {
        self.otel.reparent_tracing_span();
        if !self
            .is_address_allowed(&address)
            .await
            .map_err(|e| v2::Error::Other(e.to_string()))?
        {
            return Err(v2::Error::InvalidAddress);
        }

        self.establish_connection(address).await
    }

    #[instrument(name = "spin_outbound_redis.publish", skip(self, connection, payload), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", otel.name = format!("PUBLISH {}", channel)))]
    async fn publish(
        &mut self,
        connection: Resource<v2::Connection>,
        channel: String,
        payload: Vec<u8>,
    ) -> Result<(), v2::Error> {
        self.otel.reparent_tracing_span();

        let conn = self.get_conn(connection).await.map_err(other_error_v2)?;

        Ok(operations::publish(conn, channel, payload).await?)
    }

    #[instrument(name = "spin_outbound_redis.get", skip(self, connection), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", otel.name = format!("GET {}", key)))]
    async fn get(
        &mut self,
        connection: Resource<v2::Connection>,
        key: String,
    ) -> Result<Option<Vec<u8>>, v2::Error> {
        self.otel.reparent_tracing_span();

        let conn = self.get_conn(connection).await.map_err(other_error_v2)?;

        Ok(operations::get(conn, key).await?)
    }

    #[instrument(name = "spin_outbound_redis.set", skip(self, connection, value), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", otel.name = format!("SET {}", key)))]
    async fn set(
        &mut self,
        connection: Resource<v2::Connection>,
        key: String,
        value: Vec<u8>,
    ) -> Result<(), v2::Error> {
        self.otel.reparent_tracing_span();

        let conn = self.get_conn(connection).await.map_err(other_error_v2)?;
        Ok(operations::set(conn, key, value).await?)
    }

    #[instrument(name = "spin_outbound_redis.incr", skip(self, connection), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", otel.name = format!("INCRBY {} 1", key)))]
    async fn incr(
        &mut self,
        connection: Resource<v2::Connection>,
        key: String,
    ) -> Result<i64, v2::Error> {
        self.otel.reparent_tracing_span();

        let conn = self.get_conn(connection).await.map_err(other_error_v2)?;
        Ok(operations::incr(conn, key).await?)
    }

    #[instrument(name = "spin_outbound_redis.del", skip(self, connection), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", otel.name = format!("DEL {}", keys.join(" "))))]
    async fn del(
        &mut self,
        connection: Resource<v2::Connection>,
        keys: Vec<String>,
    ) -> Result<u32, v2::Error> {
        self.otel.reparent_tracing_span();

        let conn = self.get_conn(connection).await.map_err(other_error_v2)?;
        Ok(operations::del(conn, keys).await?)
    }

    #[instrument(name = "spin_outbound_redis.sadd", skip(self, connection, values), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", otel.name = format!("SADD {} {}", key, values.join(" "))))]
    async fn sadd(
        &mut self,
        connection: Resource<v2::Connection>,
        key: String,
        values: Vec<String>,
    ) -> Result<u32, v2::Error> {
        self.otel.reparent_tracing_span();

        let conn = self.get_conn(connection).await.map_err(other_error_v2)?;
        Ok(operations::sadd(conn, key, values).await?)
    }

    #[instrument(name = "spin_outbound_redis.smembers", skip(self, connection), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", otel.name = format!("SMEMBERS {}", key)))]
    async fn smembers(
        &mut self,
        connection: Resource<v2::Connection>,
        key: String,
    ) -> Result<Vec<String>, v2::Error> {
        self.otel.reparent_tracing_span();

        let conn = self.get_conn(connection).await.map_err(other_error_v2)?;
        Ok(operations::smembers(conn, key).await?)
    }

    #[instrument(name = "spin_outbound_redis.srem", skip(self, connection, values), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", otel.name = format!("SREM {} {}", key, values.join(" "))))]
    async fn srem(
        &mut self,
        connection: Resource<v2::Connection>,
        key: String,
        values: Vec<String>,
    ) -> Result<u32, v2::Error> {
        self.otel.reparent_tracing_span();

        let conn = self.get_conn(connection).await.map_err(other_error_v2)?;
        Ok(operations::srem(conn, key, values).await?)
    }

    #[instrument(name = "spin_outbound_redis.execute", skip(self, connection, arguments), err(level = Level::INFO), fields(otel.kind = "client", db.system = "redis", otel.name = format!("{}", command)))]
    async fn execute(
        &mut self,
        connection: Resource<v2::Connection>,
        command: String,
        arguments: Vec<v2::RedisParameter>,
    ) -> Result<Vec<v2::RedisResult>, v2::Error> {
        fn to_v3_param(value: v2::RedisParameter) -> v3::RedisParameter {
            match value {
                v2::RedisParameter::Int64(v) => v3::RedisParameter::Int64(v),
                v2::RedisParameter::Binary(v) => v3::RedisParameter::Binary(v),
            }
        }

        self.otel.reparent_tracing_span();

        let conn = self.get_conn(connection).await?;

        let arguments = arguments.into_iter().map(to_v3_param);
        Ok(operations::execute(conn, command, arguments)
            .await?
            .into_v2())
    }

    async fn drop(&mut self, connection: Resource<v2::Connection>) -> anyhow::Result<()> {
        self.connections.remove(connection.rep());
        Ok(())
    }
}

fn other_error_v2(e: impl std::fmt::Display) -> v2::Error {
    v2::Error::Other(e.to_string())
}

fn other_error_v3(e: impl std::fmt::Display) -> v3::Error {
    v3::Error::Other(e.to_string())
}

/// Delegate a function call to the v2::HostConnection implementation
macro_rules! delegate {
    ($self:ident.$name:ident($address:expr, $($arg:expr),*)) => {{
        if !$self.is_address_allowed(&$address).await.map_err(|_| v1::Error::Error)?  {
            return Err(v1::Error::Error);
        }
        let connection = match $self.establish_connection($address).await {
            Ok(c) => c,
            Err(_) => return Err(v1::Error::Error),
        };
        <Self as v2::HostConnection>::$name($self, connection, $($arg),*)
            .await
            .map_err(|_| v1::Error::Error)
    }};
}

impl v1::Host for crate::InstanceState {
    async fn publish(
        &mut self,
        address: String,
        channel: String,
        payload: Vec<u8>,
    ) -> Result<(), v1::Error> {
        delegate!(self.publish(address, channel, payload))
    }

    async fn get(&mut self, address: String, key: String) -> Result<Vec<u8>, v1::Error> {
        delegate!(self.get(address, key)).map(|v| v.unwrap_or_default())
    }

    async fn set(&mut self, address: String, key: String, value: Vec<u8>) -> Result<(), v1::Error> {
        delegate!(self.set(address, key, value))
    }

    async fn incr(&mut self, address: String, key: String) -> Result<i64, v1::Error> {
        delegate!(self.incr(address, key))
    }

    async fn del(&mut self, address: String, keys: Vec<String>) -> Result<i64, v1::Error> {
        delegate!(self.del(address, keys)).map(|v| v as i64)
    }

    async fn sadd(
        &mut self,
        address: String,
        key: String,
        values: Vec<String>,
    ) -> Result<i64, v1::Error> {
        delegate!(self.sadd(address, key, values)).map(|v| v as i64)
    }

    async fn smembers(&mut self, address: String, key: String) -> Result<Vec<String>, v1::Error> {
        delegate!(self.smembers(address, key))
    }

    async fn srem(
        &mut self,
        address: String,
        key: String,
        values: Vec<String>,
    ) -> Result<i64, v1::Error> {
        delegate!(self.srem(address, key, values)).map(|v| v as i64)
    }

    async fn execute(
        &mut self,
        address: String,
        command: String,
        arguments: Vec<v1::RedisParameter>,
    ) -> Result<Vec<v1::RedisResult>, v1::Error> {
        delegate!(self.execute(
            address,
            command,
            arguments.into_iter().map(Into::into).collect()
        ))
        .map(|v| v.into_iter().map(Into::into).collect())
    }
}

impl redis_types::Host for crate::InstanceState {
    fn convert_error(&mut self, error: redis_types::Error) -> Result<redis_types::Error> {
        Ok(error)
    }
}

struct RedisResults(Vec<v3::RedisResult>);

impl RedisResults {
    fn into_v2(self) -> Vec<v2::RedisResult> {
        fn into_v2(value: v3::RedisResult) -> v2::RedisResult {
            match value {
                v3::RedisResult::Nil => v2::RedisResult::Nil,
                v3::RedisResult::Status(v) => v2::RedisResult::Status(v),
                v3::RedisResult::Int64(v) => v2::RedisResult::Int64(v),
                v3::RedisResult::Binary(v) => v2::RedisResult::Binary(v),
            }
        }

        self.0.into_iter().map(into_v2).collect()
    }

    fn into_v3(self) -> Vec<v3::RedisResult> {
        self.0
    }
}

impl FromRedisValue for RedisResults {
    fn from_redis_value(value: &Value) -> redis::RedisResult<Self> {
        fn append(values: &mut Vec<v3::RedisResult>, value: &Value) -> redis::RedisResult<()> {
            match value {
                Value::Nil => {
                    values.push(v3::RedisResult::Nil);
                    Ok(())
                }
                Value::Int(v) => {
                    values.push(v3::RedisResult::Int64(*v));
                    Ok(())
                }
                Value::BulkString(bytes) => {
                    values.push(v3::RedisResult::Binary(bytes.to_owned()));
                    Ok(())
                }
                Value::SimpleString(s) => {
                    values.push(v3::RedisResult::Status(s.to_owned()));
                    Ok(())
                }
                Value::Okay => {
                    values.push(v3::RedisResult::Status("OK".to_string()));
                    Ok(())
                }
                Value::Map(_) => Err(redis::RedisError::from((
                    redis::ErrorKind::TypeError,
                    "Could not convert Redis response",
                    "Redis Map type is not supported".to_string(),
                ))),
                Value::Attribute { .. } => Err(redis::RedisError::from((
                    redis::ErrorKind::TypeError,
                    "Could not convert Redis response",
                    "Redis Attribute type is not supported".to_string(),
                ))),
                Value::Array(arr) | Value::Set(arr) => {
                    arr.iter().try_for_each(|value| append(values, value))
                }
                Value::Double(v) => {
                    values.push(v3::RedisResult::Binary(v.to_string().into_bytes()));
                    Ok(())
                }
                Value::VerbatimString { .. } => Err(redis::RedisError::from((
                    redis::ErrorKind::TypeError,
                    "Could not convert Redis response",
                    "Redis string with format attribute is not supported".to_string(),
                ))),
                Value::Boolean(v) => {
                    values.push(v3::RedisResult::Int64(if *v { 1 } else { 0 }));
                    Ok(())
                }
                Value::BigNumber(v) => {
                    values.push(v3::RedisResult::Binary(v.to_string().as_bytes().to_owned()));
                    Ok(())
                }
                Value::Push { .. } => Err(redis::RedisError::from((
                    redis::ErrorKind::TypeError,
                    "Could not convert Redis response",
                    "Redis Pub/Sub types are not supported".to_string(),
                ))),
                Value::ServerError(err) => Err(redis::RedisError::from((
                    redis::ErrorKind::ResponseError,
                    "Server error",
                    format!("{err:?}"),
                ))),
            }
        }
        let mut values = Vec::new();
        append(&mut values, value)?;
        Ok(RedisResults(values))
    }
}

fn memory_size(value: &v3::RedisResult) -> usize {
    match value {
        v3::RedisResult::Nil | v3::RedisResult::Int64(_) => std::mem::size_of::<v3::RedisResult>(),
        v3::RedisResult::Binary(b) => std::mem::size_of::<v3::RedisResult>() + b.len(),
        v3::RedisResult::Status(s) => std::mem::size_of::<v3::RedisResult>() + s.len(),
    }
}

/// Resolves DNS using Tokio's resolver, filtering out blocked IPs.
struct SpinDnsResolver(BlockedNetworks);

impl AsyncDNSResolver for SpinDnsResolver {
    fn resolve<'a, 'b: 'a>(
        &'a self,
        host: &'b str,
        port: u16,
    ) -> redis::RedisFuture<'a, Box<dyn Iterator<Item = std::net::SocketAddr> + Send + 'a>> {
        Box::pin(async move {
            let mut addrs = tokio::net::lookup_host((host, port))
                .await?
                .collect::<Vec<_>>();
            // Remove blocked IPs
            let blocked_addrs = self.0.remove_blocked(&mut addrs);
            if addrs.is_empty() && !blocked_addrs.is_empty() {
                tracing::error!(
                    "error.type" = "destination_ip_prohibited",
                    ?blocked_addrs,
                    "all destination IP(s) prohibited by runtime config"
                );
            }
            Ok(Box::new(addrs.into_iter()) as Box<dyn Iterator<Item = SocketAddr> + Send>)
        })
    }
}
