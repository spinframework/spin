impl Guest for Exports {
    fn hello() -> String {
        "Hello from {{project-name | snake_case}}".to_string()
    }
}

// Boilerplate below here
use crate::exports::component::{{project-name | snake_case}}::{{project-name | snake_case}}::Guest;
wit_bindgen::generate!({
    world: "component",
    path: "component.wit",
});
struct Exports;
export!(Exports);
