use super::{
    Context, TestConfig,
    mysql::{self, MysqlError},
    rdbms_types::{Column, DbDataType, DbValue, ParameterValue, RowSet},
};
use anyhow::{Result, ensure};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    iter,
};
use wasmtime::{Engine, component::InstancePre};

/// Report of which MySQL functions a module successfully used, if any
#[derive(Serialize, PartialEq, Eq, Debug)]
pub struct MysqlReport {
    /// Result of the MySQL statement execution test
    ///
    /// The guest module should expect a call according to [`super::InvocationStyle`] with \["mysql-execute",
    /// "127.0.0.1", "INSERT INTO foo (x) VALUES ($1)", "int8:42"\] as arguments.  The module should call the
    /// host-implemented `mysql::execute` function with the arguments \["127.0.0.1", "INSERT INTO foo (x) VALUES
    /// ($1)", `\[int8(42)\]`\] and expect `ok` as the result.  The host will assert that said function is called
    /// exactly once with the specified arguments.
    pub execute: Result<(), String>,

    /// Result of the MySQL query execution test
    ///
    /// The guest module should expect a call according to [`super::InvocationStyle`] with \["mysql-query",
    /// "127.0.0.1", "SELECT x FROM foo"\] as arguments.  The module should call the host-implemented
    /// `mysql::execute` function with the arguments \["127.0.0.1", "SELECT x FROM foo"\] and expect `ok({
    /// columns: \[ { name: "x", data_type: int8 } \], rows: \[ \[ int8(42) \] \]})` as the result.  The host will
    /// assert that said function is called exactly once with the specified arguments.
    pub query: Result<(), String>,
}

#[derive(Default)]
pub(crate) struct Mysql {
    execute_set: HashSet<(String, String, String)>,
    query_map: HashMap<(String, String, String), RowSet>,
}

impl mysql::Host for Mysql {
    async fn execute(
        &mut self,
        address: String,
        statement: String,
        params: Vec<ParameterValue>,
    ) -> Result<Result<(), MysqlError>> {
        Ok(
            if self.execute_set.remove(&(
                address.to_owned(),
                statement.to_owned(),
                format!("{params:?}"),
            )) {
                Ok(())
            } else {
                Err(MysqlError::OtherError(format!(
                    "expected {:?}, got {:?}",
                    self.execute_set.iter(),
                    iter::once(&(address, statement, format!("{params:?}")))
                )))
            },
        )
    }

    async fn query(
        &mut self,
        address: String,
        statement: String,
        params: Vec<ParameterValue>,
    ) -> Result<Result<RowSet, MysqlError>> {
        Ok(self
            .query_map
            .remove(&(
                address.to_owned(),
                statement.to_owned(),
                format!("{params:?}"),
            ))
            .ok_or_else(|| {
                MysqlError::OtherError(format!(
                    "expected {:?}, got {:?}",
                    self.query_map.keys(),
                    iter::once(&(address, statement, format!("{params:?}")))
                ))
            }))
    }
}

pub(crate) async fn test(
    engine: &Engine,
    test_config: TestConfig,
    pre: &InstancePre<Context>,
) -> Result<MysqlReport> {
    Ok(MysqlReport {
        execute: test_execute(engine, test_config.clone(), pre).await,
        query: test_query(engine, test_config, pre).await,
    })
}

async fn test_execute(
    engine: &Engine,
    test_config: TestConfig,
    pre: &InstancePre<Context>,
) -> Result<(), String> {
    let mut store = super::create_store_with_context(engine, test_config, |context| {
        context.mysql.execute_set.insert((
            "127.0.0.1".into(),
            "INSERT INTO foo (x) VALUES ($1)".into(),
            format!("{:?}", vec![ParameterValue::Int8(42)]),
        ));
    });

    super::run_command(
        &mut store,
        pre,
        &[
            "mysql-execute",
            "127.0.0.1",
            "INSERT INTO foo (x) VALUES ($1)",
            "int8:42",
        ],
        |store| {
            ensure!(
                store.data().mysql.execute_set.is_empty(),
                "expected module to call `mysql::execute` exactly once"
            );

            Ok(())
        },
    )
    .await
}

async fn test_query(
    engine: &Engine,
    test_config: TestConfig,
    pre: &InstancePre<Context>,
) -> Result<(), String> {
    let row_set = RowSet {
        columns: vec![Column {
            name: "x".into(),
            data_type: DbDataType::Int8,
        }],
        rows: vec![vec![DbValue::Int8(42)]],
    };

    let mut store = super::create_store_with_context(engine, test_config, |context| {
        context.mysql.query_map.insert(
            (
                "127.0.0.1".into(),
                "SELECT x FROM foo".into(),
                format!("{:?}", Vec::<()>::new()),
            ),
            row_set,
        );
    });

    super::run_command(
        &mut store,
        pre,
        &["mysql-query", "127.0.0.1", "SELECT x FROM foo"],
        |store| {
            ensure!(
                store.data().mysql.query_map.is_empty(),
                "expected module to call `mysql::query` exactly once"
            );

            Ok(())
        },
    )
    .await
}
