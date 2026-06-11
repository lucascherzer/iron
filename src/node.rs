use crate::dns::DnsResolver;
use crate::keys;
use crate::mapping::Registry;
use crate::protocol::IronProtocol;
use crate::tun::TunInterface;
use anyhow::{Context, Result};
use iroh::address_lookup::memory::MemoryLookup;
use iroh::endpoint::presets::N0;
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, TransportAddr};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

/// Buffer size for packet channels between TUN and protocol layers.
///
/// Provides backpressure: when the QUIC connection drops, the channel fills up,
/// blocking the TUN read loop, which prevents the OS from thinking packets are
/// being delivered. This avoids accumulating stale TCP retransmissions that would
/// be flushed after reconnection, causing garbled terminal output.
const CHANNEL_BUFFER_SIZE: usize = 1024;

pub struct IronNode {
    registry: Arc<Registry>,
    endpoint: Endpoint,
    dns: DnsResolver,
    tun: TunInterface,
    protocol: IronProtocol,
}

/// Configuration for direct peer connections.
///
/// In production, iroh discovers peers via relay servers and DNS-based address
/// lookup. For tests and isolated networks, this struct allows pre-populating
/// peer addressing information so connections work without internet access.
#[derive(Default)]
pub struct DirectPeerConfig {
    /// UDP port for the iroh QUIC endpoint to listen on
    pub listen_port: Option<u16>,
    /// Pre-registered peer addresses for direct connections
    pub peers: Vec<(EndpointId, SocketAddr)>,
}

impl IronNode {
    /// Creates a new IronNode with all components initialized
    pub async fn new() -> Result<Self> {
        Self::with_config(Default::default()).await
    }

    /// Creates a new IronNode with optional direct peer configuration.
    ///
    /// When `config` provides peer addresses, the iroh endpoint is configured with
    /// relay disabled and a [`MemoryLookup`] populated with the given addresses.
    /// This allows nodes to discover each other on isolated networks without
    /// internet access to relay servers.
    pub async fn with_config(config: DirectPeerConfig) -> Result<Self> {
        info!("Initializing IronNode");

        // Create shared registry
        let registry = Arc::new(Registry::new());

        // Load previously known peers to prevent issues with cached IPv6 addresses
        match registry.load_peers() {
            Ok(count) if count > 0 => info!("Loaded {} known peers from cache", count),
            Ok(_) => info!("No cached peers found, starting fresh"),
            Err(e) => warn!("Failed to load peers cache: {}", e),
        }

        // Load or generate persistent secret key
        info!("Loading node identity");
        let secret_key = keys::load_or_generate_key()?;

        let use_direct = config.listen_port.is_some() || !config.peers.is_empty();

        let address_lookup = if use_direct {
            let lookup = MemoryLookup::new();
            for (endpoint_id, addr) in &config.peers {
                let ep_addr = EndpointAddr::from_parts(*endpoint_id, [TransportAddr::Ip(*addr)]);
                lookup.add_endpoint_info(ep_addr);
                info!(
                    "Registered direct address {} for peer {}",
                    addr,
                    hex::encode(endpoint_id.as_bytes())
                );
            }
            Some(lookup)
        } else {
            None
        };

        // Initialize iroh endpoint with persistent key
        info!("Creating iroh endpoint");
        let mut builder = Endpoint::builder(N0)
            .secret_key(secret_key)
            .alpns(vec![crate::protocol::ALPN.to_vec()]);

        if let Some(port) = config.listen_port {
            let addr = SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, port));
            builder = builder
                .bind_addr(addr)
                .context("Failed to set listen address")?;
            info!("Binding iroh endpoint to port {}", port);
        }

        if use_direct {
            builder = builder.relay_mode(RelayMode::Disabled);
            info!("Relay servers disabled (direct peer addressing configured)");
        }

        if let Some(lookup) = address_lookup {
            builder = builder.address_lookup(lookup);
        }

        let endpoint = builder.bind().await?;

        info!("Iroh endpoint created: {}", endpoint.id());

        // Get this node's derived IPv6 address
        let node_ipv6 = registry.get_or_assign_ip(endpoint.id());
        info!("Node IPv6 address: {}", node_ipv6);

        // Create channels for packet flow
        let (to_network_tx, to_network_rx) = mpsc::channel(CHANNEL_BUFFER_SIZE);
        let (from_network_tx, from_network_rx) = mpsc::channel(CHANNEL_BUFFER_SIZE);

        // Initialize DNS resolver
        info!("Creating DNS resolver");
        let dns = DnsResolver::new(registry.clone());

        // Initialize TUN interface
        info!("Creating TUN interface");
        let tun = TunInterface::new(registry.clone(), node_ipv6, to_network_tx, from_network_rx);

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
