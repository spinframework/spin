#![deny(warnings)]

wit_bindgen::generate!({
    path: "../../../../wit",
    world: "spin:up/http-trigger@4.1.0",
    merge_structurally_equal_types: true,
    generate_all,
});

use {
    crate::{
        exports::wasi::http0_3_0::handler::Guest,
        spin::mysql::mysql,
        wasi::http0_3_0::types::{ErrorCode, Fields, Request, Response},
    },
    helper::{bail, ensure, ensure_matches, ensure_ok},
    std::env,
    wit_bindgen::rt::async_support,
};

struct Component;

export!(Component);

impl Guest for Component {
    async fn handle(_request: Request) -> Result<Response, ErrorCode> {
        Ok(match test().await {
            Ok(()) => respond(200, "success".into()),
            Err(message) => respond(500, message),
        })
    }
}

fn respond(status: u16, message: String) -> Response {
    let (mut tx, rx) = wit_stream::new();
    async_support::spawn_local(async move {
        tx.write_all(message.into_bytes()).await;
    });
    let response = Response::new(
        Fields::from_list(&[("content-type".into(), "text/plain".as_bytes().into())]).unwrap(),
        Some(rx),
        wit_future::new(|| Ok(None)).1,
    )
    .0;
    response.set_status_code(status).unwrap();
    response
}

async fn test() -> Result<(), String> {
    ensure_matches!(
        mysql::Connection::open("hello".into()).await,
        Err(mysql::Error::ConnectionFailed(_))
    );
    ensure_matches!(
        mysql::Connection::open("localhost:10000".into()).await,
        Err(mysql::Error::ConnectionFailed(_))
    );

    let address = ensure_ok!(env::var("DB_URL"));
    let conn = ensure_ok!(mysql::Connection::open(address).await);
    let rows = ensure_ok!(test_numeric_types(&conn).await);
    ensure!(rows.iter().all(|r| r.len() == 14));
    ensure!(matches!(rows[0][13], mysql::DbValue::Int8(1)));

    let rows = ensure_ok!(test_character_types(&conn).await);
    ensure!(rows.iter().all(|r| r.len() == 6));
    ensure!(matches!(rows[0][0], mysql::DbValue::Str(ref s) if s == "rvarchar"));

    ensure_ok!(
        conn.execute(
            "CREATE TEMPORARY TABLE big_text (rkey int, rvalue mediumtext);".into(),
            Vec::new()
        )
        .await
    );

    // Insert 256 copies of a 1MB string, which exceeds the 128MB query
    // result limit we impose in `factor-outbound-mysql`:
    let big_text = "y".repeat(1 << 20);
    for i in 0..256 {
        ensure_ok!(
            conn.execute(
                "INSERT INTO big_text(rkey, rvalue) VALUES(?, ?);".into(),
                vec![
                    mysql::ParameterValue::Int32(i),
                    mysql::ParameterValue::Str(big_text.clone())
                ]
            )
            .await
        );
    }

    // This should exceed the 128MB query result limit:
    let big = async {
        let (_, stream, future) = conn
            .query("SELECT * FROM big_text".into(), Vec::new())
            .await?;
        let rows = stream.collect().await;
        future.await?;
        Ok(rows)
    };

    match big.await {
        Ok(_) => bail!("large select should not have succeeded",),
        Err(mysql::Error::Other(s)) if s.contains("query result exceeds limit") => {}
        Err(e) => bail!("unexpected error: {e}",),
    }

    Ok(())
}

async fn test_numeric_types(conn: &mysql::Connection) -> Result<Vec<mysql::Row>, mysql::Error> {
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

    conn.execute(create_table_sql.into(), Vec::new()).await?;

    let insert_sql = r#"
        INSERT INTO test_numeric_types
            (rtiny, rsmall, rmedium, rint, rbig, rfloat, rdouble, rutiny, rusmall, rumedium, ruint, rubig, rtinyint1, rbool)
        VALUES
            (0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1);
    "#;

    conn.execute(insert_sql.into(), Vec::new()).await?;

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

    let (_, stream, future) = conn.query(sql.into(), Vec::new()).await?;
    let rows = stream.collect().await;
    future.await?;
    Ok(rows)
}

async fn test_character_types(conn: &mysql::Connection) -> Result<Vec<mysql::Row>, mysql::Error> {
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

    conn.execute(create_table_sql.into(), Vec::new()).await?;

    let insert_sql = r#"
        INSERT INTO test_character_types
            (rvarchar, rtext, rchar, rbinary, rvarbinary, rblob)
        VALUES
            ('rvarchar', 'rtext', 'rchar', 'a', 'a', 'a');
    "#;

    conn.execute(insert_sql.into(), Vec::new()).await?;

    let sql = r#"
        SELECT
            rvarchar, rtext, rchar, rbinary, rvarbinary, rblob
        FROM test_character_types;
    "#;

    let (_, stream, future) = conn.query(sql.into(), Vec::new()).await?;
    let rows = stream.collect().await;
    future.await?;
    Ok(rows)
}
