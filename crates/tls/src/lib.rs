use std::sync::Arc;

/// Returns the process-wide default rustls crypto provider, installing
/// `aws-lc-rs` if no provider is set. A provider already installed - e.g.
/// by an application embedding Spin's crates - is left in place and used.
///
/// Use this to build rustls configs so that TLS paths constructed without
/// going through a Spin entrypoint (direct factor construction, tests,
/// embedders) select the same provider as the rest of the process.
pub fn get_or_install_default_crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
    if let Some(provider) = rustls::crypto::CryptoProvider::get_default() {
        return provider.clone();
    }

    // Ignore Err: it means another thread won the install race.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    rustls::crypto::CryptoProvider::get_default()
        .expect("a default provider was just installed")
        .clone()
}
