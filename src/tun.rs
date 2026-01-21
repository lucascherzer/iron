use crate::mapping::Registry;
use anyhow::{Context, Result};
use etherparse::{Ipv6Header, TcpHeader};
use futures::StreamExt;
use iroh::EndpointId;
use std::net::Ipv6Addr;
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
/// 2. Protocol layer rewrites source IPv6 to sender's derived IPv6
/// 3. Send packet via `from_network_rx`
/// 4. We write packet to TUN device
/// 5. OS routes to listening application
pub struct TunInterface {
    registry: Arc<Registry>,
    /// This node's derived IPv6 address (used for TUN configuration)
    node_ipv6: Ipv6Addr,
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
    /// * `node_ipv6` - This node's derived IPv6 address
    /// * `to_network_tx` - Channel sender for packets going to iroh peers
    /// * `from_network_rx` - Channel receiver for packets coming from iroh peers
    pub fn new(
        registry: Arc<Registry>,
        node_ipv6: Ipv6Addr,
        to_network_tx: mpsc::UnboundedSender<(EndpointId, Vec<u8>)>,
        from_network_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    ) -> Self {
        info!("Creating TUN interface with IPv6: {}", node_ipv6);
        Self {
            registry,
            node_ipv6,
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
    /// - Address: Node's derived IPv6 with /32 prefix
    /// - MTU: 1420 bytes (accounts for QUIC overhead)
    fn create_device(&self) -> Result<AsyncDevice> {
        info!("Creating TUN device (requires root/sudo)");
        let mut config = Configuration::default();

        // Configure IPv6-only TUN device (Layer 3)
        // Note: IPv4 addresses are required by tun crate but ignored for IPv6 traffic
        // The actual IPv6 configuration happens in configure_ipv6()
        config
            .layer(Layer::L3)
            .address((169, 254, 0, 1))
            .netmask((255, 255, 255, 0))
            .destination((169, 254, 0, 2))
            .mtu(1420)
            .up();

        #[cfg(target_os = "linux")]
        config.platform_config(|platform_config| {
            platform_config.ensure_root_privileges(true);
        });

        debug!("TUN configuration: {:?}", config);

        let device = match tun::create_as_async(&config) {
            Ok(dev) => {
                debug!("TUN device creation successful");
                dev
            }
            Err(e) => {
                error!("TUN device creation failed: {:?}", e);
                error!("Error kind: {}", e);
                error!("Configuration was: {:?}", config);
                anyhow::bail!("Failed to create TUN device: {} (are you root?)", e);
            }
        };

        let tun_name = match device.as_ref().tun_name() {
            Ok(name) => name,
            Err(e) => {
                error!("Failed to get TUN device name: {:?}", e);
                anyhow::bail!("Failed to get TUN device name: {}", e);
            }
        };
        info!("TUN device created: {}", tun_name);

        // Disable IPv4 on the interface (we only use IPv6)
        match self.disable_ipv4(&tun_name) {
            Ok(_) => debug!("IPv4 disabled on interface"),
            Err(e) => {
                warn!("Failed to disable IPv4 (non-critical): {}", e);
                // Continue anyway - this is not critical
            }
        }

        // Configure IPv6 address on the interface
        match self.configure_ipv6(&tun_name) {
            Ok(_) => debug!("IPv6 configuration successful"),
            Err(e) => {
                error!("IPv6 configuration failed: {:?}", e);
                return Err(e);
            }
        }

        Ok(device)
    }

    /// Disables IPv4 on the TUN interface
    ///
    /// This ensures the interface only handles IPv6 traffic, preventing
    /// the OS from sending any IPv4 packets to the TUN device.
    fn disable_ipv4(&self, tun_name: &str) -> Result<()> {
        info!("Disabling IPv4 on {}", tun_name);

        #[cfg(target_os = "macos")]
        {
            // Remove the IPv4 address configuration
            // Format: ifconfig <interface> inet <address> delete
            let output = std::process::Command::new("ifconfig")
                .arg(tun_name)
                .arg("inet")
                .arg("169.254.0.1")
                .arg("delete")
                .output()
                .context("Failed to execute ifconfig")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // Might not exist, that's okay
                debug!("IPv4 removal returned: {}", stderr);
            } else {
                info!("IPv4 address removed from {}", tun_name);
            }
        }

        #[cfg(target_os = "linux")]
        {
            // Remove all IPv4 addresses
            let output = std::process::Command::new("ip")
                .arg("-4")
                .arg("addr")
                .arg("flush")
                .arg("dev")
                .arg(tun_name)
                .output()
                .context("Failed to execute ip command")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                debug!("IPv4 flush returned: {}", stderr);
            }

            info!("IPv4 disabled on {}", tun_name);
        }

        Ok(())
    }

    /// Configures IPv6 address and routing on the TUN interface
    ///
    /// Uses system commands to add IPv6 address and route since the tun crate
    /// doesn't handle IPv6 configuration automatically on all platforms.
    ///
    /// Sets up:
    /// - Interface IPv6 address: Node's derived IPv6 with /32 prefix
    /// - Route: fd69:726f::/32 → TUN interface
    fn configure_ipv6(&self, tun_name: &str) -> Result<()> {
        info!(
            "Configuring IPv6 on {} with address {}",
            tun_name, self.node_ipv6
        );

        #[cfg(target_os = "macos")]
        {
            // Add IPv6 address with /32 prefix
            let ipv6_with_prefix = format!("{}/32", self.node_ipv6);
            let output = std::process::Command::new("ifconfig")
                .arg(tun_name)
                .arg("inet6")
                .arg(&ipv6_with_prefix)
                .arg("up")
                .output()
                .context("Failed to execute ifconfig")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("Failed to configure IPv6 address: {}", stderr);
            }

            info!(
                "IPv6 address configured: {} on {}",
                ipv6_with_prefix, tun_name
            );

            // Add route for the entire fd69:726f::/32 network
            let output = std::process::Command::new("route")
                .arg("-n")
                .arg("add")
                .arg("-inet6")
                .arg("fd69:726f::/32")
                .arg("-interface")
                .arg(tun_name)
                .output()
                .context("Failed to execute route command")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // Route might already exist, just warn
                warn!("Failed to add route (might already exist): {}", stderr);
            } else {
                info!("IPv6 route added: fd69:726f::/32 → {}", tun_name);
            }
        }

        #[cfg(target_os = "linux")]
        {
            // Add IPv6 address with /32 prefix
            let ipv6_with_prefix = format!("{}/32", self.node_ipv6);
            let output = std::process::Command::new("ip")
                .arg("-6")
                .arg("addr")
                .arg("add")
                .arg(&ipv6_with_prefix)
                .arg("dev")
                .arg(tun_name)
                .output()
                .context("Failed to execute ip command")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("Failed to configure IPv6 address: {}", stderr);
            }

            info!(
                "IPv6 address configured: {} on {}",
                ipv6_with_prefix, tun_name
            );

            // Bring the interface up
            let output = std::process::Command::new("ip")
                .arg("link")
                .arg("set")
                .arg(tun_name)
                .arg("up")
                .output()
                .context("Failed to bring interface up")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!("Failed to bring interface up: {}", stderr);
            }

            // Add route for the entire fd69:726f::/32 network
            let output = std::process::Command::new("ip")
                .arg("-6")
                .arg("route")
                .arg("add")
                .arg("fd69:726f::/32")
                .arg("dev")
                .arg(tun_name)
                .output()
                .context("Failed to add route")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // Route might already exist, just warn
                warn!("Failed to add route (might already exist): {}", stderr);
            } else {
                info!("IPv6 route added: fd69:726f::/32 → {}", tun_name);
            }
        }

        Ok(())
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
        let device = self.create_device()?;
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
                    debug!("Received packet from network, writing to TUN ({} bytes)", packet.len());

                    // Inspect packet for debugging
                    if let Err(e) = Self::inspect_packet(&packet, "Network→OS") {
                        trace!("Failed to inspect packet: {}", e);
                    }

                    use futures::SinkExt;
                    if let Err(e) = framed.send(packet).await {
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
                .send((endpoint_id, packet.to_vec()))
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
    use iroh::SecretKey;

    fn test_endpoint_id(seed: u8) -> iroh::EndpointId {
        let secret = SecretKey::from_bytes(&[seed; 32]);
        secret.public()
    }

    #[test]
    fn test_tun_interface_new() {
        let registry = Arc::new(Registry::new());
        let endpoint_id = test_endpoint_id(1);
        let node_ipv6 = registry.get_or_assign_ip(endpoint_id);
        let (to_network_tx, _to_network_rx) = mpsc::unbounded_channel();
        let (_from_network_tx, from_network_rx) = mpsc::unbounded_channel();
        let _tun = TunInterface::new(registry, node_ipv6, to_network_tx, from_network_rx);
        // Just verify it constructs
    }

    #[tokio::test]
    async fn test_handle_os_to_network_valid_destination() {
        let registry = Arc::new(Registry::new());
        let endpoint_id = test_endpoint_id(42);

        // Get the IPv6 for this endpoint
        let dest_ip = registry.get_or_assign_ip(endpoint_id);

        let node_endpoint_id = test_endpoint_id(1);
        let node_ipv6 = registry.get_or_assign_ip(node_endpoint_id);

        let (to_network_tx, mut to_network_rx) = mpsc::unbounded_channel();
        let (_from_network_tx, from_network_rx) = mpsc::unbounded_channel();
        let tun = TunInterface::new(
            Arc::clone(&registry),
            node_ipv6,
            to_network_tx,
            from_network_rx,
        );

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
        let node_endpoint_id = test_endpoint_id(1);
        let node_ipv6 = registry.get_or_assign_ip(node_endpoint_id);
        let (to_network_tx, mut to_network_rx) = mpsc::unbounded_channel();
        let (_from_network_tx, from_network_rx) = mpsc::unbounded_channel();
        let tun = TunInterface::new(registry, node_ipv6, to_network_tx, from_network_rx);

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
        let node_endpoint_id = test_endpoint_id(1);
        let node_ipv6 = registry.get_or_assign_ip(node_endpoint_id);
        let (to_network_tx, mut to_network_rx) = mpsc::unbounded_channel();
        let (_from_network_tx, from_network_rx) = mpsc::unbounded_channel();
        let tun = TunInterface::new(registry, node_ipv6, to_network_tx, from_network_rx);

        // Invalid packet (too short, version 0)
        let packet = vec![0u8; 10];

        // Should handle gracefully (non-IPv6 packets are filtered out)
        let result = tun.handle_os_to_network(&packet).await;
        assert!(result.is_ok());

        // Verify packet was NOT sent to network channel
        let received = to_network_rx.try_recv();
        assert!(received.is_err()); // Should be empty
    }
}
