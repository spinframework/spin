{%- case http-router -%}
{%- when "axum" -%}
use axum::Router;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::routing::get;
use spin_sdk::http::{IntoResponse, Request};
use spin_sdk::http_service;
use tower_service::Service;
{%- else -%}
use spin_sdk::http::{IntoResponse, Request, Response};
use spin_sdk::http_service;
{%- endcase %}

/// A simple Spin HTTP component.
#[http_service]
async fn handle_{{project-name | snake_case}}(req: Request) -> anyhow::Result<impl IntoResponse> {
    println!("Handling request to {:?}", req.headers().get("spin-full-url"));
{%- case http-router -%}
{% when "axum" %}

    let mut router = Router::new()
        .route("/sample/{value}", get(sample))
        .fallback(not_found)
        .into_service();

    let response = router.call(req).await?;
    Ok(response)
{%- else %}
    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/plain")
        .body("Hello World!".to_string()))
{%- endcase %}
}
{%- case http-router -%}
{% when "axum" %}

async fn sample(Path(value): Path<String>) -> impl axum::response::IntoResponse {
    (StatusCode::OK, format!("The value was '{value}'\n"))
}

async fn not_found() -> impl axum::response::IntoResponse {
    (StatusCode::NOT_FOUND, "Not found\n")
}
{%- endcase %}
