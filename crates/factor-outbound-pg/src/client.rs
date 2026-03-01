use std::sync::Arc;

use anyhow::{Context, Result};
use futures::stream::TryStreamExt as _;
use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use spin_world::async_trait;
use spin_world::spin::postgres4_2_0::postgres::{
    self as v4, Column, DbValue, ParameterValue, RowSet,
};
use std::pin::pin;
use tokio_postgres::config::SslMode;
use tokio_postgres::types::ToSql;
use tokio_postgres::{NoTls, Row};

use crate::types::{convert_data_type, convert_entry, to_sql_parameter, to_sql_parameters};

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
    async fn get_client(
        &self,
        address: &str,
        root_ca: Option<HashableCertificate>,
    ) -> Result<Self::Client>;
}

#[derive(Clone)]
pub struct HashableCertificate {
    certificate: native_tls::Certificate,
    hash: String,
}

impl HashableCertificate {
    pub fn from_pem(text: &str) -> anyhow::Result<Self> {
        let cert_bytes = text.as_bytes();
        let hash = spin_common::sha256::hex_digest_from_bytes(cert_bytes);
        let certificate =
            native_tls::Certificate::from_pem(cert_bytes).context("invalid root certificate")?;
        Ok(Self { certificate, hash })
    }
}

/// A `ClientFactory` that uses a connection pool per address.
pub struct PooledTokioClientFactory {
    pools: moka::sync::Cache<(String, Option<String>), deadpool_postgres::Pool>,
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
    type Client = Arc<deadpool_postgres::Object>;

    async fn get_client(
        &self,
        address: &str,
        root_ca: Option<HashableCertificate>,
    ) -> Result<Self::Client> {
        let (root_ca, root_ca_hash) = match root_ca {
            None => (None, None),
            Some(HashableCertificate { certificate, hash }) => (Some(certificate), Some(hash)),
        };
        let pool_key = (address.to_string(), root_ca_hash);
        let pool = self
            .pools
            .try_get_with_by_ref(&pool_key, || create_connection_pool(address, root_ca))
            .map_err(ArcError)
            .context("establishing PostgreSQL connection pool")?;

        Ok(Arc::new(pool.get().await?))
    }
}

/// Creates a Postgres connection pool for the given address.
fn create_connection_pool(
    address: &str,
    root_ca: Option<native_tls::Certificate>,
) -> Result<deadpool_postgres::Pool> {
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
        let mut builder = TlsConnector::builder();
        if let Some(root_ca) = root_ca {
            builder.add_root_certificate(root_ca);
        }
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
pub trait Client: Clone + Send + Sync + 'static {
    async fn execute(
        &self,
        statement: String,
        params: Vec<ParameterValue>,
    ) -> Result<u64, v4::Error>;

    async fn query(
        &self,
        statement: String,
        params: Vec<ParameterValue>,
        max_result_bytes: usize,
    ) -> Result<RowSet, v4::Error>;

    async fn query_async(
        &self,
        statement: String,
        params: Vec<ParameterValue>,
        max_result_bytes: usize,
    ) -> Result<QueryAsyncResult, v4::Error>;
}

pub struct QueryAsyncResult {
    pub columns: Vec<v4::Column>,
    pub rows: tokio::sync::mpsc::Receiver<v4::Row>,
    pub error: tokio::sync::oneshot::Receiver<Result<(), v4::Error>>,
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
impl Client for Arc<deadpool_postgres::Object> {
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
        max_result_bytes: usize,
    ) -> Result<RowSet, v4::Error> {
        let params = to_sql_parameters(params)?;

        let mut results = pin!(self
            .as_ref()
            .query_raw(&statement, params)
            .await
            .map_err(query_failed)?);

        let mut columns = None;
        let mut byte_count = std::mem::size_of::<RowSet>();
        let mut rows = Vec::new();

        async {
            while let Some(row) = results.try_next().await? {
                if columns.is_none() {
                    columns = Some(infer_columns(&row));
                }
                let row = convert_row(&row)?;
                byte_count += row.iter().map(|v| v.memory_size()).sum::<usize>();
                if byte_count > max_result_bytes {
                    anyhow::bail!("query result exceeds limit of {max_result_bytes} bytes")
                }
                rows.push(row);
            }
            Ok(())
        }
        .await
        .map_err(|e| v4::Error::QueryFailed(v4::QueryError::Text(format!("{e:?}"))))?;

        Ok(RowSet {
            columns: columns.unwrap_or_default(),
            rows,
        })
    }

    async fn query_async(
        &self,
        statement: String,
        params: Vec<ParameterValue>,
        max_result_bytes: usize,
    ) -> Result<QueryAsyncResult, v4::Error> {
        use futures::StreamExt;

        let params = to_sql_parameters(params)?;

        let mut results = Box::pin(
            self.as_ref()
                .query_raw(&statement, params)
                .await
                .map_err(query_failed)?,
        );

        let (rows_tx, rows_rx) = tokio::sync::mpsc::channel(4);
        let (err_tx, err_rx) = tokio::sync::oneshot::channel();

        let Some(row) = results.next().await else {
            _ = err_tx.send(Ok(()));
            return Ok(QueryAsyncResult {
                columns: vec![],
                rows: rows_rx,
                error: err_rx,
            });
        };

        let row = row.map_err(query_failed)?;

        let cols = infer_columns(&row);

        // macro rather than closure to avoid taking ownership of err_tx
        macro_rules! try_send_row {
            ($row:ident) => {
                match convert_row(&$row) {
                    Ok(row) => {
                        let byte_count = row.iter().map(|v| v.memory_size()).sum::<usize>();
                        if byte_count > max_result_bytes {
                            _ = err_tx.send(Err(v4::Error::QueryFailed(v4::QueryError::Text(
                                format!("query result exceeds limit of {max_result_bytes} bytes"),
                            ))));
                            return;
                        }

                        let send_res = rows_tx.send(row).await;
                        if send_res.is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        let err = v4::Error::QueryFailed(v4::QueryError::Text(format!("{e:?}")));
                        _ = err_tx.send(Err(err));
                        return;
                    }
                }
            };
        }

        tokio::spawn(async move {
            try_send_row!(row);

            loop {
                let Some(row) = results.next().await else {
                    break;
                };

                let row = match row {
                    Ok(r) => r,
                    Err(e) => {
                        let err = query_failed(e);
                        _ = err_tx.send(Err(err));
                        return;
                    }
                };

                try_send_row!(row);
            }

            _ = err_tx.send(Ok(()));
        });

        Ok(QueryAsyncResult {
            columns: cols,
            rows: rows_rx,
            error: err_rx,
        })
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
