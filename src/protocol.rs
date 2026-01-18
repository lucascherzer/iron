use crate::mapping::Registry;
use anyhow::{Context, Result};
use iroh::{Endpoint, EndpointId};
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
/// - **Send**: Receives (EndpointId, packet) from TUN, sends via QUIC
/// - **Receive**: Accepts QUIC connections, forwards packets to TUN
pub struct IronProtocol {
    registry: Arc<Registry>,
    endpoint: Endpoint,
    /// Receives packets from TUN to send to peers
    to_network_rx: mpsc::UnboundedReceiver<(EndpointId, Vec<u8>)>,
    /// Sends received packets to TUN
    from_network_tx: mpsc::UnboundedSender<Vec<u8>>,
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

        while let Some((dest_endpoint_id, packet)) = self.to_network_rx.recv().await {
            debug!(
                "Sending packet to {} ({} bytes)",
                dest_endpoint_id,
                packet.len()
            );

            if let Err(e) = self.send_packet(&dest_endpoint_id, &packet).await {
                warn!("Failed to send packet to {}: {}", dest_endpoint_id, e);
                // Continue processing other packets
            }
        }

        info!("Send loop terminated (channel closed)");
        Ok(())
    }

    /// Sends a single packet to a peer
    async fn send_packet(&self, dest: &EndpointId, packet: &[u8]) -> Result<()> {
        trace!("Connecting to peer {}", dest);
        // Connect to peer
        let conn = self
            .endpoint
            .connect(*dest, ALPN)
            .await
            .context("Failed to connect to peer")?;

        trace!("Opening bi-directional stream to {}", dest);
        // Open bi-directional stream
        let (mut send, _recv) = conn
            .open_bi()
            .await
            .context("Failed to open bi-directional stream")?;

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
    async fn handle_connection(
        conn: iroh::endpoint::Connection,
        sender_id: EndpointId,
        registry: Arc<Registry>,
        from_network_tx: mpsc::UnboundedSender<Vec<u8>>,
    ) -> Result<()> {
        trace!("Accepting bi-directional stream from {}", sender_id);
        // Accept bi-directional stream
        let (mut send, mut recv) = conn
            .accept_bi()
            .await
            .context("Failed to accept bi-directional stream")?;

        // Read packet data
        let packet = recv
            .read_to_end(MAX_PACKET_SIZE)
            .await
            .context("Failed to read packet data")?;

        debug!(
            "Received packet from {} ({} bytes)",
            sender_id,
            packet.len()
        );

        // Verify source address in packet matches sender
        if let Err(e) = Self::verify_source_address(&packet, &sender_id, &registry) {
            warn!(
                "Source address verification failed for {}: {}",
                sender_id, e
            );
            return Err(e);
        }

        trace!("Forwarding verified packet to TUN");
        // Forward packet to TUN
        from_network_tx
            .send(packet)
            .context("Failed to send packet to TUN")?;

        // Close send side
        send.finish().context("Failed to finish send stream")?;

        Ok(())
    }

    /// Verifies that the packet's source IPv6 matches the sender's EndpointId
    ///
    /// This prevents peers from spoofing packets from other peers.
    fn verify_source_address(
        packet: &[u8],
        sender_id: &EndpointId,
        registry: &Arc<Registry>,
    ) -> Result<()> {
        use etherparse::Ipv6Header;

        // Parse IPv6 header
        let (header, _) = Ipv6Header::from_slice(packet).context("Failed to parse IPv6 header")?;

        let packet_src = header.source_addr();
        let expected_src = registry.get_or_assign_ip(*sender_id);

        if packet_src != expected_src {
            anyhow::bail!(
                "Source address mismatch: packet claims {} but sender is {} (expected {})",
                packet_src,
                sender_id,
                expected_src
            );
        }

        Ok(())
    }
}
