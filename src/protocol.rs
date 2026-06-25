use crate::mapping::Registry;
use crate::packet::Packet;
use anyhow::{Context, Result};
use bytes::Bytes;
use dashmap::DashMap;
use iroh::{Endpoint, EndpointId};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};

/// ALPN protocol identifier for iron packet transport
pub const ALPN: &[u8] = b"iron/packet/1";

/// Maximum packet size (TUN MTU match)
#[allow(dead_code)]
const MAX_PACKET_SIZE: usize = 1280;

/// Iroh protocol handler for packet transport
///
/// Bridges TUN interface and iroh QUIC connections using **datagrams**
/// (unreliable, unordered). This avoids the TCP-over-TCP double-retransmit
/// problem that stream-based transport would cause, and is consistent with
/// how IP itself is a best-effort protocol.
pub struct IronProtocol {
    registry: Arc<Registry>,
    endpoint: Endpoint,
    to_network_rx: mpsc::Receiver<(EndpointId, Packet)>,
    from_network_tx: mpsc::Sender<Packet>,
    connections: Arc<DashMap<EndpointId, iroh::endpoint::Connection>>,
}

impl IronProtocol {
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
            connections: Arc::new(DashMap::new()),
        }
    }

    pub async fn run(mut self) -> Result<()> {
        let endpoint = self.endpoint.clone();
        let from_network_tx = self.from_network_tx.clone();
        let registry = self.registry.clone();
        let connections = self.connections.clone();

        let accept_handle = tokio::spawn(async move {
            if let Err(e) = Self::accept_loop(endpoint, connections, registry, from_network_tx).await
            {
                error!("Accept loop failed: {}", e);
            }
        });

        let send_result = self.send_loop().await;

        accept_handle.abort();
        let _ = accept_handle.await;

        send_result
    }

    /// Send loop: reads packets from TUN and sends them to peers via QUIC datagrams
    async fn send_loop(&mut self) -> Result<()> {
        info!("Starting send loop");
        let self_id = self.endpoint.id();

        while let Some((dest_endpoint_id, packet)) = self.to_network_rx.recv().await {
            if dest_endpoint_id == self_id {
                debug!("Loopback detected: dropping self-bound packet");
                continue;
            }

            if let Err(e) = self.send_packet(&dest_endpoint_id, &packet).await {
                warn!("Failed to send packet to {}: {}", dest_endpoint_id, e);
            }
        }

        info!("Send loop terminated (channel closed)");
        Ok(())
    }

    /// Sends a single packet as a QUIC datagram to a peer.
    ///
    /// Caches the connection for reuse; on send failure removes the stale
    /// cache entry and retries with a fresh connection.
    async fn send_packet(&self, dest: &EndpointId, packet: &Packet) -> Result<()> {
        let raw = packet.as_bytes().context("Packet has no raw bytes")?.to_vec();

        let conn = self.get_or_create_connection(dest).await?;

        // send_datagram is non-async: it pushes into the QUIC send buffer.
        // If the buffer is full it returns an error and we fall through to retry.
        if let Err(e) = conn.send_datagram(Bytes::from(raw.clone())) {
            warn!("send_datagram to {} failed: {}, reconnecting", dest, e);
            self.connections.remove(dest);

            let new_conn = self
                .endpoint
                .connect(*dest, ALPN)
                .await
                .context("Failed to connect to peer on retry")?;

            self.connections.insert(*dest, new_conn.clone());

            new_conn
                .send_datagram(Bytes::from(raw))
                .context("Failed to send datagram on retry")?;
        }

        Ok(())
    }

    /// Get or create a connection to a peer, caching it for reuse.
    async fn get_or_create_connection(&self, dest: &EndpointId) -> Result<iroh::endpoint::Connection> {
        if let Some(entry) = self.connections.get(dest) {
            trace!("Reusing cached connection to {}", dest);
            return Ok(entry.clone());
        }

        info!("Connecting to peer {}", dest);
        let conn = self
            .endpoint
            .connect(*dest, ALPN)
            .await
            .context("Failed to connect to peer")?;

        self.connections.insert(*dest, conn.clone());
        info!("Successfully connected to {} and cached connection", dest);
        Ok(conn)
    }

    /// Accept loop: accepts incoming connections and spawns per-peer datagram readers
    async fn accept_loop(
        endpoint: Endpoint,
        connections: Arc<DashMap<EndpointId, iroh::endpoint::Connection>>,
        registry: Arc<Registry>,
        from_network_tx: mpsc::Sender<Packet>,
    ) -> Result<()> {
        info!("Starting accept loop on {}", endpoint.id());

        loop {
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

            let peer_id = conn.remote_id();
            let base32_id = data_encoding::BASE32_NOPAD
                .encode(peer_id.as_bytes())
                .to_lowercase();
            debug!("Accepted connection from {}", base32_id);

            // Cache the accepted connection so send_packet can reuse it
            connections.insert(peer_id, conn.clone());

            // Spawn a reader that forwards datagrams from this peer to the TUN
            let conn_clone = connections.clone();
            let registry = registry.clone();
            let from_network_tx = from_network_tx.clone();
            tokio::spawn(async move {
                let result = Self::handle_datagram_reader(conn, peer_id, registry, from_network_tx).await;
                conn_clone.remove(&peer_id);
                if let Err(e) = result {
                    warn!("Datagram reader failed for {}: {}", peer_id, e);
                }
            });
        }

        Ok(())
    }

    /// Reads QUIC datagrams from a single peer connection and forwards them to the TUN.
    async fn handle_datagram_reader(
        conn: iroh::endpoint::Connection,
        sender_id: EndpointId,
        registry: Arc<Registry>,
        from_network_tx: mpsc::Sender<Packet>,
    ) -> Result<()> {
        loop {
            let datagram = match conn.read_datagram().await {
                Ok(d) => d,
                Err(e) => {
                    debug!("Connection from {} closed: {}", sender_id, e);
                    break;
                }
            };

            trace!("Received datagram from {} ({} bytes)", sender_id, datagram.len());

            let packet = match Self::rewrite_source_address(datagram.to_vec(), &sender_id, &registry)
            {
                Ok(p) => p,
                Err(e) => {
                    warn!("Failed to rewrite source for {}: {}", sender_id, e);
                    continue;
                }
            };

            if let Err(e) = from_network_tx.send(Packet::raw(packet)).await {
                error!("Failed to forward packet to TUN: {}", e);
                break;
            }
        }

        Ok(())
    }

    /// Rewrites the packet's source IPv6 to match the sender's derived IPv6
    fn rewrite_source_address(
        packet: Vec<u8>,
        sender_id: &EndpointId,
        registry: &Arc<Registry>,
    ) -> Result<Vec<u8>> {
        use etherparse::Ipv6Header;

        let (ipv6_header, payload) =
            Ipv6Header::from_slice(&packet).context("Failed to parse IPv6 header")?;

        let sender_ipv6 = registry.get_or_assign_ip(*sender_id);

        let mut header = ipv6_header;
        header.source = sender_ipv6.octets();

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
    use iroh::RelayMode;
    use iroh::SecretKey;
    use iroh::endpoint::presets::N0;
    use std::net::Ipv6Addr;

    use crate::test_utils::test_endpoint_id;

    fn create_ipv6_packet(src: Ipv6Addr, dst: Ipv6Addr, payload_size: usize) -> Vec<u8> {
        let mut packet = vec![0u8; 40 + payload_size];
        packet[0] = 0x60;
        packet[4] = (payload_size >> 8) as u8;
        packet[5] = (payload_size & 0xFF) as u8;
        packet[6] = 59;
        packet[7] = 64;
        packet[8..24].copy_from_slice(&src.octets());
        packet[24..40].copy_from_slice(&dst.octets());
        for i in 0..payload_size {
            packet[40 + i] = (i % 256) as u8;
        }
        packet
    }

    #[test]
    fn test_alpn_constant() {
        assert_eq!(ALPN, b"iron/packet/1");
        assert_eq!(ALPN.len(), 13);
    }

    #[test]
    fn test_max_packet_size_constant() {
        assert_eq!(MAX_PACKET_SIZE, 1280);
    }

    #[test]
    fn test_rewrite_source_address_valid_packet() {
        let registry = Arc::new(Registry::new());
        let sender_id = test_endpoint_id(42);
        let expected_sender_ipv6 = registry.get_or_assign_ip(sender_id);
        let original_src = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
        let dst = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 2);
        let packet = create_ipv6_packet(original_src, dst, 0);
        let result = IronProtocol::rewrite_source_address(packet, &sender_id, &registry);
        assert!(result.is_ok());
        let rewritten_packet = result.unwrap();
        use etherparse::Ipv6Header;
        let (header, _) = Ipv6Header::from_slice(&rewritten_packet).unwrap();
        assert_eq!(Ipv6Addr::from(header.source), expected_sender_ipv6);
        assert_eq!(Ipv6Addr::from(header.destination), dst);
    }

    #[test]
    fn test_rewrite_source_address_preserves_payload() {
        let registry = Arc::new(Registry::new());
        let sender_id = test_endpoint_id(99);
        let src = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
        let dst = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 2);
        let payload_size = 100;
        let packet = create_ipv6_packet(src, dst, payload_size);
        let original_payload = packet[40..].to_vec();
        let rewritten_packet =
            IronProtocol::rewrite_source_address(packet, &sender_id, &registry).unwrap();
        assert_eq!(&rewritten_packet[40..], original_payload.as_slice());
        assert_eq!(rewritten_packet.len(), 40 + payload_size);
    }

    #[test]
    fn test_rewrite_source_address_invalid_packet() {
        let registry = Arc::new(Registry::new());
        let sender_id = test_endpoint_id(1);
        let invalid_packet = vec![0x60, 0x00, 0x00];
        let result = IronProtocol::rewrite_source_address(invalid_packet, &sender_id, &registry);
        assert!(result.is_err());
    }

    #[test]
    fn test_rewrite_source_address_non_ipv6_packet() {
        let registry = Arc::new(Registry::new());
        let sender_id = test_endpoint_id(1);
        let mut packet = vec![0u8; 40];
        packet[0] = 0x40;
        let result = IronProtocol::rewrite_source_address(packet, &sender_id, &registry);
        assert!(result.is_err());
    }

    #[test]
    fn test_rewrite_source_address_different_senders() {
        let registry = Arc::new(Registry::new());
        let sender1 = test_endpoint_id(10);
        let sender2 = test_endpoint_id(20);
        let expected_ipv6_1 = registry.get_or_assign_ip(sender1);
        let expected_ipv6_2 = registry.get_or_assign_ip(sender2);
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
        assert_ne!(expected_ipv6_1, expected_ipv6_2);
    }

    #[test]
    fn test_rewrite_source_address_preserves_header_fields() {
        let registry = Arc::new(Registry::new());
        let sender_id = test_endpoint_id(55);
        let src = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
        let dst = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 2);
        let mut packet = create_ipv6_packet(src, dst, 50);
        packet[6] = 6;
        packet[7] = 32;
        let rewritten =
            IronProtocol::rewrite_source_address(packet, &sender_id, &registry).unwrap();
        use etherparse::Ipv6Header;
        let (header, _) = Ipv6Header::from_slice(&rewritten).unwrap();
        assert_eq!(header.next_header.0, 6);
        assert_eq!(header.hop_limit, 32);
        assert_eq!(header.payload_length, 50);
    }

    #[test]
    fn test_rewrite_source_address_idempotent() {
        let registry = Arc::new(Registry::new());
        let sender_id = test_endpoint_id(88);
        let sender_ipv6 = registry.get_or_assign_ip(sender_id);
        let src = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
        let dst = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 2);
        let packet = create_ipv6_packet(src, dst, 10);
        let rewritten1 =
            IronProtocol::rewrite_source_address(packet, &sender_id, &registry).unwrap();
        let rewritten2 =
            IronProtocol::rewrite_source_address(rewritten1.clone(), &sender_id, &registry)
                .unwrap();
        use etherparse::Ipv6Header;
        let (header1, _) = Ipv6Header::from_slice(&rewritten1).unwrap();
        let (header2, _) = Ipv6Header::from_slice(&rewritten2).unwrap();
        assert_eq!(Ipv6Addr::from(header1.source), Ipv6Addr::from(header2.source));
        assert_eq!(Ipv6Addr::from(header1.source), sender_ipv6);
        assert_eq!(rewritten1, rewritten2);
    }

    #[tokio::test]
    async fn test_protocol_construction() {
        let registry = Arc::new(Registry::new());
        let secret = SecretKey::generate();
        let endpoint = Endpoint::builder(N0)
            .secret_key(secret)
            .relay_mode(RelayMode::Disabled)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .expect("Failed to create endpoint");
        let (_to_network_tx, to_network_rx) = mpsc::channel(1024);
        let (from_network_tx, _from_network_rx) = mpsc::channel(1024);
        let protocol = IronProtocol::new(registry, endpoint, to_network_rx, from_network_tx);
        assert_eq!(protocol.connections.len(), 0);
    }

    #[tokio::test]
    async fn test_connection_cache_empty_on_creation() {
        let endp = Endpoint::builder(N0)
            .secret_key(SecretKey::generate())
            .relay_mode(RelayMode::Disabled)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .unwrap();
        let registry = Arc::new(Registry::new());
        let (_tx, rx) = mpsc::channel(1024);
        let (tx2, _rx2) = mpsc::channel(1024);
        let proto = IronProtocol::new(registry, endp, rx, tx2);
        assert_eq!(proto.connections.len(), 0);
    }

    #[test]
    fn test_dashmap_operations() {
        let map: DashMap<EndpointId, String> = DashMap::new();
        let e1 = test_endpoint_id(1);
        let e2 = test_endpoint_id(2);
        map.insert(e1, "conn1".to_string());
        assert_eq!(map.len(), 1);
        assert!(map.get(&e1).is_some());
        assert!(map.get(&e2).is_none());
        map.remove(&e1);
        assert_eq!(map.len(), 0);
    }

    #[test]
    fn test_dashmap_concurrent_access() {
        use std::thread;
        let map: Arc<DashMap<EndpointId, usize>> = Arc::new(DashMap::new());
        let mut handles = vec![];
        for i in 0..10 {
            let map = Arc::clone(&map);
            handles.push(thread::spawn(move || {
                map.insert(test_endpoint_id(i as u8), i);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(map.len(), 10);
    }

    #[test]
    fn test_packet_size_boundaries() {
        let test_cases = [
            (0, "Empty payload"),
            (1, "Single byte"),
            (100, "Small packet"),
            (1240, "Near MTU (1280 - 40 IPv6 header)"),
            (MAX_PACKET_SIZE, "At MAX_PACKET_SIZE boundary"),
        ];
        let registry = Arc::new(Registry::new());
        let sender_id = test_endpoint_id(33);
        let src = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
        let dst = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 2);
        for (payload_size, description) in test_cases.iter() {
            let packet = create_ipv6_packet(src, dst, *payload_size);
            let result = IronProtocol::rewrite_source_address(packet, &sender_id, &registry);
            assert!(result.is_ok(), "Should handle packet with {}: {}", payload_size, description);
            let rewritten = result.unwrap();
            assert_eq!(rewritten.len(), 40 + payload_size, "Size preserved for {}", description);
        }
    }
}
