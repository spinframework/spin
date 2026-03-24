use std::sync::Arc;

use anyhow::Result;
use spin_factor_outbound_networking::config::allowed_hosts::OutboundAllowedHosts;

#[derive(Clone)]
pub struct AllowedHostChecker {
    allowed_hosts: Arc<OutboundAllowedHosts>,
}

impl AllowedHostChecker {
    pub fn new(allowed_hosts: OutboundAllowedHosts) -> Self {
        Self {
            allowed_hosts: Arc::new(allowed_hosts),
        }
    }

    pub async fn is_address_allowed(&self, address: &str) -> Result<bool> {
        self.allowed_hosts.check_url(address, "mqtt").await
    }
}
