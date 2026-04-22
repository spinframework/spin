use miniserde::{Serialize, json};
use std::collections::BTreeMap;

#[derive(Serialize)]
struct Output {
    args: Vec<String>,
    vars: BTreeMap<String, String>,
}

fn main() {
    println!(
        "Content-Type: application/json\n\n{}",
        json::to_string(&Output {
            args: std::env::args().collect(),
            vars: std::env::vars().collect(),
        })
    );
}
