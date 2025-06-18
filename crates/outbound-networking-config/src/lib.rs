use std::sync::Arc;

use crate::allowed_hosts::{AllowedHostsConfig, OutboundUrl};

use futures_util::future::{BoxFuture, Shared};

pub mod allowed_hosts;
pub mod blocked_networks;

/// An easily cloneable, shared, boxed future of result
pub type SharedFutureResult<T> = Shared<BoxFuture<'static, Result<Arc<T>, Arc<anyhow::Error>>>>;

/// A check for whether a URL is allowed by the outbound networking configuration.
#[derive(Clone)]
pub struct OutboundAllowedHosts {
    allowed_hosts_future: SharedFutureResult<AllowedHostsConfig>,
    disallowed_host_handler: Option<Arc<dyn DisallowedHostHandler>>,
}

impl OutboundAllowedHosts {
    /// Creates a new `OutboundAllowedHosts` instance.
    pub fn new(
        allowed_hosts_future: SharedFutureResult<AllowedHostsConfig>,
        disallowed_host_handler: Option<Arc<dyn DisallowedHostHandler>>,
    ) -> Self {
        Self {
            allowed_hosts_future,
            disallowed_host_handler,
        }
    }

    /// Checks address against allowed hosts
    ///
    /// Calls the [`DisallowedHostHandler`] if set and URL is disallowed.
    /// If `url` cannot be parsed, `{scheme}://` is prepended to `url` and retried.
    pub async fn check_url(&self, url: &str, scheme: &str) -> anyhow::Result<bool> {
        tracing::debug!("Checking outbound networking request to '{url}'");
        let url = match OutboundUrl::parse(url, scheme) {
            Ok(url) => url,
            Err(err) => {
                tracing::warn!(%err,
                    "A component tried to make a request to a url that could not be parsed: {url}",
                );
                return Ok(false);
            }
        };

        let allowed_hosts = self.resolve().await?;
        let is_allowed = allowed_hosts.allows(&url);
        if !is_allowed {
            tracing::debug!("Disallowed outbound networking request to '{url}'");
            self.report_disallowed_host(url.scheme(), &url.authority());
        }
        Ok(is_allowed)
    }

    /// Checks if allowed hosts permit relative requests
    ///
    /// Calls the [`DisallowedHostHandler`] if set and relative requests are
    /// disallowed.
    pub async fn check_relative_url(&self, schemes: &[&str]) -> anyhow::Result<bool> {
        tracing::debug!("Checking relative outbound networking request with schemes {schemes:?}");
        let allowed_hosts = self.resolve().await?;
        let is_allowed = allowed_hosts.allows_relative_url(schemes);
        if !is_allowed {
            tracing::debug!(
                "Disallowed relative outbound networking request with schemes {schemes:?}"
            );
            let scheme = schemes.first().unwrap_or(&"");
            self.report_disallowed_host(scheme, "self");
        }
        Ok(is_allowed)
    }

    async fn resolve(&self) -> anyhow::Result<Arc<AllowedHostsConfig>> {
        self.allowed_hosts_future
            .clone()
            .await
            .map_err(anyhow::Error::msg)
    }

    fn report_disallowed_host(&self, scheme: &str, authority: &str) {
        if let Some(handler) = &self.disallowed_host_handler {
            handler.handle_disallowed_host(scheme, authority);
        }
    }
}

/// A trait for handling disallowed hosts
pub trait DisallowedHostHandler: Send + Sync {
    /// Called when a host is disallowed
    fn handle_disallowed_host(&self, scheme: &str, authority: &str);
}

impl<F: Fn(&str, &str) + Send + Sync> DisallowedHostHandler for F {
    fn handle_disallowed_host(&self, scheme: &str, authority: &str) {
        self(scheme, authority);
    }
}
