use anyhow::Context;
use rustls_pki_types::pem::PemObject;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_rustls::{TlsAcceptor, rustls};

// TODO: dedupe with spin-factor-outbound-networking (spin-tls crate?)

/// TLS configuration for the server.
#[derive(Clone)]
pub struct TlsConfig {
    /// Path to TLS certificate.
    pub cert_path: PathBuf,
    /// Path to TLS key.
    pub key_path: PathBuf,
}

impl TlsConfig {
    // Creates a TLS acceptor from server config.
    pub(super) fn server_config(&self) -> anyhow::Result<TlsAcceptor> {
        let certs = load_certs(&self.cert_path)?;
        let private_key = load_key(&self.key_path)?;

        let cfg = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, private_key)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        Ok(Arc::new(cfg).into())
    }
}

// load_certs parse and return the certs from the provided file
fn load_certs(
    path: impl AsRef<Path>,
) -> anyhow::Result<Vec<rustls_pki_types::CertificateDer<'static>>> {
    rustls_pki_types::CertificateDer::pem_file_iter(&path)
        .and_then(Iterator::collect)
        .with_context(|| {
            format!(
                "failed to load certificate(s) from '{}'",
                path.as_ref().display()
            )
        })
}

// parse and return the first private key from the provided file
fn load_key(path: impl AsRef<Path>) -> anyhow::Result<rustls_pki_types::PrivateKeyDer<'static>> {
    rustls_pki_types::PrivateKeyDer::from_pem_file(&path).with_context(|| {
        format!(
            "failed to load private key from '{}'",
            path.as_ref().display()
        )
    })
}

#[cfg(test)]
mod tests {
    use rustls_pki_types::pem;

    use super::*;

    const TESTDATA_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata");

    #[test]
    fn test_read_non_existing_cert() {
        let path = Path::new(TESTDATA_DIR).join("non-existing-file.pem");
        match load_certs(path).unwrap_err().downcast().unwrap() {
            pem::Error::Io(err) => assert_eq!(err.kind(), std::io::ErrorKind::NotFound),
            other => panic!("expected Error::Io error got {other:?}"),
        }
    }

    #[test]
    fn test_read_invalid_cert() {
        let path = Path::new(TESTDATA_DIR).join("invalid-cert.pem");
        match load_certs(path).unwrap_err().downcast().unwrap() {
            pem::Error::MissingSectionEnd { .. } => (),
            other => panic!("expected Error::MissingSectionEnd got {other:?}"),
        }
    }

    #[test]
    fn test_read_valid_cert() {
        let path = Path::new(TESTDATA_DIR).join("valid-cert.pem");
        let certs = load_certs(path).unwrap();
        assert_eq!(certs.len(), 2);
    }

    #[test]
    fn test_read_non_existing_private_key() {
        let path = Path::new(TESTDATA_DIR).join("non-existing-file.pem");
        match load_key(path).unwrap_err().downcast().unwrap() {
            pem::Error::Io(err) => assert_eq!(err.kind(), std::io::ErrorKind::NotFound),
            other => panic!("expected Error::Io error got {other:?}"),
        }
    }

    #[test]
    fn test_read_invalid_private_key() {
        let path = Path::new(TESTDATA_DIR).join("invalid-private-key.pem");
        match load_key(path).unwrap_err().downcast().unwrap() {
            pem::Error::MissingSectionEnd { .. } => (),
            other => panic!("expected Error::MissingSectionEnd got {other:?}"),
        }
    }

    #[test]
    fn test_read_valid_private_key() {
        let path = Path::new(TESTDATA_DIR).join("valid-private-key.pem");
        load_key(path).unwrap();
    }
}
