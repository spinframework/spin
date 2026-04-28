use std::{collections::HashMap, ops::Deref, sync::Arc};

use anyhow::{Context, ensure};
use rustls::client::{WantsClientCert, danger::ServerCertVerifier};

use crate::runtime_config::{ClientCertRuntimeConfig, ClientTlsRuntimeConfig};

/// TLS client configs
#[derive(Default)]
pub struct TlsClientConfigs {
    /// Shared map of component ID -> HostTlsClientConfigs
    component_host_tls_client_configs: HashMap<String, HostTlsClientConfigs>,
    /// The default [`ClientConfig`] for a host if one is not explicitly configured for it.
    default_tls_client_config: TlsClientConfig,
}

impl TlsClientConfigs {
    pub(crate) fn new(
        client_tls_configs: impl IntoIterator<Item = ClientTlsRuntimeConfig>,
    ) -> anyhow::Result<Self> {
        // Construct nested map of <component ID> -> <host authority> -> TLS client config
        let mut component_host_tls_client_configs = HashMap::<String, HostTlsClientConfigs>::new();
        for ClientTlsRuntimeConfig {
            components,
            hosts,
            root_certificates,
            use_platform_roots,
            use_webpki_roots,
            client_cert,
        } in client_tls_configs
        {
            ensure!(
                !components.is_empty(),
                "client TLS 'components' list may not be empty"
            );
            ensure!(
                !hosts.is_empty(),
                "client TLS 'hosts' list may not be empty"
            );
            let tls_client_config = TlsClientConfig::new(
                root_certificates,
                use_webpki_roots,
                use_platform_roots,
                client_cert,
            )
            .context("error building TLS client config")?;
            for component in components {
                let host_configs = component_host_tls_client_configs
                    .entry(component.clone())
                    .or_default();
                for host in &hosts {
                    validate_host(host)?;
                    // First matching (component, host) pair wins
                    Arc::get_mut(host_configs)
                        .unwrap()
                        .entry(host.clone())
                        .or_insert_with(|| tls_client_config.clone());
                }
            }
        }

        Ok(Self {
            component_host_tls_client_configs,
            ..Default::default()
        })
    }

    /// Returns [`ComponentTlsClientConfigs`] for the given component.
    pub fn get_component_tls_configs(&self, component_id: &str) -> ComponentTlsClientConfigs {
        let host_client_configs = self
            .component_host_tls_client_configs
            .get(component_id)
            .cloned();
        ComponentTlsClientConfigs {
            host_client_configs,
            default_client_config: self.default_tls_client_config.clone(),
        }
    }
}

/// Shared maps of host authority -> TlsClientConfig
type HostTlsClientConfigs = Arc<HashMap<String, TlsClientConfig>>;

/// TLS configurations for a specific component.
#[derive(Clone)]
pub struct ComponentTlsClientConfigs {
    pub(crate) host_client_configs: Option<HostTlsClientConfigs>,
    pub(crate) default_client_config: TlsClientConfig,
}

impl ComponentTlsClientConfigs {
    /// Returns a [`ClientConfig`] for the given host authority.
    pub fn get_client_config(&self, host: &str) -> &TlsClientConfig {
        self.host_client_configs
            .as_ref()
            .and_then(|configs| configs.get(host))
            .unwrap_or(&self.default_client_config)
    }
}

/// Shared TLS client configuration
#[derive(Clone)]
pub struct TlsClientConfig(Arc<rustls::ClientConfig>);

impl TlsClientConfig {
    fn new(
        root_certificates: Vec<rustls_pki_types::CertificateDer<'static>>,
        use_webpki_roots: bool,
        use_platform_roots: bool,
        client_cert: Option<ClientCertRuntimeConfig>,
    ) -> anyhow::Result<Self> {
        let builder = if use_platform_roots {
            let crypto_provider = rustls::crypto::CryptoProvider::get_default()
                .cloned()
                .unwrap_or_else(|| Arc::new(rustls::crypto::ring::default_provider()));

            let verifier_result = if root_certificates.is_empty() {
                rustls_platform_verifier::Verifier::new(crypto_provider.clone())
                    .map(|v| Arc::new(v) as Arc<dyn ServerCertVerifier>)
            } else {
                rustls_platform_verifier::Verifier::new_with_extra_roots(
                    root_certificates.clone(),
                    crypto_provider.clone(),
                )
                .map(|v| Arc::new(v) as Arc<dyn ServerCertVerifier>)
            };

            match verifier_result {
                Ok(verifier) => rustls::ClientConfig::builder_with_provider(crypto_provider)
                    .with_safe_default_protocol_versions()?
                    .dangerous()
                    .with_custom_certificate_verifier(verifier),
                Err(err) if use_webpki_roots => {
                    // Fall back to webpki roots if platform verifier creation failed
                    Self::builder_with_root_store(use_webpki_roots, root_certificates)?
                }
                Err(err) => {
                    return Err(err).context("failed to initialize platform certificate verifier");
                }
            }
        } else {
            Self::builder_with_root_store(use_webpki_roots, root_certificates)?
        };

        let client_config = if let Some(ClientCertRuntimeConfig {
            cert_chain,
            key_der,
        }) = client_cert
        {
            builder.with_client_auth_cert(cert_chain, key_der)?
        } else {
            builder.with_no_client_auth()
        };
        Ok(Self(client_config.into()))
    }

    fn builder_with_root_store(
        use_webpki_roots: bool,
        root_certificates: Vec<rustls_pki_types::CertificateDer<'static>>,
    ) -> anyhow::Result<rustls::ConfigBuilder<rustls::ClientConfig, WantsClientCert>> {
        let mut root_store = rustls::RootCertStore::empty();
        if use_webpki_roots {
            root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }
        for cert in root_certificates {
            root_store.add(cert)?;
        }
        Ok(rustls::ClientConfig::builder().with_root_certificates(root_store))
    }

    /// Returns the inner [`rustls::ClientConfig`] for consumption by rustls APIs.
    pub fn inner(&self) -> Arc<rustls::ClientConfig> {
        self.0.clone()
    }
}

impl Deref for TlsClientConfig {
    type Target = rustls::ClientConfig;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Default for TlsClientConfig {
    fn default() -> Self {
        Self::new(vec![], false, true, None).expect("default client config should be valid")
    }
}

/// Validate host name (authority without port)
pub(crate) fn validate_host(host: &str) -> anyhow::Result<()> {
    let authority: http::uri::Authority = host
        .parse()
        .with_context(|| format!("invalid TLS 'host' {host:?}"))?;
    ensure!(
        authority.port().is_none(),
        "invalid TLS 'host' {host:?}; ports not currently supported"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use anyhow::Context;
    use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};

    use super::*;

    #[test]
    fn test_empty_config() -> anyhow::Result<()> {
        // Just make sure the default doesn't panic
        let configs = TlsClientConfigs::default();
        configs.get_tls_client_config("foo", "bar");
        Ok(())
    }

    #[test]
    fn test_minimal_config() -> anyhow::Result<()> {
        let configs = TlsClientConfigs::new([ClientTlsRuntimeConfig {
            components: vec!["test-component".into()],
            hosts: vec!["test-host".into()],
            root_certificates: vec![],
            use_platform_roots: false,
            use_webpki_roots: false,
            client_cert: None,
        }])?;
        let config = configs.get_tls_client_config("test-component", "test-host");
        // Check that we didn't just get the default
        let default_config = configs.get_tls_client_config("other_component", "test-host");
        assert!(!Arc::ptr_eq(&config.0, &default_config.0));
        Ok(())
    }

    #[test]
    fn test_maximal_config() -> anyhow::Result<()> {
        let test_certs = test_certs()?;
        let test_key = test_key()?;
        let configs = TlsClientConfigs::new([ClientTlsRuntimeConfig {
            components: vec!["test-component".into()],
            hosts: vec!["test-host".into()],
            root_certificates: vec![test_certs[0].clone()],
            use_platform_roots: false,
            use_webpki_roots: false,
            client_cert: Some(ClientCertRuntimeConfig {
                cert_chain: test_certs,
                key_der: test_key,
            }),
        }])?;
        let config = configs.get_tls_client_config("test-component", "test-host");
        assert!(config.client_auth_cert_resolver.has_certs());
        Ok(())
    }

    #[test]
    fn test_config_overrides() -> anyhow::Result<()> {
        let test_certs = test_certs()?;
        let test_key = test_key()?;
        let configs = TlsClientConfigs::new([
            ClientTlsRuntimeConfig {
                components: vec!["test-component1".into()],
                hosts: vec!["test-host".into()],
                client_cert: Some(ClientCertRuntimeConfig {
                    cert_chain: test_certs,
                    key_der: test_key,
                }),
                ..Default::default()
            },
            ClientTlsRuntimeConfig {
                components: vec!["test-component1".into(), "test-component2".into()],
                hosts: vec!["test-host".into()],
                ..Default::default()
            },
        ])?;
        // First match wins
        let config1 = configs.get_tls_client_config("test-component1", "test-host");
        assert!(config1.client_auth_cert_resolver.has_certs());

        // Correctly select by differing component ID
        let config2 = configs.get_tls_client_config("test-component-2", "test-host");
        assert!(!config2.client_auth_cert_resolver.has_certs());
        Ok(())
    }

    const TESTDATA_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata");

    fn test_certs() -> anyhow::Result<Vec<CertificateDer<'static>>> {
        CertificateDer::pem_file_iter(Path::new(TESTDATA_DIR).join("valid-cert.pem"))?
            .collect::<Result<Vec<_>, _>>()
            .context("certs")
    }

    fn test_key() -> anyhow::Result<PrivateKeyDer<'static>> {
        PrivateKeyDer::from_pem_file(Path::new(TESTDATA_DIR).join("valid-private-key.pem"))
            .context("key")
    }

    impl TlsClientConfigs {
        fn get_tls_client_config(&self, component_id: &str, host: &str) -> TlsClientConfig {
            let component_config = self.get_component_tls_configs(component_id);
            component_config.get_client_config(host).clone()
        }
    }
}
