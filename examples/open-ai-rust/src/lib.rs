use spin_sdk::http::{IntoResponse, Request, Response};
use spin_sdk::http_component;

/// A simple Spin HTTP component.
#[http_component]
fn handle_open_ai_rust(req: Request) -> anyhow::Result<impl IntoResponse> {
    let llm_chat = spin_sdk::llm::infer(
        spin_sdk::llm::InferencingModel::Other("gpt-oss:20b"),
        "tell me about Epe in Lagos, Nigeria",
    )?;

    println!("Handling request to {:?}", req.header("spin-full-url"));

    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/plain")
        .body(format!(
            "Here's your response: {}\n Total tokens used: {}",
            llm_chat.text, llm_chat.usage.prompt_token_count
        ))
        .build())
}
