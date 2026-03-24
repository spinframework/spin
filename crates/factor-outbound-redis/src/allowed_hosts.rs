use std::sync::Arc;

use spin_factor_outbound_networking::config::allowed_hosts::OutboundAllowedHosts;

/// Encapsulates checking of a PostgreSQL address/connection string against
/// an allow-list.
///
/// This is broken out as a distinct object to allow it to be synchronously retrieved
/// within a P3 Accessor block and then asynchronously queried outside the block.
#[derive(Clone)]
pub(crate) struct AllowedHostChecker {
    allowed_hosts: Arc<OutboundAllowedHosts>,
}

impl AllowedHostChecker {
    pub fn new(allowed_hosts: OutboundAllowedHosts) -> Self {
        Self {
            allowed_hosts: Arc::new(allowed_hosts),
        }
    }
}

impl AllowedHostChecker {
    pub async fn is_address_allowed(&self, address: &str) -> anyhow::Result<bool> {
        self.allowed_hosts.check_url(address, "redis").await
    }
}
