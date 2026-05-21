pub mod badger;
mod catalogue;
pub mod error;
mod git;
mod lookup;
pub mod manager;
pub mod manifest;
mod store;
mod util;

pub use catalogue::Catalogue;
pub use lookup::PluginRef;
pub use manager::PluginManager;

/// List of Spin internal subcommands
pub(crate) const SPIN_INTERNAL_COMMANDS: &[&str] = &[
    "template",
    "templates",
    "up",
    "new",
    "add",
    "login",
    "deploy",
    "build",
    "plugin",
    "plugins",
    "trigger",
    "external",
    "doctor",
    "registry",
    "watch",
    "oci",
];
