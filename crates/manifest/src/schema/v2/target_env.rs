use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Identifies a deployment target.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum TargetEnvironmentRef {
    /// Environment definition doc reference e.g. `spin-up:3.2`, `my-host`. This is looked up
    /// in the default environment catalogue (the `spin-environments` repo, `env` directory).
    Catalogue(String),
    /// An environment definition doc HTTP URL
    Http {
        /// The environment document URL e.g. `https://github.com/me/environments/blob/main/target-envs/spin-up.3.6.toml`.
        url: String,
    },
    /// A local environment document file. This is expected to contain a serialised
    /// EnvironmentDefinition in TOML format.
    File {
        /// The file path of the document.
        path: PathBuf,
    },
}

impl std::fmt::Display for TargetEnvironmentRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Catalogue(e) => e.fmt(f),
            Self::Http { url } => url.fmt(f),
            Self::File { path } => path.display().fmt(f),
        }
    }
}
