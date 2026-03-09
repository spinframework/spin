#[cfg(feature = "spin-cli")]
pub mod spin;

pub use rustls_pki_types::{CertificateDer, PrivateKeyDer};

/// Runtime configuration for outbound networking.
#[derive(Debug, Default)]
pub struct RuntimeConfig {
    /// Blocked IP networks
    pub blocked_ip_networks: Vec<ip_network::IpNetwork>,
    /// If true, non-globally-routable networks are blocked
    pub block_private_networks: bool,
    /// TLS client configs
    pub client_tls_configs: Vec<ClientTlsRuntimeConfig>,
}

/// TLS configuration for one or more component(s) and host(s).
#[derive(Debug)]
pub struct ClientTlsRuntimeConfig {
    /// The component(s) this configuration applies to.
    pub components: Vec<String>,
    /// The host(s) this configuration applies to.
    pub hosts: Vec<String>,
    /// A set of CA certs that should be considered valid roots.
    pub root_certificates: Vec<CertificateDer<'static>>,
    /// If true, the operating system's certificate store will be used for
    /// root certificate verification via `rustls-platform-verifier`.
    pub use_platform_roots: bool,
    /// If true, the "standard" CA certs defined by `webpki-roots` crate will be
    /// considered valid roots in addition to `root_certificates`.
    /// Only used when `use_platform_roots` is false.
    pub use_webpki_roots: bool,
    /// A certificate and private key to be used as the client certificate for
    /// "mutual TLS" (mTLS).
    pub client_cert: Option<ClientCertRuntimeConfig>,
}

impl Default for ClientTlsRuntimeConfig {
    fn default() -> Self {
        Self {
            components: vec![],
            hosts: vec![],
            root_certificates: vec![],
            // Use platform roots by default
            use_platform_roots: true,
            use_webpki_roots: false,
            client_cert: None,
        }
    }
}

#[derive(Debug)]
pub struct ClientCertRuntimeConfig {
    pub cert_chain: Vec<CertificateDer<'static>>,
    pub key_der: PrivateKeyDer<'static>,
}
