use spin_sdk::http::{IntoResponse, Request, Response};
use spin_sdk::http_component;
use spin_sdk::pg4::{Connection, Decode};

/// A simple Spin HTTP component.
#[http_component]
fn handle_pg_app(_req: Request) -> anyhow::Result<impl IntoResponse> {
    do_db_operation()?;
    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/plain")
        .body("Done")
        .build())
}

fn do_db_operation() -> anyhow::Result<()> {
    // Works without TLS
    let db = Connection::open("host=localhost dbname=mydb")?;
    // Fails when TLS is configured AND CA is set
    // let db = Connection::open("host=localhost dbname=mydb sslmode=require")?;
    let query_result = db.query("SELECT COUNT(*) FROM users", &[])?;
    let count = i64::decode(&query_result.rows[0][0])?;
    println!("User count: {}", count);
    Ok(())
}