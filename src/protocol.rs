use crate::mapping::Registry;
use crate::packet::Packet;
use anyhow::{Context, Result};
use dashmap::DashMap;
use iroh::{Endpoint, EndpointId};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::{debug, error, info, trace, warn};

/// ALPN protocol identifier for iron packet transport
pub const ALPN: &[u8] = b"iron/packet/0";

/// Maximum packet size (MTU)
const MAX_PACKET_SIZE: usize = 1500;

/// TTL for cached connections: evict after 60s of inactivity
const CONNECTION_TTL: Duration = Duration::from_secs(60);

/// A connection with TTL tracking for proactive stale-connection eviction
struct CachedConnection {
    conn: iroh::endpoint::Connection,
    last_used: Instant,
}

/// Iroh protocol handler for packet transport
///
/// This component bridges TUN interface and iroh QUIC connections:
/// - **Send**: Receives (EndpointId, Packet) from TUN, sends via QUIC with connection pooling
/// - **Receive**: Accepts QUIC connections, forwards Packets to TUN
pub struct IronProtocol {
    registry: Arc<Registry>,
    endpoint: Endpoint,
    /// Receives packets from TUN to send to peers
    to_network_rx: mpsc::Receiver<(EndpointId, Packet)>,
    /// Sends received packets to TUN
     from_network_tx: mpsc::Sender<Packet>,
     /// Connection pool: maps EndpointId -> CachedConnection for reuse with TTL eviction
     connection_pool: Arc<DashMap<EndpointId, CachedConnection>>,
 }
 
impl IronProtocol {
    /// Creates a new protocol handler
    pub fn new(
        registry: Arc<Registry>,
        endpoint: Endpoint,
        to_network_rx: mpsc::Receiver<(EndpointId, Packet)>,
        from_network_tx: mpsc::Sender<Packet>,
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

    /// Get or create a connection to a peer, with TTL-based eviction.
    ///
    /// If a cached connection exists and hasn't exceeded `CONNECTION_TTL` since
    /// last use, it is returned. Otherwise the stale entry is evicted and a new
    /// connection is established and cached.
    async fn get_or_create_connection(
        &self,
        dest: &EndpointId,
    ) -> Result<iroh::endpoint::Connection> {
        if let Some(entry) = self.connection_pool.get(dest) {
            if entry.last_used.elapsed() < CONNECTION_TTL {
                trace!("Reusing cached connection to {}", dest);
                return Ok(entry.conn.clone());
            }
            drop(entry);
            trace!("Cached connection to {} expired, reconnecting", dest);
            self.connection_pool.remove(dest);
        }

        info!("Connecting to peer {} (no cached connection)", dest);
        let new_conn = self
            .endpoint
            .connect(*dest, ALPN)
            .await
            .context("Failed to connect to peer")?;

        self.connection_pool.insert(
            *dest,
            CachedConnection {
                conn: new_conn.clone(),
                last_used: Instant::now(),
            },
        );

        info!("Successfully connected to {} and cached connection", dest);
        Ok(new_conn)
    }

    /// Refresh the `last_used` timestamp for a cached connection (refresh on use).
    fn refresh_connection(&self, dest: &EndpointId) {
        if let Some(mut entry) = self.connection_pool.get_mut(dest) {
            entry.last_used = Instant::now();
        }
    }

    /// Sends a single packet to a peer
    ///
    /// Uses connection pooling with TTL-based eviction to reuse existing
    /// connections when possible, avoiding repeated handshakes.
    async fn send_packet(&self, dest: &EndpointId, packet: &Packet) -> Result<()> {
        let conn = self.get_or_create_connection(dest).await?;

        trace!("Opening bi-directional stream to {}", dest);
        let stream_result = conn.open_bi().await;

        let (mut send, _recv) = match stream_result {
            Ok(s) => {
                self.refresh_connection(dest);
                s
            }
            Err(e) => {
                warn!(
                    "Failed to open stream on cached connection to {}: {}",
                    dest, e
                );
                self.connection_pool.remove(dest);

                trace!("Retrying with new connection to {}", dest);
                let new_conn = self
                    .endpoint
                    .connect(*dest, ALPN)
                    .await
                    .context("Failed to connect to peer on retry")?;

                self.connection_pool.insert(
                    *dest,
                    CachedConnection {
                        conn: new_conn.clone(),
                        last_used: Instant::now(),
                    },
                );

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

    /// Accept loop: accepts incoming connections and receives packets
    async fn accept_loop(
        endpoint: Endpoint,
        registry: Arc<Registry>,
        from_network_tx: mpsc::Sender<Packet>,
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
            let base32_id = data_encoding::BASE32_NOPAD
                .encode(sender_id.as_bytes())
                .to_lowercase();

            debug!("Accepted connection from {}", base32_id);

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
        from_network_tx: mpsc::Sender<Packet>,
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
            // Forward packet to TUN (wrap in Packet type)
            if let Err(e) = from_network_tx
                .send(Packet::raw(packet_with_correct_source))
                .await
            {
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

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;
    use iroh::endpoint::presets::N0;
    use std::net::Ipv6Addr;

    use crate::test_utils::test_endpoint_id;

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
        let secret = SecretKey::generate();
        let endpoint = Endpoint::builder(N0)
            .secret_key(secret)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .expect("Failed to create endpoint");

        let (_to_network_tx, to_network_rx) = mpsc::channel(1024);
        let (from_network_tx, _from_network_rx) = mpsc::channel(1024);

        let protocol = IronProtocol::new(registry, endpoint, to_network_rx, from_network_tx);

        // Verify construction doesn't panic
        assert_eq!(
            protocol.connection_pool.len(),
            0,
            "Connection pool should start empty"
        );
    }

    #[tokio::test]
    async fn test_connection_ttl_expires_cached_connections() {
        let secret1 = SecretKey::generate();
        let secret2 = SecretKey::generate();
        let id2 = secret2.public();

        let ep1 = Endpoint::builder(N0)
            .secret_key(secret1)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .unwrap();

        let ep2 = Endpoint::builder(N0)
            .secret_key(secret2)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .unwrap();

        // Accept incoming connections on ep2 so ep1's connect succeeds
        let accept_task = tokio::spawn(async move {
            loop {
                if let Some(incoming) = ep2.accept().await
                    && let Ok(_conn) = incoming.await
                {}
            }
        });

        let registry = Arc::new(Registry::new());
        let (_to_network_tx, to_network_rx) = mpsc::unbounded_channel();
        let (from_network_tx, _from_network_rx) = mpsc::unbounded_channel();

        let protocol = IronProtocol::new(registry, ep1, to_network_rx, from_network_tx);

        assert_eq!(protocol.connection_pool.len(), 0);

        // First call establishes and caches a connection
        let _conn1 = protocol.get_or_create_connection(&id2).await.unwrap();
        assert_eq!(protocol.connection_pool.len(), 1);

        // Manually age the entry past TTL
        if let Some(mut entry) = protocol.connection_pool.get_mut(&id2) {
            entry.last_used = Instant::now() - CONNECTION_TTL - Duration::from_secs(1);
        }

        // get_or_create_connection should evict the expired entry and create a new connection
        let _conn2 = protocol.get_or_create_connection(&id2).await.unwrap();
        assert_eq!(protocol.connection_pool.len(), 1);

        // Verify the last_used timestamp was refreshed
        if let Some(entry) = protocol.connection_pool.get(&id2) {
            assert!(
                entry.last_used.elapsed() < Duration::from_secs(1),
                "Cached timestamp should be recent after reconnection"
            );
        }

        accept_task.abort();
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
}
