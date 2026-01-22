use crate::firewall::FirewallConfig;
use crate::mapping::Registry;
use crate::packet::{AuthMessage, AuthResponse, Packet};
use anyhow::{Context, Result};
use dashmap::DashMap;
use iroh::{Endpoint, EndpointId, Watcher};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error, info, trace, warn};

/// ALPN protocol identifier for iron packet transport
pub const ALPN: &[u8] = b"iron/packet/0";

/// Maximum packet size (MTU)
const MAX_PACKET_SIZE: usize = 1500;

/// Iroh protocol handler for packet transport
///
/// This component bridges TUN interface and iroh QUIC connections:
/// - **Send**: Receives (EndpointId, Packet) from TUN, sends via QUIC with connection pooling
/// - **Receive**: Accepts QUIC connections, forwards Packets to TUN
/// - **Firewall**: Optionally authenticates devices using person ownership claims
pub struct IronProtocol {
    registry: Arc<Registry>,
    endpoint: Endpoint,
    /// Receives packets from TUN to send to peers
    to_network_rx: mpsc::UnboundedReceiver<(EndpointId, Packet)>,
    /// Sends received packets to TUN
    from_network_tx: mpsc::UnboundedSender<Packet>,
    /// Connection pool: maps EndpointId -> Connection for reuse
    connection_pool: Arc<DashMap<EndpointId, iroh::endpoint::Connection>>,
    /// Firewall configuration (wrapped in RwLock for concurrent access and mutation)
    firewall: Arc<RwLock<FirewallConfig>>,
    /// Track connections we've successfully authenticated to (to avoid re-sending claims)
    authenticated_to: Arc<DashMap<EndpointId, bool>>,
}

impl IronProtocol {
    /// Creates a new protocol handler
    pub fn new(
        registry: Arc<Registry>,
        endpoint: Endpoint,
        to_network_rx: mpsc::UnboundedReceiver<(EndpointId, Packet)>,
        from_network_tx: mpsc::UnboundedSender<Packet>,
        firewall: FirewallConfig,
    ) -> Self {
        info!("Creating protocol handler for endpoint {}", endpoint.id());
        Self {
            registry,
            endpoint,
            to_network_rx,
            from_network_tx,
            connection_pool: Arc::new(DashMap::new()),
            firewall: Arc::new(RwLock::new(firewall)),
            authenticated_to: Arc::new(DashMap::new()),
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
        let firewall = self.firewall.clone();

        // Spawn accept loop to handle incoming connections
        let accept_handle = tokio::spawn(async move {
            if let Err(e) = Self::accept_loop(endpoint, registry, from_network_tx, firewall).await {
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
    async fn send_packet(&self, dest: &EndpointId, packet: &Packet) -> Result<()> {
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

            // Try to authenticate if we have a claim (ignore failures - peer might not have firewall enabled)
            if let Err(e) = self.try_authenticate(&new_conn, dest).await {
                debug!(
                    "Authentication to {} failed (peer might not require it): {}",
                    dest, e
                );
                // Don't fail the connection - peer might not have firewall enabled
                // The actual data packet send will fail if authentication was required
            }

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

                // Try to authenticate
                match self.try_authenticate(&new_conn, dest).await {
                    Ok(_) => {
                        debug!("Authentication successful or not required");
                    }
                    Err(e) => {
                        warn!("Authentication to {} failed: {}", dest, e);
                        // Remove from pool since authentication failed
                        self.connection_pool.remove(dest);
                        return Err(e).context("Failed to authenticate to peer");
                    }
                }

                new_conn
                    .open_bi()
                    .await
                    .context("Failed to open bi-directional stream on retry")?
            }
        };

        // Write packet data (extract raw bytes)
        let packet_bytes = packet.as_bytes().context("Packet has no raw bytes")?;
        send.write_all(packet_bytes)
            .await
            .context("Failed to write packet data")?;

        // Finish stream (sends FIN)
        send.finish().context("Failed to finish stream")?;

        debug!("Successfully sent packet to {}", dest);
        Ok(())
    }

    /// Try to authenticate to a peer by sending our ownership claim
    /// Returns Ok(true) if authentication succeeded, Ok(false) if we don't have a claim
    async fn try_authenticate(
        &self,
        conn: &iroh::endpoint::Connection,
        dest: &EndpointId,
    ) -> Result<bool> {
        // Check if we've already authenticated to this peer
        if self.authenticated_to.contains_key(dest) {
            trace!("Already authenticated to {}", dest);
            return Ok(true);
        }

        // Try to load our claim for this device
        let our_device_key = self.endpoint.id();
        let claim = match FirewallConfig::load_claim(&our_device_key)? {
            Some(c) => c,
            None => {
                debug!("No ownership claim found for this device, skipping authentication");
                return Ok(false);
            }
        };

        info!("Sending ownership claim to {} for authentication", dest);

        // Open stream for authentication
        let (mut send, mut recv) = conn
            .open_bi()
            .await
            .context("Failed to open authentication stream")?;

        // Send the claim
        let auth_packet = Packet::Auth(AuthMessage::Claim(claim));
        let auth_bytes =
            postcard::to_allocvec(&auth_packet).context("Failed to serialize auth packet")?;

        send.write_all(&auth_bytes)
            .await
            .context("Failed to send auth packet")?;
        send.finish().context("Failed to finish auth stream")?;

        // Wait for response
        let response_bytes = recv
            .read_to_end(MAX_PACKET_SIZE)
            .await
            .context("Failed to read auth response")?;

        let response: Packet =
            postcard::from_bytes(&response_bytes).context("Failed to deserialize auth response")?;

        match response {
            Packet::Auth(AuthMessage::Response(AuthResponse::Accepted)) => {
                info!("Authentication accepted by {}", dest);
                self.authenticated_to.insert(*dest, true);
                Ok(true)
            }
            Packet::Auth(AuthMessage::Response(AuthResponse::Rejected { reason })) => {
                warn!("Authentication rejected by {}: {}", dest, reason);
                anyhow::bail!("Authentication rejected: {}", reason);
            }
            _ => {
                warn!("Unexpected response to authentication from {}", dest);
                anyhow::bail!("Unexpected authentication response");
            }
        }
    }

    /// Accept loop: accepts incoming connections and receives packets
    async fn accept_loop(
        endpoint: Endpoint,
        registry: Arc<Registry>,
        from_network_tx: mpsc::UnboundedSender<Packet>,
        firewall: Arc<RwLock<FirewallConfig>>,
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
            let firewall = firewall.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    Self::handle_connection(conn, sender_id, registry, from_network_tx, firewall)
                        .await
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
    /// If firewall is enabled, the first packet must be an Auth packet.
    async fn handle_connection(
        conn: iroh::endpoint::Connection,
        sender_id: EndpointId,
        registry: Arc<Registry>,
        from_network_tx: mpsc::UnboundedSender<Packet>,
        firewall: Arc<RwLock<FirewallConfig>>,
    ) -> Result<()> {
        debug!(
            "Handling connection from {}, accepting streams...",
            sender_id
        );

        // Check if firewall is enabled
        let firewall_enabled = {
            let fw = firewall.read().await;
            fw.enabled
        };

        // If firewall is enabled, we need authentication first (to know which person this device belongs to)
        if firewall_enabled {
            info!(
                "Firewall enabled: waiting for authentication from {}",
                sender_id
            );

            // Accept first stream for authentication
            let stream_result = conn.accept_bi().await;
            let (mut send, mut recv) = match stream_result {
                Ok(s) => s,
                Err(e) => {
                    warn!("Connection from {} closed before auth: {}", sender_id, e);
                    return Ok(());
                }
            };

            // Read auth packet
            let packet_bytes = match recv.read_to_end(MAX_PACKET_SIZE).await {
                Ok(p) => p,
                Err(e) => {
                    warn!("Failed to read auth packet from {}: {}", sender_id, e);
                    return Ok(());
                }
            };

            // Deserialize packet
            let packet: Packet = match postcard::from_bytes(&packet_bytes) {
                Ok(p) => p,
                Err(e) => {
                    warn!(
                        "Failed to deserialize auth packet from {}: {}",
                        sender_id, e
                    );
                    // Send rejection
                    let response = Packet::Auth(AuthMessage::Response(AuthResponse::Rejected {
                        reason: "Invalid packet format".to_string(),
                    }));
                    let response_bytes = postcard::to_allocvec(&response).unwrap_or_default();
                    let _ = send.write_all(&response_bytes).await;
                    let _ = send.finish();
                    return Ok(());
                }
            };

            // Verify it's an auth packet with a claim
            match packet {
                Packet::Auth(AuthMessage::Claim(claim)) => {
                    debug!("Received ownership claim from {}", sender_id);

                    // Verify the claim matches the sender
                    if claim.device_key != sender_id {
                        warn!(
                            "Claim device key mismatch: claim={}, sender={}",
                            claim.device_key, sender_id
                        );
                        let response =
                            Packet::Auth(AuthMessage::Response(AuthResponse::Rejected {
                                reason: "Device key mismatch".to_string(),
                            }));
                        let response_bytes = postcard::to_allocvec(&response).unwrap_or_default();
                        let _ = send.write_all(&response_bytes).await;
                        let _ = send.finish();
                        return Ok(());
                    }

                    // Verify claim and check if person is trusted
                    let mut fw = firewall.write().await;
                    match fw.verify_claim(&claim) {
                        Ok(true) => {
                            info!("Authentication successful for {}", sender_id);
                            // Save cache to persist the verification
                            if let Err(e) = fw.save_cache() {
                                warn!("Failed to save firewall cache: {}", e);
                            }
                            drop(fw); // Release lock before sending response

                            // Send acceptance
                            let response =
                                Packet::Auth(AuthMessage::Response(AuthResponse::Accepted));
                            let response_bytes =
                                postcard::to_allocvec(&response).unwrap_or_default();
                            if let Err(e) = send.write_all(&response_bytes).await {
                                warn!("Failed to send auth response: {}", e);
                                return Ok(());
                            }
                            if let Err(e) = send.finish() {
                                warn!("Failed to finish auth stream: {}", e);
                            }
                        }
                        Ok(false) => {
                            warn!(
                                "Authentication failed for {}: person not trusted",
                                sender_id
                            );
                            drop(fw);
                            let response =
                                Packet::Auth(AuthMessage::Response(AuthResponse::Rejected {
                                    reason: "Person not trusted".to_string(),
                                }));
                            let response_bytes =
                                postcard::to_allocvec(&response).unwrap_or_default();
                            let _ = send.write_all(&response_bytes).await;
                            let _ = send.finish();
                            return Ok(());
                        }
                        Err(e) => {
                            warn!(
                                "Authentication failed for {}: claim verification error: {}",
                                sender_id, e
                            );
                            drop(fw);
                            let response =
                                Packet::Auth(AuthMessage::Response(AuthResponse::Rejected {
                                    reason: format!("Claim verification failed: {}", e),
                                }));
                            let response_bytes =
                                postcard::to_allocvec(&response).unwrap_or_default();
                            let _ = send.write_all(&response_bytes).await;
                            let _ = send.finish();
                            return Ok(());
                        }
                    }
                }
                _ => {
                    warn!(
                        "Expected Auth packet from {}, got different packet type",
                        sender_id
                    );
                    let response = Packet::Auth(AuthMessage::Response(AuthResponse::Rejected {
                        reason: "Expected authentication packet".to_string(),
                    }));
                    let response_bytes = postcard::to_allocvec(&response).unwrap_or_default();
                    let _ = send.write_all(&response_bytes).await;
                    let _ = send.finish();
                    return Ok(());
                }
            }
        }

        // Authentication successful or not required - proceed with normal packet handling
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

            // Check firewall policy (if enabled and has policies)
            {
                let fw = firewall.read().await;
                if fw.enabled {
                    // Extract destination port from packet (returns None for non-TCP/UDP like ICMP)
                    let dst_port = Self::extract_dst_port(&packet);

                    // If we have a destination port, check policy
                    // If packet has no port (e.g., ICMP), use port 0 as a sentinel
                    let port_to_check = dst_port.unwrap_or(0);

                    if !fw.is_packet_allowed(&sender_id, port_to_check) {
                        warn!(
                            "Packet from {} to port {} blocked by firewall policy",
                            sender_id,
                            if let Some(p) = dst_port {
                                p.to_string()
                            } else {
                                "N/A (non-TCP/UDP)".to_string()
                            }
                        );
                        continue; // Drop packet, try next stream
                    }

                    trace!(
                        "Packet from {} to port {} allowed by firewall",
                        sender_id,
                        if let Some(p) = dst_port {
                            p.to_string()
                        } else {
                            "N/A".to_string()
                        }
                    );
                }
            }

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
            // Forward packet to TUN (wrap in Packet type)
            if let Err(e) = from_network_tx.send(Packet::raw(packet_with_correct_source)) {
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

    /// Extracts the destination port from an IPv6 packet
    ///
    /// Returns None if the packet doesn't have a port (e.g., ICMP) or if parsing fails.
    /// Supports TCP and UDP protocols.
    fn extract_dst_port(packet: &[u8]) -> Option<u16> {
        use etherparse::{Ipv6Header, TcpHeader, UdpHeader};

        // Parse IPv6 header
        let (ipv6_header, payload) = Ipv6Header::from_slice(packet).ok()?;

        // Check protocol in next_header field
        match ipv6_header.next_header.0 {
            6 => {
                // TCP
                let (tcp_header, _) = TcpHeader::from_slice(payload).ok()?;
                Some(tcp_header.destination_port)
            }
            17 => {
                // UDP
                let (udp_header, _) = UdpHeader::from_slice(payload).ok()?;
                Some(udp_header.destination_port)
            }
            _ => {
                // Other protocols (ICMP, etc.) don't have ports
                None
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;
    use std::net::Ipv6Addr;

    /// Helper to create test EndpointIds
    fn test_endpoint_id(seed: u8) -> EndpointId {
        let secret = SecretKey::from_bytes(&[seed; 32]);
        secret.public()
    }

    /// Helper to create a minimal valid IPv6 packet
    fn create_ipv6_packet(src: Ipv6Addr, dst: Ipv6Addr, payload_size: usize) -> Vec<u8> {
        let mut packet = vec![0u8; 40 + payload_size];
        packet[0] = 0x60; // Version 6
        packet[4] = (payload_size >> 8) as u8; // Payload length high byte
        packet[5] = (payload_size & 0xFF) as u8; // Payload length low byte
        packet[6] = 59; // No next header
        packet[7] = 64; // Hop limit

        // Source address
        packet[8..24].copy_from_slice(&src.octets());
        // Destination address
        packet[24..40].copy_from_slice(&dst.octets());

        // Add some payload if requested
        for i in 0..payload_size {
            packet[40 + i] = (i % 256) as u8;
        }

        packet
    }

    #[test]
    fn test_alpn_constant() {
        assert_eq!(ALPN, b"iron/packet/0");
        assert_eq!(ALPN.len(), 13);
    }

    #[test]
    fn test_max_packet_size_constant() {
        assert_eq!(MAX_PACKET_SIZE, 1500);
    }

    #[test]
    fn test_rewrite_source_address_valid_packet() {
        let registry = Arc::new(Registry::new());
        let sender_id = test_endpoint_id(42);
        let expected_sender_ipv6 = registry.get_or_assign_ip(sender_id);

        // Create packet with arbitrary source/destination
        let original_src = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
        let dst = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 2);
        let packet = create_ipv6_packet(original_src, dst, 0);

        // Rewrite source address
        let result = IronProtocol::rewrite_source_address(packet, &sender_id, &registry);
        assert!(result.is_ok(), "Should successfully rewrite source address");

        let rewritten_packet = result.unwrap();

        // Parse the rewritten packet to verify source was changed
        use etherparse::Ipv6Header;
        let (header, _) = Ipv6Header::from_slice(&rewritten_packet).unwrap();

        assert_eq!(
            Ipv6Addr::from(header.source),
            expected_sender_ipv6,
            "Source should be rewritten to sender's derived IPv6"
        );
        assert_eq!(
            Ipv6Addr::from(header.destination),
            dst,
            "Destination should remain unchanged"
        );
    }

    #[test]
    fn test_rewrite_source_address_preserves_payload() {
        let registry = Arc::new(Registry::new());
        let sender_id = test_endpoint_id(99);

        let src = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
        let dst = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 2);
        let payload_size = 100;
        let packet = create_ipv6_packet(src, dst, payload_size);

        // Original payload - clone to avoid borrow issues
        let original_payload = packet[40..].to_vec();

        let rewritten_packet =
            IronProtocol::rewrite_source_address(packet, &sender_id, &registry).unwrap();

        // Check payload is preserved
        let rewritten_payload = &rewritten_packet[40..];
        assert_eq!(
            original_payload.as_slice(),
            rewritten_payload,
            "Payload should be preserved exactly"
        );
        assert_eq!(
            rewritten_payload.len(),
            payload_size,
            "Payload size should remain unchanged"
        );
    }

    #[test]
    fn test_rewrite_source_address_with_large_payload() {
        let registry = Arc::new(Registry::new());
        let sender_id = test_endpoint_id(77);

        let src = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
        let dst = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 2);
        let payload_size = 1460; // Max for 1500 MTU
        let packet = create_ipv6_packet(src, dst, payload_size);

        assert_eq!(
            packet.len(),
            40 + payload_size,
            "Packet should be exactly MTU size"
        );

        let result = IronProtocol::rewrite_source_address(packet, &sender_id, &registry);
        assert!(result.is_ok(), "Should handle max MTU packets");

        let rewritten = result.unwrap();
        assert_eq!(
            rewritten.len(),
            40 + payload_size,
            "Rewritten packet size should match original"
        );
    }

    #[test]
    fn test_rewrite_source_address_invalid_packet() {
        let registry = Arc::new(Registry::new());
        let sender_id = test_endpoint_id(1);

        // Packet too small to be valid IPv6
        let invalid_packet = vec![0x60, 0x00, 0x00]; // Only 3 bytes

        let result = IronProtocol::rewrite_source_address(invalid_packet, &sender_id, &registry);
        assert!(
            result.is_err(),
            "Should fail on malformed packet (too small)"
        );
    }

    #[test]
    fn test_rewrite_source_address_non_ipv6_packet() {
        let registry = Arc::new(Registry::new());
        let sender_id = test_endpoint_id(1);

        // Valid size but wrong version (IPv4 = 0x40)
        let mut packet = vec![0u8; 40];
        packet[0] = 0x40; // IPv4 version

        let result = IronProtocol::rewrite_source_address(packet, &sender_id, &registry);
        assert!(result.is_err(), "Should fail on non-IPv6 packet");
    }

    #[test]
    fn test_rewrite_source_address_different_senders() {
        let registry = Arc::new(Registry::new());
        let sender1 = test_endpoint_id(10);
        let sender2 = test_endpoint_id(20);

        let expected_ipv6_1 = registry.get_or_assign_ip(sender1);
        let expected_ipv6_2 = registry.get_or_assign_ip(sender2);

        // Create same packet structure
        let src = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
        let dst = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 2);
        let packet1 = create_ipv6_packet(src, dst, 0);
        let packet2 = create_ipv6_packet(src, dst, 0);

        let rewritten1 =
            IronProtocol::rewrite_source_address(packet1, &sender1, &registry).unwrap();
        let rewritten2 =
            IronProtocol::rewrite_source_address(packet2, &sender2, &registry).unwrap();

        use etherparse::Ipv6Header;
        let (header1, _) = Ipv6Header::from_slice(&rewritten1).unwrap();
        let (header2, _) = Ipv6Header::from_slice(&rewritten2).unwrap();

        assert_eq!(Ipv6Addr::from(header1.source), expected_ipv6_1);
        assert_eq!(Ipv6Addr::from(header2.source), expected_ipv6_2);
        assert_ne!(
            expected_ipv6_1, expected_ipv6_2,
            "Different senders should get different source IPs"
        );
    }

    #[test]
    fn test_rewrite_source_address_preserves_header_fields() {
        let registry = Arc::new(Registry::new());
        let sender_id = test_endpoint_id(55);

        let src = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
        let dst = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 2);

        // Create packet with specific header values
        let mut packet = create_ipv6_packet(src, dst, 50);
        packet[6] = 6; // TCP next header
        packet[7] = 32; // Custom hop limit

        let rewritten =
            IronProtocol::rewrite_source_address(packet, &sender_id, &registry).unwrap();

        use etherparse::Ipv6Header;
        let (header, _) = Ipv6Header::from_slice(&rewritten).unwrap();

        assert_eq!(
            header.next_header.0, 6,
            "Next header should be preserved (TCP)"
        );
        assert_eq!(header.hop_limit, 32, "Hop limit should be preserved");
        assert_eq!(
            header.payload_length, 50,
            "Payload length should be preserved"
        );
    }

    #[test]
    fn test_rewrite_source_address_idempotent() {
        let registry = Arc::new(Registry::new());
        let sender_id = test_endpoint_id(88);
        let sender_ipv6 = registry.get_or_assign_ip(sender_id);

        let src = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
        let dst = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 2);
        let packet = create_ipv6_packet(src, dst, 10);

        // Rewrite once
        let rewritten1 =
            IronProtocol::rewrite_source_address(packet, &sender_id, &registry).unwrap();

        // Rewrite the already-rewritten packet again
        let rewritten2 =
            IronProtocol::rewrite_source_address(rewritten1.clone(), &sender_id, &registry)
                .unwrap();

        use etherparse::Ipv6Header;
        let (header1, _) = Ipv6Header::from_slice(&rewritten1).unwrap();
        let (header2, _) = Ipv6Header::from_slice(&rewritten2).unwrap();

        assert_eq!(
            Ipv6Addr::from(header1.source),
            Ipv6Addr::from(header2.source)
        );
        assert_eq!(Ipv6Addr::from(header1.source), sender_ipv6);
        assert_eq!(rewritten1, rewritten2, "Rewriting should be idempotent");
    }

    #[tokio::test]
    async fn test_protocol_construction() {
        let registry = Arc::new(Registry::new());
        let secret = SecretKey::generate(&mut rand::rng());
        let endpoint = Endpoint::builder()
            .secret_key(secret)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .expect("Failed to create endpoint");

        let (_to_network_tx, to_network_rx) = mpsc::unbounded_channel();
        let (from_network_tx, _from_network_rx) = mpsc::unbounded_channel();

        let firewall = FirewallConfig::new();
        let protocol =
            IronProtocol::new(registry, endpoint, to_network_rx, from_network_tx, firewall);

        // Verify construction doesn't panic
        assert_eq!(
            protocol.connection_pool.len(),
            0,
            "Connection pool should start empty"
        );
    }

    #[test]
    fn test_connection_pool_operations() {
        // Test DashMap operations that would be used in connection pooling
        let pool: DashMap<EndpointId, String> = DashMap::new();
        let endpoint1 = test_endpoint_id(1);
        let endpoint2 = test_endpoint_id(2);

        // Insert
        pool.insert(endpoint1, "conn1".to_string());
        assert_eq!(pool.len(), 1);

        // Get
        assert!(pool.get(&endpoint1).is_some());
        assert!(pool.get(&endpoint2).is_none());

        // Remove
        pool.remove(&endpoint1);
        assert_eq!(pool.len(), 0);
        assert!(pool.get(&endpoint1).is_none());
    }

    #[test]
    fn test_connection_pool_concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let pool: Arc<DashMap<EndpointId, usize>> = Arc::new(DashMap::new());
        let mut handles = vec![];

        // Spawn 10 threads inserting different endpoints
        for i in 0..10 {
            let pool = Arc::clone(&pool);
            let handle = thread::spawn(move || {
                let endpoint = test_endpoint_id(i as u8);
                pool.insert(endpoint, i);
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(pool.len(), 10, "All insertions should succeed concurrently");
    }

    #[test]
    fn test_packet_size_boundaries() {
        // Test packets at various size boundaries
        let test_cases = [
            (0, "Empty payload"),
            (1, "Single byte"),
            (100, "Small packet"),
            (1460, "Max TCP payload (1500 MTU - 40 IPv6)"),
            (1500, "At MAX_PACKET_SIZE boundary"),
        ];

        let registry = Arc::new(Registry::new());
        let sender_id = test_endpoint_id(33);
        let src = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
        let dst = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 2);

        for (payload_size, description) in test_cases.iter() {
            let packet = create_ipv6_packet(src, dst, *payload_size);
            let result = IronProtocol::rewrite_source_address(packet, &sender_id, &registry);

            assert!(
                result.is_ok(),
                "Should handle packet with {}: {}",
                payload_size,
                description
            );

            let rewritten = result.unwrap();
            assert_eq!(
                rewritten.len(),
                40 + payload_size,
                "Packet size should be preserved for {}",
                description
            );
        }
    }

    #[test]
    fn test_registry_integration_in_rewrite() {
        // Test that source rewriting properly integrates with Registry's deterministic mapping
        let registry = Arc::new(Registry::new());
        let sender = test_endpoint_id(123);

        // Pre-register the sender in registry
        let pre_registered_ipv6 = registry.get_or_assign_ip(sender);

        let src = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
        let dst = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 2);
        let packet = create_ipv6_packet(src, dst, 0);

        let rewritten = IronProtocol::rewrite_source_address(packet, &sender, &registry).unwrap();

        use etherparse::Ipv6Header;
        let (header, _) = Ipv6Header::from_slice(&rewritten).unwrap();

        assert_eq!(
            Ipv6Addr::from(header.source),
            pre_registered_ipv6,
            "Should use pre-registered IPv6 from registry"
        );

        // Verify it's still in the registry and consistent
        let post_rewrite_ipv6 = registry.get_or_assign_ip(sender);
        assert_eq!(
            pre_registered_ipv6, post_rewrite_ipv6,
            "Registry mapping should remain consistent"
        );
    }

    /// Helper to create an IPv6 packet with TCP header
    fn create_tcp_packet(src: Ipv6Addr, dst: Ipv6Addr, src_port: u16, dst_port: u16) -> Vec<u8> {
        use etherparse::{Ipv6Header, TcpHeader};

        let mut tcp_header = TcpHeader::new(src_port, dst_port, 0, 0);
        tcp_header.syn = true;

        // Build packet
        let mut packet = Vec::new();

        // Write IPv6 header
        let ipv6_header = Ipv6Header {
            traffic_class: 0,
            flow_label: 0.try_into().unwrap(),
            payload_length: tcp_header.header_len() as u16,
            next_header: 6.into(), // TCP
            hop_limit: 64,
            source: src.octets(),
            destination: dst.octets(),
        };
        ipv6_header.write(&mut packet).unwrap();

        // Write TCP header
        tcp_header.write(&mut packet).unwrap();

        packet
    }

    /// Helper to create an IPv6 packet with UDP header
    fn create_udp_packet(src: Ipv6Addr, dst: Ipv6Addr, src_port: u16, dst_port: u16) -> Vec<u8> {
        use etherparse::{Ipv6Header, UdpHeader};

        let udp_header = UdpHeader {
            source_port: src_port,
            destination_port: dst_port,
            length: 8, // UDP header only, no payload
            checksum: 0,
        };

        // Build packet
        let mut packet = Vec::new();

        // Write IPv6 header
        let ipv6_header = Ipv6Header {
            traffic_class: 0,
            flow_label: 0.try_into().unwrap(),
            payload_length: 8,
            next_header: 17.into(), // UDP
            hop_limit: 64,
            source: src.octets(),
            destination: dst.octets(),
        };
        ipv6_header.write(&mut packet).unwrap();

        // Write UDP header
        udp_header.write(&mut packet).unwrap();

        packet
    }

    #[test]
    fn test_extract_dst_port_tcp() {
        let src = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
        let dst = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 2);

        let packet = create_tcp_packet(src, dst, 12345, 80);
        let port = IronProtocol::extract_dst_port(&packet);

        assert_eq!(port, Some(80), "Should extract TCP destination port");
    }

    #[test]
    fn test_extract_dst_port_udp() {
        let src = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
        let dst = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 2);

        let packet = create_udp_packet(src, dst, 54321, 53);
        let port = IronProtocol::extract_dst_port(&packet);

        assert_eq!(port, Some(53), "Should extract UDP destination port");
    }

    #[test]
    fn test_extract_dst_port_various_ports() {
        let src = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
        let dst = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 2);

        // Test various common ports
        let test_ports = [80, 443, 22, 3000, 8080, 65535];

        for &port in &test_ports {
            let tcp_packet = create_tcp_packet(src, dst, 50000, port);
            assert_eq!(
                IronProtocol::extract_dst_port(&tcp_packet),
                Some(port),
                "Should extract TCP port {}",
                port
            );

            let udp_packet = create_udp_packet(src, dst, 50000, port);
            assert_eq!(
                IronProtocol::extract_dst_port(&udp_packet),
                Some(port),
                "Should extract UDP port {}",
                port
            );
        }
    }

    #[test]
    fn test_extract_dst_port_icmp() {
        // Create ICMP packet (no port)
        let src = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
        let dst = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 2);

        let packet = create_ipv6_packet(src, dst, 8); // next_header = 59 (no next header)
        let port = IronProtocol::extract_dst_port(&packet);

        assert_eq!(port, None, "ICMP packets should return None");
    }

    #[test]
    fn test_extract_dst_port_invalid_packet() {
        let invalid_packet = vec![0x60, 0x00, 0x00]; // Too small
        let port = IronProtocol::extract_dst_port(&invalid_packet);

        assert_eq!(port, None, "Invalid packets should return None");
    }
}
