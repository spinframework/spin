{%- case http-router -%}
{%- when "spin" -%}
// For the built-in router documentation refer to
// https://spinframework.dev/rust-components#the-spin-http-router
use spin_sdk::http::router::{Params, Router};
use spin_sdk::http::{IntoResponse, Request, StatusCode};
use spin_sdk::http_service;

/// A simple Spin HTTP component using the built-in router.
#[http_service]
async fn handle_{{project-name | snake_case}}(req: Request) -> impl IntoResponse {
    let mut router = Router::new();
    router.get("/", index);
    router.get("/hello/:name", hello);
    router.any("/*", not_found);
    router.handle(req).await
}

async fn index(_req: Request, _params: Params) -> impl IntoResponse {
    (StatusCode::OK, "Hello, Spin!")
}

async fn hello(_req: Request, params: Params) -> impl IntoResponse {
    let name = params.get("name").unwrap_or("world").to_owned();
    (StatusCode::OK, format!("Hello, {name}!"))
}

async fn not_found(_req: Request, _params: Params) -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "Not Found")
}
{%- when "axum" -%}
// For Axum documentation refer to https://docs.rs/axum/latest/axum/
use axum::extract::Path;
use axum::routing::get;
use axum::Router;
use spin_sdk::http::{IntoResponse, Request};
use spin_sdk::http_service;
use tower_service::Service;

/// A simple Spin HTTP component using the Axum router.
#[http_service]
async fn handle_{{project-name | snake_case}}(req: Request) -> impl IntoResponse {
    Router::new()
        .route("/", get(index))
        .route("/hello/{name}", get(hello))
        .call(req)
        .await
}

async fn index() -> &'static str {
    "Hello, Spin!"
}

async fn hello(Path(name): Path<String>) -> String {
    format!("Hello, {name}!")
}
{%- else -%}
use spin_sdk::http::{IntoResponse, Request, Response};
use spin_sdk::http_service;

/// A simple Spin HTTP component.
#[http_service]
async fn handle_{{project-name | snake_case}}(req: Request) -> anyhow::Result<impl IntoResponse> {
    println!("Handling request to {:?}", req.headers().get("spin-full-url"));
    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/plain")
        .body("Hello World!".to_string()))
}
{%- endcase %}