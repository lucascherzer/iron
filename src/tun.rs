use crate::mapping::Registry;
use anyhow::Result;
use std::sync::Arc;

pub struct TunInterface {
    registry: Arc<Registry>,
}

impl TunInterface {
    pub fn new(registry: Arc<Registry>) -> Self {
        todo!()
    }

    /// Initializes the TUN device and starts the packet processing loop.
    pub async fn run(&self) -> Result<()> {
        todo!()
    }

    /// Handles an incoming packet from the OS.
    async fn handle_packet(&self, packet: &[u8]) -> Result<()> {
        todo!()
    }
}
