use anyhow::{bail, ensure, Context};
use ip_network::IpNetwork;
use rustls_pki_types::pem::PemObject;
use serde::{Deserialize, Deserializer};
use spin_factors::runtime_config::toml::GetTomlValue;
use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use super::ClientTlsRuntimeConfig;

/// Spin's default handling of the runtime configuration for outbound networking.
pub struct SpinRuntimeConfig {
    runtime_config_dir: PathBuf,
}

impl SpinRuntimeConfig {
    /// Creates a new `SpinRuntimeConfig`.
    ///
    /// The given `runtime_config_dir` will be used as the root to resolve any
    /// relative paths.
    pub fn new(runtime_config_dir: impl Into<PathBuf>) -> Self {
        Self {
            runtime_config_dir: runtime_config_dir.into(),
        }
    }

    /// Get the runtime configuration for client TLS from a TOML table.
    ///
    /// Expects table to be in the format:
    /// ````toml
    /// [outbound_networking]
    /// block_networks = ["1.1.1.1/32", "private"]
    ///
    /// [[client_tls]]
    /// component_ids = ["example-component"]
    /// hosts = ["example.com"]
    /// ca_use_webpki_roots = true
    /// ca_roots_file = "path/to/roots.crt"
    /// client_cert_file = "path/to/client.crt"
    /// client_private_key_file = "path/to/client.key"
    /// ```
    pub fn config_from_table(
        &self,
        table: &impl GetTomlValue,
    ) -> anyhow::Result<Option<super::RuntimeConfig>> {
        let maybe_blocked_networks = self
            .blocked_networks_from_table(table)
            .context("failed to parse [outbound_networking] table")?;
        let maybe_tls_configs = self
            .tls_configs_from_table(table)
            .context("failed to parse [[client_tls]] table")?;

        if maybe_blocked_networks.is_none() && maybe_tls_configs.is_none() {
            return Ok(None);
        }

        let (blocked_ip_networks, block_private_networks) =
            maybe_blocked_networks.unwrap_or_default();

        let client_tls_configs = maybe_tls_configs.unwrap_or_default();

        let runtime_config = super::RuntimeConfig {
            blocked_ip_networks,
            block_private_networks,
            client_tls_configs,
        };
        Ok(Some(runtime_config))
    }

    /// Attempts to parse (blocked_ip_networks, block_private_networks) from a
    /// `[outbound_networking]` table.
    fn blocked_networks_from_table(
        &self,
        table: &impl GetTomlValue,
    ) -> anyhow::Result<Option<(Vec<ip_network::IpNetwork>, bool)>> {
        let Some(value) = table.get("outbound_networking") else {
            return Ok(None);
        };
        let outbound_networking: OutboundNetworkingToml = value.clone().try_into()?;

        let mut ip_networks = vec![];
        let mut private_networks = false;
        for block_network in outbound_networking.block_networks {
            match block_network {
                CidrOrPrivate::Cidr(ip_network) => ip_networks.push(ip_network),
                CidrOrPrivate::Private => {
                    private_networks = true;
                }
            }
        }
        Ok(Some((ip_networks, private_networks)))
    }

    fn tls_configs_from_table<T: GetTomlValue>(
        &self,
        table: &T,
    ) -> anyhow::Result<Option<Vec<ClientTlsRuntimeConfig>>> {
        let Some(array) = table.get("client_tls") else {
            return Ok(None);
        };
        let toml_configs: Vec<ClientTlsToml> = array.clone().try_into()?;

        let tls_configs = toml_configs
            .into_iter()
            .map(|toml_config| self.load_tls_config(toml_config))
            .collect::<anyhow::Result<Vec<_>>>()
            .context("failed to parse TLS config")?;
        Ok(Some(tls_configs))
    }

    fn load_tls_config(
        &self,
        toml_config: ClientTlsToml,
    ) -> anyhow::Result<ClientTlsRuntimeConfig> {
        let ClientTlsToml {
            component_ids,
            hosts,
            ca_use_webpki_roots,
            ca_roots_file,
            client_cert_file,
            client_private_key_file,
        } = toml_config;
        ensure!(
            !component_ids.is_empty(),
            "'component_ids' list may not be empty"
        );
        ensure!(!hosts.is_empty(), "'hosts' list may not be empty");

        let components = component_ids.into_iter().map(Into::into).collect();

        let hosts = hosts
            .iter()
            .map(|host| {
                host.parse()
                    .map_err(|err| anyhow::anyhow!("invalid host {host:?}: {err:?}"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        let use_webpki_roots = if let Some(ca_use_webpki_roots) = ca_use_webpki_roots {
            ca_use_webpki_roots
        } else {
            // Use webpki roots by default *unless* explicit roots were given
            ca_roots_file.is_none()
        };

        let root_certificates = ca_roots_file
            .map(|path| self.load_certs(path))
            .transpose()?
            .unwrap_or_default();

        let client_cert = match (client_cert_file, client_private_key_file) {
            (Some(cert_path), Some(key_path)) => Some(super::ClientCertRuntimeConfig {
                cert_chain: self.load_certs(cert_path)?,
                key_der: self.load_key(key_path)?,
            }),
            (None, None) => None,
            (Some(_), None) => bail!("client_cert_file specified without client_private_key_file"),
            (None, Some(_)) => bail!("client_private_key_file specified without client_cert_file"),
        };

        Ok(ClientTlsRuntimeConfig {
            components,
            hosts,
            root_certificates,
            use_webpki_roots,
            client_cert,
        })
    }

    // Parse certs from the provided file
    fn load_certs(
        &self,
        path: impl AsRef<Path>,
    ) -> anyhow::Result<Vec<rustls_pki_types::CertificateDer<'static>>> {
        let path = self.runtime_config_dir.join(path);
        rustls_pki_types::CertificateDer::pem_file_iter(&path)
            .and_then(Iterator::collect)
            .with_context(|| format!("failed to load certificate(s) from '{}'", path.display()))
    }

    // Parse a private key from the provided file
    fn load_key(
        &self,
        path: impl AsRef<Path>,
    ) -> anyhow::Result<rustls_pki_types::PrivateKeyDer<'static>> {
        let path = self.runtime_config_dir.join(path);
        rustls_pki_types::PrivateKeyDer::from_pem_file(&path)
            .with_context(|| format!("failed to load key from '{}'", path.display()))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientTlsToml {
    component_ids: Vec<spin_serde::KebabId>,
    #[serde(deserialize_with = "deserialize_hosts")]
    hosts: Vec<String>,
    ca_use_webpki_roots: Option<bool>,
    ca_roots_file: Option<PathBuf>,
    client_cert_file: Option<PathBuf>,
    client_private_key_file: Option<PathBuf>,
}

fn deserialize_hosts<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    let hosts = Vec::<String>::deserialize(deserializer)?;
    for host in &hosts {
        crate::tls::validate_host(host).map_err(serde::de::Error::custom)?;
    }
    Ok(hosts)
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutboundNetworkingToml {
    #[serde(default)]
    block_networks: Vec<CidrOrPrivate>,
}

#[derive(Debug)]
enum CidrOrPrivate {
    Cidr(ip_network::IpNetwork),
    Private,
}

impl<'de> Deserialize<'de> for CidrOrPrivate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = Cow::<str>::deserialize(deserializer)?;
        if s == "private" {
            return Ok(Self::Private);
        }
        if let Ok(net) = IpNetwork::from_str_truncate(&s) {
            return Ok(Self::Cidr(net));
        }
        Err(serde::de::Error::invalid_value(
            serde::de::Unexpected::Str(&s),
            &"an IP network in CIDR notation or the keyword 'private'",
        ))
    }
}

#[cfg(test)]
mod tests {
    use spin_outbound_networking_config::blocked_networks::test::cidr;

    use super::*;

    const TESTDATA_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata");

    #[test]
    fn test_no_config() -> anyhow::Result<()> {
        let maybe_config = SpinRuntimeConfig::new("").config_from_table(&toml::toml! {
            [some_other_config]
            relevant = false
        })?;
        assert!(maybe_config.is_none(), "{maybe_config:?}");
        Ok(())
    }

    #[test]
    fn test_no_blocked_networks() -> anyhow::Result<()> {
        for table in &[
            toml::toml! {
                [outbound_networking]
            },
            toml::toml! {
                [outbound_networking]
                block_networks = []
            },
        ] {
            let config = SpinRuntimeConfig::new("")
                .config_from_table(table)
                .with_context(|| table.to_string())?
                .context("expected config, got None")?;
            assert!(config.blocked_ip_networks.is_empty(), "{config:?}");
            assert!(!config.block_private_networks);
        }
        Ok(())
    }

    #[test]
    fn test_some_blocked_networks() -> anyhow::Result<()> {
        let config = SpinRuntimeConfig::new("")
            .config_from_table(&toml::toml! {
                [outbound_networking]
                block_networks = ["1.1.1.1/32", "8.8.8.8/16", "private"]
            })
            .context("config_from_table")?
            .context("expected config, got None")?;
        assert!(config.blocked_ip_networks.contains(&cidr("1.1.1.1/32")));
        // Networks get normalized ("truncated")
        assert!(config.blocked_ip_networks.contains(&cidr("8.8.0.0/16")));
        assert!(config.block_private_networks, "{config:?}");
        Ok(())
    }

    #[test]
    fn test_min_tls_config() -> anyhow::Result<()> {
        let config = SpinRuntimeConfig::new("/doesnt-matter");

        let tls_configs = config
            .tls_configs_from_table(&toml::toml! {
                [[client_tls]]
                component_ids = ["test-component"]
                hosts = ["test-host"]

            })?
            .context("missing config section")?;
        assert_eq!(tls_configs.len(), 1);

        assert_eq!(tls_configs[0].components, ["test-component"]);
        assert_eq!(tls_configs[0].hosts[0].as_str(), "test-host");
        assert!(tls_configs[0].use_webpki_roots);
        Ok(())
    }

    #[test]
    fn test_max_tls_config() -> anyhow::Result<()> {
        let config = SpinRuntimeConfig::new(TESTDATA_DIR);

        let tls_configs = config
            .tls_configs_from_table(&toml::toml! {
                [[client_tls]]
                component_ids = ["test-component"]
                hosts = ["test-host"]
                ca_use_webpki_roots = true
                ca_roots_file = "valid-cert.pem"
                client_cert_file = "valid-cert.pem"
                client_private_key_file = "valid-private-key.pem"
            })?
            .context("missing config section")?;
        assert_eq!(tls_configs.len(), 1);

        assert!(tls_configs[0].use_webpki_roots);
        assert_eq!(tls_configs[0].root_certificates.len(), 2);
        assert!(tls_configs[0].client_cert.is_some());
        Ok(())
    }

    #[test]
    fn test_use_webpki_roots_default_with_explicit_roots() -> anyhow::Result<()> {
        let config = SpinRuntimeConfig::new(TESTDATA_DIR);

        let tls_configs = config
            .tls_configs_from_table(&toml::toml! {
                [[client_tls]]
                component_ids = ["test-component"]
                hosts = ["test-host"]
                ca_roots_file = "valid-cert.pem"
            })?
            .context("missing config section")?;

        assert!(!tls_configs[0].use_webpki_roots);
        Ok(())
    }

    #[test]
    fn test_invalid_cert() {
        let config = SpinRuntimeConfig::new(TESTDATA_DIR);

        config
            .tls_configs_from_table(&toml::toml! {
                [[client_tls]]
                component_ids = ["test-component"]
                hosts = ["test-host"]
                ca_roots_file = "invalid-cert.pem"
            })
            .unwrap_err();
    }

    #[test]
    fn test_invalid_private_key() {
        let config = SpinRuntimeConfig::new(TESTDATA_DIR);

        config
            .tls_configs_from_table(&toml::toml! {
                [[client_tls]]
                component_ids = ["test-component"]
                hosts = ["test-host"]
                client_cert_file = "valid-cert.pem"
                client_private_key_file = "invalid-key.pem"
            })
            .unwrap_err();
    }
}
