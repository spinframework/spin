use helper::http_trigger_bindings::fermyon::spin2_0_0::{mysql, rdbms_types};
use helper::{bail, ensure, ensure_matches, ensure_ok};

helper::define_component!(Component);
const DB_URL_ENV: &str = "DB_URL";

impl Component {
    fn main() -> Result<(), String> {
        ensure_matches!(
            mysql::Connection::open("hello"),
            Err(mysql::Error::ConnectionFailed(_))
        );
        ensure_matches!(
            mysql::Connection::open("localhost:10000"),
            Err(mysql::Error::ConnectionFailed(_))
        );

        let address = ensure_ok!(std::env::var(DB_URL_ENV));
        let conn = ensure_ok!(mysql::Connection::open(&address));
        let rowset = ensure_ok!(test_numeric_types(&conn));
        ensure!(rowset.rows.iter().all(|r| r.len() == 14));
        ensure!(matches!(rowset.rows[0][13], rdbms_types::DbValue::Int8(1)));

        let rowset = ensure_ok!(test_character_types(&conn));
        ensure!(rowset.rows.iter().all(|r| r.len() == 6));
        ensure!(matches!(rowset.rows[0][0], rdbms_types::DbValue::Str(ref s) if s == "rvarchar"));

        ensure_ok!(conn.execute(
            "CREATE TEMPORARY TABLE big_text (rkey int, rvalue mediumtext);",
            &[]
        ));

        // Insert 256 copies of a 1MB string, which exceeds the 128MB query
        // result limit we impose in `factor-outbound-pg`:
        let big_text = "y".repeat(1 << 20);
        for i in 0..256 {
            ensure_ok!(conn.execute(
                "INSERT INTO big_text(rkey, rvalue) VALUES(?, ?);",
                &[
                    rdbms_types::ParameterValue::Int32(i),
                    rdbms_types::ParameterValue::Str(big_text.clone())
                ]
            ));
        }

        // This should exceed the 128MB query result limit:
        match conn.query("SELECT * FROM big_text", &[]) {
            Ok(_) => bail!("large select should not have succeeded",),
            Err(rdbms_types::Error::Other(s)) if s.contains("query result exceeds limit") => {}
            Err(e) => bail!("unexpected error: {e}",),
        }

        Ok(())
    }
}

fn test_numeric_types(conn: &mysql::Connection) -> Result<mysql::RowSet, mysql::Error> {
    let create_table_sql = r#"
        CREATE TEMPORARY TABLE test_numeric_types (
            rtiny TINYINT NOT NULL,
            rsmall SMALLINT NOT NULL,
            rmedium MEDIUMINT NOT NULL,
            rint INT NOT NULL,
            rbig BIGINT NOT NULL,
            rfloat FLOAT NOT NULL,
            rdouble DOUBLE NOT NULL,
            rutiny TINYINT UNSIGNED NOT NULL,
            rusmall SMALLINT UNSIGNED NOT NULL,
            rumedium MEDIUMINT UNSIGNED NOT NULL,
            ruint INT UNSIGNED NOT NULL,
            rubig BIGINT UNSIGNED NOT NULL,
            rtinyint1 TINYINT(1) NOT NULL,
            rbool BOOLEAN NOT NULL
         );
    "#;

    conn.execute(create_table_sql, &[])?;

    let insert_sql = r#"
        INSERT INTO test_numeric_types
            (rtiny, rsmall, rmedium, rint, rbig, rfloat, rdouble, rutiny, rusmall, rumedium, ruint, rubig, rtinyint1, rbool)
        VALUES
            (0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1);
    "#;

    conn.execute(insert_sql, &[])?;

    let sql = r#"
        SELECT
            rtiny,
            rsmall,
            rmedium,
            rint,
            rbig,
            rfloat,
            rdouble,
            rutiny,
            rusmall,
            rumedium,
            ruint,
            rubig,
            rtinyint1,
            rbool
        FROM test_numeric_types;
    "#;

    conn.query(sql, &[])
}

fn test_character_types(conn: &mysql::Connection) -> Result<mysql::RowSet, mysql::Error> {
    let create_table_sql = r#"
        CREATE TEMPORARY TABLE test_character_types (
            rvarchar varchar(40) NOT NULL,
            rtext text NOT NULL,
            rchar char(10) NOT NULL,
            rbinary binary(10) NOT NULL,
            rvarbinary varbinary(10) NOT NULL,
            rblob BLOB NOT NULL
         );
    "#;

    conn.execute(create_table_sql, &[])?;

    let insert_sql = r#"
        INSERT INTO test_character_types
            (rvarchar, rtext, rchar, rbinary, rvarbinary, rblob)
        VALUES
            ('rvarchar', 'rtext', 'rchar', 'a', 'a', 'a');
    "#;

    conn.execute(insert_sql, &[])?;

    let sql = r#"
        SELECT
            rvarchar, rtext, rchar, rbinary, rvarbinary, rblob
        FROM test_character_types;
    "#;

    conn.query(sql, &[])
}
