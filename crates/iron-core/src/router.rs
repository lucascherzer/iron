use crate::mapping::Registry;
use crate::packet::Packet;
use crate::platform::TunIo;
use anyhow::{Context, Result};
use etherparse::{Ipv6Header, TcpHeader};
use futures::{SinkExt, StreamExt};
use iroh::EndpointId;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};

/// Routes raw IP packets between the OS network stack and the iroh network.
///
/// The router is platform-independent: it operates on a [`TunIo`] (raw packet
/// stream/sink) produced by a [`crate::platform::TunBackend`], and never
/// touches the TUN device itself.
///
/// **Packet Flow:**
///
/// **OS → Network (Outbound to peers):**
/// 1. OS application sends to `fd69:726f::xxxx` (peer's IPv6)
/// 2. OS writes packet to TUN device
/// 3. We read from TUN, parse destination IPv6
/// 4. Lookup EndpointId from IPv6 in registry
/// 5. Send (EndpointId, Packet) to iroh via `to_network_tx`
///
/// **Network → OS (Inbound from peers):**
/// 1. Iroh receives packet from peer (knows sender EndpointId)
/// 2. Protocol layer rewrites source IPv6 to sender's derived IPv6
/// 3. Send Packet via `from_network_rx`
/// 4. We write packet to TUN device
/// 5. OS routes to listening application
pub struct PacketRouter {
    registry: Arc<Registry>,
    /// Channel for sending packets TO network (OS → iroh)
    /// Format: (destination_endpoint_id, Packet)
    to_network_tx: mpsc::UnboundedSender<(EndpointId, Packet)>,
    /// Channel for receiving packets FROM network (iroh → OS)
    /// Format: Packet (with correct source IPv6 already set)
    from_network_rx: mpsc::UnboundedReceiver<Packet>,
}

impl PacketRouter {
    /// Creates a new packet router
    ///
    /// # Arguments
    ///
    /// * `registry` - Shared registry for IPv6 <-> EndpointId mapping
    /// * `to_network_tx` - Channel sender for Packets going to iroh peers
    /// * `from_network_rx` - Channel receiver for Packets coming from iroh peers
    pub fn new(
        registry: Arc<Registry>,
        to_network_tx: mpsc::UnboundedSender<(EndpointId, Packet)>,
        from_network_rx: mpsc::UnboundedReceiver<Packet>,
    ) -> Self {
        Self {
            registry,
            to_network_tx,
            from_network_rx,
        }
    }

    /// Runs the packet processing loop on the given TUN packet I/O.
    ///
    /// This is the main event loop that handles bidirectional packet flow:
    /// - **OS → Network**: Read from TUN, lookup EndpointId, send to iroh
    /// - **Network → OS**: Receive from iroh, write to TUN
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Packet parsing fails repeatedly
    /// - Device I/O errors occur
    pub async fn run(mut self, io: TunIo) -> Result<()> {
        let TunIo {
            mut incoming,
            mut outgoing,
        } = io;

        info!("Packet router running, ready to process packets");

        loop {
            tokio::select! {
                // OS → Network: Read packet from TUN, send to iroh
                Some(packet) = incoming.next() => {
                    let packet = packet.context("Failed to read packet from TUN")?;
                    trace!("Received packet from OS ({} bytes)", packet.len());
                    if let Err(e) = self.handle_os_to_network(&packet).await {
                        warn!("Failed to handle OS→Network packet: {}", e);
                    }
                }

                // Network → OS: Receive packet from iroh, write to TUN
                Some(packet) = self.from_network_rx.recv() => {
                    debug!("Received packet from network, writing to TUN ({} bytes)", packet.len());

                    // Inspect packet for debugging
                    if let Some(packet_bytes) = packet.as_bytes()
                        && let Err(e) = Self::inspect_packet(packet_bytes, "Network→OS") {
                            trace!("Failed to inspect packet: {}", e);
                        }

                    // Extract raw bytes from Packet
                    let packet_bytes = packet.into_bytes();
                    if let Err(e) = outgoing.send(packet_bytes).await {
                        error!("Failed to write packet to TUN: {}", e);
                    } else {
                        debug!("Successfully wrote packet to TUN");
                    }
                }
            }
        }
    }

    /// Handles a packet from OS going to network (OS → iroh)
    ///
    /// # Packet Processing
    ///
    /// 1. Parse IPv6 header to extract destination address
    /// 2. Lookup EndpointId for destination IPv6 in registry
    /// 3. Send (EndpointId, packet) to iroh via channel
    ///
    /// # Arguments
    ///
    /// * `packet` - Raw IPv6 packet from TUN device (from OS application)
    ///
    /// # Visibility
    ///
    /// This method is public to allow integration testing without requiring
    /// actual TUN device creation (which needs root privileges).
    pub async fn handle_os_to_network(&self, packet: &[u8]) -> Result<()> {
        // Filter out non-IPv6 packets
        if packet.is_empty() {
            return Ok(());
        }

        // Check IP version (first 4 bits should be 6 for IPv6)
        let version = (packet[0] >> 4) & 0x0F;
        if version != 6 {
            trace!("Ignoring non-IPv6 packet (version {})", version);
            return Ok(());
        }

        // Parse IPv6 header
        let ipv6_header = Ipv6Header::from_slice(packet).context("Failed to parse IPv6 header")?;

        let dest_addr = ipv6_header.0.destination_addr();

        // Filter out multicast packets (ff00::/8)
        // These are broadcast packets (MLD, mDNS, etc.) that don't have specific destinations
        if dest_addr.octets()[0] == 0xff {
            trace!(
                "Ignoring multicast packet to {} (not a peer address)",
                dest_addr
            );
            return Ok(());
        }

        debug!(
            "TUN received OS→Network: {} -> {}, {} bytes",
            ipv6_header.0.source_addr(),
            dest_addr,
            packet.len()
        );

        // Inspect packet for debugging
        if let Err(e) = Self::inspect_packet(packet, "OS→Network") {
            trace!("Failed to inspect packet: {}", e);
        }

        // Lookup EndpointId for destination
        if let Some(endpoint_id) = self.registry.get_endpoint_id(&dest_addr) {
            debug!(
                "Resolved {} -> EndpointId {}",
                dest_addr,
                hex::encode(endpoint_id.as_bytes())
            );

            // Send to network layer (iroh will handle actual transmission)
            self.to_network_tx
                .send((endpoint_id, Packet::raw(packet.to_vec())))
                .context("Failed to send packet to network layer")?;
        } else {
            warn!(
                "No EndpointId found for destination {}, dropping packet",
                dest_addr
            );
            warn!(
                "This can happen if the destination IPv6 was cached by the application \
                 but iron doesn't have the corresponding peer information. \
                 Try accessing the peer by its .iron domain name to establish the mapping."
            );
        }

        Ok(())
    }

    /// Inspect a packet for debugging purposes
    ///
    /// Parses IPv6 and TCP headers to log detailed packet information
    fn inspect_packet(packet: &[u8], direction: &str) -> Result<()> {
        if packet.is_empty() {
            return Ok(());
        }

        // Parse IPv6 header
        let (ipv6_header, ipv6_payload) = Ipv6Header::from_slice(packet)?;

        // Check if it's TCP (next_header == 6)
        if ipv6_header.next_header == etherparse::IpNumber::TCP {
            // Parse TCP header
            if let Ok((tcp_header, _)) = TcpHeader::from_slice(ipv6_payload) {
                trace!(
                    "{}: TCP {}:{} -> {}:{} [seq={}, ack={}, flags={}{}{}{}{}] checksum=0x{:04x}",
                    direction,
                    ipv6_header.source_addr(),
                    tcp_header.source_port,
                    ipv6_header.destination_addr(),
                    tcp_header.destination_port,
                    tcp_header.sequence_number,
                    tcp_header.acknowledgment_number,
                    if tcp_header.syn { "SYN " } else { "" },
                    if tcp_header.ack { "ACK " } else { "" },
                    if tcp_header.fin { "FIN " } else { "" },
                    if tcp_header.rst { "RST " } else { "" },
                    if tcp_header.psh { "PSH " } else { "" },
                    tcp_header.checksum
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_endpoint_id;

    #[test]
    fn test_packet_router_new() {
        let registry = Arc::new(Registry::new());
        let (to_network_tx, _to_network_rx) = mpsc::unbounded_channel();
        let (_from_network_tx, from_network_rx) = mpsc::unbounded_channel();
        let _router = PacketRouter::new(registry, to_network_tx, from_network_rx);
        // Just verify it constructs
    }

    #[tokio::test]
    async fn test_handle_os_to_network_valid_destination() {
        let registry = Arc::new(Registry::new());
        let endpoint_id = test_endpoint_id(42);

        // Get the IPv6 for this endpoint
        let dest_ip = registry.get_or_assign_ip(endpoint_id);

        let (to_network_tx, mut to_network_rx) = mpsc::unbounded_channel();
        let (_from_network_tx, from_network_rx) = mpsc::unbounded_channel();
        let router = PacketRouter::new(Arc::clone(&registry), to_network_tx, from_network_rx);

        // Create a minimal IPv6 packet
        // IPv6 header: 40 bytes
        let mut packet = vec![0u8; 40];

        // Version (4 bits) = 6, Traffic Class (8 bits) = 0, Flow Label (20 bits) = 0
        packet[0] = 0x60; // Version 6

        // Payload length = 0 (no payload)
        packet[4] = 0x00;
        packet[5] = 0x00;

        // Next header = 59 (no next header)
        packet[6] = 59;

        // Hop limit = 64
        packet[7] = 64;

        // Source address: fd69:726f::1
        packet[8..24].copy_from_slice(&[
            0xfd, 0x69, 0x72, 0x6f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x01,
        ]);

        // Destination address: from registry
        packet[24..40].copy_from_slice(&dest_ip.octets());

        // Handle the packet (OS → Network)
        let result = router.handle_os_to_network(&packet).await;
        assert!(result.is_ok());

        // Verify packet was sent to network channel
        let received = to_network_rx.try_recv();
        assert!(received.is_ok());
        let (recv_endpoint_id, recv_packet) = received.unwrap();
        assert_eq!(recv_endpoint_id, endpoint_id);
        assert_eq!(recv_packet.as_bytes(), Some(packet.as_slice()));
    }

    #[tokio::test]
    async fn test_handle_os_to_network_unknown_destination() {
        let registry = Arc::new(Registry::new());
        let (to_network_tx, mut to_network_rx) = mpsc::unbounded_channel();
        let (_from_network_tx, from_network_rx) = mpsc::unbounded_channel();
        let router = PacketRouter::new(registry, to_network_tx, from_network_rx);

        // Create IPv6 packet with unknown destination
        let mut packet = vec![0u8; 40];
        packet[0] = 0x60; // Version 6
        packet[6] = 59; // No next header
        packet[7] = 64; // Hop limit

        // Source: fd69:726f::1
        packet[8..24].copy_from_slice(&[
            0xfd, 0x69, 0x72, 0x6f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x01,
        ]);

        // Destination: fd69:726f::9999 (not in registry)
        packet[24..40].copy_from_slice(&[
            0xfd, 0x69, 0x72, 0x6f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x99, 0x99,
        ]);

        // Should handle gracefully (log warning but not error)
        let result = router.handle_os_to_network(&packet).await;
        assert!(result.is_ok());

        // Verify packet was NOT sent to network channel
        let received = to_network_rx.try_recv();
        assert!(received.is_err()); // Should be empty
    }

    #[tokio::test]
    async fn test_handle_os_to_network_invalid_packet() {
        let registry = Arc::new(Registry::new());
        let (to_network_tx, mut to_network_rx) = mpsc::unbounded_channel();
        let (_from_network_tx, from_network_rx) = mpsc::unbounded_channel();
        let router = PacketRouter::new(registry, to_network_tx, from_network_rx);

        // Invalid packet (too short, version 0)
        let packet = vec![0u8; 10];

        // Should handle gracefully (non-IPv6 packets are filtered out)
        let result = router.handle_os_to_network(&packet).await;
        assert!(result.is_ok());

        // Verify packet was NOT sent to network channel
        let received = to_network_rx.try_recv();
        assert!(received.is_err()); // Should be empty
    }

    /// The router loop must move packets from a TunIo stream into the
    /// to-network channel and from the from-network channel into the TunIo
    /// sink, without a real TUN device.
    #[tokio::test]
    async fn test_run_routes_packets_through_tun_io() {
        let registry = Arc::new(Registry::new());
        let endpoint_id = test_endpoint_id(7);
        let dest_ip = registry.get_or_assign_ip(endpoint_id);

        let (to_network_tx, mut to_network_rx) = mpsc::unbounded_channel();
        let (from_network_tx, from_network_rx) = mpsc::unbounded_channel();
        let router = PacketRouter::new(Arc::clone(&registry), to_network_tx, from_network_rx);

        // Fake TUN: incoming packets from a channel, outgoing packets into a channel
        let (os_tx, os_rx) = mpsc::unbounded_channel::<std::io::Result<Vec<u8>>>();
        let (tun_written_tx, mut tun_written_rx) = mpsc::unbounded_channel::<Vec<u8>>();

        let incoming = Box::pin(futures::stream::unfold(os_rx, |mut rx| async move {
            rx.recv().await.map(|packet| (packet, rx))
        }));
        let outgoing = Box::pin(futures::sink::unfold(
            tun_written_tx,
            |tx, packet: Vec<u8>| async move {
                tx.send(packet).unwrap();
                Ok::<_, std::io::Error>(tx)
            },
        ));

        let io = TunIo { incoming, outgoing };
        let handle = tokio::spawn(router.run(io));

        // OS → Network
        let mut packet = vec![0u8; 40];
        packet[0] = 0x60;
        packet[6] = 59;
        packet[7] = 64;
        packet[24..40].copy_from_slice(&dest_ip.octets());
        os_tx.send(Ok(packet.clone())).unwrap();

        let (recv_endpoint_id, recv_packet) = to_network_rx.recv().await.unwrap();
        assert_eq!(recv_endpoint_id, endpoint_id);
        assert_eq!(recv_packet.as_bytes(), Some(packet.as_slice()));

        // Network → OS
        let inbound = Packet::raw(packet.clone());
        from_network_tx.send(inbound).unwrap();
        let written = tun_written_rx.recv().await.unwrap();
        assert_eq!(written, packet);

        handle.abort();
    }
}
