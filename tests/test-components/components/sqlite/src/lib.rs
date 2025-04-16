use helper::http_trigger_bindings::spin::sqlite::sqlite::{Connection, Error, Value};
use helper::{ensure_eq, ensure_matches, ensure_ok, ensure_some};

helper::define_component!(Component);

impl Component {
    fn main() -> Result<(), String> {
        ensure_matches!(Connection::open("forbidden"), Err(Error::AccessDenied));

        let conn = ensure_ok!(Connection::open("default"));

        ensure_ok!(conn.execute(
            "CREATE TABLE IF NOT EXISTS test_data(key TEXT NOT NULL, value TEXT NOT NULL);",
            &[]
        ));

        let prev_id = current_max_row_id(&conn)?;

        ensure_ok!(conn.execute(
            "INSERT INTO test_data(key, value) VALUES('my_key', 'my_value');",
            &[]
        ));

        ensure_eq!(1, conn.changes());
        ensure_eq!(prev_id + 1, conn.last_insert_rowid());

        let results = ensure_ok!(conn.execute(
            "SELECT * FROM test_data WHERE key = ?",
            &[Value::Text("my_key".to_owned())],
        ));

        ensure_eq!(1, results.rows.len());
        ensure_eq!(2, results.columns.len());

        let key_index = ensure_some!(results.columns.iter().position(|c| c == "key"));
        let value_index = ensure_some!(results.columns.iter().position(|c| c == "value"));

        let fetched_key = &results.rows[0].values[key_index];
        let fetched_value = &results.rows[0].values[value_index];

        ensure_matches!(fetched_key, Value::Text(t) if t == "my_key");
        ensure_matches!(fetched_value, Value::Text(t) if t == "my_value");

        Ok(())
    }
}

fn current_max_row_id(conn: &Connection) -> Result<i64, String> {
    let prev_id_rs = ensure_ok!(conn.execute("SELECT MAX(rowid) FROM test_data", &[]));
    let prev_id = match prev_id_rs.rows[0].values[0] {
        Value::Integer(i) => i,
        _ => 0,
    };
    Ok(prev_id)
}
