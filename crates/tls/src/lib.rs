use std::sync::Once;

static INSTALL_DEFAULT_CRYPTO_PROVIDER: Once = Once::new();

/// Install Spin's process-wide rustls crypto provider.
///
/// This is idempotent for Spin's own duplicate calls from `main` and the trigger,
/// but fails loudly if something else installed a rustls provider first.
pub fn install_default_crypto_provider() {
    INSTALL_DEFAULT_CRYPTO_PROVIDER.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("failed to install rustls ring crypto provider");
    });
}
