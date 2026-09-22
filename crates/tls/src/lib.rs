use std::sync::Arc;

/// Ensures a process-wide rustls crypto provider is installed, installing
/// `aws-lc-rs` if no provider is set. A provider already installed - e.g.
/// by an application embedding Spin's crates - is left in place.
///
/// Call this from entrypoints, and before handing a URL or config to a
/// library that builds its own rustls config, so that rustls never has to
/// pick a provider from crate features.
pub fn install_default_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        // Ignore Err: it means another thread won the install race.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }
}

/// Returns the process-wide default rustls crypto provider, installing
/// `aws-lc-rs` first if no provider is set (see
/// [`install_default_crypto_provider`]).
///
/// Use this to build rustls configs so that TLS paths constructed without
/// going through a Spin entrypoint (direct factor construction, tests,
/// embedders) select the same provider as the rest of the process.
pub fn get_or_install_default_crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
    install_default_crypto_provider();

    rustls::crypto::CryptoProvider::get_default()
        .expect("a default rustls crypto provider was just installed")
        .clone()
}
