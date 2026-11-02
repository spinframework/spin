//! Package for working with Wasm component templates.

#![deny(missing_docs)]

mod app_info;
mod cancellable;
mod constraints;
mod directory;
mod environment;
mod filters;
mod interaction;
mod manager;
mod reader;
mod renderer;
mod run;
mod source;
mod store;
mod template;
mod toml;
mod writer;

pub use manager::*;
pub use run::{Run, RunOptions};
pub use source::TemplateSource;
pub use template::{Template, TemplateVariantInfo};

#[cfg(test)]
mod test_built_ins;
