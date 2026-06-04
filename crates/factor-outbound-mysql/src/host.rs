use std::sync::Arc;

use anyhow::Result;
use opentelemetry_semantic_conventions::attribute as otel_attribute;
use spin_core::wasmtime::component::{Accessor, FutureReader, Resource, StreamReader};
use spin_factor_outbound_networking::ConnectionPermit;
use spin_telemetry::traces::{self, Blame};
use spin_world::MAX_HOST_BUFFERED_BYTES;
use spin_world::spin::mysql::mysql as v3;
use spin_world::v1::mysql as v1;
use spin_world::v2::mysql as v2;
use spin_world::v2::rdbms_types as v2_types;
use tokio::sync::Mutex;
use tracing::field::Empty;
use tracing::{Level, instrument};

use crate::client::Client;
use crate::{InstanceState, InstanceStateInner, MysqlFactorData};

impl<C: Client> InstanceStateInner<C> {
    async fn open_connection(
        &mut self,
        address: &str,
        permit: ConnectionPermit,
    ) -> Result<u32, v2::Error> {
        spin_factor_outbound_networking::record_address_fields(address);

        if !self.is_address_allowed(address).await.map_err(|e| {
            // The allow-list check infrastructure itself failed; that's a
            // host problem, not anything the guest did wrong.
            let err = v2::Error::Other(e.to_string());
            traces::mark_as_error(&err, Some(Blame::Host));
            err
        })? {
            // The check succeeded but returned false: the guest supplied an
            // address that isn't on the allow list.
            let err = v2::Error::ConnectionFailed(format!("address {address} is not permitted"));
            traces::mark_as_error(&err, Some(Blame::Guest));
            return Err(err);
        }
        let client = C::build_client(address).await.map_err(|e| {
            // The guest supplies the address and credentials; connection
            // failures (wrong password, TLS error, unreachable host, etc.)
            // are the guest's problem.
            let err = v2::Error::ConnectionFailed(format!("{e:?}"));
            traces::mark_as_error(&err, Some(Blame::Guest));
            err
        })?;
        self.connections
            .push((Arc::new(Mutex::new(client)), permit))
            .map_err(|_| {
                // The guest exceeded the host-imposed connection limit.
                let err = v2::Error::ConnectionFailed("too many connections".into());
                traces::mark_as_error(&err, Some(Blame::Guest));
                err
            })
    }

    fn get_client(&mut self, connection: u32) -> Result<Arc<Mutex<C>>, v2::Error> {
        self.connections
            .get(connection)
            .map(|(conn, _permit)| conn.clone())
            .ok_or_else(|| {
                // The connection table is managed entirely by the host, so a
                // missing handle indicates a host-side bug, not a guest mistake.
                let err = v2::Error::ConnectionFailed("no connection found".into());
                traces::mark_as_error(&err, Some(Blame::Host));
                err
            })
    }

    async fn is_address_allowed(&self, address: &str) -> Result<bool> {
        self.allowed_hosts.check_url(address, "mysql").await
    }
}

impl<C: Client> v3::Host for InstanceState<C> {
    fn convert_error(&mut self, error: v3::Error) -> Result<v3::Error> {
        Ok(error)
    }
}

impl<C: Client> v3::HostConnection for InstanceState<C> {
    async fn drop(&mut self, connection: Resource<v3::Connection>) -> Result<()> {
        let mut state = self.inner.lock().await;
        state.connections.remove(connection.rep());
        Ok(())
    }
}

type QueryTuple = (
    Vec<v3::Column>,
    StreamReader<v3::Row>,
    FutureReader<Result<(), v3::Error>>,
);

impl<C: Client, T> v3::HostConnectionWithStore<T> for MysqlFactorData<C> {
    #[instrument(name = "spin_outbound_mysql.open", skip(accessor, address), err(level = Level::INFO), fields(otel.kind = "client", {otel_attribute::DB_SYSTEM_NAME} = "mysql", {otel_attribute::SERVER_ADDRESS} = Empty, {otel_attribute::SERVER_PORT} = Empty, {otel_attribute::DB_NAMESPACE} = Empty))]
    async fn open(
        accessor: &Accessor<T, Self>,
        address: String,
    ) -> Result<Resource<v3::Connection>, v3::Error> {
        let (state_arc, semaphore) = accessor.with(|mut access| {
            let host = access.get();
            (host.inner.clone(), host.semaphore.clone())
        });
        let permit = semaphore
            .acquire()
            .await
            .map_err(|_| v3::Error::ConnectionFailed("too many connections".into()))?;
        let mut state = state_arc.lock().await;
        state.otel.reparent_tracing_span();
        Ok(Resource::new_own(
            state.open_connection(&address, permit).await?,
        ))
    }

    #[instrument(name = "spin_outbound_mysql.execute", skip(accessor, connection, params), err(level = Level::INFO), fields(otel.kind = "client", {otel_attribute::DB_SYSTEM_NAME} = "mysql", otel.name = statement))]
    async fn execute(
        accessor: &Accessor<T, Self>,
        connection: Resource<v3::Connection>,
        statement: String,
        params: Vec<v3::ParameterValue>,
    ) -> Result<(), v3::Error> {
        let state = accessor.with(|mut access| access.get().inner.clone());
        let client = {
            let mut state = state.lock().await;
            state.otel.reparent_tracing_span();
            state.get_client(connection.rep())?
        };
        client
            .lock()
            .await
            .execute(statement, params.into_iter().map(Into::into).collect())
            .await
            .map_err(track_db_error_on_span)?;
        Ok(())
    }

    #[instrument(name = "spin_outbound_mysql.query", skip(accessor, connection, params), err(level = Level::INFO), fields(otel.kind = "client", {otel_attribute::DB_SYSTEM_NAME} = "mysql", otel.name = statement))]
    async fn query(
        accessor: &Accessor<T, Self>,
        connection: Resource<v3::Connection>,
        statement: String,
        params: Vec<v3::ParameterValue>,
    ) -> Result<QueryTuple, v3::Error> {
        let state = accessor.with(|mut access| access.get().inner.clone());
        let client = {
            let mut state = state.lock().await;
            state.otel.reparent_tracing_span();
            state.get_client(connection.rep())?
        };

        let (columns, stream, future) =
            C::query_async(client, statement, params, MAX_HOST_BUFFERED_BYTES)
                .await
                .map_err(|v| v3::Error::from(track_db_error_on_span(v2::Error::from(v))))?;

        let (stream, future) = accessor
            .with(|mut access| {
                anyhow::Ok((
                    StreamReader::new(&mut access, spin_wasi_async::stream::producer(stream))?,
                    FutureReader::new(&mut access, future)?,
                ))
            })
            .map_err(|e| {
                // Setting up the async stream/future channels is a host
                // implementation detail; if it fails, that's a host bug.
                let err = v3::Error::Other(e.to_string());
                traces::mark_as_error(&err, Some(Blame::Host));
                err
            })?;

        Ok((columns, stream, future))
    }
}

impl<C: Client> v2::Host for InstanceState<C> {}

impl<C: Client> v2::HostConnection for InstanceState<C> {
    #[instrument(name = "spin_outbound_mysql.open", skip(self, address), err(level = Level::INFO),
        fields(otel.kind = "client", {otel_attribute::DB_SYSTEM_NAME} = "mysql", {otel_attribute::SERVER_ADDRESS} = Empty, {otel_attribute::SERVER_PORT} = Empty, {otel_attribute::DB_NAMESPACE} = Empty))]
    async fn open(&mut self, address: String) -> Result<Resource<v2::Connection>, v2::Error> {
        let permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| v2::Error::ConnectionFailed("too many connections".into()))?;
        let mut state = self.inner.lock().await;
        state.otel.reparent_tracing_span();
        state
            .open_connection(&address, permit)
            .await
            .map(Resource::new_own)
    }

    #[instrument(name = "spin_outbound_mysql.execute", skip(self, connection, params), err(level = Level::INFO),
        fields(otel.kind = "client", {otel_attribute::DB_SYSTEM_NAME} = "mysql"))]
    async fn execute(
        &mut self,
        connection: Resource<v2::Connection>,
        statement: String,
        params: Vec<v2_types::ParameterValue>,
    ) -> Result<(), v2::Error> {
        let mut state = self.inner.lock().await;
        state.otel.reparent_tracing_span();
        state
            .get_client(connection.rep())?
            .lock()
            .await
            .execute(statement, params)
            .await
            .map_err(track_db_error_on_span)
    }

    #[instrument(name = "spin_outbound_mysql.query", skip(self, connection, params), err(level = Level::INFO),
        fields(otel.kind = "client", {otel_attribute::DB_SYSTEM_NAME} = "mysql"))]
    async fn query(
        &mut self,
        connection: Resource<v2::Connection>,
        statement: String,
        params: Vec<v2_types::ParameterValue>,
    ) -> Result<v2_types::RowSet, v2::Error> {
        let mut state = self.inner.lock().await;
        state.otel.reparent_tracing_span();
        state
            .get_client(connection.rep())?
            .lock()
            .await
            .query(statement, params, MAX_HOST_BUFFERED_BYTES)
            .await
            .map_err(track_db_error_on_span)
    }

    async fn drop(&mut self, connection: Resource<v2::Connection>) -> Result<()> {
        let mut state = self.inner.lock().await;
        state.connections.remove(connection.rep());
        Ok(())
    }
}

impl<C: Send> v2_types::Host for InstanceState<C> {
    fn convert_error(&mut self, error: v2::Error) -> Result<v2::Error> {
        Ok(error)
    }
}

/// Delegate a function call to the v2::HostConnection implementation
macro_rules! delegate {
    ($self:ident.$name:ident($address:expr, $($arg:expr),*)) => {{
        let permit = $self
            .semaphore
            .acquire()
            .await
            .map_err(|_| v2::Error::ConnectionFailed("too many connections".into()))?;
        let connection = {
            let mut state = $self.inner.lock().await;
            Resource::new_own(state.open_connection(&$address, permit).await?)
        };
        // v1 has no persistent connections, so remove the table entry immediately
        // after the call to release the semaphore permit.
        let rep = connection.rep();
        let result = <Self as v2::HostConnection>::$name($self, connection, $($arg),*)
            .await
            .map_err(Into::into);
        $self.inner.lock().await.connections.remove(rep);
        result
    }};
}

impl<C: Client> v1::Host for InstanceState<C> {
    async fn execute(
        &mut self,
        address: String,
        statement: String,
        params: Vec<v1::ParameterValue>,
    ) -> Result<(), v1::MysqlError> {
        delegate!(self.execute(
            address,
            statement,
            params.into_iter().map(Into::into).collect()
        ))
    }

    async fn query(
        &mut self,
        address: String,
        statement: String,
        params: Vec<v1::ParameterValue>,
    ) -> Result<v1::RowSet, v1::MysqlError> {
        delegate!(self.query(
            address,
            statement,
            params.into_iter().map(Into::into).collect()
        ))
        .map(Into::into)
    }

    fn convert_mysql_error(&mut self, error: v1::MysqlError) -> Result<v1::MysqlError> {
        Ok(error)
    }
}

/// Only for actual DB client calls (execute/query).
/// Blame is inferred from the error variant returned by the DB driver.
fn track_db_error_on_span(err: v2::Error) -> v2::Error {
    let blame = match &err {
        // The guest brings their own database, so connection failures during
        // execution (dropped connection, auth rejected mid-session, etc.) are
        // the guest's problem, not the host's.
        v2::Error::ConnectionFailed(_) => Blame::Guest,
        v2::Error::BadParameter(_) => Blame::Guest,
        v2::Error::QueryFailed(_) => Blame::Guest,
        // The host is responsible for mapping DB wire types to WIT types;
        // a conversion failure is a host-side limitation or bug.
        v2::Error::ValueConversionFailed(_) => Blame::Host,
        v2::Error::Other(_) => Blame::Host,
    };
    traces::mark_as_error(&err, Some(blame));
    err
}
