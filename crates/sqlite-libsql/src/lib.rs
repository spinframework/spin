use anyhow::Context;
use async_trait::async_trait;
use spin_factor_sqlite::Connection;
use spin_world::spin::sqlite::sqlite as v3;
use spin_world::spin::sqlite::sqlite::{self, RowResult};
use tokio::sync::OnceCell;

/// A lazy wrapper around a [`LibSqlConnection`] that implements the [`Connection`] trait.
pub struct LazyLibSqlConnection {
    url: String,
    token: String,
    // Since the libSQL client can only be created asynchronously, we wait until
    // we're in the `Connection` implementation to create. Since we only want to do
    // this once, we use a `OnceCell` to store it.
    inner: OnceCell<LibSqlConnection>,
}

impl LazyLibSqlConnection {
    pub fn new(url: String, token: String) -> Self {
        Self {
            url,
            token,
            inner: OnceCell::new(),
        }
    }

    pub async fn get_or_create_connection(&self) -> Result<&LibSqlConnection, v3::Error> {
        self.inner
            .get_or_try_init(|| async {
                LibSqlConnection::create(self.url.clone(), self.token.clone())
                    .await
                    .context("failed to create SQLite client")
            })
            .await
            .map_err(|_| v3::Error::InvalidConnection)
    }
}

#[async_trait]
impl Connection for LazyLibSqlConnection {
    async fn query(
        &self,
        query: &str,
        parameters: Vec<v3::Value>,
    ) -> Result<v3::QueryResult, v3::Error> {
        let client = self.get_or_create_connection().await?;
        client.query(query, parameters).await
    }

    async fn execute_batch(&self, statements: &str) -> anyhow::Result<()> {
        let client = self.get_or_create_connection().await?;
        client.execute_batch(statements).await
    }

    async fn changes(&self) -> Result<u64, sqlite::Error> {
        let client = self.get_or_create_connection().await?;
        Ok(client.changes())
    }

    async fn last_insert_rowid(&self) -> Result<i64, sqlite::Error> {
        let client = self.get_or_create_connection().await?;
        Ok(client.last_insert_rowid())
    }

    fn summary(&self) -> Option<String> {
        Some(format!("libSQL at {}", self.url))
    }
}

/// An open connection to a libSQL server.
#[derive(Clone)]
pub struct LibSqlConnection {
    inner: libsql::Connection,
}

impl LibSqlConnection {
    pub async fn create(url: String, token: String) -> anyhow::Result<Self> {
        let db = libsql::Builder::new_remote(url, token).build().await?;
        let inner = db.connect()?;
        Ok(Self { inner })
    }
}

impl LibSqlConnection {
    pub async fn query(
        &self,
        query: &str,
        parameters: Vec<sqlite::Value>,
    ) -> Result<sqlite::QueryResult, sqlite::Error> {
        let result = self
            .inner
            .query(query, convert_parameters(&parameters))
            .await
            .map_err(|e| sqlite::Error::Io(e.to_string()))?;

        Ok(sqlite::QueryResult {
            columns: columns(&result),
            rows: convert_rows(result)
                .await
                .map_err(|e| sqlite::Error::Io(e.to_string()))?,
        })
    }

    pub async fn execute_batch(&self, statements: &str) -> anyhow::Result<()> {
        self.inner.execute_batch(statements).await?;

        Ok(())
    }

    pub fn changes(&self) -> u64 {
        self.inner.changes()
    }

    pub fn last_insert_rowid(&self) -> i64 {
        self.inner.last_insert_rowid()
    }
}

fn columns(rows: &libsql::Rows) -> Vec<String> {
    (0..rows.column_count())
        .map(|index| rows.column_name(index).unwrap_or("").to_owned())
        .collect()
}

async fn convert_rows(mut rows: libsql::Rows) -> anyhow::Result<Vec<RowResult>> {
    let mut result_rows = vec![];

    let column_count = rows.column_count();

    while let Some(row) = rows.next().await? {
        result_rows.push(convert_row(row, column_count));
    }

    Ok(result_rows)
}

fn convert_row(row: libsql::Row, column_count: i32) -> RowResult {
    let values = (0..column_count)
        .map(|index| convert_value(row.get_value(index).unwrap()))
        .collect();
    RowResult { values }
}

fn convert_value(v: libsql::Value) -> sqlite::Value {
    use libsql::Value;

    match v {
        Value::Null => sqlite::Value::Null,
        Value::Integer(value) => sqlite::Value::Integer(value),
        Value::Real(value) => sqlite::Value::Real(value),
        Value::Text(value) => sqlite::Value::Text(value),
        Value::Blob(value) => sqlite::Value::Blob(value),
    }
}

fn convert_parameters(parameters: &[sqlite::Value]) -> Vec<libsql::Value> {
    use libsql::Value;

    parameters
        .iter()
        .map(|v| match v {
            sqlite::Value::Integer(value) => Value::Integer(*value),
            sqlite::Value::Real(value) => Value::Real(*value),
            sqlite::Value::Text(t) => Value::Text(t.clone()),
            sqlite::Value::Blob(b) => Value::Blob(b.clone()),
            sqlite::Value::Null => Value::Null,
        })
        .collect()
}
