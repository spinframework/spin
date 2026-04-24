use anyhow::Result;
use spin_core::wasmtime::component::Resource;
use spin_telemetry::traces::{self, Blame};
use spin_world::MAX_HOST_BUFFERED_BYTES;
use spin_world::v1::mysql as v1;
use spin_world::v2::mysql::{self as v2, Connection};
use spin_world::v2::rdbms_types as v2_types;
use spin_world::v2::rdbms_types::ParameterValue;
use tracing::field::Empty;
use tracing::{Level, instrument};

use crate::InstanceState;
use crate::client::Client;

impl<C: Client> InstanceState<C> {
    async fn open_connection(&mut self, address: &str) -> Result<Resource<Connection>, v2::Error> {
        let client = C::build_client(address).await.map_err(|e| {
            // The guest supplies the address and credentials; connection
            // failures (wrong password, TLS error, unreachable host, etc.)
            // are the guest's problem.
            let err = v2::Error::ConnectionFailed(format!("{e:?}"));
            traces::mark_as_error(&err, Some(Blame::Guest));
            err
        })?;
        self.connections
            .push(client)
            .map_err(|_| {
                // The guest exceeded the host-imposed connection limit.
                let err = v2::Error::ConnectionFailed("too many connections".into());
                traces::mark_as_error(&err, Some(Blame::Guest));
                err
            })
            .map(Resource::new_own)
    }

    async fn get_client(&mut self, connection: Resource<Connection>) -> Result<&mut C, v2::Error> {
        self.connections.get_mut(connection.rep()).ok_or_else(|| {
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

impl<C: Client> v2::Host for InstanceState<C> {}

impl<C: Client> v2::HostConnection for InstanceState<C> {
    #[instrument(name = "spin_outbound_mysql.open", skip(self, address), err(level = Level::INFO), fields(otel.kind = "client", db.system = "mysql", db.address = Empty, server.port = Empty, db.namespace = Empty))]
    async fn open(&mut self, address: String) -> Result<Resource<Connection>, v2::Error> {
        self.otel.reparent_tracing_span();
        spin_factor_outbound_networking::record_address_fields(&address);

        if !self.is_address_allowed(&address).await.map_err(|e| {
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
        self.open_connection(&address).await
    }

    #[instrument(name = "spin_outbound_mysql.execute", skip(self, connection, params), err(level = Level::INFO), fields(otel.kind = "client", db.system = "mysql", otel.name = statement))]
    async fn execute(
        &mut self,
        connection: Resource<Connection>,
        statement: String,
        params: Vec<ParameterValue>,
    ) -> Result<(), v2::Error> {
        self.otel.reparent_tracing_span();
        self.get_client(connection)
            .await?
            .execute(statement, params)
            .await
            .map_err(track_db_error_on_span)
    }

    #[instrument(name = "spin_outbound_mysql.query", skip(self, connection, params), err(level = Level::INFO), fields(otel.kind = "client", db.system = "mysql", otel.name = statement))]
    async fn query(
        &mut self,
        connection: Resource<Connection>,
        statement: String,
        params: Vec<ParameterValue>,
    ) -> Result<v2_types::RowSet, v2::Error> {
        self.otel.reparent_tracing_span();
        self.get_client(connection)
            .await?
            .query(statement, params, MAX_HOST_BUFFERED_BYTES)
            .await
            .map_err(track_db_error_on_span)
    }

    async fn drop(&mut self, connection: Resource<Connection>) -> Result<()> {
        self.connections.remove(connection.rep());
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
        if !$self.is_address_allowed(&$address).await.map_err(|e| {
            let err = v2::Error::Other(e.to_string());
            traces::mark_as_error(&err, Some(Blame::Host));
            err
        })? {
            let err = v2::Error::ConnectionFailed(format!("address {} is not permitted", $address));
            traces::mark_as_error(&err, Some(Blame::Guest));
            return Err(err.into());
        }
        let connection = $self.open_connection(&$address).await?;
        <Self as v2::HostConnection>::$name($self, connection, $($arg),*)
            .await
            .map_err(Into::into)
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
