# Complete Packet Flow Documentation

## EndpointId Format

**Iroh Display Format**: Lowercase hex, 64 characters
```
197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61
```

**DNS Domain Format**: Base32 encoding (no padding), 52 characters, fits in single DNS label
```
df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron
└────────────────── 52 chars ──────────────────┘
```

Our DNS parser decodes the base32 label before `.iron` to reconstruct the 32-byte EndpointId.
Base32 encoding is case-insensitive and avoids the need for multi-label DNS splitting.

---

## Send Path: OS → Peer

### Step-by-Step Flow

**1. User Action**
```
User in browser navigates to: http://df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron
```

**2. DNS Resolution**
```
Browser → OS DNS resolver
         ↓
         127.0.0.1:5333 (our hickory-server DNS)
         ↓
Query: "df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron" AAAA?

DNS Handler:
  - Extract label: "df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq"
  - Base32 decode → EndpointId (32 bytes)
  - Registry: get_or_assign_ip(EndpointId)
  - Derive IPv6: fd69:726f::3d36:8b3d:fa9b:0339 (deterministic)

Response: fd69:726f::3d36:8b3d:fa9b:0339
```

**3. TCP Connection**
```
Browser creates socket:
  Source: [fd69:726f::1]:54321 (ephemeral port assigned by OS)
  Destination: [fd69:726f::3d36:8b3d:fa9b:0339]:80

OS sees destination in fd69:726f::/32 range
OS routing table: "Send to iron0 TUN interface"
```

**4. Packet Reaches TUN**
```
OS writes IPv6 packet to TUN device:
┌─────────────────────────────────────────┐
│ IPv6 Header                             │
│  - Version: 6                           │
│  - Source: fd69:726f::1                 │
│  - Destination: fd69:726f::3d36:...:0339│
│  - Next Header: TCP (6)                 │
├─────────────────────────────────────────┤
│ TCP Header                              │
│  - Source Port: 54321                   │
│  - Destination Port: 80                 │
│  - Flags: SYN                           │
├─────────────────────────────────────────┤
│ Payload (HTTP GET request)              │
└─────────────────────────────────────────┘

TUN reads via framed.next()
```

**5. TUN Processing**
```rust
// handle_os_to_network()
let ipv6_header = Ipv6Header::from_slice(packet)?;
let dest_ipv6 = ipv6_header.0.destination_addr(); // fd69:726f::3d36:...:0339

// Reverse lookup
let endpoint_id = registry.get_endpoint_id(&dest_ipv6)?;

// Send to iroh
to_network_tx.send((endpoint_id, packet.to_vec()))?;
```

**6. Iroh Transmission (Phase 5)**
```rust
// IronProtocol receives from channel
let (endpoint_id, packet_bytes) = to_network_rx.recv().await?;

// Open QUIC bi-directional stream
let mut stream = endpoint.connect(endpoint_id, b"iron/packet/0").await?;

// Write packet
stream.write_all(&packet_bytes).await?;
stream.finish().await?;
```

---

## Receive Path: Peer → OS

### Step-by-Step Flow

**1. Iroh Receives Packet (Phase 5)**
```rust
// IronProtocol polls endpoint
loop {
    let Some(incoming) = endpoint.accept().await else { break };
    let conn = incoming.await?;
    let sender_endpoint_id = conn.remote_id(); // Iroh tells us who sent it!
    
    tokio::spawn(async move {
        let (mut send, mut recv) = conn.accept_bi().await?;
        let packet_bytes = recv.read_to_end(1500).await?;
        
        // Packet structure:
        // ┌─────────────────────────────────────────┐
        // │ IPv6 Header (built by sender)           │
        // │  - Source: fd69:726f::yyyy (sender)     │
        // │  - Destination: fd69:726f::1 (us)       │
        // │  - Next Header: TCP (6)                 │
        // ├─────────────────────────────────────────┤
        // │ TCP Header                              │
        // │  - Source Port: 80                      │
        // │  - Destination Port: 54321 (our browser)│
        // │  - Flags: SYN-ACK                       │
        // ├─────────────────────────────────────────┤
        // │ Payload (HTTP response)                 │
        // └─────────────────────────────────────────┘
        
        // Verify source IPv6 matches sender
        let ipv6_header = Ipv6Header::from_slice(&packet_bytes)?;
        let expected_src = registry.get_or_assign_ip(sender_endpoint_id);
        assert_eq!(ipv6_header.0.source_addr(), expected_src);
        
        // Send to TUN
        from_network_tx.send(packet_bytes)?;
    });
}
```

**2. TUN Writes to Device**
```rust
// In TUN run loop
Some(packet) = from_network_rx.recv() => {
    framed.send(packet.into()).await?;
}
```

**3. OS Receives Packet**
```
OS reads packet from TUN device
IPv6 packet with:
  - Source: fd69:726f::yyyy (peer)
  - Destination: fd69:726f::1 (local)
  - TCP Dest Port: 54321
```

**4. OS Routes to Process**
```
OS socket table lookup:
  "Who's listening on [fd69:726f::1]:54321?"
  → Browser process (PID 12345)

OS delivers packet to browser's socket
Browser receives HTTP response
```

---

## How OS Knows Which Process

**Question**: When a response packet arrives with destination `[fd69:726f::1]:54321`, how does the OS know to deliver it to the browser?

**Answer**: Standard OS socket multiplexing!

### Socket Binding
When the browser creates a connection:
```rust
// Browser (simplified)
let socket = TcpStream::connect("[fd69:726f::3d36:...:0339]:80")?;
```

The OS:
1. Allocates an ephemeral port (e.g., 54321)
2. Creates socket entry in kernel table:
   ```
   Socket ID: 42
   Local:  [fd69:726f::1]:54321
   Remote: [fd69:726f::3d36:...:0339]:80
   PID: 12345 (browser process)
   ```

### Packet Delivery
When response arrives:
```
Destination: [fd69:726f::1]:54321
```

OS kernel:
1. Looks up socket table by (local IP, local port)
2. Finds Socket ID 42 → PID 12345
3. Copies packet data to socket buffer
4. Wakes up browser process
5. Browser calls `recv()` → gets data

**We don't need to do anything special!** The OS handles all port multiplexing automatically.

---

## Registry Role

### Forward Lookup (DNS)
```
EndpointId → IPv6 (for DNS queries)
```
Used when resolving `.iron` domains.

### Reverse Lookup (TUN Send)
```
IPv6 → EndpointId (for outbound packets)
```
Used when OS sends packet to TUN device.

### Forward Lookup (TUN Receive)
```
EndpointId → IPv6 (for verifying source)
```
Used when iroh receives packet to verify source address.

---

## Full Round-Trip Example

```
[Browser] http://df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron
    ↓
[DNS] Query: df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron → fd69:726f::3d36:...:0339
    ↓
[Browser] Connect to [fd69:726f::3d36:...:0339]:80
    ↓ (Socket: [fd69:726f::1]:54321 → [fd69:726f::3d36:...:0339]:80)
    ↓
[OS] Route to iron0 TUN
    ↓
[TUN] Read packet, extract dest IPv6
    ↓
[Registry] fd69:726f::3d36:...:0339 → EndpointId
    ↓
[Channel] (EndpointId, packet) → to_network_tx
    ↓
[Iroh] Open QUIC stream, send packet
    ↓
═══════════════════════════════════════════════════════════
    ↓
[Peer] Receive via iroh, process HTTP request
    ↓
[Peer] Send HTTP response packet
    ↓
═══════════════════════════════════════════════════════════
    ↓
[Iroh] Receive packet from peer (knows sender EndpointId)
    ↓
[Registry] EndpointId → fd69:726f::yyyy (verify source)
    ↓
[Channel] packet → from_network_tx
    ↓
[TUN] Write packet to device
    ↓
[OS] Read from iron0, see dest port 54321
    ↓
[OS] Socket table lookup → Browser PID 12345
    ↓
[Browser] recv() → HTTP response!
```

---

## Phase 5 Implementation Notes

The iroh integration layer needs to:

1. **Accept connections**:
   ```rust
   while let Some(incoming) = endpoint.accept().await {
       let conn = incoming.await?;
       let peer_id = conn.remote_id(); // Who sent this?
       handle_connection(peer_id, conn).await;
   }
   ```

2. **Send packets**:
   ```rust
   let (endpoint_id, packet) = to_network_rx.recv().await?;
   let conn = endpoint.connect(endpoint_id, ALPN).await?;
   let (mut send, _recv) = conn.open_bi().await?;
   send.write_all(&packet).await?;
   ```

3. **Receive packets**:
   ```rust
   let (mut send, mut recv) = conn.accept_bi().await?;
   let packet = recv.read_to_end(MTU).await?;
   
   // Verify source IPv6 matches sender
   let expected_src = registry.get_or_assign_ip(peer_id);
   // ... verify packet source ...
   
   from_network_tx.send(packet)?;
   ```

---

## Security Considerations

### Source Address Verification
When receiving packets, we MUST verify:
```rust
let packet_src = parse_ipv6_source(&packet)?;
let expected_src = registry.get_or_assign_ip(sender_endpoint_id);
if packet_src != expected_src {
    return Err("Source address spoofing attempt");
}
```

This prevents peers from sending packets claiming to be from other peers.

### Why This Works
- Iroh provides authenticated connections (public key cryptography)
- Each EndpointId → IPv6 mapping is deterministic
- Peer can't fake another peer's EndpointId (would need their private key)
- We verify packet source matches the authenticated EndpointId

---

## Summary

**Key Insight**: We're building a "virtual IPv6 network" where:
- Each iroh peer gets a deterministic IPv6 address
- The OS handles all normal networking (routing, port multiplexing, etc.)
- We just shuttle packets between TUN device and iroh connections
- Iroh handles the hard parts (NAT traversal, QUIC, authentication)

**Our job is simple**:
1. DNS: Map `.iron` domains → IPv6 addresses
2. TUN Send: Read packets from OS, send via iroh
3. TUN Receive: Get packets from iroh, write to OS

The OS does the rest!
