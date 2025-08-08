use anyhow::Result;
use spin_core::wasmtime::component::Resource;
use spin_world::spin::postgres3_0_0::postgres::{self as v3};
use spin_world::spin::postgres4_0_0::postgres::{self as v4};
use spin_world::v1::postgres as v1;
use spin_world::v1::rdbms_types as v1_types;
use spin_world::v2::postgres::{self as v2};
use spin_world::v2::rdbms_types as v2_types;
use tracing::field::Empty;
use tracing::instrument;
use tracing::Level;

use crate::client::{Client, ClientFactory};
use crate::InstanceState;

impl<CF: ClientFactory> InstanceState<CF> {
    async fn open_connection<Conn: 'static>(
        &mut self,
        address: &str,
    ) -> Result<Resource<Conn>, v4::Error> {
        self.connections
            .push(
                self.client_factory
                    .get_client(address)
                    .await
                    .map_err(|e| v4::Error::ConnectionFailed(format!("{e:?}")))?,
            )
            .map_err(|_| v4::Error::ConnectionFailed("too many connections".into()))
            .map(Resource::new_own)
    }

    async fn get_client<Conn: 'static>(
        &self,
        connection: Resource<Conn>,
    ) -> Result<&CF::Client, v4::Error> {
        self.connections
            .get(connection.rep())
            .ok_or_else(|| v4::Error::ConnectionFailed("no connection found".into()))
    }

    async fn is_address_allowed(&self, address: &str) -> Result<bool> {
        let Ok(config) = address.parse::<tokio_postgres::Config>() else {
            return Ok(false);
        };
        for (i, host) in config.get_hosts().iter().enumerate() {
            match host {
                tokio_postgres::config::Host::Tcp(address) => {
                    let ports = config.get_ports();
                    // The port we use is either:
                    // * The port at the same index as the host
                    // * The first port if there is only one port
                    let port =
                        ports
                            .get(i)
                            .or_else(|| if ports.len() == 1 { ports.get(1) } else { None });
                    let port_str = port.map(|p| format!(":{p}")).unwrap_or_default();
                    let url = format!("{address}{port_str}");
                    if !self.allowed_hosts.check_url(&url, "postgres").await? {
                        return Ok(false);
                    }
                }
                #[cfg(unix)]
                tokio_postgres::config::Host::Unix(_) => return Ok(false),
            }
        }
        Ok(true)
    }
}

fn v2_params_to_v3(
    params: Vec<v2_types::ParameterValue>,
) -> Result<Vec<v4::ParameterValue>, v2::Error> {
    params.into_iter().map(|p| p.try_into()).collect()
}

fn v3_params_to_v4(params: Vec<v3::ParameterValue>) -> Vec<v4::ParameterValue> {
    params.into_iter().map(|p| p.into()).collect()
}

impl<CF: ClientFactory> v3::HostConnection for InstanceState<CF> {
    #[instrument(name = "spin_outbound_pg.open", skip(self, address), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", db.address = Empty, server.port = Empty, db.namespace = Empty))]
    async fn open(&mut self, address: String) -> Result<Resource<v3::Connection>, v3::Error> {
        spin_factor_outbound_networking::record_address_fields(&address);

        if !self
            .is_address_allowed(&address)
            .await
            .map_err(|e| v3::Error::Other(e.to_string()))?
        {
            return Err(v3::Error::ConnectionFailed(format!(
                "address {address} is not permitted"
            )));
        }
        Ok(self.open_connection(&address).await?)
    }

    #[instrument(name = "spin_outbound_pg.execute", skip(self, connection, params), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", otel.name = statement))]
    async fn execute(
        &mut self,
        connection: Resource<v3::Connection>,
        statement: String,
        params: Vec<v3::ParameterValue>,
    ) -> Result<u64, v3::Error> {
        Ok(self
            .get_client(connection)
            .await?
            .execute(statement, v3_params_to_v4(params))
            .await?)
    }

    #[instrument(name = "spin_outbound_pg.query", skip(self, connection, params), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", otel.name = statement))]
    async fn query(
        &mut self,
        connection: Resource<v3::Connection>,
        statement: String,
        params: Vec<v3::ParameterValue>,
    ) -> Result<v3::RowSet, v3::Error> {
        Ok(self
            .get_client(connection)
            .await?
            .query(statement, v3_params_to_v4(params))
            .await?
            .into())
    }

    async fn drop(&mut self, connection: Resource<v3::Connection>) -> anyhow::Result<()> {
        self.connections.remove(connection.rep());
        Ok(())
    }
}

impl<CF: ClientFactory> v4::HostConnection for InstanceState<CF> {
    #[instrument(name = "spin_outbound_pg.open", skip(self, address), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", db.address = Empty, server.port = Empty, db.namespace = Empty))]
    async fn open(&mut self, address: String) -> Result<Resource<v4::Connection>, v4::Error> {
        spin_factor_outbound_networking::record_address_fields(&address);

        if !self
            .is_address_allowed(&address)
            .await
            .map_err(|e| v4::Error::Other(e.to_string()))?
        {
            return Err(v4::Error::ConnectionFailed(format!(
                "address {address} is not permitted"
            )));
        }
        self.open_connection(&address).await
    }

    #[instrument(name = "spin_outbound_pg.execute", skip(self, connection, params), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", otel.name = statement))]
    async fn execute(
        &mut self,
        connection: Resource<v4::Connection>,
        statement: String,
        params: Vec<v4::ParameterValue>,
    ) -> Result<u64, v4::Error> {
        self.get_client(connection)
            .await?
            .execute(statement, params)
            .await
    }

    #[instrument(name = "spin_outbound_pg.query", skip(self, connection, params), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", otel.name = statement))]
    async fn query(
        &mut self,
        connection: Resource<v4::Connection>,
        statement: String,
        params: Vec<v4::ParameterValue>,
    ) -> Result<v4::RowSet, v4::Error> {
        self.get_client(connection)
            .await?
            .query(statement, params)
            .await
    }

    async fn drop(&mut self, connection: Resource<v4::Connection>) -> anyhow::Result<()> {
        self.connections.remove(connection.rep());
        Ok(())
    }
}

impl<CF: ClientFactory> v2_types::Host for InstanceState<CF> {
    fn convert_error(&mut self, error: v2::Error) -> Result<v2::Error> {
        Ok(error)
    }
}

impl<CF: ClientFactory> v3::Host for InstanceState<CF> {
    fn convert_error(&mut self, error: v3::Error) -> Result<v3::Error> {
        Ok(error)
    }
}

impl<CF: ClientFactory> v4::Host for InstanceState<CF> {
    fn convert_error(&mut self, error: v4::Error) -> Result<v4::Error> {
        Ok(error)
    }
}

/// Delegate a function call to the v3::HostConnection implementation
macro_rules! delegate {
    ($self:ident.$name:ident($address:expr, $($arg:expr),*)) => {{
        if !$self.is_address_allowed(&$address).await.map_err(|e| v4::Error::Other(e.to_string()))? {
            return Err(v1::PgError::ConnectionFailed(format!(
                "address {} is not permitted", $address
            )));
        }
        let connection = match $self.open_connection(&$address).await {
            Ok(c) => c,
            Err(e) => return Err(e.into()),
        };
        <Self as v4::HostConnection>::$name($self, connection, $($arg),*)
            .await
            .map_err(|e| e.into())
    }};
}

impl<CF: ClientFactory> v2::Host for InstanceState<CF> {}

impl<CF: ClientFactory> v2::HostConnection for InstanceState<CF> {
    #[instrument(name = "spin_outbound_pg.open", skip(self, address), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", db.address = Empty, server.port = Empty, db.namespace = Empty))]
    async fn open(&mut self, address: String) -> Result<Resource<v2::Connection>, v2::Error> {
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
        Ok(self.open_connection(&address).await?)
    }

    #[instrument(name = "spin_outbound_pg.execute", skip(self, connection, params), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", otel.name = statement))]
    async fn execute(
        &mut self,
        connection: Resource<v2::Connection>,
        statement: String,
        params: Vec<v2_types::ParameterValue>,
    ) -> Result<u64, v2::Error> {
        Ok(self
            .get_client(connection)
            .await?
            .execute(statement, v2_params_to_v3(params)?)
            .await?)
    }

    #[instrument(name = "spin_outbound_pg.query", skip(self, connection, params), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", otel.name = statement))]
    async fn query(
        &mut self,
        connection: Resource<v2::Connection>,
        statement: String,
        params: Vec<v2_types::ParameterValue>,
    ) -> Result<v2_types::RowSet, v2::Error> {
        Ok(self
            .get_client(connection)
            .await?
            .query(statement, v2_params_to_v3(params)?)
            .await?
            .into())
    }

    async fn drop(&mut self, connection: Resource<v2::Connection>) -> anyhow::Result<()> {
        self.connections.remove(connection.rep());
        Ok(())
    }
}

impl<CF: ClientFactory> v1::Host for InstanceState<CF> {
    async fn execute(
        &mut self,
        address: String,
        statement: String,
        params: Vec<v1_types::ParameterValue>,
    ) -> Result<u64, v1::PgError> {
        delegate!(self.execute(
            address,
            statement,
            params
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?
        ))
    }

    async fn query(
        &mut self,
        address: String,
        statement: String,
        params: Vec<v1_types::ParameterValue>,
    ) -> Result<v1_types::RowSet, v1::PgError> {
        delegate!(self.query(
            address,
            statement,
            params
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?
        ))
        .map(Into::into)
    }

    fn convert_pg_error(&mut self, error: v1::PgError) -> Result<v1::PgError> {
        Ok(error)
    }
}
