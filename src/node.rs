use crate::dns::DnsResolver;
use crate::mapping::Registry;
use crate::protocol::IronProtocol;
use crate::tun::TunInterface;
use anyhow::Result;
use iroh::Endpoint;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info};

pub struct IronNode {
    registry: Arc<Registry>,
    endpoint: Endpoint,
    dns: DnsResolver,
    tun: TunInterface,
    protocol: IronProtocol,
}

impl IronNode {
    /// Creates a new IronNode with all components initialized
    ///
    /// This sets up:
    /// - Registry for EndpointId <-> IPv6 mapping
    /// - Iroh endpoint for QUIC connections
    /// - DNS resolver for `.iron` domains
    /// - TUN interface for packet interception
    /// - Protocol handler for packet transport
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Iroh endpoint fails to bind
    /// - Any component initialization fails
    pub async fn new() -> Result<Self> {
        info!("Initializing IronNode");

        // Create shared registry
        let registry = Arc::new(Registry::new());

        // Initialize iroh endpoint
        info!("Creating iroh endpoint");
        let endpoint = Endpoint::builder()
            .alpns(vec![crate::protocol::ALPN.to_vec()])
            .bind()
            .await?;

        info!("Iroh endpoint created: {}", endpoint.id());

        // Create channels for packet flow
        // OS → Network: TUN sends packets to protocol handler
        let (to_network_tx, to_network_rx) = mpsc::unbounded_channel();
        // Network → OS: Protocol handler sends packets to TUN
        let (from_network_tx, from_network_rx) = mpsc::unbounded_channel();

        // Initialize DNS resolver
        info!("Creating DNS resolver");
        let dns = DnsResolver::new(registry.clone());

        // Initialize TUN interface
        info!("Creating TUN interface");
        let tun = TunInterface::new(registry.clone(), to_network_tx, from_network_rx);

        // Initialize protocol handler
        info!("Creating protocol handler");
        let protocol = IronProtocol::new(
            registry.clone(),
            endpoint.clone(),
            to_network_rx,
            from_network_tx,
        );

        info!("IronNode initialized successfully");

        Ok(Self {
            registry,
            endpoint,
            dns,
            tun,
            protocol,
        })
    }

    /// Returns the EndpointId of this node
    pub fn endpoint_id(&self) -> iroh::EndpointId {
        self.endpoint.id()
    }

    /// Returns a reference to the shared registry
    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// Orchestrates the startup of all components.
    ///
    /// This starts:
    /// 1. DNS resolver (listening on 127.0.0.1:5333)
    /// 2. TUN interface (requires root/sudo)
    /// 3. Protocol handler (iroh packet transport)
    ///
    /// All components run concurrently. If any component fails, all are shut down.
    ///
    /// # Errors
    ///
    /// Returns an error if any component fails to start or encounters a fatal error.
    pub async fn start(self) -> Result<()> {
        info!("Starting IronNode (ID: {})", self.endpoint_id());

        // Start DNS resolver
        let dns_handle = tokio::spawn(async move {
            if let Err(e) = self.dns.run("127.0.0.1:5333").await {
                error!("DNS resolver failed: {}", e);
            }
        });

        // Start TUN interface
        let tun_handle = tokio::spawn(async move {
            if let Err(e) = self.tun.run().await {
                error!("TUN interface failed: {}", e);
            }
        });

        // Start protocol handler (runs in current task)
        let protocol_result = self.protocol.run().await;

        // If protocol handler exits, stop other components
        info!("Protocol handler exited, shutting down other components");
        dns_handle.abort();
        tun_handle.abort();

        let _ = dns_handle.await;
        let _ = tun_handle.await;

        info!("IronNode shutdown complete");
        protocol_result
    }
}
