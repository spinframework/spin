//! Operations on URLs

use anyhow::{Context, anyhow};

use std::path::PathBuf;

/// Parse the path from a 'file:' URL
pub fn parse_file_url(url: &str) -> anyhow::Result<PathBuf> {
    url::Url::parse(url)
        .with_context(|| format!("Invalid URL: {url:?}"))?
        .to_file_path()
        .map_err(|_| anyhow!("Invalid file URL path: {url:?}"))
}

/// Remove the credentials from a URL string
pub fn remove_credentials(url: &str) -> anyhow::Result<String> {
    let mut url = url::Url::parse(url).with_context(|| format!("Invalid URL: {url:?}"))?;
    url.set_username("")
        .map_err(|_| anyhow!("Could not remove username"))?;
    url.set_password(None)
        .map_err(|_| anyhow!("Could not remove password"))?;
    Ok(url.to_string())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn remove_credentials_removes_credentials() {
        assert_eq!(
            "redis://example.com:4567",
            remove_credentials("redis://example.com:4567").unwrap()
        );
        assert_eq!(
            "redis://example.com:4567",
            remove_credentials("redis://me:secret@example.com:4567").unwrap()
        );
        assert_eq!(
            "http://example.com/users",
            remove_credentials("http://me:secret@example.com/users").unwrap()
        );
    }
}
