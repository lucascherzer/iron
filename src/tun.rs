use crate::mapping::Registry;
use anyhow::{Context, Result};
use etherparse::Ipv6Header;
use futures::StreamExt;
use iroh::EndpointId;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};
use tun::{AsyncDevice, Configuration, Layer};

/// TUN interface for bidirectional packet routing
///
/// **Packet Flow:**
///
/// **OS → Network (Outbound to peers):**
/// 1. OS application sends to `fd69:726f::xxxx` (peer's IPv6)
/// 2. OS writes packet to TUN device
/// 3. We read from TUN, parse destination IPv6
/// 4. Lookup EndpointId from IPv6 in registry
/// 5. Send (EndpointId, packet) to iroh via `to_network_tx`
///
/// **Network → OS (Inbound from peers):**
/// 1. Iroh receives packet from peer (knows sender EndpointId)
/// 2. Derive source IPv6 from EndpointId via registry
/// 3. Rewrite packet's source IPv6 if needed
/// 4. Send packet via `from_network_rx`
/// 5. We write packet to TUN device
/// 6. OS routes to listening application
pub struct TunInterface {
    registry: Arc<Registry>,
    /// Channel for sending packets TO network (OS → iroh)
    /// Format: (destination_endpoint_id, raw_packet_bytes)
    to_network_tx: mpsc::UnboundedSender<(EndpointId, Vec<u8>)>,
    /// Channel for receiving packets FROM network (iroh → OS)
    /// Format: raw_packet_bytes (with correct source IPv6 already set)
    from_network_rx: mpsc::UnboundedReceiver<Vec<u8>>,
}

impl TunInterface {
    /// Creates a new TUN interface
    ///
    /// # Arguments
    ///
    /// * `registry` - Shared registry for IPv6 <-> EndpointId mapping
    /// * `to_network_tx` - Channel sender for packets going to iroh peers
    /// * `from_network_rx` - Channel receiver for packets coming from iroh peers
    pub fn new(
        registry: Arc<Registry>,
        to_network_tx: mpsc::UnboundedSender<(EndpointId, Vec<u8>)>,
        from_network_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    ) -> Self {
        info!("Creating TUN interface");
        Self {
            registry,
            to_network_tx,
            from_network_rx,
        }
    }

    /// Creates and configures the TUN device
    ///
    /// # Platform Notes
    ///
    /// - macOS: Creates a `utun` device (requires root/sudo)
    /// - Linux: Creates a `tun` device (requires root/sudo)
    ///
    /// # Configuration
    ///
    /// - IPv6 only (Layer3)
    /// - Address: `fd69:726f::1/32`
    /// - MTU: 1420 bytes (accounts for QUIC overhead)
    fn create_device() -> Result<AsyncDevice> {
        info!("Creating TUN device (requires root/sudo)");
        let mut config = Configuration::default();
        config
            .layer(Layer::L3)
            .address((169, 254, 0, 1)) // Link-local IPv4 (required but unused)
            .destination((169, 254, 0, 2))
            .mtu(1420)
            .up();

        #[cfg(target_os = "macos")]
        config.tun_name("utun");

        #[cfg(target_os = "linux")]
        config.tun_name("iron0");

        let device =
            tun::create_as_async(&config).context("Failed to create TUN device (are you root?)")?;

        info!("TUN device created: {}", device.as_ref().tun_name()?);

        Ok(device)
    }

    /// Initializes the TUN device and starts the packet processing loop.
    ///
    /// This is the main event loop that handles bidirectional packet flow:
    /// - **OS → Network**: Read from TUN, lookup EndpointId, send to iroh
    /// - **Network → OS**: Receive from iroh, write to TUN
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - TUN device creation fails (likely permission issue)
    /// - Packet parsing fails repeatedly
    /// - Device I/O errors occur
    pub async fn run(mut self) -> Result<()> {
        let device = Self::create_device()?;
        let mut framed = device.into_framed();

        info!("TUN interface running, ready to process packets");

        loop {
            tokio::select! {
                // OS → Network: Read packet from TUN, send to iroh
                Some(packet) = framed.next() => {
                    let packet = packet.context("Failed to read packet from TUN")?;
                    trace!("Received packet from OS ({} bytes)", packet.len());
                    if let Err(e) = self.handle_os_to_network(&packet).await {
                        warn!("Failed to handle OS→Network packet: {}", e);
                    }
                }

                // Network → OS: Receive packet from iroh, write to TUN
                Some(packet) = self.from_network_rx.recv() => {
                    trace!("Writing packet to OS ({} bytes)", packet.len());
                    use futures::SinkExt;
                    if let Err(e) = framed.send(packet.into()).await {
                        error!("Failed to write packet to TUN: {}", e);
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
        // Parse IPv6 header
        let ipv6_header = Ipv6Header::from_slice(packet).context("Failed to parse IPv6 header")?;

        let dest_addr = ipv6_header.0.destination_addr();

        debug!(
            "TUN received OS→Network: {} -> {}, {} bytes",
            ipv6_header.0.source_addr(),
            dest_addr,
            packet.len()
        );

        // Lookup EndpointId for destination
        if let Some(endpoint_id) = self.registry.get_endpoint_id(&dest_addr) {
            debug!(
                "Resolved {} -> EndpointId {}",
                dest_addr,
                hex::encode(endpoint_id.as_bytes())
            );

            // Send to network layer (iroh will handle actual transmission)
            self.to_network_tx
                .send((endpoint_id, packet.to_vec()))
                .context("Failed to send packet to network layer")?;
        } else {
            warn!(
                "No EndpointId found for destination {}, dropping packet",
                dest_addr
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    fn test_endpoint_id(seed: u8) -> iroh::EndpointId {
        let secret = SecretKey::from_bytes(&[seed; 32]);
        secret.public()
    }

    #[test]
    fn test_tun_interface_new() {
        let registry = Arc::new(Registry::new());
        let (to_network_tx, _to_network_rx) = mpsc::unbounded_channel();
        let (_from_network_tx, from_network_rx) = mpsc::unbounded_channel();
        let _tun = TunInterface::new(registry, to_network_tx, from_network_rx);
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
        let tun = TunInterface::new(Arc::clone(&registry), to_network_tx, from_network_rx);

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
        let result = tun.handle_os_to_network(&packet).await;
        assert!(result.is_ok());

        // Verify packet was sent to network channel
        let received = to_network_rx.try_recv();
        assert!(received.is_ok());
        let (recv_endpoint_id, recv_packet) = received.unwrap();
        assert_eq!(recv_endpoint_id, endpoint_id);
        assert_eq!(recv_packet, packet);
    }

    #[tokio::test]
    async fn test_handle_os_to_network_unknown_destination() {
        let registry = Arc::new(Registry::new());
        let (to_network_tx, mut to_network_rx) = mpsc::unbounded_channel();
        let (_from_network_tx, from_network_rx) = mpsc::unbounded_channel();
        let tun = TunInterface::new(registry, to_network_tx, from_network_rx);

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
        let result = tun.handle_os_to_network(&packet).await;
        assert!(result.is_ok());

        // Verify packet was NOT sent to network channel
        let received = to_network_rx.try_recv();
        assert!(received.is_err()); // Should be empty
    }

    #[tokio::test]
    async fn test_handle_os_to_network_invalid_packet() {
        let registry = Arc::new(Registry::new());
        let (to_network_tx, _to_network_rx) = mpsc::unbounded_channel();
        let (_from_network_tx, from_network_rx) = mpsc::unbounded_channel();
        let tun = TunInterface::new(registry, to_network_tx, from_network_rx);

        // Invalid packet (too short)
        let packet = vec![0u8; 10];

        let result = tun.handle_os_to_network(&packet).await;
        assert!(result.is_err());
    }
}
