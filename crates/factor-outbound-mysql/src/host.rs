use anyhow::Result;
use spin_core::wasmtime::component::Resource;
use spin_world::v1::mysql as v1;
use spin_world::v2::mysql::{self as v2, Connection};
use spin_world::v2::rdbms_types as v2_types;
use spin_world::v2::rdbms_types::ParameterValue;
use tracing::field::Empty;
use tracing::{instrument, Level};

use crate::client::Client;
use crate::InstanceState;

impl<C: Client> InstanceState<C> {
    async fn open_connection(&mut self, address: &str) -> Result<Resource<Connection>, v2::Error> {
        self.connections
            .push(
                C::build_client(address)
                    .await
                    .map_err(|e| v2::Error::ConnectionFailed(format!("{e:?}")))?,
            )
            .map_err(|_| v2::Error::ConnectionFailed("too many connections".into()))
            .map(Resource::new_own)
    }

    async fn get_client(&mut self, connection: Resource<Connection>) -> Result<&mut C, v2::Error> {
        self.connections
            .get_mut(connection.rep())
            .ok_or_else(|| v2::Error::ConnectionFailed("no connection found".into()))
    }

    async fn is_address_allowed(&self, address: &str) -> Result<bool> {
        self.allowed_hosts.check_url(address, "mysql").await
    }
}

impl<C: Client> v2::Host for InstanceState<C> {}

impl<C: Client> v2::HostConnection for InstanceState<C> {
    #[instrument(name = "spin_outbound_mysql.open", skip(self, address), err(level = Level::INFO), fields(otel.kind = "client", db.system = "mysql", db.address = Empty, server.port = Empty, db.namespace = Empty))]
    async fn open(&mut self, address: String) -> Result<Resource<Connection>, v2::Error> {
        self.otel_context.reparent_tracing_span();
        spin_factor_outbound_networking::record_address_fields(&address);

        if !self
            .is_address_allowed(&address)
            .await
            .map_err(|e| v2::Error::Other(e.to_string()))?
        {
            return Err(v2::Error::ConnectionFailed(format!(
                "address {address} is not permitted"
            )));
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
        self.otel_context.reparent_tracing_span();
        self.get_client(connection)
            .await?
            .execute(statement, params)
            .await
    }

    #[instrument(name = "spin_outbound_mysql.query", skip(self, connection, params), err(level = Level::INFO), fields(otel.kind = "client", db.system = "mysql", otel.name = statement))]
    async fn query(
        &mut self,
        connection: Resource<Connection>,
        statement: String,
        params: Vec<ParameterValue>,
    ) -> Result<v2_types::RowSet, v2::Error> {
        self.otel_context.reparent_tracing_span();
        self.get_client(connection)
            .await?
            .query(statement, params)
            .await
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
        if !$self.is_address_allowed(&$address).await.map_err(|e| v2::Error::Other(e.to_string()))? {
            return Err(v1::MysqlError::ConnectionFailed(format!(
                "address {} is not permitted", $address
            )));
        }
        let connection = match $self.open_connection(&$address).await {
            Ok(c) => c,
            Err(e) => return Err(e.into()),
        };
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
