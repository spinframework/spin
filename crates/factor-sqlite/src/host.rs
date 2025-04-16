use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use spin_factors::wasmtime::component::Resource;
use spin_factors::{anyhow, SelfInstanceBuilder};
use spin_world::spin::sqlite::sqlite as v3;
use spin_world::v1::sqlite as v1;
use spin_world::v2::sqlite as v2;
use tracing::field::Empty;
use tracing::{instrument, Level};

use crate::{Connection, ConnectionCreator};

pub struct InstanceState {
    allowed_databases: Arc<HashSet<String>>,
    /// A resource table of connections.
    connections: spin_resource_table::Table<Box<dyn Connection>>,
    /// A map from database label to connection creators.
    connection_creators: HashMap<String, Arc<dyn ConnectionCreator>>,
}

impl InstanceState {
    /// Create a new `InstanceState`
    ///
    /// Takes the list of allowed databases, and a function for getting a connection creator given a database label.
    pub fn new(
        allowed_databases: Arc<HashSet<String>>,
        connection_creators: HashMap<String, Arc<dyn ConnectionCreator>>,
    ) -> Self {
        Self {
            allowed_databases,
            connections: spin_resource_table::Table::new(256),
            connection_creators,
        }
    }

    /// Get a connection for a given database label.
    fn get_connection<T: 'static>(
        &self,
        connection: Resource<T>,
    ) -> Result<&dyn Connection, v3::Error> {
        self.connections
            .get(connection.rep())
            .map(|conn| conn.as_ref())
            .ok_or(v3::Error::InvalidConnection)
    }

    async fn open_impl<T: 'static>(&mut self, database: String) -> Result<Resource<T>, v3::Error> {
        if !self.allowed_databases.contains(&database) {
            return Err(v3::Error::AccessDenied);
        }
        let conn = self
            .connection_creators
            .get(&database)
            .ok_or(v3::Error::NoSuchDatabase)?
            .create_connection(&database)
            .await?;
        tracing::Span::current().record(
            "sqlite.backend",
            conn.summary().as_deref().unwrap_or("unknown"),
        );
        self.connections
            .push(conn)
            .map_err(|()| v3::Error::Io("too many connections opened".to_string()))
            .map(Resource::new_own)
    }

    async fn execute_impl<T: 'static>(
        &mut self,
        connection: Resource<T>,
        query: String,
        parameters: Vec<v3::Value>,
    ) -> Result<v3::QueryResult, v3::Error> {
        let conn = self.get_connection(connection)?;
        tracing::Span::current().record(
            "sqlite.backend",
            conn.summary().as_deref().unwrap_or("unknown"),
        );
        conn.query(&query, parameters).await
    }

    /// Get the set of allowed databases.
    pub fn allowed_databases(&self) -> &HashSet<String> {
        &self.allowed_databases
    }
}

impl SelfInstanceBuilder for InstanceState {}

impl v3::Host for InstanceState {
    fn convert_error(&mut self, error: v3::Error) -> anyhow::Result<v3::Error> {
        Ok(error)
    }
}

impl v3::HostConnection for InstanceState {
    #[instrument(name = "spin_sqlite.open", skip(self), err(level = Level::INFO), fields(otel.kind = "client", db.system = "sqlite", sqlite.backend = Empty))]
    async fn open(&mut self, database: String) -> Result<Resource<v3::Connection>, v3::Error> {
        self.open_impl(database).await
    }

    #[instrument(name = "spin_sqlite.execute", skip(self, connection, parameters), err(level = Level::INFO), fields(otel.kind = "client", db.system = "sqlite", otel.name = query, sqlite.backend = Empty))]
    async fn execute(
        &mut self,
        connection: Resource<v3::Connection>,
        query: String,
        parameters: Vec<v3::Value>,
    ) -> Result<v3::QueryResult, v3::Error> {
        self.execute_impl(connection, query, parameters).await
    }

    async fn changes(
        &mut self,
        connection: Resource<v3::Connection>,
    ) -> spin_factors::wasmtime::Result<u64> {
        let conn = match self.get_connection(connection) {
            Ok(c) => c,
            Err(err) => return Err(err.into()),
        };
        tracing::Span::current().record(
            "sqlite.backend",
            conn.summary().as_deref().unwrap_or("unknown"),
        );
        conn.changes().await.map_err(|e| e.into())
    }

    async fn last_insert_rowid(
        &mut self,
        connection: Resource<v3::Connection>,
    ) -> spin_factors::wasmtime::Result<i64> {
        let conn = match self.get_connection(connection) {
            Ok(c) => c,
            Err(err) => return Err(err.into()),
        };
        tracing::Span::current().record(
            "sqlite.backend",
            conn.summary().as_deref().unwrap_or("unknown"),
        );
        conn.last_insert_rowid().await.map_err(|e| e.into())
    }

    async fn drop(&mut self, connection: Resource<v3::Connection>) -> anyhow::Result<()> {
        let _ = self.connections.remove(connection.rep());
        Ok(())
    }
}

impl v2::Host for InstanceState {
    fn convert_error(&mut self, error: v2::Error) -> anyhow::Result<v2::Error> {
        Ok(error)
    }
}

impl v2::HostConnection for InstanceState {
    #[instrument(name = "spin_sqlite.open", skip(self), err(level = Level::INFO), fields(otel.kind = "client", db.system = "sqlite", sqlite.backend = Empty))]
    async fn open(&mut self, database: String) -> Result<Resource<v2::Connection>, v2::Error> {
        self.open_impl(database).await.map_err(to_v2_error)
    }

    #[instrument(name = "spin_sqlite.execute", skip(self, connection, parameters), err(level = Level::INFO), fields(otel.kind = "client", db.system = "sqlite", otel.name = query, sqlite.backend = Empty))]
    async fn execute(
        &mut self,
        connection: Resource<v2::Connection>,
        query: String,
        parameters: Vec<v2::Value>,
    ) -> Result<v2::QueryResult, v2::Error> {
        self.execute_impl(
            connection,
            query,
            parameters.into_iter().map(from_v2_value).collect(),
        )
        .await
        .map(to_v2_query_result)
        .map_err(to_v2_error)
    }

    async fn drop(&mut self, connection: Resource<v2::Connection>) -> anyhow::Result<()> {
        let _ = self.connections.remove(connection.rep());
        Ok(())
    }
}

impl v1::Host for InstanceState {
    async fn open(&mut self, database: String) -> Result<u32, v1::Error> {
        let result = <Self as v3::HostConnection>::open(self, database).await;
        result.map_err(to_legacy_error).map(|s| s.rep())
    }

    async fn execute(
        &mut self,
        connection: u32,
        query: String,
        parameters: Vec<spin_world::v1::sqlite::Value>,
    ) -> Result<spin_world::v1::sqlite::QueryResult, v1::Error> {
        let this = Resource::new_borrow(connection);
        let result = <Self as v3::HostConnection>::execute(
            self,
            this,
            query,
            parameters.into_iter().map(from_legacy_value).collect(),
        )
        .await;
        result.map_err(to_legacy_error).map(to_legacy_query_result)
    }

    async fn close(&mut self, connection: u32) -> anyhow::Result<()> {
        <Self as v2::HostConnection>::drop(self, Resource::new_own(connection)).await
    }

    fn convert_error(&mut self, error: v1::Error) -> anyhow::Result<v1::Error> {
        Ok(error)
    }
}

fn to_v2_error(error: v3::Error) -> v2::Error {
    match error {
        v3::Error::NoSuchDatabase => v2::Error::NoSuchDatabase,
        v3::Error::AccessDenied => v2::Error::AccessDenied,
        v3::Error::InvalidConnection => v2::Error::InvalidConnection,
        v3::Error::DatabaseFull => v2::Error::DatabaseFull,
        v3::Error::Io(s) => v2::Error::Io(s),
    }
}

fn to_legacy_error(error: v3::Error) -> v1::Error {
    match error {
        v3::Error::NoSuchDatabase => v1::Error::NoSuchDatabase,
        v3::Error::AccessDenied => v1::Error::AccessDenied,
        v3::Error::InvalidConnection => v1::Error::InvalidConnection,
        v3::Error::DatabaseFull => v1::Error::DatabaseFull,
        v3::Error::Io(s) => v1::Error::Io(s),
    }
}

fn to_v2_query_result(result: v3::QueryResult) -> v2::QueryResult {
    v2::QueryResult {
        columns: result.columns,
        rows: result.rows.into_iter().map(to_v2_row_result).collect(),
    }
}

fn to_legacy_query_result(result: v3::QueryResult) -> v1::QueryResult {
    v1::QueryResult {
        columns: result.columns,
        rows: result.rows.into_iter().map(to_legacy_row_result).collect(),
    }
}

fn to_v2_row_result(result: v3::RowResult) -> v2::RowResult {
    v2::RowResult {
        values: result.values.into_iter().map(to_v2_value).collect(),
    }
}

fn to_legacy_row_result(result: v3::RowResult) -> v1::RowResult {
    v1::RowResult {
        values: result.values.into_iter().map(to_legacy_value).collect(),
    }
}

fn to_v2_value(value: v3::Value) -> v2::Value {
    match value {
        v3::Value::Integer(i) => v2::Value::Integer(i),
        v3::Value::Real(r) => v2::Value::Real(r),
        v3::Value::Text(t) => v2::Value::Text(t),
        v3::Value::Blob(b) => v2::Value::Blob(b),
        v3::Value::Null => v2::Value::Null,
    }
}

fn to_legacy_value(value: v3::Value) -> v1::Value {
    match value {
        v3::Value::Integer(i) => v1::Value::Integer(i),
        v3::Value::Real(r) => v1::Value::Real(r),
        v3::Value::Text(t) => v1::Value::Text(t),
        v3::Value::Blob(b) => v1::Value::Blob(b),
        v3::Value::Null => v1::Value::Null,
    }
}

fn from_v2_value(value: v2::Value) -> v3::Value {
    match value {
        v2::Value::Integer(i) => v3::Value::Integer(i),
        v2::Value::Real(r) => v3::Value::Real(r),
        v2::Value::Text(t) => v3::Value::Text(t),
        v2::Value::Blob(b) => v3::Value::Blob(b),
        v2::Value::Null => v3::Value::Null,
    }
}

fn from_legacy_value(value: v1::Value) -> v3::Value {
    match value {
        v1::Value::Integer(i) => v3::Value::Integer(i),
        v1::Value::Real(r) => v3::Value::Real(r),
        v1::Value::Text(t) => v3::Value::Text(t),
        v1::Value::Blob(b) => v3::Value::Blob(b),
        v1::Value::Null => v3::Value::Null,
    }
}
