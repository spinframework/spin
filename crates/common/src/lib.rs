//! Spin common modules

#![deny(missing_docs)]

// In order to prevent this crate from becoming a dumping ground, we observe
// the following practices:
// - No code in the root module; everything must be in a focused `pub mod`
// - No dependencies on other Spin crates
// - Code should have at least 2 dependents

pub mod arg_parser;
pub mod data_dir;
pub mod paths;
pub mod sha256;
pub mod sloth;
pub mod ui;
pub mod url;
