use spin_sdk::http::{IntoResponse, Request, Response};
use spin_sdk::http_component;
use spin_sdk::pg4::Connection;

// For testing with TLS:
// mod pg41 {
//     spin_sdk::wit_bindgen::generate!({
//         path: "../../../wit/deps/spin-postgres@4.1.0",
//         inline: "package pg:pg;\nworld pg { import spin:postgres/postgres@4.1.0; }",
//         runtime_path: "spin_sdk::wit_bindgen::rt",
//         generate_all
//     });
// }

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
    // For testing without TLS:
    let db = Connection::open("host=localhost dbname=mydb")?;

    // For testing with TLS (turn on sslmode, set CA) - also enable files in spin.toml:
    // let cb = pg41::spin::postgres::postgres::ConnectionBuilder::new("host=localhost dbname=mydb sslmode=require");
    // let ca_root = std::fs::read_to_string("ca.crt")?;
    // cb.set_ca_root(&ca_root).unwrap();
    // let db = cb.build()?;

    let query_result = db.query("SELECT COUNT(*) FROM users", &[])?;
    // let count = i64::decode(&query_result.rows[0][0])?;
    // println!("User count: {}", count);
    println!("qres: {query_result:?}");
    Ok(())
}
