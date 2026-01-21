use crate::mapping::Registry;
use anyhow::{Context, Result};
use dashmap::DashMap;
use iroh::{Endpoint, EndpointId, Watcher};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};

/// ALPN protocol identifier for iron packet transport
pub const ALPN: &[u8] = b"iron/packet/0";

/// Maximum packet size (MTU)
const MAX_PACKET_SIZE: usize = 1500;

/// Iroh protocol handler for packet transport
///
/// This component bridges TUN interface and iroh QUIC connections:
/// - **Send**: Receives (EndpointId, packet) from TUN, sends via QUIC with connection pooling
/// - **Receive**: Accepts QUIC connections, forwards packets to TUN
pub struct IronProtocol {
    registry: Arc<Registry>,
    endpoint: Endpoint,
    /// Receives packets from TUN to send to peers
    to_network_rx: mpsc::UnboundedReceiver<(EndpointId, Vec<u8>)>,
    /// Sends received packets to TUN
    from_network_tx: mpsc::UnboundedSender<Vec<u8>>,
    /// Connection pool: maps EndpointId -> Connection for reuse
    connection_pool: Arc<DashMap<EndpointId, iroh::endpoint::Connection>>,
}

impl IronProtocol {
    /// Creates a new protocol handler
    pub fn new(
        registry: Arc<Registry>,
        endpoint: Endpoint,
        to_network_rx: mpsc::UnboundedReceiver<(EndpointId, Vec<u8>)>,
        from_network_tx: mpsc::UnboundedSender<Vec<u8>>,
    ) -> Self {
        info!("Creating protocol handler for endpoint {}", endpoint.id());
        Self {
            registry,
            endpoint,
            to_network_rx,
            from_network_tx,
            connection_pool: Arc::new(DashMap::new()),
        }
    }

    /// Starts the protocol handler
    ///
    /// This spawns two concurrent tasks:
    /// 1. **Send loop**: Reads from `to_network_rx`, sends packets via QUIC
    /// 2. **Accept loop**: Accepts incoming connections, receives packets
    pub async fn run(mut self) -> Result<()> {
        let endpoint = self.endpoint.clone();
        let from_network_tx = self.from_network_tx.clone();
        let registry = self.registry.clone();

        // Spawn accept loop to handle incoming connections
        let accept_handle = tokio::spawn(async move {
            if let Err(e) = Self::accept_loop(endpoint, registry, from_network_tx).await {
                error!("Accept loop failed: {}", e);
            }
        });

        // Run send loop in current task
        let send_result = self.send_loop().await;

        // Wait for accept loop to complete
        accept_handle.abort();
        let _ = accept_handle.await;

        send_result
    }

    /// Send loop: reads packets from TUN and sends them to peers via QUIC
    async fn send_loop(&mut self) -> Result<()> {
        info!("Starting send loop");
        let self_id = self.endpoint.id();

        while let Some((dest_endpoint_id, packet)) = self.to_network_rx.recv().await {
            debug!(
                "Sending packet to {} ({} bytes)",
                dest_endpoint_id,
                packet.len()
            );

            // Check for loopback (sending to ourselves)
            if dest_endpoint_id == self_id {
                debug!("Loopback detected: cannot connect to self (P2P requires two nodes)");
                // Note: We cannot implement proper loopback without protocol-specific
                // packet rewriting (ICMP echo reply, TCP handshake, etc.).
                // This is by design - iron is for peer-to-peer networking.
                continue;
            }

            if let Err(e) = self.send_packet(&dest_endpoint_id, &packet).await {
                warn!("Failed to send packet to {}: {}", dest_endpoint_id, e);
                // Continue processing other packets
            }
        }

        info!("Send loop terminated (channel closed)");
        Ok(())
    }

    /// Sends a single packet to a peer
    ///
    /// Uses connection pooling to reuse existing connections when possible,
    /// avoiding repeated handshakes.
    async fn send_packet(&self, dest: &EndpointId, packet: &[u8]) -> Result<()> {
        // Try to get existing connection from pool
        let conn = if let Some(cached_conn) = self.connection_pool.get(dest) {
            trace!("Reusing cached connection to {}", dest);
            cached_conn.value().clone()
        } else {
            info!(
                "Attempting to connect to peer {} (no cached connection)",
                dest
            );
            // Create new connection
            let new_conn = self
                .endpoint
                .connect(*dest, ALPN)
                .await
                .context("Failed to connect to peer")?;

            // Cache it for future use
            self.connection_pool.insert(*dest, new_conn.clone());

            // Log connection type for diagnostics
            if let Some(mut conn_type_watcher) = self.endpoint.conn_type(*dest) {
                match conn_type_watcher.get() {
                    iroh::endpoint::ConnectionType::Direct(addr) => {
                        debug!("Direct connection established to {} via {}", dest, addr);
                    }
                    iroh::endpoint::ConnectionType::Relay(url) => {
                        debug!("Relayed connection to {} via {}", dest, url);
                    }
                    iroh::endpoint::ConnectionType::Mixed(addr, url) => {
                        debug!(
                            "Mixed connection to {} (direct: {}, relay: {})",
                            dest, addr, url
                        );
                    }
                    iroh::endpoint::ConnectionType::None => {
                        debug!("Connection to {} established but type unknown", dest);
                    }
                }
            }

            info!("Successfully connected to {} and cached connection", dest);
            new_conn
        };

        trace!("Opening bi-directional stream to {}", dest);
        // Open bi-directional stream
        let stream_result = conn.open_bi().await;

        let (mut send, _recv) = match stream_result {
            Ok(s) => s,
            Err(e) => {
                // Connection might be stale, remove from pool and retry once
                warn!(
                    "Failed to open stream on cached connection to {}: {}",
                    dest, e
                );
                self.connection_pool.remove(dest);

                // Retry with new connection
                trace!("Retrying with new connection to {}", dest);
                let new_conn = self
                    .endpoint
                    .connect(*dest, ALPN)
                    .await
                    .context("Failed to connect to peer on retry")?;

                self.connection_pool.insert(*dest, new_conn.clone());

                new_conn
                    .open_bi()
                    .await
                    .context("Failed to open bi-directional stream on retry")?
            }
        };

        // Write packet data
        send.write_all(packet)
            .await
            .context("Failed to write packet data")?;

        // Finish stream (sends FIN)
        send.finish().context("Failed to finish stream")?;

        debug!("Successfully sent packet to {}", dest);
        Ok(())
    }

    /// Accept loop: accepts incoming connections and receives packets
    async fn accept_loop(
        endpoint: Endpoint,
        registry: Arc<Registry>,
        from_network_tx: mpsc::UnboundedSender<Vec<u8>>,
    ) -> Result<()> {
        info!("Starting accept loop on {}", endpoint.id());

        loop {
            // Accept incoming connection
            let Some(incoming) = endpoint.accept().await else {
                info!("Endpoint closed, stopping accept loop");
                break;
            };

            let conn = match incoming.await {
                Ok(conn) => conn,
                Err(e) => {
                    warn!("Failed to accept connection: {}", e);
                    continue;
                }
            };

            let sender_id = conn.remote_id();
            debug!("Accepted connection from {}", sender_id);

            // Log connection type for diagnostics
            if let Some(mut conn_type_watcher) = endpoint.conn_type(sender_id) {
                match conn_type_watcher.get() {
                    iroh::endpoint::ConnectionType::Direct(addr) => {
                        debug!("Incoming direct connection from {} via {}", sender_id, addr);
                    }
                    iroh::endpoint::ConnectionType::Relay(url) => {
                        debug!("Incoming relayed connection from {} via {}", sender_id, url);
                    }
                    iroh::endpoint::ConnectionType::Mixed(addr, url) => {
                        debug!(
                            "Incoming mixed connection from {} (direct: {}, relay: {})",
                            sender_id, addr, url
                        );
                    }
                    iroh::endpoint::ConnectionType::None => {
                        debug!(
                            "Incoming connection from {} accepted but type unknown",
                            sender_id
                        );
                    }
                }
            }

            // Spawn task to handle this connection
            let registry = registry.clone();
            let from_network_tx = from_network_tx.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    Self::handle_connection(conn, sender_id, registry, from_network_tx).await
                {
                    warn!("Connection handler failed for {}: {}", sender_id, e);
                }
            });
        }

        Ok(())
    }

    /// Handles a single connection from a peer
    ///
    /// This function loops to handle multiple streams on the same connection,
    /// allowing for connection reuse and avoiding repeated handshakes.
    async fn handle_connection(
        conn: iroh::endpoint::Connection,
        sender_id: EndpointId,
        registry: Arc<Registry>,
        from_network_tx: mpsc::UnboundedSender<Vec<u8>>,
    ) -> Result<()> {
        debug!(
            "Handling connection from {}, accepting streams...",
            sender_id
        );

        // Loop to handle multiple streams on this connection
        loop {
            trace!("Waiting for bi-directional stream from {}", sender_id);

            // Accept bi-directional stream
            let stream_result = conn.accept_bi().await;

            let (mut send, mut recv) = match stream_result {
                Ok(s) => s,
                Err(e) => {
                    // Connection closed or error - this is normal when peer closes connection
                    debug!("Connection from {} closed: {}", sender_id, e);
                    break;
                }
            };

            // Read packet data
            let packet = match recv.read_to_end(MAX_PACKET_SIZE).await {
                Ok(p) => p,
                Err(e) => {
                    warn!("Failed to read packet from {}: {}", sender_id, e);
                    continue; // Try next stream
                }
            };

            debug!(
                "Received packet from {} ({} bytes)",
                sender_id,
                packet.len()
            );

            // Rewrite source address to sender's derived IPv6
            // This ensures the OS sees the correct source for routing return packets
            let packet_with_correct_source =
                match Self::rewrite_source_address(packet, &sender_id, &registry) {
                    Ok(p) => p,
                    Err(e) => {
                        warn!("Failed to rewrite source address for {}: {}", sender_id, e);
                        continue; // Try next stream
                    }
                };

            trace!("Forwarding packet to TUN with rewritten source");
            // Forward packet to TUN
            if let Err(e) = from_network_tx.send(packet_with_correct_source) {
                error!("Failed to send packet to TUN: {}", e);
                break; // TUN channel closed, exit
            }

            // Close send side
            if let Err(e) = send.finish() {
                warn!("Failed to finish send stream for {}: {}", sender_id, e);
            }
        }

        debug!("Connection handler for {} exiting", sender_id);
        Ok(())
    }

    /// Rewrites the packet's source IPv6 to match the sender's derived IPv6
    ///
    /// Since iroh provides cryptographic authentication via EndpointId, we don't
    /// need to verify the source address. Instead, we rewrite it to the sender's
    /// derived IPv6 so the OS can properly route return packets.
    fn rewrite_source_address(
        packet: Vec<u8>,
        sender_id: &EndpointId,
        registry: &Arc<Registry>,
    ) -> Result<Vec<u8>> {
        use etherparse::Ipv6Header;

        // Parse IPv6 header - returns (header, remaining_payload_slice)
        let (ipv6_header, payload) =
            Ipv6Header::from_slice(&packet).context("Failed to parse IPv6 header")?;

        // Get sender's derived IPv6
        let sender_ipv6 = registry.get_or_assign_ip(*sender_id);

        // Rewrite source address
        let mut header = ipv6_header;
        header.source = sender_ipv6.octets();

        // Rebuild packet with new source
        let mut new_packet = Vec::with_capacity(packet.len());
        header
            .write(&mut new_packet)
            .context("Failed to write IPv6 header")?;
        new_packet.extend_from_slice(payload);

        trace!(
            "Rewrote source address to {} for sender {}",
            sender_ipv6, sender_id
        );

        Ok(new_packet)
    }
}
