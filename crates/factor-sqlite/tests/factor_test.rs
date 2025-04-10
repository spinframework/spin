use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use spin_factor_sqlite::{RuntimeConfig, SqliteFactor};
use spin_factors::{
    anyhow::{self, bail, Context as _},
    RuntimeFactors,
};
use spin_factors_test::{toml, TestEnvironment};
use spin_world::{async_trait, spin::sqlite::sqlite as v3, v2::sqlite as v2};
use v2::HostConnection as _;

#[derive(RuntimeFactors)]
struct TestFactors {
    sqlite: SqliteFactor,
}

#[tokio::test]
async fn errors_when_non_configured_database_used() -> anyhow::Result<()> {
    let factors = TestFactors {
        sqlite: SqliteFactor::new(),
    };
    let env = TestEnvironment::new(factors).extend_manifest(toml! {
        [component.test-component]
        source = "does-not-exist.wasm"
        sqlite_databases = ["foo"]
    });
    let Err(err) = env.build_instance_state().await else {
        bail!("Expected build_instance_state to error but it did not");
    };

    assert!(err
        .to_string()
        .contains("One or more components use SQLite databases which are not defined."));

    Ok(())
}

#[tokio::test]
async fn errors_when_database_not_allowed() -> anyhow::Result<()> {
    let factors = TestFactors {
        sqlite: SqliteFactor::new(),
    };
    let env = TestEnvironment::new(factors).extend_manifest(toml! {
        [component.test-component]
        source = "does-not-exist.wasm"
        sqlite_databases = []
    });
    let mut state = env
        .build_instance_state()
        .await
        .context("build_instance_state failed")?;

    assert!(matches!(
        state.sqlite.open("foo".into()).await,
        Err(spin_world::v2::sqlite::Error::AccessDenied)
    ));

    Ok(())
}

#[tokio::test]
async fn it_works_when_database_is_configured() -> anyhow::Result<()> {
    let factors = TestFactors {
        sqlite: SqliteFactor::new(),
    };
    let mut connection_creators = HashMap::new();
    connection_creators.insert("foo".to_owned(), Arc::new(MockConnectionCreator) as _);
    let runtime_config = TestFactorsRuntimeConfig {
        sqlite: Some(RuntimeConfig {
            connection_creators,
        }),
    };
    let env = TestEnvironment::new(factors)
        .extend_manifest(toml! {
            [component.test-component]
            source = "does-not-exist.wasm"
            sqlite_databases = ["foo"]
        })
        .runtime_config(runtime_config)?;

    let mut state = env
        .build_instance_state()
        .await
        .context("build_instance_state failed")?;

    assert_eq!(
        state.sqlite.allowed_databases(),
        &["foo".into()].into_iter().collect::<HashSet<_>>()
    );

    assert!(state.sqlite.open("foo".into()).await.is_ok());
    Ok(())
}

/// A connection creator that returns a mock connection.
struct MockConnectionCreator;

#[async_trait]
impl spin_factor_sqlite::ConnectionCreator for MockConnectionCreator {
    async fn create_connection(
        &self,
        label: &str,
    ) -> Result<Box<dyn spin_factor_sqlite::Connection + 'static>, v3::Error> {
        let _ = label;
        Ok(Box::new(MockConnection))
    }
}

/// A mock connection that always errors.
struct MockConnection;

#[async_trait]
impl spin_factor_sqlite::Connection for MockConnection {
    async fn query(
        &self,
        query: &str,
        parameters: Vec<v3::Value>,
    ) -> Result<v3::QueryResult, v3::Error> {
        let _ = (query, parameters);
        Err(v3::Error::Io("Mock connection".into()))
    }

    async fn execute_batch(&self, statements: &str) -> anyhow::Result<()> {
        let _ = statements;
        bail!("Mock connection")
    }

    async fn changes(&self) -> Result<u64, v3::Error> {
        Ok(123)
    }

    async fn last_insert_rowid(&self) -> Result<i64, v3::Error> {
        Ok(456)
    }
}
