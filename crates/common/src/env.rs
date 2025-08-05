//! Environment variable utilities

/// Defines the default environment variable prefix used by Spin.
pub const DEFAULT_ENV_PREFIX: &str = "SPIN_VARIABLE";

/// Creates an environment variable key based on the given prefix and key.
pub fn env_key(prefix: Option<String>, key: &str) -> String {
    let prefix = prefix.unwrap_or_else(|| DEFAULT_ENV_PREFIX.to_string());
    let upper_key = key.to_ascii_uppercase();
    let key = format!("{prefix}_{upper_key}");
    key
}
