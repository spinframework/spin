use anyhow::{Context, Result};
use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use spin_world::async_trait;
use spin_world::spin::postgres4_0_0::postgres::{
    self as v4, Column, DbValue, ParameterValue, RowSet,
};
use tokio_postgres::types::ToSql;
use tokio_postgres::{config::SslMode, NoTls, Row};

use crate::types::{convert_data_type, convert_entry, to_sql_parameter};

/// Max connections in a given address' connection pool
const CONNECTION_POOL_SIZE: usize = 64;
/// Max addresses for which to keep pools in cache.
const CONNECTION_POOL_CACHE_CAPACITY: u64 = 16;

/// A factory object for Postgres clients. This abstracts
/// details of client creation such as pooling.
#[async_trait]
pub trait ClientFactory: Default + Send + Sync + 'static {
    /// The type of client produced by `get_client`.
    type Client: Client;
    /// Gets a client from the factory.
    async fn get_client(&self, address: &str) -> Result<Self::Client>;
}

/// A `ClientFactory` that uses a connection pool per address.
pub struct PooledTokioClientFactory {
    pools: moka::sync::Cache<String, deadpool_postgres::Pool>,
}

impl Default for PooledTokioClientFactory {
    fn default() -> Self {
        Self {
            pools: moka::sync::Cache::new(CONNECTION_POOL_CACHE_CAPACITY),
        }
    }
}

#[async_trait]
impl ClientFactory for PooledTokioClientFactory {
    type Client = deadpool_postgres::Object;

    async fn get_client(&self, address: &str) -> Result<Self::Client> {
        let pool = self
            .pools
            .try_get_with_by_ref(address, || create_connection_pool(address))
            .map_err(ArcError)
            .context("establishing PostgreSQL connection pool")?;

        Ok(pool.get().await?)
    }
}

/// Creates a Postgres connection pool for the given address.
fn create_connection_pool(address: &str) -> Result<deadpool_postgres::Pool> {
    let config = address
        .parse::<tokio_postgres::Config>()
        .context("parsing Postgres connection string")?;

    tracing::debug!("Build new connection: {}", address);

    let mgr_config = deadpool_postgres::ManagerConfig {
        recycling_method: deadpool_postgres::RecyclingMethod::Clean,
    };

    let mgr = if config.get_ssl_mode() == SslMode::Disable {
        deadpool_postgres::Manager::from_config(config, NoTls, mgr_config)
    } else {
        let builder = TlsConnector::builder();
        let connector = MakeTlsConnector::new(builder.build()?);
        deadpool_postgres::Manager::from_config(config, connector, mgr_config)
    };

    // TODO: what is our max size heuristic?  Should this be passed in so that different
    // hosts can manage it according to their needs?  Will a plain number suffice for
    // sophisticated hosts anyway?
    let pool = deadpool_postgres::Pool::builder(mgr)
        .max_size(CONNECTION_POOL_SIZE)
        .build()
        .context("building Postgres connection pool")?;

    Ok(pool)
}

#[async_trait]
pub trait Client: Send + Sync + 'static {
    async fn execute(
        &self,
        statement: String,
        params: Vec<ParameterValue>,
    ) -> Result<u64, v4::Error>;

    async fn query(
        &self,
        statement: String,
        params: Vec<ParameterValue>,
    ) -> Result<RowSet, v4::Error>;
}

/// Extract weak-typed error data for WIT purposes
fn pg_extras(dbe: &tokio_postgres::error::DbError) -> Vec<(String, String)> {
    let mut extras = vec![];

    macro_rules! pg_extra {
        ( $n:ident ) => {
            if let Some(value) = dbe.$n() {
                extras.push((stringify!($n).to_owned(), value.to_string()));
            }
        };
    }

    pg_extra!(column);
    pg_extra!(constraint);
    pg_extra!(routine);
    pg_extra!(hint);
    pg_extra!(table);
    pg_extra!(datatype);
    pg_extra!(schema);
    pg_extra!(file);
    pg_extra!(line);
    pg_extra!(where_);

    extras
}

fn query_failed(e: tokio_postgres::error::Error) -> v4::Error {
    let flattened = format!("{e:?}");
    let query_error = match e.as_db_error() {
        None => v4::QueryError::Text(flattened),
        Some(dbe) => v4::QueryError::DbError(v4::DbError {
            as_text: flattened,
            severity: dbe.severity().to_owned(),
            code: dbe.code().code().to_owned(),
            message: dbe.message().to_owned(),
            detail: dbe.detail().map(|s| s.to_owned()),
            extras: pg_extras(dbe),
        }),
    };
    v4::Error::QueryFailed(query_error)
}

#[async_trait]
impl Client for deadpool_postgres::Object {
    async fn execute(
        &self,
        statement: String,
        params: Vec<ParameterValue>,
    ) -> Result<u64, v4::Error> {
        let params = params
            .iter()
            .map(to_sql_parameter)
            .collect::<Result<Vec<_>>>()
            .map_err(|e| v4::Error::ValueConversionFailed(format!("{e:?}")))?;

        let params_refs: Vec<&(dyn ToSql + Sync)> = params
            .iter()
            .map(|b| b.as_ref() as &(dyn ToSql + Sync))
            .collect();

        self.as_ref()
            .execute(&statement, params_refs.as_slice())
            .await
            .map_err(query_failed)
    }

    async fn query(
        &self,
        statement: String,
        params: Vec<ParameterValue>,
    ) -> Result<RowSet, v4::Error> {
        let params = params
            .iter()
            .map(to_sql_parameter)
            .collect::<Result<Vec<_>>>()
            .map_err(|e| v4::Error::BadParameter(format!("{e:?}")))?;

        let params_refs: Vec<&(dyn ToSql + Sync)> = params
            .iter()
            .map(|b| b.as_ref() as &(dyn ToSql + Sync))
            .collect();

        let results = self
            .as_ref()
            .query(&statement, params_refs.as_slice())
            .await
            .map_err(query_failed)?;

        if results.is_empty() {
            return Ok(RowSet {
                columns: vec![],
                rows: vec![],
            });
        }

        let columns = infer_columns(&results[0]);
        let rows = results
            .iter()
            .map(convert_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| v4::Error::QueryFailed(v4::QueryError::Text(format!("{e:?}"))))?;

        Ok(RowSet { columns, rows })
    }
}

fn infer_columns(row: &Row) -> Vec<Column> {
    let mut result = Vec::with_capacity(row.len());
    for index in 0..row.len() {
        result.push(infer_column(row, index));
    }
    result
}

fn infer_column(row: &Row, index: usize) -> Column {
    let column = &row.columns()[index];
    let name = column.name().to_owned();
    let data_type = convert_data_type(column.type_());
    Column { name, data_type }
}

fn convert_row(row: &Row) -> anyhow::Result<Vec<DbValue>> {
    let mut result = Vec::with_capacity(row.len());
    for index in 0..row.len() {
        result.push(convert_entry(row, index)?);
    }
    Ok(result)
}

/// Workaround for moka returning Arc<Error> which, although
/// necessary for concurrency, does not play well with others.
struct ArcError(std::sync::Arc<anyhow::Error>);

impl std::error::Error for ArcError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl std::fmt::Debug for ArcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}

impl std::fmt::Display for ArcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}
