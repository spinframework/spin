//! OCI registries integration.
#![deny(missing_docs)]

mod auth;
pub mod client;
mod loader;
pub mod utils;

pub use client::{Client, ComposeMode};
pub use loader::OciLoader;

/// URL scheme used for the locked app "origin" metadata field for OCI-sourced apps.
pub const ORIGIN_URL_SCHEME: &str = "vnd.fermyon.origin-oci";

/// Applies heuristics to check if the given string "looks like" it may be
/// an OCI reference.
///
/// This is primarily intended to differentiate OCI references from file paths,
/// which determines the particular heuristics applied.
pub fn is_probably_oci_reference(maybe_oci: &str) -> bool {
    // A relative file path such as foo/spin.toml will successfully
    // parse as an OCI reference, because the parser infers the Docker
    // registry and the `latest` version.  So if the registry resolves
    // to Docker, but the source *doesn't* contain the string 'docker',
    // we can guess this is likely a false positive.
    //
    // This could be fooled by, e.g., dockerdemo/spin.toml.  But we only
    // go down this path if the file does not exist, and the chances of
    // a user choosing a filename containing 'docker' THAT ALSO does not
    // exist are A MILLION TO ONE...

    // If it doesn't parse as a reference, it isn't a reference
    let Ok(reference) = oci_distribution::Reference::try_from(maybe_oci) else {
        return false;
    };
    // If it has an explicit tag, its probably a reference
    if reference.tag() != Some("latest") || maybe_oci.ends_with(":latest") {
        return true;
    }
    // If it doesn't have an explicit registry it's hard to tell whether its a
    // reference; we'll lean towards "no". The reference parser will set the
    // registry to the Docker default if none is set, which we try to detect.
    if reference.registry().contains("docker") && !maybe_oci.contains("docker") {
        return false;
    }
    // Passed all the tests; likely a reference
    true
}
