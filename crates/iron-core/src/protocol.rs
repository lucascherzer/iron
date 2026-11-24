use crate::mapping::Registry;
use crate::packet::Packet;
use anyhow::{Context, Result};
use iroh::{Endpoint, EndpointId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::Instant;
use tracing::{debug, error, info, trace, warn};

/// ALPN protocol identifier for iron packet transport
pub const ALPN: &[u8] = b"iron/packet/0";

/// Maximum packet size (MTU)
const MAX_PACKET_SIZE: usize = 1500;

/// Idle time after which cached connections and peer senders are retired.
const CONNECTION_TTL: Duration = Duration::from_secs(60);
/// Deadline for one packet send (dial + stream open + write), and for an
/// incoming handshake.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// After a failed send, a peer's traffic is dropped for this long.
const RETRY_COOLDOWN: Duration = Duration::from_secs(10);
const MAX_HANDSHAKES: usize = 32;
const MAX_PEERS: usize = 128;
const PEER_QUEUE_SIZE: usize = 64;
/// Router-to-protocol packet capacity. Overload drops packets rather than
/// blocking the TUN loop (and therefore unrelated inbound traffic).
pub const OUTBOUND_QUEUE_SIZE: usize = 256;

// Checkout refreshes `last_used`, so the idle sweep cannot close a connection
// during a send only while the TTL exceeds the send deadline.
const _: () = assert!(CONNECTION_TTL.as_secs() > SEND_TIMEOUT.as_secs());

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
    connections: Arc<PeerConnections>,
    to_network_rx: mpsc::Receiver<(EndpointId, Packet)>,
}

/// State shared by the concurrent peer senders and the accept loop. The
/// single-consumer outbound channel stays in IronProtocol.
///
/// Each direction uses its own connection: the pool only holds connections we
/// dialed, and accepted connections are only read from. Two peers talking both
/// ways therefore hold two connections, but neither side ever has to agree with
/// the other on which connection to keep.
struct PeerConnections {
    registry: Arc<Registry>,
    endpoint: Endpoint,
    /// Sends received packets to TUN
    from_network_tx: mpsc::UnboundedSender<Packet>,
    /// Outbound connections we dialed, by destination.
    connection_pool: Mutex<HashMap<EndpointId, CachedConnection>>,
}

impl IronProtocol {
    /// Creates a new protocol handler
    pub fn new(
        registry: Arc<Registry>,
        endpoint: Endpoint,
        to_network_rx: mpsc::Receiver<(EndpointId, Packet)>,
        from_network_tx: mpsc::UnboundedSender<Packet>,
    ) -> Self {
        info!("Creating protocol handler for endpoint {}", endpoint.id());
        Self {
            connections: Arc::new(PeerConnections {
                registry,
                endpoint,
                from_network_tx,
                connection_pool: Mutex::new(HashMap::new()),
            }),
            to_network_rx,
        }
    }

    /// Starts the protocol handler
    ///
    /// Runs two concurrent loops:
    /// 1. **Send loop**: Reads from `to_network_rx`, sends packets via QUIC
    /// 2. **Accept loop**: Accepts incoming connections, receives packets
    pub async fn run(mut self) -> Result<()> {
        let connections = self.connections.clone();
        tokio::select! {
            result = connections.accept_loop() => result,
            result = self.send_loop() => result,
        }
    }

    /// Send loop: hands each packet to its destination's sender task, so a
    /// stalled peer only ever blocks its own queue.
    async fn send_loop(&mut self) -> Result<()> {
        info!("Starting send loop");
        let self_id = self.connections.endpoint.id();
        let mut peers: HashMap<EndpointId, mpsc::Sender<Packet>> = HashMap::new();
        let mut workers = JoinSet::new();
        let mut cleanup = tokio::time::interval(Duration::from_secs(1));

        loop {
            let (dest_endpoint_id, packet) = tokio::select! {
                packet = self.to_network_rx.recv() => match packet {
                    Some(packet) => packet,
                    None => break,
                },
                _ = cleanup.tick() => {
                    self.connections.prune_connections();
                    peers.retain(|_, tx| !tx.is_closed());
                    continue;
                }
                Some(result) = workers.join_next() => {
                    if let Err(e) = result {
                        warn!("Peer sender failed: {}", e);
                    }
                    continue;
                }
            };
            debug!(
                "Sending packet to {} ({} bytes)",
                dest_endpoint_id,
                packet.len()
            );

            // Check for loopback (sending to ourselves)
            if dest_endpoint_id == self_id {
                debug!("Loopback detected: cannot connect to self (P2P requires two nodes)");
                // The OS may deliver traffic to its assigned iron address locally.
                // If it reaches TUN instead, local reinjection is not implemented
                // or verified on our supported platforms yet.
                continue;
            }

            let connections = self.connections.clone();
            enqueue_peer_packet(&mut peers, dest_endpoint_id, packet, || {
                let (tx, rx) = mpsc::channel(PEER_QUEUE_SIZE);
                workers.spawn(run_peer_sender(rx, move |packet| {
                    let connections = connections.clone();
                    async move { connections.send_packet(&dest_endpoint_id, &packet).await }
                }));
                tx
            });
        }

        info!("Send loop terminated (channel closed)");
        Ok(())
    }
}

/// Queues a packet for its peer's sender task, spawning one if needed.
/// A full queue drops the packet; a closed queue means the sender retired
/// while idle, so the packet goes to a fresh one.
fn enqueue_peer_packet(
    peers: &mut HashMap<EndpointId, mpsc::Sender<Packet>>,
    dest: EndpointId,
    packet: Packet,
    spawn: impl FnOnce() -> mpsc::Sender<Packet>,
) {
    let packet = match peers.get(&dest) {
        Some(tx) => match tx.try_send(packet) {
            Ok(()) => return,
            Err(mpsc::error::TrySendError::Full(_)) => {
                debug!("Dropping packet to {}: peer queue full", dest);
                return;
            }
            Err(mpsc::error::TrySendError::Closed(packet)) => packet,
        },
        None => packet,
    };
    peers.retain(|_, tx| !tx.is_closed());
    if peers.len() >= MAX_PEERS {
        debug!("Dropping packet: peer sender limit reached");
        return;
    }
    let tx = spawn();
    // A fresh queue always has room.
    let _ = tx.try_send(packet);
    peers.insert(dest, tx);
}

/// Sends a peer's packets one at a time until it has been idle for
/// `CONNECTION_TTL`. After a failure, the packets that piled up behind it are
/// stale and would each cost another dial: wait out the cooldown, then drop them.
async fn run_peer_sender<F, Fut>(mut rx: mpsc::Receiver<Packet>, mut send: F)
where
    F: FnMut(Packet) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    while let Some(packet) = next_peer_packet(&mut rx).await {
        if let Err(e) = send(packet).await {
            debug!("Peer send failed; dropping backlog: {:#}", e);
            tokio::time::sleep(RETRY_COOLDOWN).await;
            for _ in 0..rx.len() {
                let _ = rx.try_recv();
            }
        }
    }
}

async fn next_peer_packet(rx: &mut mpsc::Receiver<Packet>) -> Option<Packet> {
    match tokio::time::timeout(CONNECTION_TTL, rx.recv()).await {
        Ok(packet) => packet,
        Err(_) => {
            // Close admission before retiring, then drain anything accepted in
            // the race with the idle deadline. Closed sends are retried above.
            rx.close();
            rx.recv().await
        }
    }
}

impl PeerConnections {
    /// Closes idle connections and drops closed ones, without waiting for
    /// another send to the same peer.
    fn prune_connections(&self) {
        self.connection_pool.lock().unwrap().retain(|_, entry| {
            if entry.last_used.elapsed() >= CONNECTION_TTL {
                entry.conn.close(0u32.into(), b"idle connection");
            }
            entry.conn.close_reason().is_none()
        });
    }

    /// Removes `conn` from the pool, unless a newer connection replaced it.
    fn remove_connection(&self, dest: &EndpointId, conn: &iroh::endpoint::Connection) {
        let mut pool = self.connection_pool.lock().unwrap();
        if pool
            .get(dest)
            .is_some_and(|entry| entry.conn.stable_id() == conn.stable_id())
        {
            pool.remove(dest);
        }
    }

    /// Returns a live cached connection to `dest`, or dials a new one.
    /// Checkout refreshes `last_used`, protecting an in-flight send from the sweep.
    async fn get_or_create_connection(
        &self,
        dest: &EndpointId,
    ) -> Result<iroh::endpoint::Connection> {
        {
            let mut pool = self.connection_pool.lock().unwrap();
            if let Some(entry) = pool.get_mut(dest) {
                if entry.conn.close_reason().is_none() && entry.last_used.elapsed() < CONNECTION_TTL
                {
                    entry.last_used = Instant::now();
                    return Ok(entry.conn.clone());
                }
                trace!("Cached connection to {} closed or expired", dest);
                entry.conn.close(0u32.into(), b"idle connection");
                pool.remove(dest);
            }
        }

        info!("Connecting to peer {} (no cached connection)", dest);
        let conn = self
            .endpoint
            .connect(*dest, ALPN)
            .await
            .context("Failed to connect to peer")?;
        self.connection_pool.lock().unwrap().insert(
            *dest,
            CachedConnection {
                conn: conn.clone(),
                last_used: Instant::now(),
            },
        );
        info!("Successfully connected to {}", dest);
        Ok(conn)
    }

    /// Refresh the `last_used` timestamp for a cached connection (refresh on use).
    fn refresh_connection(&self, dest: &EndpointId, conn: &iroh::endpoint::Connection) {
        if let Some(entry) = self.connection_pool.lock().unwrap().get_mut(dest)
            && entry.conn.stable_id() == conn.stable_id()
        {
            entry.last_used = Instant::now();
        }
    }

    /// Sends a single packet to a peer within `SEND_TIMEOUT`.
    ///
    /// Success means queued for QUIC, not acknowledged delivery.
    async fn send_packet(&self, dest: &EndpointId, packet: &Packet) -> Result<()> {
        self.send_packet_with_timeout(dest, packet, SEND_TIMEOUT)
            .await
    }

    async fn send_packet_with_timeout(
        &self,
        dest: &EndpointId,
        packet: &Packet,
        timeout: Duration,
    ) -> Result<()> {
        let packet_bytes = packet.as_bytes().context("Packet has no raw bytes")?;
        let mut used = None;
        let mut stream = None;
        let result = tokio::time::timeout(timeout, async {
            let mut conn = self.get_or_create_connection(dest).await?;
            used = Some(conn.clone());
            let opened = match conn.open_bi().await {
                Ok(stream) => stream,
                Err(_) => {
                    self.remove_connection(dest, &conn);
                    conn = self.get_or_create_connection(dest).await?;
                    used = Some(conn.clone());
                    conn.open_bi()
                        .await
                        .context("Failed to open stream on retry")?
                }
            };
            let (send, _recv) = opened;
            stream = Some(send);
            let send = stream.as_mut().unwrap();
            // Never replay after writing begins: delivery is ambiguous on failure.
            send.write_all(packet_bytes)
                .await
                .context("Failed to write packet")?;
            send.finish().context("Failed to finish stream")?;
            self.refresh_connection(dest, &conn);
            Ok::<_, anyhow::Error>(())
        })
        .await;
        if let Err(e) = result
            .context("Packet send timed out")
            .and_then(|result| result)
        {
            // Cancel this packet only. A stream blocked on credit or a slow
            // write does not prove the connection itself died.
            if let Some(mut send) = stream {
                let _ = send.reset(0u32.into());
            }
            if let Some(conn) = used.filter(|conn| conn.close_reason().is_some()) {
                self.remove_connection(dest, &conn);
            }
            return Err(e);
        }
        debug!("Queued packet for transport to {}", dest);
        Ok(())
    }

    /// Accept loop: accepts incoming connections and receives packets.
    ///
    /// Handshakes run concurrently (bounded, with a deadline) so a slow one
    /// cannot hold up the others. Both task sets are owned here and aborted
    /// when the protocol stops.
    async fn accept_loop(self: Arc<Self>) -> Result<()> {
        info!("Starting accept loop on {}", self.endpoint.id());
        let mut handshakes = JoinSet::new();
        let mut receivers = JoinSet::new();

        loop {
            let incoming = tokio::select! {
                incoming = self.endpoint.accept(), if handshakes.len() < MAX_HANDSHAKES => incoming,
                Some(result) = handshakes.join_next() => {
                    if let Ok(Some(conn)) = result {
                        let connections = self.clone();
                        receivers.spawn(async move { connections.handle_connection(conn).await });
                    }
                    continue;
                }
                Some(_) = receivers.join_next() => continue,
            };
            let Some(incoming) = incoming else {
                info!("Endpoint closed, stopping accept loop");
                break;
            };

            handshakes.spawn(async move {
                match tokio::time::timeout(SEND_TIMEOUT, incoming).await {
                    Ok(Ok(conn)) => {
                        debug!(
                            "Accepted connection from {}",
                            crate::id::to_base32(&conn.remote_id())
                        );
                        Some(conn)
                    }
                    result => {
                        warn!("Failed to accept connection: {:?}", result);
                        None
                    }
                }
            });
        }

        Ok(())
    }

    /// Handles a single connection from a peer
    ///
    /// This function loops to handle multiple streams on the same connection,
    /// allowing for connection reuse and avoiding repeated handshakes.
    async fn handle_connection(&self, conn: iroh::endpoint::Connection) {
        let sender_id = conn.remote_id();
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
                match IronProtocol::rewrite_source_address(packet, &sender_id, &self.registry) {
                    Ok(p) => p,
                    Err(e) => {
                        warn!("Failed to rewrite source address for {}: {}", sender_id, e);
                        continue; // Try next stream
                    }
                };

            trace!("Forwarding packet to TUN with rewritten source");
            // Forward packet to TUN (wrap in Packet type)
            if let Err(e) = self
                .from_network_tx
                .send(Packet::raw(packet_with_correct_source))
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
    }
}

impl IronProtocol {
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
        let sender_ipv6 = registry.register(*sender_id);

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
    use iroh::address_lookup::memory::MemoryLookup;
    use iroh::endpoint::presets::N0;
    use std::net::Ipv6Addr;

    use crate::test_utils::test_endpoint_id;

    struct TestNode {
        protocol: IronProtocol,
        tx: mpsc::Sender<(EndpointId, Packet)>,
        rx: mpsc::UnboundedReceiver<Packet>,
    }

    async fn test_node(lookup: &MemoryLookup, blocked: bool) -> TestNode {
        use iroh::endpoint::{QuicTransportConfig, presets::Minimal};
        let mut builder = Endpoint::builder(Minimal)
            .alpns(vec![ALPN.to_vec()])
            .address_lookup(lookup.clone());
        if blocked {
            builder = builder.transport_config(
                QuicTransportConfig::builder()
                    .max_concurrent_bidi_streams(0u32.into())
                    .build(),
            );
        }
        let endpoint = builder.bind().await.unwrap();
        lookup.add_endpoint_info(endpoint.addr());
        let (tx, to_network_rx) = mpsc::channel(OUTBOUND_QUEUE_SIZE);
        let (from_network_tx, rx) = mpsc::unbounded_channel();
        TestNode {
            protocol: IronProtocol::new(
                Arc::new(Registry::new()),
                endpoint,
                to_network_rx,
                from_network_tx,
            ),
            tx,
            rx,
        }
    }

    fn test_packet() -> Packet {
        Packet::raw(create_ipv6_packet(
            Ipv6Addr::LOCALHOST,
            Ipv6Addr::LOCALHOST,
            8,
        ))
    }

    #[tokio::test]
    async fn test_each_direction_dials_and_closed_connection_recovers() {
        let lookup = MemoryLookup::default();
        let mut a = test_node(&lookup, false).await;
        let mut b = test_node(&lookup, false).await;
        let ta = a.protocol.connections.clone();
        let tb = b.protocol.connections.clone();
        let aid = ta.endpoint.id();
        let bid = tb.endpoint.id();
        let mut tasks = JoinSet::new();
        tasks.spawn(a.protocol.run());
        tasks.spawn(b.protocol.run());

        a.tx.send((bid, test_packet())).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), b.rx.recv())
            .await
            .unwrap()
            .unwrap();
        let ac = ta.connection_pool.lock().unwrap()[&bid].conn.clone();
        // Replies dial their own connection; the accepted one is never cached.
        assert!(tb.connection_pool.lock().unwrap().is_empty());
        b.tx.send((aid, test_packet())).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), a.rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(tb.connection_pool.lock().unwrap().contains_key(&aid));
        assert_eq!(
            ta.connection_pool.lock().unwrap()[&bid].conn.stable_id(),
            ac.stable_id()
        );

        ac.close(0u32.into(), b"test closed cache entry");
        // A closed entry must never be returned, even before the cleanup tick.
        let replacement =
            tokio::time::timeout(Duration::from_secs(3), ta.get_or_create_connection(&bid))
                .await
                .unwrap()
                .unwrap();
        assert_ne!(replacement.stable_id(), ac.stable_id());
        // An old failure must not evict the replacement.
        ta.remove_connection(&bid, &ac);
        assert_eq!(
            ta.connection_pool.lock().unwrap()[&bid].conn.stable_id(),
            replacement.stable_id()
        );
        a.tx.send((bid, test_packet())).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), b.rx.recv())
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn test_stalled_peer_isolated() {
        let lookup = MemoryLookup::default();
        let a = test_node(&lookup, false).await;
        let mut healthy = test_node(&lookup, false).await;
        let blocked = test_node(&lookup, true).await;
        let ta = a.protocol.connections.clone();
        let blocked_id = blocked.protocol.connections.endpoint.id();
        let healthy_id = healthy.protocol.connections.endpoint.id();
        let mut tasks = JoinSet::new();
        tasks.spawn(blocked.protocol.run());
        tasks.spawn(healthy.protocol.run());

        // Establish the blocked connection first. Its stream credit is zero.
        let _conn = ta.get_or_create_connection(&blocked_id).await.unwrap();
        tasks.spawn(a.protocol.run());
        for _ in 0..PEER_QUEUE_SIZE * 2 {
            a.tx.send((blocked_id, test_packet())).await.unwrap();
        }
        a.tx.send((healthy_id, test_packet())).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), healthy.rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            ta.connection_pool.lock().unwrap()[&blocked_id]
                .conn
                .close_reason()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_stream_deadline_keeps_connection() {
        let lookup = MemoryLookup::default();
        let a = test_node(&lookup, false).await;
        let blocked = test_node(&lookup, true).await;
        let connections = a.protocol.connections.clone();
        let bid = blocked.protocol.connections.endpoint.id();
        let mut tasks = JoinSet::new();
        tasks.spawn(blocked.protocol.run());
        let conn = connections.get_or_create_connection(&bid).await.unwrap();

        // Inject an immediate deadline without advancing iroh's real-I/O timers.
        let packet = test_packet();
        assert!(
            connections
                .send_packet_with_timeout(&bid, &packet, Duration::ZERO)
                .await
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        // A stuck stream does not prove the connection died.
        assert!(conn.close_reason().is_none());
        assert_eq!(
            connections.connection_pool.lock().unwrap()[&bid]
                .conn
                .stable_id(),
            conn.stable_id()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_worker_idle_retirement_and_closed_queue_retry() {
        let (old_tx, mut old_rx) = mpsc::channel(PEER_QUEUE_SIZE);
        let idle = next_peer_packet(&mut old_rx);
        tokio::pin!(idle);
        assert!(futures::poll!(&mut idle).is_pending());
        tokio::time::advance(CONNECTION_TTL).await;
        assert!(idle.await.is_none());

        let dest = test_endpoint_id(42);
        let mut peers = HashMap::from([(dest, old_tx)]);
        let (new_tx, mut new_rx) = mpsc::channel(PEER_QUEUE_SIZE);
        let packet = test_packet();
        let expected = packet.as_bytes().unwrap().to_vec();
        enqueue_peer_packet(&mut peers, dest, packet, || new_tx);
        assert_eq!(new_rx.try_recv().unwrap().as_bytes().unwrap(), expected);
        assert!(new_rx.try_recv().is_err());

        // A live but full queue is not replaced or retried.
        let (tx, _rx) = mpsc::channel(1);
        tx.try_send(test_packet()).unwrap();
        peers.insert(dest, tx);
        enqueue_peer_packet(&mut peers, dest, test_packet(), || {
            panic!("a full worker must not be replaced")
        });
    }

    #[tokio::test(start_paused = true)]
    async fn test_retiring_worker_drains_packet_accepted_at_deadline() {
        let (tx, mut rx) = mpsc::channel(1);
        let idle = next_peer_packet(&mut rx);
        tokio::pin!(idle);
        assert!(futures::poll!(&mut idle).is_pending());
        tokio::time::advance(CONNECTION_TTL).await;
        // Admission before the receiver observes its deadline must not lose data.
        tx.try_send(test_packet()).unwrap();
        assert!(idle.await.is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn test_failed_send_discards_backlog_and_throttles_retries() {
        let (tx, rx) = mpsc::channel(PEER_QUEUE_SIZE);
        for _ in 0..3 {
            tx.try_send(test_packet()).unwrap();
        }
        let (attempts_tx, mut attempts) = mpsc::unbounded_channel();
        let mut tasks = JoinSet::new();
        tasks.spawn(run_peer_sender(rx, move |_| {
            attempts_tx.send(()).unwrap();
            async { anyhow::bail!("peer unreachable") }
        }));
        attempts.recv().await.unwrap();
        // Traffic arriving during the cooldown piles up and is dropped too.
        tx.try_send(test_packet()).unwrap();
        tokio::task::yield_now().await;
        assert!(attempts.try_recv().is_err());
        tokio::time::advance(RETRY_COOLDOWN).await;
        tokio::task::yield_now().await;
        assert!(attempts.try_recv().is_err(), "old backlog must not redial");
        tx.send(test_packet()).await.unwrap();
        attempts.recv().await.unwrap();
        assert!(attempts.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_cleanup_evicts_idle_connections_without_another_send() {
        let lookup = MemoryLookup::default();
        let a = test_node(&lookup, false).await;
        let b = test_node(&lookup, false).await;
        let ta = a.protocol.connections.clone();
        let bid = b.protocol.connections.endpoint.id();
        let mut tasks = JoinSet::new();
        tasks.spawn(b.protocol.run());
        let conn = ta.get_or_create_connection(&bid).await.unwrap();
        ta.connection_pool
            .lock()
            .unwrap()
            .get_mut(&bid)
            .unwrap()
            .last_used = Instant::now() - CONNECTION_TTL + Duration::from_secs(1);
        let checked_out = ta.get_or_create_connection(&bid).await.unwrap();
        ta.prune_connections();
        assert_eq!(conn.stable_id(), checked_out.stable_id());
        assert!(checked_out.close_reason().is_none());
        assert!(
            ta.connection_pool.lock().unwrap()[&bid].last_used.elapsed() < Duration::from_secs(1)
        );
        ta.connection_pool
            .lock()
            .unwrap()
            .get_mut(&bid)
            .unwrap()
            .last_used = Instant::now() - CONNECTION_TTL;
        // Exercise the sweep directly, without a wall-clock ticker window.
        ta.prune_connections();
        assert!(conn.close_reason().is_some());
        assert!(!ta.connection_pool.lock().unwrap().contains_key(&bid));
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
        let expected_sender_ipv6 = registry.register(sender_id);

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

        let expected_ipv6_1 = registry.register(sender1);
        let expected_ipv6_2 = registry.register(sender2);

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
        let sender_ipv6 = registry.register(sender_id);

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

        let (_to_network_tx, to_network_rx) = mpsc::channel(OUTBOUND_QUEUE_SIZE);
        let (from_network_tx, _from_network_rx) = mpsc::unbounded_channel();

        let protocol = IronProtocol::new(registry, endpoint, to_network_rx, from_network_tx);

        // Verify construction doesn't panic
        assert_eq!(
            protocol.connections.connection_pool.lock().unwrap().len(),
            0,
            "Connection pool should start empty"
        );
    }

    #[tokio::test]
    async fn test_connection_ttl_expires_cached_connections() {
        let secret1 = SecretKey::generate();
        let secret2 = SecretKey::generate();
        let id2 = secret2.public();

        // Shared in-memory address lookup so ep1 can resolve ep2's address
        // in-process even when sandboxed.
        let lookup = MemoryLookup::default();

        let ep1 = Endpoint::builder(N0)
            .secret_key(secret1)
            .alpns(vec![ALPN.to_vec()])
            .address_lookup(lookup.clone())
            .bind()
            .await
            .unwrap();

        let ep2 = Endpoint::builder(N0)
            .secret_key(secret2)
            .alpns(vec![ALPN.to_vec()])
            .address_lookup(lookup.clone())
            .bind()
            .await
            .unwrap();

        lookup.add_endpoint_info(ep2.addr());

        // Accept incoming connections on ep2 so ep1's connect succeeds
        let accept_task = tokio::spawn(async move {
            loop {
                if let Some(incoming) = ep2.accept().await
                    && let Ok(_conn) = incoming.await
                {}
            }
        });

        let registry = Arc::new(Registry::new());
        let (_to_network_tx, to_network_rx) = mpsc::channel(OUTBOUND_QUEUE_SIZE);
        let (from_network_tx, _from_network_rx) = mpsc::unbounded_channel();

        let protocol = IronProtocol::new(registry, ep1, to_network_rx, from_network_tx);
        let protocol = &protocol.connections;

        assert_eq!(protocol.connection_pool.lock().unwrap().len(), 0);

        // First call establishes and caches a connection
        let _conn1 = protocol.get_or_create_connection(&id2).await.unwrap();
        assert_eq!(protocol.connection_pool.lock().unwrap().len(), 1);

        // Manually age the entry past TTL
        if let Some(entry) = protocol.connection_pool.lock().unwrap().get_mut(&id2) {
            entry.last_used = Instant::now() - CONNECTION_TTL - Duration::from_secs(1);
        }

        // get_or_create_connection should evict the expired entry and create a new connection
        let _conn2 = protocol.get_or_create_connection(&id2).await.unwrap();
        assert_eq!(protocol.connection_pool.lock().unwrap().len(), 1);

        // Verify the last_used timestamp was refreshed
        if let Some(entry) = protocol.connection_pool.lock().unwrap().get(&id2) {
            assert!(
                entry.last_used.elapsed() < Duration::from_secs(1),
                "Cached timestamp should be recent after reconnection"
            );
        }

        accept_task.abort();
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
        let pre_registered_ipv6 = registry.register(sender);

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
        let post_rewrite_ipv6 = registry.register(sender);
        assert_eq!(
            pre_registered_ipv6, post_rewrite_ipv6,
            "Registry mapping should remain consistent"
        );
    }
}
