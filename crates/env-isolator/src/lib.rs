//! Environment variable isolation for composed Spin components.
//!
//! When composing multiple WebAssembly components, all components share the same
//! host `wasi:cli/environment` import by default. This crate generates an
//! **isolator component** that interposes on that import to give each component
//! its own filtered view of the environment, based on per-component prefixes.

pub mod core_module;
pub mod isolator;
pub mod wrapper;

/// Compute the environment variable prefix for a component ID or dependency name.
///
/// Extracts the last meaningful segment from the name (the interface name for
/// package-style names like `foo:bar/baz`, or the full name for plain names),
/// converts to uppercase, replaces non-alphanumeric chars with underscores,
/// and appends a trailing underscore.
///
/// # Examples
/// - `"my-app"` → `"MY_APP_"`
/// - `"worker"` → `"WORKER_"`
/// - `"foo:bar/baz"` → `"BAZ_"`
/// - `"hello:components/dep"` → `"DEP_"`
pub fn compute_prefix(id: &str) -> String {
    // Extract the last segment: after the last '/' if present, otherwise the whole thing
    let name_part = id.rsplit('/').next().unwrap_or(id);
    let mut prefix: String = name_part
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    prefix.push('_');
    prefix
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_prefix() {
        assert_eq!(compute_prefix("my-app"), "MY_APP_");
        assert_eq!(compute_prefix("worker"), "WORKER_");
        assert_eq!(compute_prefix("main"), "MAIN_");
        assert_eq!(compute_prefix("foo:bar/baz"), "BAZ_");
        assert_eq!(compute_prefix("hello:components/dep"), "DEP_");
    }
}
