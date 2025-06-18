//! Serde serialization helpers

#![deny(missing_docs)]

pub mod base64;
pub mod dependencies;
pub mod id;
mod version;

pub use version::{FixedStringVersion, FixedVersion, FixedVersionBackwardCompatible};

pub use dependencies::{DependencyName, DependencyPackageName, PackageRef, Registry};

/// A "kebab-case" identifier.
pub type KebabId = id::Id<'-', false>;

/// A "snake_case" identifier.
pub type SnakeId = id::Id<'_', false>;

/// A lower-case "snake_case" identifier.
pub type LowerSnakeId = id::Id<'_', true>;
