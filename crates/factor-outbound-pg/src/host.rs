use anyhow::Result;
use spin_core::wasmtime;
use spin_core::wasmtime::component::{Accessor, FutureReader, Resource, StreamReader};
use spin_world::spin::postgres3_0_0::postgres::{self as v3};
use spin_world::spin::postgres4_2_0::postgres::{self as v4};
use spin_world::v1::postgres as v1;
use spin_world::v1::rdbms_types as v1_types;
use spin_world::v2::postgres::{self as v2};
use spin_world::v2::rdbms_types as v2_types;
use spin_world::MAX_HOST_BUFFERED_BYTES;
use tracing::field::Empty;
use tracing::instrument;
use tracing::Level;

use crate::allowed_hosts::AllowedHostChecker;
use crate::client::{Client, ClientFactory, HashableCertificate};
use crate::InstanceState;

// Declare some types to make Clippy less mad
pub type RowStream = StreamReader<Result<v4::Row, v4::Error>>;
pub type ColumnsFuture = FutureReader<Vec<v4::Column>>;

impl<CF: ClientFactory> InstanceState<CF> {
    async fn open_connection<Conn: 'static>(
        &mut self,
        address: &str,
        root_ca: Option<HashableCertificate>,
    ) -> Result<Resource<Conn>, v4::Error> {
        self.connections
            .push(
                self.client_factory
                    .get_client(address, root_ca)
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

    fn allowed_host_checker(&self) -> AllowedHostChecker {
        self.allowed_host_checker.clone()
    }

    #[allow(clippy::result_large_err)]
    async fn ensure_address_allowed(&self, address: &str) -> Result<(), v4::Error> {
        self.allowed_host_checker
            .ensure_address_allowed(address)
            .await
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

        self.ensure_address_allowed(&address).await?;

        Ok(self.open_connection(&address, None).await?)
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
            .query(statement, v3_params_to_v4(params), MAX_HOST_BUFFERED_BYTES)
            .await?
            .into())
    }

    async fn drop(&mut self, connection: Resource<v3::Connection>) -> anyhow::Result<()> {
        self.connections.remove(connection.rep());
        Ok(())
    }
}

pub(crate) struct ConnectionBuilder {
    address: String,
    root_ca: Option<HashableCertificate>,
}

impl<CF: ClientFactory> v4::HostConnectionBuilder for InstanceState<CF> {
    async fn new(&mut self, address: String) -> Result<Resource<v4::ConnectionBuilder>> {
        let builder = ConnectionBuilder {
            address,
            root_ca: None,
        };
        let rep = self
            .builders
            .push(builder)
            .map_err(|_| anyhow::anyhow!("out of builder table space"))?;
        let rsrc = Resource::new_own(rep);
        Ok(rsrc)
    }

    async fn set_ca_root(
        &mut self,
        self_: Resource<v4::ConnectionBuilder>,
        certificate: String,
    ) -> Result<(), v4::Error> {
        let root_ca = HashableCertificate::from_pem(&certificate)
            .map_err(|e| v4::Error::Other(format!("invalid root certificate: {e}")))?;
        let builder = self
            .builders
            .get_mut(self_.rep())
            .ok_or_else(|| v4::Error::ConnectionFailed("no builder found".into()))?;
        builder.root_ca = Some(root_ca);
        Ok(())
    }

    async fn build(
        &mut self,
        self_: Resource<v4::ConnectionBuilder>,
    ) -> Result<Resource<v4::Connection>, v4::Error> {
        let builder = self
            .builders
            .get_mut(self_.rep())
            .ok_or_else(|| v4::Error::ConnectionFailed("no builder found".into()))?;
        // borrow checker gets pedantic here, so we need to outsmart it
        let address = builder.address.clone();
        let root_ca = builder.root_ca.clone();
        let conn = self.open_connection(&address, root_ca).await;
        conn
    }

    async fn drop(&mut self, builder: Resource<v4::ConnectionBuilder>) -> Result<()> {
        self.builders.remove(builder.rep());
        Ok(())
    }
}

impl<CF: ClientFactory> v4::HostConnection for InstanceState<CF> {
    #[instrument(name = "spin_outbound_pg.open", skip(self, address), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", db.address = Empty, server.port = Empty, db.namespace = Empty))]
    async fn open(&mut self, address: String) -> Result<Resource<v4::Connection>, v4::Error> {
        spin_factor_outbound_networking::record_address_fields(&address);

        self.ensure_address_allowed(&address).await?;

        self.open_connection(&address, None).await
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
            .query(statement, params, MAX_HOST_BUFFERED_BYTES)
            .await
    }

    async fn drop(&mut self, connection: Resource<v4::Connection>) -> anyhow::Result<()> {
        self.connections.remove(connection.rep());
        Ok(())
    }
}

impl<CF: ClientFactory> spin_world::spin::postgres4_2_0::postgres::HostConnectionWithStore
    for crate::PgFactorData<CF>
{
    #[instrument(name = "spin_outbound_pg.open_async", skip(accessor, address), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", db.address = Empty, server.port = Empty, db.namespace = Empty))]
    async fn open_async<T>(
        accessor: &Accessor<T, Self>,
        address: String,
    ) -> Result<Resource<v4::Connection>, v4::Error> {
        spin_factor_outbound_networking::record_address_fields(&address);

        // A merry dance to avoid doing the async allow check under the accessor
        let allowed_host_checker = accessor.with(|mut access| {
            let host = access.get();
            host.allowed_host_checker()
        });

        allowed_host_checker
            .ensure_address_allowed(&address)
            .await?;

        let cf = accessor.with(|mut access| {
            let host = access.get();
            host.client_factory.clone()
        });
        let client = cf
            .get_client(&address, None)
            .await
            .map_err(|e| v4::Error::ConnectionFailed(format!("{e:?}")))?;
        let rsrc = accessor.with(|mut access| {
            let host = access.get();
            host.connections
                .push(client)
                .map_err(|_| v4::Error::ConnectionFailed("too many connections".into()))
                .map(wasmtime::component::Resource::new_own)
        });
        rsrc
    }

    #[instrument(name = "spin_outbound_pg.execute", skip(accessor, connection, params), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", otel.name = statement))]
    async fn execute_async<T>(
        accessor: &Accessor<T, Self>,
        connection: Resource<v4::Connection>,
        statement: String,
        params: Vec<v4::ParameterValue>,
    ) -> Result<u64, v4::Error> {
        let client = accessor.with(|mut access| {
            let host = access.get();
            host.connections.get(connection.rep()).unwrap().clone()
        });

        client.execute(statement, params).await
    }

    #[instrument(name = "spin_outbound_pg.query", skip(accessor, params), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", otel.name = statement))]
    async fn query_async<T>(
        accessor: &Accessor<T, Self>,
        connection: Resource<v4::Connection>,
        statement: String,
        params: Vec<v4::ParameterValue>,
    ) -> Result<(ColumnsFuture, RowStream), v4::Error> {
        use wasmtime::AsContextMut;

        let client = accessor.with(|mut access| {
            let host = access.get();
            host.connections.get(connection.rep()).unwrap().clone()
        });

        let (col_rx, row_rx) = client.query_async(statement, params).await?;

        let row_producer = spin_wasi_async::stream::producer(row_rx);
        let col_producer = spin_wasi_async::future::producer(col_rx);

        let (fr, sr) = accessor.with(|mut access| {
            let fr = FutureReader::new(access.as_context_mut(), col_producer);
            let sr = StreamReader::new(access.as_context_mut(), row_producer);
            (fr, sr)
        });

        Ok((fr, sr))
    }
}

impl<CF: ClientFactory> spin_world::spin::postgres4_2_0::postgres::HostConnectionBuilderWithStore
    for crate::PgFactorData<CF>
{
    async fn build_async<T>(
        accessor: &Accessor<T, Self>,
        builder: Resource<v4::ConnectionBuilder>,
    ) -> Result<Resource<v4::Connection>, v4::Error> {
        // TODO: so much deduplicating
        let rep = builder.rep();

        let (address, root_ca) = accessor.with(|mut access| {
            let host = access.get();

            let builder = host
                .builders
                .get_mut(rep)
                .ok_or_else(|| v4::Error::ConnectionFailed("no builder found".into()))?;

            let address = builder.address.clone();
            let root_ca = builder.root_ca.clone();

            Ok((address, root_ca))
        })?;

        // TODO: this is from open_async. TODO: so much deduplication

        spin_factor_outbound_networking::record_address_fields(&address);

        // A merry dance to avoid doing the async allow check under the accessor
        let allowed_host_checker = accessor.with(|mut access| {
            let host = access.get();
            host.allowed_host_checker()
        });

        allowed_host_checker
            .ensure_address_allowed(&address)
            .await?;

        let cf = accessor.with(|mut access| {
            let host = access.get();
            host.client_factory.clone()
        });
        let client = cf
            .get_client(&address, root_ca)
            .await
            .map_err(|e| v4::Error::ConnectionFailed(format!("{e:?}")))?;
        let rsrc = accessor.with(|mut access| {
            let host = access.get();
            host.connections
                .push(client)
                .map_err(|_| v4::Error::ConnectionFailed("too many connections".into()))
                .map(wasmtime::component::Resource::new_own)
        });
        rsrc
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
        $self.ensure_address_allowed(&$address).await?;
        let connection = match $self.open_connection(&$address, None).await {
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
        self.otel.reparent_tracing_span();
        spin_factor_outbound_networking::record_address_fields(&address);

        self.ensure_address_allowed(&address).await?;

        Ok(self.open_connection(&address, None).await?)
    }

    #[instrument(name = "spin_outbound_pg.execute", skip(self, connection, params), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", otel.name = statement))]
    async fn execute(
        &mut self,
        connection: Resource<v2::Connection>,
        statement: String,
        params: Vec<v2_types::ParameterValue>,
    ) -> Result<u64, v2::Error> {
        self.otel.reparent_tracing_span();
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
        self.otel.reparent_tracing_span();
        Ok(self
            .get_client(connection)
            .await?
            .query(statement, v2_params_to_v3(params)?, MAX_HOST_BUFFERED_BYTES)
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
