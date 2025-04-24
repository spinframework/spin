pub mod build_info;
pub mod commands;
mod directory_rels;
pub(crate) mod opts;
pub mod subprocess;

// Fake change
// This is included third-party code (see NOTICES and included licence files)
// Skip formatting to minimise changes from upstream.
#[rustfmt::skip]
#[allow(clippy::all, dead_code)]
mod clap_markdown;

pub use opts::HELP_ARGS_ONLY_TRIGGER_TYPE;
