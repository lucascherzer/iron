use crate::dns::DnsResolver;
use crate::keys;
use crate::mapping::Registry;
use crate::paths::StatePaths;
use crate::platform::{PrivateFileWriter, TunBackend};
use crate::protocol::IronProtocol;
use crate::router::PacketRouter;
use crate::state::AppState;
use anyhow::{Context, Result};
use iroh::Endpoint;
use iroh::endpoint::presets::N0;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

/// Platform-dependent pieces an [`IronNode`] is assembled from.
///
/// The node itself is platform-independent; everything OS-specific is
/// injected here. Desktop callers can use
/// `iron_desktop::desktop_node_config`, while other platforms construct the
/// fields explicitly.
pub struct NodeConfig {
    /// Where persistent state (secret key, known-peers cache) lives.
    pub paths: StatePaths,
    /// Secure create-only storage implementation for private key material.
    pub storage: Arc<dyn PrivateFileWriter>,
    /// Iroh peer discovery and relay policy.
    pub iroh: IrohConfig,
    /// Address the UDP DNS frontend listens on.
    pub dns_listen: SocketAddr,
    /// Upstream DNS servers for non-`.iron` queries; empty refuses them.
    pub dns_upstream: Vec<SocketAddr>,
    /// Provisions the TUN device and exposes its packet I/O.
    pub tun: Box<dyn TunBackend>,
}

/// Available discovery and relay infrastructure, not a transport policy.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IrohConfig {
    #[serde(default)]
    /// Overrides both publishing and lookup; omission retains iroh defaults.
    pub pkarr_server: Option<url::Url>,
    #[serde(default = "default_relays")]
    /// Whether built-in relay servers participate alongside custom servers.
    pub default_relays: bool,
    #[serde(default)]
    /// Additional servers, or the entire server set when defaults are off.
    pub relay_servers: Vec<iroh::RelayUrl>,
}

fn default_relays() -> bool {
    true
}

impl Default for IrohConfig {
    fn default() -> Self {
        Self {
            pkarr_server: None,
            default_relays: true,
            relay_servers: Vec::new(),
        }
    }
}

impl IrohConfig {
    /// Resolve available relay infrastructure without changing iroh's
    /// connection coordination, hole punching, or fallback transport.
    pub fn relay_mode(&self) -> Result<iroh::RelayMode> {
        if !self.default_relays && self.relay_servers.is_empty() {
            anyhow::bail!("iroh.default_relays=false requires at least one relay_server");
        }
        if self.default_relays && self.relay_servers.is_empty() {
            return Ok(iroh::endpoint::default_relay_mode());
        }
        let mut urls = if self.default_relays {
            iroh::endpoint::default_relay_mode()
                .relay_map()
                .urls::<Vec<_>>()
        } else {
            Vec::new()
        };
        urls.extend(self.relay_servers.iter().cloned());
        Ok(iroh::RelayMode::custom(urls))
    }

    fn endpoint_builder(&self) -> Result<iroh::endpoint::Builder> {
        let mut builder = Endpoint::builder(N0).relay_mode(self.relay_mode()?);
        if let Some(server) = &self.pkarr_server {
            // Clear all N0 lookup services, including DNS, so an explicit
            // discovery override never leaks back to public infrastructure.
            builder = builder
                .clear_address_lookup()
                .address_lookup(iroh::address_lookup::PkarrPublisher::builder(
                    server.clone(),
                ))
                .address_lookup(iroh::address_lookup::PkarrResolver::builder(server.clone()));
        }
        Ok(builder)
    }
}

impl NodeConfig {
    /// Set the UDP DNS frontend port.
    pub fn with_dns_port(mut self, port: u16) -> Self {
        self.dns_listen.set_port(port);
        self
    }
}

pub struct IronNode {
    state: AppState,
    registry: Arc<Registry>,
    endpoint: Endpoint,
    dns: Arc<DnsResolver>,
    dns_listen: SocketAddr,
    router: PacketRouter,
    protocol: IronProtocol,
    tun: Box<dyn TunBackend>,
}

impl IronNode {
    /// Creates a new IronNode with all components initialized
    ///
    /// This sets up:
    /// - Registry for EndpointId <-> IPv6 mapping
    /// - Iroh endpoint for QUIC connections
    /// - DNS resolver for `.iron` domains
    /// - Packet router bridging TUN packet I/O and the protocol handler
    /// - Protocol handler for packet transport
    ///
    /// The TUN device itself is provisioned by `config.tun` when
    /// [`IronNode::start`] runs.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Iroh endpoint fails to bind
    /// - Any component initialization fails
    pub async fn new(config: NodeConfig) -> Result<Self> {
        info!("Initializing IronNode");

        let NodeConfig {
            paths,
            storage,
            iroh,
            dns_listen,
            dns_upstream,
            tun,
        } = config;

        let builder = iroh.endpoint_builder()?;
        // Open state before identity generation or network startup; malformed
        // state is fatal and must never be replaced with an empty database.
        let state = AppState::open(&paths.state_file(), storage.as_ref()).await?;
        let saved_peers = state.load_peers().await?;

        // Create shared registry
        let registry = Arc::new(Registry::new());
        for peer in saved_peers.iter().copied() {
            registry.register(peer);
        }
        info!(
            "Loaded {} known peers from application state",
            saved_peers.len()
        );

        // Load or generate persistent secret key
        info!("Loading node identity");
        let secret_key = keys::load_or_generate_key(&paths, storage.as_ref())?;

        // Initialize iroh endpoint with persistent key
        info!("Creating iroh endpoint");
        let endpoint = builder
            .secret_key(secret_key)
            .alpns(vec![crate::protocol::ALPN.to_vec()])
            .bind()
            .await?;

        info!("Iroh endpoint created: {}", endpoint.id());

        // This node's own address. Registering ourselves (as before) means a
        // packet to our own address reaches the protocol's loopback check
        // instead of being dropped as an unknown destination.
        let node_ipv6 = registry.register(endpoint.id());
        info!("Node IPv6 address: {}", node_ipv6);

        // Create channels for packet flow
        // OS → Network: packet router sends packets to protocol handler
        let (to_network_tx, to_network_rx) = mpsc::unbounded_channel();
        // Network → OS: protocol handler sends packets to packet router
        let (from_network_tx, from_network_rx) = mpsc::unbounded_channel();

        // Initialize DNS resolver
        info!("Creating DNS resolver");
        let dns = Arc::new(DnsResolver::new(registry.clone(), dns_upstream));

        // Initialize packet router
        info!("Creating packet router");
        let router = PacketRouter::new(
            registry.clone(),
            Arc::clone(&dns),
            to_network_tx,
            from_network_rx,
        );

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
            state,
            registry,
            endpoint,
            dns,
            dns_listen,
            router,
            protocol,
            tun,
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

    /// Returns the shared application-state store for snapshot persistence.
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Orchestrates the startup of all components.
    ///
    /// This starts:
    /// 1. The TUN device (provisioned by the platform backend; on desktop
    ///    this requires root/sudo)
    /// 2. DNS resolver (listening on the configured address)
    /// 3. Packet router (bridging TUN packet I/O and iroh)
    /// 4. Protocol handler (iroh packet transport)
    ///
    /// All components run concurrently. If any component fails, all are shut down.
    ///
    /// # Errors
    ///
    /// Returns an error if the TUN device cannot be provisioned, or if any
    /// component encounters a fatal error.
    pub async fn start(self) -> Result<()> {
        info!("Starting IronNode (ID: {})", self.endpoint_id());

        // Provision the TUN device first: without it there is no data plane,
        // so failing fast beats running a node that cannot carry traffic.
        let node_ipv6 = crate::id::derive_ip(&self.endpoint.id());
        let tun_io = self.tun.open(node_ipv6)?;

        // Run DNS, router and protocol concurrently. They are all meant to
        // run forever, so whichever returns first (normally with an error,
        // e.g. the DNS port is taken or TUN I/O failed) ends the node, and
        // the others are dropped with it instead of limping on.
        let result = tokio::select! {
            r = crate::dns::serve_udp(self.dns, self.dns_listen) => r.context("DNS server failed"),
            r = self.router.run(tun_io) => r.context("packet router failed"),
            r = self.protocol.run() => r.context("protocol handler exited"),
        };
        info!("IronNode shutdown complete");
        result
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn relay_policy_preserves_or_replaces_defaults() {
        let custom: iroh::RelayUrl = "https://relay.example.test".parse().unwrap();
        let defaults = IrohConfig::default().relay_mode().unwrap().relay_map();
        let cases = [
            (true, vec![], defaults.urls::<Vec<_>>().len()),
            (
                true,
                vec![custom.clone()],
                defaults.urls::<Vec<_>>().len() + 1,
            ),
            (false, vec![custom.clone()], 1),
        ];
        for (default_relays, relay_servers, expected) in cases {
            let config = IrohConfig {
                default_relays,
                relay_servers,
                ..Default::default()
            };
            assert_eq!(
                config
                    .relay_mode()
                    .unwrap()
                    .relay_map()
                    .urls::<Vec<_>>()
                    .len(),
                expected
            );
        }
        assert!(
            IrohConfig {
                default_relays: false,
                ..Default::default()
            }
            .relay_mode()
            .is_err()
        );
    }
}
