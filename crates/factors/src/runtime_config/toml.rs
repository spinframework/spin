//! Helpers for reading runtime configuration from a TOML file.

use std::{cell::RefCell, collections::HashSet};

/// A trait for getting a TOML value by key.
pub trait GetTomlValue {
    fn get(&self, key: &str) -> Option<&toml::Value>;
}

impl GetTomlValue for toml::Table {
    fn get(&self, key: &str) -> Option<&toml::Value> {
        self.get(key)
    }
}

#[derive(Debug, Clone)]
/// A helper for tracking which keys have been used in a TOML table.
pub struct TomlKeyTracker<'a> {
    unused_keys: RefCell<HashSet<&'a str>>,
    table: &'a toml::Table,
}

impl<'a> TomlKeyTracker<'a> {
    pub fn new(table: &'a toml::Table) -> Self {
        Self {
            unused_keys: RefCell::new(table.keys().map(String::as_str).collect()),
            table,
        }
    }

    pub fn validate_all_keys_used(&self) -> crate::Result<()> {
        if !self.unused_keys.borrow().is_empty() {
            return Err(crate::Error::RuntimeConfigUnusedKeys {
                keys: self
                    .unused_keys
                    .borrow()
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect(),
            });
        }
        Ok(())
    }
}

impl GetTomlValue for TomlKeyTracker<'_> {
    fn get(&self, key: &str) -> Option<&toml::Value> {
        self.unused_keys.borrow_mut().remove(key);
        self.table.get(key)
    }
}

impl AsRef<toml::Table> for TomlKeyTracker<'_> {
    fn as_ref(&self) -> &toml::Table {
        self.table
    }
}

/// Returns true. For use with `#[derive(Deserialize)]`:
///
/// ```
/// use spin_factors::runtime_config::toml::default_true;
/// #[derive(serde::Deserialize)]
/// struct Options {
///     #[serde(default = "default_true")]
///     turbo_mode: bool,
/// }
/// ```
pub fn default_true() -> bool {
    true
}
