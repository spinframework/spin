use spin_sdk::http::{IntoResponse, Request, StatusCode};
use spin_sdk::http_service;

#[http_service]
async fn handle_app(request: Request) -> impl IntoResponse {
    let animal_fact = request
        .headers()
        .get("animal-fact")
        .and_then(|hval| hval.to_str().ok());
    let message = match animal_fact {
        Some(animal_fact) => format!("Did you know: {animal_fact}"),
        None => "Oh no! No animal facts today!".to_string(),
    };

    (
        StatusCode::OK,
        format!("Hello, world!\n{message}\n\nWell, goodbye until next time!\n"),
    )
}
