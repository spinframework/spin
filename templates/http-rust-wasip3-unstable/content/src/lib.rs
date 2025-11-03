use spin_sdk::http_wasip3::{http_service, IntoResponse, Request, Response};

/// A simple Spin HTTP component.
#[http_service]
async fn handle_{{project-name | snake_case}}(_req: Request) -> impl IntoResponse {
    Response::new("Hello, world!".to_string())
}
