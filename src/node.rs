use crate::dns::DnsResolver;
use crate::mapping::Registry;
use crate::tun::TunInterface;
use anyhow::Result;
use iroh::Endpoint;
use std::sync::Arc;

pub struct IronNode {
    registry: Arc<Registry>,
    endpoint: Endpoint,
    dns: DnsResolver,
    tun: TunInterface,
}

impl IronNode {
    pub async fn new() -> Result<Self> {
        todo!()
    }

    /// Orchestrates the startup of all components.
    pub async fn start(&self) -> Result<()> {
        todo!()
    }
}
