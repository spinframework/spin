use anyhow::Context as _;
use spin_core::async_trait;
use spin_factor_sqlite::SqliteFactor;
use spin_factors::RuntimeFactors;
use spin_factors_executor::ExecutorHooks;

/// The default sqlite label
const DEFAULT_SQLITE_LABEL: &str = "default";

/// ExecutorHook for executing sqlite statements.
///
/// This executor assumes that the configured app has access to `SqliteFactor`.
/// It will silently ignore the hook if the app does not have access to `SqliteFactor`.
pub struct SqlStatementExecutorHook {
    sql_statements: Vec<String>,
}

impl SqlStatementExecutorHook {
    /// Creates a new SqlStatementExecutorHook
    ///
    /// The statements can be either a list of raw SQL statements or a list of `@{file:label}` statements.
    pub fn new(sql_statements: Vec<String>) -> Self {
        Self { sql_statements }
    }

    /// Executes the sql statements.
    pub async fn execute(&self, sqlite: &spin_factor_sqlite::AppState) -> anyhow::Result<()> {
        if self.sql_statements.is_empty() {
            return Ok(());
        }
        let get_database = |label| async move {
            sqlite
                .get_connection(label)
                .await
                .transpose()
                .with_context(|| format!("failed connect to database with label '{label}'"))
        };

        for statement in &self.sql_statements {
            if let Some(config) = statement.strip_prefix('@') {
                let (file, label) = parse_file_and_label(config)?;
                let database = get_database(label).await?.with_context(|| {
                    format!(
                        "based on the '@{config}' a registered database named '{label}' was expected but not found."
                    )
                })?;
                let sql = std::fs::read_to_string(file).with_context(|| {
                    format!("could not read file '{file}' containing sql statements")
                })?;
                database.execute_batch(&sql).await.with_context(|| {
                    format!("failed to execute sql against database '{label}' from file '{file}'")
                })?;
            } else {
                let Some(default) = get_database(DEFAULT_SQLITE_LABEL).await? else {
                    debug_assert!(false, "the '{DEFAULT_SQLITE_LABEL}' sqlite database should always be available but for some reason was not");
                    return Ok(());
                };
                default
                    .query(statement, Vec::new())
                    .await
                    .with_context(|| format!("failed to execute following sql statement against default database: '{statement}'"))?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl<F, U> ExecutorHooks<F, U> for SqlStatementExecutorHook
where
    F: RuntimeFactors,
{
    async fn configure_app(
        &self,
        configured_app: &spin_factors::ConfiguredApp<F>,
    ) -> anyhow::Result<()> {
        let Some(sqlite) = configured_app.app_state::<SqliteFactor>().ok() else {
            return Ok(());
        };
        self.execute(sqlite).await?;
        Ok(())
    }
}

/// Parses a @{file:label} sqlite statement
fn parse_file_and_label(config: &str) -> anyhow::Result<(&str, &str)> {
    let config = config.trim();
    if config.is_empty() {
        anyhow::bail!("database configuration is empty in the '@{config}' sqlite statement");
    }
    let (file, label) = match config.split_once(':') {
        Some((_, label)) if label.trim().is_empty() => {
            anyhow::bail!("database label is empty in the '@{config}' sqlite statement")
        }
        Some((file, _)) if file.trim().is_empty() => {
            anyhow::bail!("file path is empty in the '@{config}' sqlite statement")
        }
        Some((file, label)) => (file.trim(), label.trim()),
        None => (config, "default"),
    };
    Ok((file, label))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::{collections::VecDeque, sync::mpsc::Sender};

    use spin_core::async_trait;
    use spin_factor_sqlite::{Connection, ConnectionCreator};
    use spin_world::spin::sqlite::sqlite as v3;
    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn test_parse_file_and_label() {
        assert_eq!(
            parse_file_and_label("file:label").unwrap(),
            ("file", "label")
        );
        assert!(parse_file_and_label("file:").is_err());
        assert_eq!(parse_file_and_label("file").unwrap(), ("file", "default"));
        assert!(parse_file_and_label(":label").is_err());
        assert!(parse_file_and_label("").is_err());
    }

    #[tokio::test]
    async fn test_execute() {
        let sqlite_file = NamedTempFile::new().unwrap();
        std::fs::write(&sqlite_file, "select 2;").unwrap();

        let hook = SqlStatementExecutorHook::new(vec![
            "SELECT 1;".to_string(),
            format!("@{path}:label", path = sqlite_file.path().display()),
        ]);
        let (tx, rx) = std::sync::mpsc::channel();
        let creator = Arc::new(MockCreator { tx });
        let mut connection_creators = HashMap::new();
        connection_creators.insert(
            "default".into(),
            creator.clone() as Arc<dyn ConnectionCreator>,
        );
        connection_creators.insert("label".into(), creator);
        let sqlite = spin_factor_sqlite::AppState::new(Default::default(), connection_creators);
        let result = hook.execute(&sqlite).await;
        assert!(result.is_ok());

        let mut expected: VecDeque<Action> = vec![
            Action::CreateConnection("default".to_string()),
            Action::Query("SELECT 1;".to_string()),
            Action::CreateConnection("label".to_string()),
            Action::Execute("select 2;".to_string()),
        ]
        .into_iter()
        .collect();
        while let Ok(action) = rx.try_recv() {
            assert_eq!(action, expected.pop_front().unwrap(), "unexpected action");
        }

        assert!(
            expected.is_empty(),
            "Expected actions were never seen: {:?}",
            expected
        );
    }

    struct MockCreator {
        tx: Sender<Action>,
    }

    impl MockCreator {
        fn push(&self, label: &str) {
            self.tx
                .send(Action::CreateConnection(label.to_string()))
                .unwrap();
        }
    }

    #[async_trait]
    impl ConnectionCreator for MockCreator {
        async fn create_connection(
            &self,
            label: &str,
        ) -> Result<Box<dyn Connection + 'static>, v3::Error> {
            self.push(label);
            Ok(Box::new(MockConnection {
                tx: self.tx.clone(),
            }))
        }
    }

    struct MockConnection {
        tx: Sender<Action>,
    }

    #[async_trait]
    impl Connection for MockConnection {
        async fn query(
            &self,
            query: &str,
            parameters: Vec<v3::Value>,
        ) -> Result<v3::QueryResult, v3::Error> {
            self.tx.send(Action::Query(query.to_string())).unwrap();
            let _ = parameters;
            Ok(v3::QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
            })
        }

        async fn execute_batch(&self, statements: &str) -> anyhow::Result<()> {
            self.tx
                .send(Action::Execute(statements.to_string()))
                .unwrap();
            Ok(())
        }

        async fn changes(&self) -> Result<u64, v3::Error> {
            self.tx.send(Action::Changes).unwrap();
            Ok(123)
        }

        async fn last_insert_rowid(&self) -> Result<i64, v3::Error> {
            self.tx.send(Action::LastInsertRowid).unwrap();
            Ok(456)
        }
    }

    #[derive(Debug, PartialEq)]
    enum Action {
        CreateConnection(String),
        Query(String),
        Execute(String),
        Changes,
        LastInsertRowid,
    }
}
