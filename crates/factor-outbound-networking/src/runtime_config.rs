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
    /// Maximum number of outbound TCP/UDP socket connections across all instances of this app.
    /// `None` means unlimited (default).
    pub max_socket_connections: Option<usize>,
    /// Maximum number of outbound connections across ALL connection types (global cap).
    /// `None` means unlimited (default).
    pub max_total_connections: Option<usize>,
    /// If set, limits how long `acquire` will wait for a socket connection permit.
    pub wait_timeout: Option<std::time::Duration>,
}

/// TLS configuration for one or more component(s) and host(s).
#[derive(Debug)]
pub struct ClientTlsRuntimeConfig {
    /// The component(s) this configuration applies to.
    pub components: Vec<String>,

    /// The host(s) this configuration applies to.
    pub hosts: Vec<String>,

    /// If `true`, the operating system's certificate store will be used for
    /// root certificate verification
    /// [`rustls-platform-verifier`](rustls_platform_verifier).
    ///
    /// By default this is `true`.
    pub use_platform_roots: bool,

    /// If `true`, the "standard" CA certs in the
    /// [`webpki-root-certs`](webpki_root_certs) crate will be considered valid
    /// roots.
    ///
    /// By default this is `true`.
    pub use_webpki_roots: bool,

    /// A set of CA certs that should be considered valid roots.
    ///
    /// These will be used _in addition_ to roots enabled by
    /// [`use_platform_roots`](Self::use_platform_roots) and
    /// [`use_webpki_roots`](Self::use_webpki_roots).
    pub root_certificates: Vec<CertificateDer<'static>>,

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
            use_platform_roots: true,
            use_webpki_roots: true,
            client_cert: None,
        }
    }
}

#[derive(Debug)]
pub struct ClientCertRuntimeConfig {
    pub cert_chain: Vec<CertificateDer<'static>>,
    pub key_der: PrivateKeyDer<'static>,
}
