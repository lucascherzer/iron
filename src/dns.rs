use crate::mapping::Registry;
use anyhow::Result;
use std::sync::Arc;

pub struct DnsResolver {
    registry: Arc<Registry>,
}

impl DnsResolver {
    pub fn new(registry: Arc<Registry>) -> Self {
        todo!()
    }

    /// Starts the DNS server listening on the specified address.
    pub async fn run(&self, listen_addr: &str) -> Result<()> {
        todo!()
    }
}
