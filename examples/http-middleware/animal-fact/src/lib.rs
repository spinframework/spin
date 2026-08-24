use spin_sdk::http::body::IncomingBodyExt;
use spin_sdk::http::{HeaderValue, Request, Response};
use spin_sdk::http_service;

#[http_service]
async fn add_animal_fact_header(
    request: Request,
) -> Result<Response, spin_sdk::wasip3::http::types::ErrorCode> {
    // Gather an animal fa-- I MEAN MAKE A REQUEST TO THE AUTHORISATION SERVICE
    let animal_fact_response =
        spin_sdk::http::get("https://random-data-api.fermyon.app/animals/json").await?;
    let animal_fact_json = animal_fact_response.into_body().bytes().await?;
    let animal_fact: AnimalFact = serde_json::from_slice(&animal_fact_json).unwrap();

    // Add a header to the request being passed along the pipeline.
    let (mut parts, body) = request.into_parts();
    parts.headers.append(
        "animal-fact",
        HeaderValue::from_str(&animal_fact.fact).unwrap(),
    );

    // Reconstruct the request, now with the adjusted headers
    let request = Request::from_parts(parts, body);

    // Pass the request on down the middleware pipeline,
    // and return the response unaltered.
    spin_sdk::http::next(request).await
}

#[derive(serde::Deserialize)]
struct AnimalFact {
    fact: String,
}
