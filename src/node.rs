use crate::dns::DnsResolver;
use crate::keys;
use crate::mapping::Registry;
use crate::protocol::IronProtocol;
use crate::tun::TunInterface;
use anyhow::{Context, Result};
use iroh::address_lookup::memory::MemoryLookup;
use iroh::endpoint::presets::N0;
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMap, RelayMode, RelayUrl, TransportAddr};
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

/// Configuration for `IronNode` that allows overriding defaults for testing.
///
/// When `default()` is used, the standard N0 preset is applied.
/// The config can be populated from environment variables for VM/integration tests.
#[derive(Default)]
pub struct IronNodeConfig {
    /// Override the relay URL (disables N0 preset relay, uses custom relay map)
    pub relay_url: Option<String>,
    /// Pre-registered peer addresses for direct connections
    pub peers: Vec<(EndpointId, String)>,
}

impl IronNodeConfig {
    /// Load configuration from environment variables.
    ///
    /// - `IROH_RELAY_URL`: Custom relay server URL
    /// - `IROH_PEER_*`: Repeating variable, format `base32_id@ip:port` or `base32_id@relay_url`
    pub fn from_env() -> Self {
        let relay_url = std::env::var("IROH_RELAY_URL").ok();
        let mut peers = Vec::new();
        for (key, value) in std::env::vars() {
            if key.starts_with("IROH_PEER_")
                && let Some((base32, addr)) = value.split_once('@')
            {
                let bytes = data_encoding::BASE32_NOPAD
                    .decode(base32.to_uppercase().as_bytes())
                    .ok();
                if let Some(bytes) = bytes
                    && bytes.len() == 32
                {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&bytes);
                    if let Ok(id) = EndpointId::from_bytes(&arr) {
                        peers.push((id, addr.to_string()));
                    }
                }
            }
        }
        Self { relay_url, peers }
    }
}

impl IronNode {
    /// Creates a new IronNode with all components initialized.
    ///
    /// Configures the iroh endpoint using the N0 preset (public relays + Pkarr DNS discovery).
    /// Custom infrastructure can be configured via `IronNodeConfig` or environment variables.
    pub async fn new() -> Result<Self> {
        Self::with_config(&IronNodeConfig::default()).await
    }

    /// Creates a new IronNode with the given configuration.
    ///
    /// When `config` specifies a custom relay URL or peers, the endpoint is configured
    /// with `RelayMode::Custom` and a `MemoryLookup` populated with peer addresses.
    pub async fn with_config(config: &IronNodeConfig) -> Result<Self> {
        info!("Initializing IronNode");

        let registry = Arc::new(Registry::new());

        match registry.load_peers() {
            Ok(count) if count > 0 => info!("Loaded {} known peers from cache", count),
            Ok(_) => info!("No cached peers found, starting fresh"),
            Err(e) => warn!("Failed to load peers cache: {}", e),
        }

        let secret_key = keys::load_or_generate_key()?;

        // Build endpoint
        info!("Creating iroh endpoint");
        let mut builder = Endpoint::builder(N0)
            .secret_key(secret_key)
            .alpns(vec![crate::protocol::ALPN.to_vec()]);

        // Apply custom relay configuration
        if let Some(ref relay_url_str) = config.relay_url {
            let relay_url: RelayUrl = relay_url_str.parse().context("Invalid IROH_RELAY_URL")?;
            let relay_map = RelayMap::from_iter([relay_url]);
            builder = builder.relay_mode(RelayMode::Custom(relay_map));
            info!("Using custom relay: {}", relay_url_str);
        }

        // Apply custom peer addresses
        if !config.peers.is_empty() {
            let lookup = MemoryLookup::new();
            for (endpoint_id, addr_str) in &config.peers {
                let transport = if addr_str.contains("://") {
                    let relay_url: RelayUrl = addr_str.parse().context("Invalid peer relay URL")?;
                    TransportAddr::Relay(relay_url)
                } else {
                    let socket_addr: std::net::SocketAddr =
                        addr_str.parse().context("Invalid peer socket address")?;
                    TransportAddr::Ip(socket_addr)
                };
                let ep_addr = EndpointAddr::from_parts(*endpoint_id, [transport]);
                lookup.add_endpoint_info(ep_addr);
                info!(
                    "Registered peer {} via {}",
                    hex::encode(endpoint_id.as_bytes()),
                    addr_str
                );
            }
            builder = builder.address_lookup(lookup);
        }

        let endpoint = builder.bind().await?;

        info!("Iroh endpoint created: {}", endpoint.id());

        // Get this node's derived IPv6 address
        let node_ipv6 = registry.get_or_assign_ip(endpoint.id());
        info!("Node IPv6 address: {}", node_ipv6);

        // Create channels for packet flow
        // OS → Network: TUN sends packets to protocol handler
        let (to_network_tx, to_network_rx) = mpsc::channel(CHANNEL_BUFFER_SIZE);
        // Network → OS: Protocol handler sends packets to TUN
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
