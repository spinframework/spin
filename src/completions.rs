use std::{collections::HashSet, path::PathBuf};

use clap_complete::CompletionCandidate;

pub fn profiles() -> Vec<CompletionCandidate> {
    let Some(toml) = load_manifest_toml() else {
        return vec![];
    };

    let Some(components) = toml.get("component").and_then(|t| t.as_table()) else {
        return vec![];
    };

    let mut all_profiles = HashSet::new();

    for component in components.values() {
        if let Some(profiles) = component
            .get("profile")
            .and_then(|t| t.as_table())
            .map(|t| t.keys())
        {
            all_profiles.extend(profiles);
        }
    }

    all_profiles.iter().map(CompletionCandidate::new).collect()
}

pub fn components() -> Vec<CompletionCandidate> {
    let Some(toml) = load_manifest_toml() else {
        return vec![];
    };

    let Some(components) = toml.get("component").and_then(|t| t.as_table()) else {
        return vec![];
    };

    components.keys().map(CompletionCandidate::new).collect()
}

fn load_manifest_toml() -> Option<toml::Table> {
    let mut args = std::env::args();

    let default_path = PathBuf::from("spin.toml");

    let manifest_path = if args.len() <= 2 {
        default_path
    } else {
        match args.position(|a| a == "-f" || a == "--from" || a == "--file" || a == "--from-file") {
            None => default_path,
            Some(_) => match args.next() {
                None => default_path,
                Some(arg) => {
                    spin_common::paths::resolve_manifest_file_path(arg).unwrap_or(default_path)
                }
            },
        }
    };

    std::fs::read_to_string(manifest_path)
        .ok()
        .and_then(|text| toml::from_str::<toml::Table>(&text).ok())
}
