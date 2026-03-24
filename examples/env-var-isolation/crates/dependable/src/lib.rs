wit_bindgen::generate!({
    world: "dependable-world",
    path: "../../wit",
});

pub struct Dependable;

impl exports::hello::components::dependable::Guest for Dependable {
    fn get_message() -> String {
        format!(
            "dependable's env vars: {}",
            std::env::vars()
                .map(|(key, value)| format!("{key}='{value}'"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

export!(Dependable);
