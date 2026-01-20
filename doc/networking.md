# iron Networking Details

This document describes the networking architecture and protocols used by iron.

## IPv6 Address Allocation

### ULA Prefix Selection

**Chosen Prefix**: `fd69:726f::/32`

**Encoding**:
- `0xfd` - ULA marker (RFC 4193)
- `0x69` - ASCII 'i' (105 decimal)
- `0x72` - ASCII 'r' (114 decimal)  
- `0x6f` - ASCII 'o' (111 decimal)
- `0x6e` would be 'n' but we only use 32 bits for the prefix

This creates a memorable, iron-branded address space that's unlikely to conflict with other ULA networks.

### Address Derivation Algorithm

**Input**: iroh `EndpointId` (32 bytes / 256 bits)  
**Output**: IPv6 address in `fd69:726f::/32` range

**Algorithm**:
```rust
pub fn derive_ip(endpoint_id: &EndpointId) -> Ipv6Addr {
    let bytes = endpoint_id.as_bytes(); // 32 bytes
    let suffix = &bytes[24..32];        // Last 8 bytes (64 bits)
    
    Ipv6Addr::new(
        0xfd69,  // ULA + 'i'
        0x726f,  // 'r' + 'o'
        0x0000,  // Reserved
        0x0000,  // Reserved
        u16::from_be_bytes([suffix[0], suffix[1]]),
        u16::from_be_bytes([suffix[2], suffix[3]]),
        u16::from_be_bytes([suffix[4], suffix[5]]),
        u16::from_be_bytes([suffix[6], suffix[7]]),
    )
}
```

**Properties**:
- **Deterministic**: Same EndpointId always produces same IPv6
- **Fast**: O(1) operation, no cryptographic hashing
- **Collision-resistant**: 64-bit suffix space = 18 quintillion addresses
  - Birthday paradox: ~50% collision probability after 2^32 (~4 billion) nodes
  - Acceptable for local-only networks
- **Reversible**: Can lookup EndpointId from IPv6 via Registry cache

### Example Mapping

```
EndpointId (hex):
  01234567 89abcdef 01234567 89abcdef
  fedcba98 76543210 fedcba98 76543210
  ^^^^^^^^^^^^^^^^^ ^^^^^^^^^^^^^^^^^^^
  First 24 bytes    Last 8 bytes (used)
                    ↓
IPv6 Address:
  fd69:726f:0000:0000:fedc:ba98:7654:3210
  ^^^^^^^^^^^^^^^^^^^^ ^^^^^^^^^^^^^^^^^^^^
  Fixed prefix         Derived from EndpointId
```

## DNS Resolution

### Domain Format

`.iron` domains use base32 or hex encoding of EndpointIds:

```
<endpoint_id_encoded>.iron
```

**Example**:
```
abc123def456...xyz789.iron  →  fd69:726f::xxxx:xxxx:xxxx:xxxx
```

### DNS Server Configuration

- **Listen Address**: `127.0.0.1:5333` (non-standard port, no root required)
  - Configurable via `--dns-port` flag
- **Protocol**: UDP/TCP DNS (hickory-server)
- **Record Type**: AAAA (IPv6 only)
- **TTL**: 300s (5 minutes)
- **Auto-Configuration**: Automatically sets up system DNS on startup (macOS, Linux systemd)

### Query Flow

1. Client queries: `dig @127.0.0.1 -p 5333 <endpoint>.iron AAAA`
2. DNS server receives query
3. Parse EndpointId from domain name
4. Call `registry.get_or_assign_ip(endpoint_id)`
5. Return AAAA record with mapped IPv6
6. Cache in client's resolver

### Error Handling

- Non-.iron domains: `NXDOMAIN` or forward to upstream resolver
- Invalid EndpointId encoding: `SERVFAIL`
- Registry errors: `SERVFAIL` with logging

## TUN Interface

### Configuration

**Device Parameters**:
- **Type**: TUN (Layer 3, IP packets only)
- **Address**: `fd69:726f::1` (gateway address)
- **Netmask**: `/32` (route entire fd69:726f::/32 network)
- **MTU**: 1420 bytes (accounts for QUIC/UDP overhead)
- **Flags**: 
  - `IFF_TUN` (not TAP)
  - `IFF_NO_PI` on Linux (no packet info header)

**macOS-specific**:
- Uses `utun` devices (user-space TUN)
- No kernel extension required
- Requires root/sudo for device creation

**Linux-specific**:
- Uses `/dev/net/tun`
- Supports multi-queue (`IFF_MULTI_QUEUE`) for future optimization
- Requires `CAP_NET_ADMIN` capability

### Packet Flow

#### Inbound (OS → Network)

```
Application
    ↓ (sends to fd69:726f::xxxx)
OS Network Stack
    ↓ (routes to TUN device)
TUN Interface (iron)
    ↓ (read packet)
Parse IPv6 header (etherparse)
    ↓ (extract dest address)
Registry Lookup
    ↓ (dest IPv6 → EndpointId)
Iroh QUIC Connection
    ↓ (forward packet)
Remote Peer
```

#### Outbound (Network → OS)

```
Remote Peer
    ↓ (sends packet)
Iroh QUIC Connection
    ↓ (receive packet)
TUN Interface (iron)
    ↓ (write packet)
OS Network Stack
    ↓ (routes to application)
Application
```

### Packet Format

**IPv6 Header** (40 bytes minimum):
```
| Version (4) | Traffic Class (8) | Flow Label (20) |
| Payload Length (16) | Next Header (8) | Hop Limit (8) |
| Source Address (128 bits)                            |
| Destination Address (128 bits)                       |
```

**Supported Protocols**:
- ICMPv6 (ping6)
- TCP over IPv6
- UDP over IPv6

**Not Supported** (MVP):
- IPv4 (no dual-stack)
- IPv6 extension headers (future)
- Fragmentation (MTU sized to avoid)

## Iroh Transport

### ALPN Protocol

**Protocol ID**: `b"iron/packet/0"`

**Negotiation**:
```rust
endpoint.connect(peer_addr, b"iron/packet/0").await?;
```

### Connection Management

**Strategy**: One QUIC connection per remote EndpointId
- Persistent connections (no reconnect per packet)
- Bi-directional streams for packet forwarding
- Leverage iroh's NAT traversal and relay servers

**Connection Lifecycle**:
1. First packet to new EndpointId triggers connection
2. Connection maintained while traffic flows
3. Idle timeout (configurable, default 30s)
4. Reconnect on next packet if timed out

### Packet Encapsulation

**Over QUIC Streams**:

**Current Implementation**: Stream per packet with connection pooling
```rust
// Open bi-directional stream per packet
let (mut send, _recv) = conn.open_bi().await?;
send.write_all(packet).await?;
send.finish()?;
```

**Connection Pooling Optimization**:
- Maintains cache of QUIC connections per EndpointId (`DashMap<EndpointId, Connection>`)
- Reuses existing connections instead of repeated handshakes
- Automatically retries with new connection if cached connection is stale
- Significant performance improvement over naive per-packet connect

**Why Not Framed Streams?**
- Stream-per-packet is simpler and sufficient for current throughput
- Connection pooling eliminates most handshake overhead
- Can migrate to framed streams later if needed

**Performance Characteristics**:
- First packet to peer: Full QUIC handshake (~1-2 RTT)
- Subsequent packets: Reuse cached connection (~0 RTT for stream setup)
- Stale connection: Automatic retry with new connection

### NAT Traversal

Leverages iroh's built-in capabilities:
- **Direct connections** when possible (LAN, public IPs)
- **Hole punching** for peers behind NAT
- **Relay servers** as fallback (iroh's default relays)

No additional configuration needed in MVP.

## Performance Considerations

### Latency Budget

Typical packet flow (same host testing):
- TUN read: ~1-5μs
- IPv6 parse: ~500ns
- Registry lookup: ~100ns (DashMap)
- Iroh send: ~1-5μs (local processing)
- **Total (local)**: ~3-11μs

Network latency (real-world):
- LAN: 1-5ms
- Internet: 10-100ms
- With relay: 20-200ms

**Conclusion**: Local processing (~10μs) is negligible compared to network latency (~10-100ms).

### Throughput Estimates

**Single-threaded MVP**:
- ~50-100k packets/second
- ~50-600 Mbps (depending on packet size)
- Sufficient for personal VPN usage

**Future optimizations**:
- Pipeline architecture: ~200-500k pps
- Multi-queue (Linux): ~1M+ pps
- Multiple CPU cores: scales linearly

### Memory Usage

**Per peer**:
- Registry entry: ~48 bytes (EndpointId + IPv6 + overhead)
- QUIC connection: ~10-50 KB (buffers, state)

**Estimated**:
- 100 peers: ~1-5 MB
- 1000 peers: ~10-50 MB

Acceptable for desktop/server deployment.

## Security Considerations

### Threat Model

**In scope**:
- Authenticated connections (iroh's TLS-based QUIC)
- Encrypted transport (QUIC encryption)
- EndpointId verification (public key authentication)
- Source address spoofing prevention (address rewriting)

**Out of scope** (future):
- Traffic analysis resistance
- Onion routing (planned post-MVP)
- DDoS protection
- Rate limiting

### Trust Model

- Trust iroh's cryptographic implementation
- Trust peer's EndpointId (no PKI/certificate authority)
- Local-only registry (no trust in remote registries)
- **DNS is unauthenticated** - actual security comes from iroh's crypto

### Source Address Rewriting

**Security Feature**: Prevents IPv6 source address spoofing

**How it works**:
1. Peer sends packet with any source IPv6 address
2. Iron receives packet via authenticated QUIC connection (knows sender's EndpointId)
3. Iron **rewrites** source IPv6 to sender's derived address
4. OS receives packet with correct, verified source address

**Why it's secure**:
- Iroh provides cryptographic authentication via EndpointId
- Peer cannot fake another peer's EndpointId (would need private key)
- We trust iroh's authentication, not the packet's claimed source
- OS sees consistent source addresses for routing return packets

**Implementation** (see `src/protocol.rs:292-328`):
```rust
fn rewrite_source_address(packet: Vec<u8>, sender_id: &EndpointId) -> Result<Vec<u8>> {
    let (mut header, payload) = Ipv6Header::from_slice(&packet)?;
    let sender_ipv6 = registry.get_or_assign_ip(*sender_id);
    header.source = sender_ipv6.octets(); // Rewrite to verified address
    // Rebuild packet...
}
```

This is more secure than trusting the source address in the packet itself.

### Attack Vectors

**Potential attacks**:
1. **IPv6 collision**: Attacker generates EndpointId with same IPv6 suffix
   - Probability: ~2^-64 for random collision
   - Mitigation: Check registry before accepting connection, source rewriting prevents spoofing
   
2. **DNS spoofing**: Attacker intercepts DNS queries
   - Mitigation: Use 127.0.0.1 DNS server (no network exposure)
   - Note: DNS only maps names to IPs - actual security is in iroh's crypto
   - Future: DNSSEC for external DNS integration

3. **Source address spoofing**: Peer sends packet with fake source IPv6
   - Mitigation: **Source address rewriting** (implemented)
   - We rewrite source to verified EndpointId-derived address
   - OS always sees correct source for return packets
   
4. **TUN device hijacking**: Attacker creates conflicting TUN device
   - Mitigation: Root privileges required, OS-level protection
   
5. **Connection hijacking**: Standard QUIC/TLS protections apply
   - iroh handles this with endpoint authentication

## Testing Strategy

### Unit Tests

**Registry** (Phase 2):
- Deterministic derivation
- Bi-directional lookup consistency
- Concurrent access safety
- Collision detection

### Integration Tests

**DNS** (Phase 3):
```bash
# Test AAAA query
dig @127.0.0.1 -p 5333 <endpoint>.iron AAAA

# Verify response
# Expected: fd69:726f::xxxx:xxxx:xxxx:xxxx
```

**TUN** (Phase 4):
```bash
# Test ping
ping6 -c 3 fd69:726f::xxxx:xxxx:xxxx:xxxx

# Expected: 3 packets transmitted, 3 received, 0% packet loss
```

**End-to-End** (Phase 5):
```bash
# Terminal 1: Start node A
sudo ./iron

# Terminal 2: Start node B
sudo ./iron

# Terminal 3: Ping from A to B
ping6 fd69:726f::<B's IPv6>
```

### Performance Tests

**Throughput**:
```bash
# Use iperf3 over iron network
iperf3 -s -B fd69:726f::1  # Server
iperf3 -c fd69:726f::<peer> # Client
```

**Latency**:
```bash
# Measure RTT
ping6 -i 0.2 fd69:726f::<peer> | tee latency.log
```

## Future Enhancements

### Short-term
- DNS caching
- Connection pooling
- Metrics and monitoring
- Configuration file support

### Medium-term
- Pipeline architecture for TUN
- Multi-queue support (Linux)
- IPv4 tunneling
- Custom relay servers

### Long-term
- Onion routing integration
- Mobile platform support (Android/iOS)
- Mesh network routing
- DHT-based peer discovery
