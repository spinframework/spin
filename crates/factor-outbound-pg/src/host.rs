#![allow(clippy::result_large_err)]

use anyhow::Result;
use spin_core::wasmtime::component::{Accessor, FutureReader, Resource, StreamReader};
use spin_telemetry::traces::{self, Blame};
use spin_world::MAX_HOST_BUFFERED_BYTES;
use spin_world::spin::postgres3_0_0::postgres::{self as v3};
use spin_world::spin::postgres4_2_0::postgres::{self as v4};
use spin_world::v1::postgres as v1;
use spin_world::v1::rdbms_types as v1_types;
use spin_world::v2::postgres::{self as v2};
use spin_world::v2::rdbms_types as v2_types;
use tracing::Level;
use tracing::field::Empty;
use tracing::instrument;

use crate::InstanceState;
use crate::allowed_hosts::AllowedHostChecker;
use crate::client::{Client, ClientFactory, HashableCertificate, QueryAsyncResult};

impl<CF: ClientFactory> InstanceState<CF> {
    async fn open_connection<Conn: 'static>(
        &mut self,
        address: &str,
        root_ca: Option<HashableCertificate>,
    ) -> Result<Resource<Conn>, v4::Error> {
        let client = self
            .client_factory
            .get_client(address, root_ca)
            .await
            .map_err(|e| {
                // The guest supplies the address and credentials; connection
                // failures (wrong password, TLS error, unreachable host, etc.)
                // are the guest's problem.
                let err = v4::Error::ConnectionFailed(format!("{e:?}"));
                traces::mark_as_error(&err, Some(Blame::Guest));
                err
            })?;
        self.connections
            .push(client)
            .map_err(|_| {
                // The guest exceeded the host-imposed connection limit.
                let err = v4::Error::ConnectionFailed("too many connections".into());
                traces::mark_as_error(&err, Some(Blame::Guest));
                err
            })
            .map(Resource::new_own)
    }

    async fn get_client<Conn: 'static>(
        &self,
        connection: Resource<Conn>,
    ) -> Result<&CF::Client, v4::Error> {
        self.connections.get(connection.rep()).ok_or_else(|| {
            // The connection table is managed entirely by the host, so a
            // missing handle indicates a host-side bug, not a guest mistake.
            let err = v4::Error::ConnectionFailed("no connection found".into());
            traces::mark_as_error(&err, Some(Blame::Host));
            err
        })
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

        self.ensure_address_allowed(&address)
            .await
            .map_err(v3::Error::from)
            .map_err(track_address_check_error_v3)?;

        self.open_connection(&address, None)
            .await
            .map_err(v3::Error::from)
    }

    #[instrument(name = "spin_outbound_pg.execute", skip(self, connection, params), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", otel.name = statement))]
    async fn execute(
        &mut self,
        connection: Resource<v3::Connection>,
        statement: String,
        params: Vec<v3::ParameterValue>,
    ) -> Result<u64, v3::Error> {
        self.get_client(connection)
            .await
            .map_err(v3::Error::from)?
            .execute(statement, v3_params_to_v4(params))
            .await
            .map_err(v3::Error::from)
            .map_err(track_db_error_on_span_v3)
    }

    #[instrument(name = "spin_outbound_pg.query", skip(self, connection, params), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", otel.name = statement))]
    async fn query(
        &mut self,
        connection: Resource<v3::Connection>,
        statement: String,
        params: Vec<v3::ParameterValue>,
    ) -> Result<v3::RowSet, v3::Error> {
        let rowset = self
            .get_client(connection)
            .await
            .map_err(v3::Error::from)?
            .query(statement, v3_params_to_v4(params), MAX_HOST_BUFFERED_BYTES)
            .await
            .map_err(v3::Error::from)
            .map_err(track_db_error_on_span_v3)?;
        Ok(rowset.into())
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
        let root_ca = HashableCertificate::from_pem(&certificate).map_err(|e| {
            let err = v4::Error::Other(format!("invalid root certificate: {e}"));
            traces::mark_as_error(&err, Some(Blame::Guest));
            err
        })?;
        let builder = self.builders.get_mut(self_.rep()).ok_or_else(|| {
            let err = v4::Error::ConnectionFailed("no builder found".into());
            traces::mark_as_error(&err, Some(Blame::Host));
            err
        })?;
        builder.root_ca = Some(root_ca);
        Ok(())
    }

    async fn build(
        &mut self,
        self_: Resource<v4::ConnectionBuilder>,
    ) -> Result<Resource<v4::Connection>, v4::Error> {
        let (address, root_ca) = self.get_builder_info(self_.rep())?;
        self.open_connection(&address, root_ca).await
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

        self.ensure_address_allowed(&address)
            .await
            .map_err(track_address_check_error_v4)?;

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
            .map_err(track_db_error_on_span_v4)
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
            .map_err(track_db_error_on_span_v4)
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

        Self::ensure_address_allowed_async(accessor, &address)
            .await
            .map_err(track_address_check_error_v4)?;
        Self::open_connection_async(accessor, &address, None).await
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

        client
            .execute(statement, params)
            .await
            .map_err(track_db_error_on_span_v4)
    }

    #[allow(clippy::type_complexity)] // blame bindgen, clippy, blame bindgen
    #[instrument(name = "spin_outbound_pg.query_async", skip(accessor, params), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", otel.name = statement))]
    async fn query_async<T>(
        accessor: &Accessor<T, Self>,
        connection: Resource<v4::Connection>,
        statement: String,
        params: Vec<v4::ParameterValue>,
    ) -> Result<
        (
            Vec<v4::Column>,
            StreamReader<v4::Row>,
            FutureReader<Result<(), v4::Error>>,
        ),
        v4::Error,
    > {
        let client = accessor.with(|mut access| {
            let host = access.get();
            host.connections.get(connection.rep()).unwrap().clone()
        });

        let QueryAsyncResult {
            columns,
            rows,
            error,
        } = client
            .query_async(statement, params, MAX_HOST_BUFFERED_BYTES)
            .await
            .map_err(track_db_error_on_span_v4)?;

        let row_producer = spin_wasi_async::stream::producer(rows);

        let (sr, efr) = accessor
            .with(|mut access| {
                let sr = StreamReader::new(&mut access, row_producer)?;
                let efr = FutureReader::new(&mut access, error)?;
                anyhow::Ok((sr, efr))
            })
            .map_err(|e| {
                // Setting up the async stream/future channels is a host
                // implementation detail; if it fails, that's a host bug.
                let err = v4::Error::Other(e.to_string());
                traces::mark_as_error(&err, Some(Blame::Host));
                err
            })?;

        Ok((columns, sr, efr))
    }
}

impl<CF: ClientFactory> InstanceState<CF> {
    #[allow(clippy::result_large_err)]
    fn get_builder_info(
        &mut self,
        builder_rep: u32,
    ) -> Result<(String, Option<HashableCertificate>), v4::Error> {
        let builder = self.builders.get_mut(builder_rep).ok_or_else(|| {
            let err = v4::Error::ConnectionFailed("no builder found".into());
            traces::mark_as_error(&err, Some(Blame::Host));
            err
        })?;

        let address = builder.address.clone();
        let root_ca = builder.root_ca.clone();

        Ok((address, root_ca))
    }
}

impl<CF: ClientFactory> crate::PgFactorData<CF> {
    #[allow(clippy::result_large_err)]
    fn get_builder_info<T>(
        accessor: &Accessor<T, Self>,
        builder: Resource<v4::ConnectionBuilder>,
    ) -> Result<(String, Option<HashableCertificate>), v4::Error> {
        let builder_rep = builder.rep();
        accessor.with(|mut access| {
            let host = access.get();
            host.get_builder_info(builder_rep)
        })
    }

    async fn ensure_address_allowed_async<T>(
        accessor: &Accessor<T, Self>,
        address: &str,
    ) -> Result<(), v4::Error> {
        // A merry dance to avoid doing the async allow check under the accessor
        let allowed_host_checker = accessor.with(|mut access| {
            let host = access.get();
            host.allowed_host_checker()
        });

        allowed_host_checker.ensure_address_allowed(address).await
    }

    async fn open_connection_async<T>(
        accessor: &Accessor<T, Self>,
        address: &str,
        root_ca: Option<HashableCertificate>,
    ) -> Result<Resource<v4::Connection>, v4::Error> {
        let cf = accessor.with(|mut access| {
            let host = access.get();
            host.client_factory.clone()
        });

        let client = cf.get_client(address, root_ca).await.map_err(|e| {
            let err = v4::Error::ConnectionFailed(format!("{e:?}"));
            traces::mark_as_error(&err, Some(Blame::Guest));
            err
        })?;

        accessor.with(|mut access| {
            let host = access.get();
            host.connections
                .push(client)
                .map_err(|_| {
                    let err = v4::Error::ConnectionFailed("too many connections".into());
                    traces::mark_as_error(&err, Some(Blame::Guest));
                    err
                })
                .map(Resource::new_own)
        })
    }
}

impl<CF: ClientFactory> spin_world::spin::postgres4_2_0::postgres::HostConnectionBuilderWithStore
    for crate::PgFactorData<CF>
{
    async fn build_async<T>(
        accessor: &Accessor<T, Self>,
        builder: Resource<v4::ConnectionBuilder>,
    ) -> Result<Resource<v4::Connection>, v4::Error> {
        let (address, root_ca) = Self::get_builder_info(accessor, builder)?;

        spin_factor_outbound_networking::record_address_fields(&address);

        Self::ensure_address_allowed_async(accessor, &address)
            .await
            .map_err(track_address_check_error_v4)?;
        Self::open_connection_async(accessor, &address, root_ca).await
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

        self.ensure_address_allowed(&address)
            .await
            .map_err(v2::Error::from)
            .map_err(track_address_check_error_v2)?;
        self.open_connection(&address, None)
            .await
            .map_err(v2::Error::from)
    }

    #[instrument(name = "spin_outbound_pg.execute", skip(self, connection, params), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", otel.name = statement))]
    async fn execute(
        &mut self,
        connection: Resource<v2::Connection>,
        statement: String,
        params: Vec<v2_types::ParameterValue>,
    ) -> Result<u64, v2::Error> {
        self.otel.reparent_tracing_span();
        let params = v2_params_to_v3(params).inspect_err(|e| {
            traces::mark_as_error(e, Some(Blame::Guest));
        })?;
        self.get_client(connection)
            .await
            .map_err(v2::Error::from)?
            .execute(statement, params)
            .await
            .map_err(v2::Error::from)
            .map_err(track_db_error_on_span_v2)
    }

    #[instrument(name = "spin_outbound_pg.query", skip(self, connection, params), err(level = Level::INFO), fields(otel.kind = "client", db.system = "postgresql", otel.name = statement))]
    async fn query(
        &mut self,
        connection: Resource<v2::Connection>,
        statement: String,
        params: Vec<v2_types::ParameterValue>,
    ) -> Result<v2_types::RowSet, v2::Error> {
        self.otel.reparent_tracing_span();
        let params = v2_params_to_v3(params).inspect_err(|e| {
            traces::mark_as_error(e, Some(Blame::Guest));
        })?;
        Ok(self
            .get_client(connection)
            .await
            .map_err(v2::Error::from)?
            .query(statement, params, MAX_HOST_BUFFERED_BYTES)
            .await
            .map_err(v2::Error::from)
            .map_err(track_db_error_on_span_v2)?
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
        delegate!(
            self.execute(
                address,
                statement,
                params
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<Vec<_>, _>>()?
            )
        )
    }

    async fn query(
        &mut self,
        address: String,
        statement: String,
        params: Vec<v1_types::ParameterValue>,
    ) -> Result<v1_types::RowSet, v1::PgError> {
        delegate!(
            self.query(
                address,
                statement,
                params
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<Vec<_>, _>>()?
            )
        )
        .map(Into::into)
    }

    fn convert_pg_error(&mut self, error: v1::PgError) -> Result<v1::PgError> {
        Ok(error)
    }
}

/// Mark errors from `ensure_address_allowed` on the current span.
///
/// Address check errors where the check infrastructure itself fails (`Other`) are Host-blamed.
/// All other errors (address not permitted, malformed address, unsupported socket type) are
/// Guest-blamed since the guest supplied the address.
fn track_address_check_error_v4(err: v4::Error) -> v4::Error {
    let blame = match &err {
        v4::Error::Other(_) => Blame::Host,
        _ => Blame::Guest,
    };
    traces::mark_as_error(&err, Some(blame));
    err
}

fn track_address_check_error_v3(err: v3::Error) -> v3::Error {
    let blame = match &err {
        v3::Error::Other(_) => Blame::Host,
        _ => Blame::Guest,
    };
    traces::mark_as_error(&err, Some(blame));
    err
}

fn track_address_check_error_v2(err: v2::Error) -> v2::Error {
    let blame = match &err {
        v2::Error::Other(_) => Blame::Host,
        _ => Blame::Guest,
    };
    traces::mark_as_error(&err, Some(blame));
    err
}

/// Mark errors from actual DB client calls (execute/query) on the current span.
fn track_db_error_on_span_v4(err: v4::Error) -> v4::Error {
    let blame = match &err {
        // The guest brings their own database, so connection failures during
        // execution (dropped connection, auth rejected mid-session, etc.) are
        // the guest's problem, not the host's.
        v4::Error::ConnectionFailed(_) => Blame::Guest,
        v4::Error::BadParameter(_) => Blame::Guest,
        v4::Error::QueryFailed(_) => Blame::Guest,
        // The host is responsible for mapping DB wire types to WIT types;
        // a conversion failure is a host-side limitation or bug.
        v4::Error::ValueConversionFailed(_) => Blame::Host,
        v4::Error::Other(_) => Blame::Host,
    };
    traces::mark_as_error(&err, Some(blame));
    err
}

fn track_db_error_on_span_v3(err: v3::Error) -> v3::Error {
    let blame = match &err {
        v3::Error::ConnectionFailed(_) => Blame::Guest,
        v3::Error::BadParameter(_) => Blame::Guest,
        v3::Error::QueryFailed(_) => Blame::Guest,
        v3::Error::ValueConversionFailed(_) => Blame::Host,
        v3::Error::Other(_) => Blame::Host,
    };
    traces::mark_as_error(&err, Some(blame));
    err
}

fn track_db_error_on_span_v2(err: v2::Error) -> v2::Error {
    let blame = match &err {
        v2::Error::ConnectionFailed(_) => Blame::Guest,
        v2::Error::BadParameter(_) => Blame::Guest,
        v2::Error::QueryFailed(_) => Blame::Guest,
        v2::Error::ValueConversionFailed(_) => Blame::Host,
        v2::Error::Other(_) => Blame::Host,
    };
    traces::mark_as_error(&err, Some(blame));
    err
}
