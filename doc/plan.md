# Implementation Plan - iron

## Status Summary
- ✅ Phase 1: Complete - Foundation & scaffolding established
- ✅ Phase 2: Complete - Registry implementation with full test coverage
- ⏳ Phase 3: Not Started - DNS Resolver
- ⏳ Phase 4-6: Not Started

## Phase 1: Foundation & Scaffolding ✅ COMPLETE
- ✅ Initialize `Cargo.toml` with dependencies
  - Updated: `iroh` (0.95.1), `tun` (0.8.5), `hickory-server` (0.25)
  - `tokio` (1.x with full features), `dashmap` (6), `etherparse` (0.19)
- ✅ Define the core module structure (`mapping`, `dns`, `tun`, `node`)
- ✅ Implement skeleton structs and method signatures with `todo!()`
- ✅ Project builds successfully (all dependency issues resolved)

**Key Achievements**:
- Migrated from `trust-dns-server` to `hickory-server` (actively maintained)
- Fixed netwatch/socket2 compatibility issues by upgrading iroh
- Updated EndpointId terminology (was NodeId)
- Established Rust 2024 edition compatibility

---

## Phase 2: Address Mapping (The Registry) ✅ COMPLETE

### Overview
Implement bidirectional mapping store (`EndpointId <-> Ipv6Addr`) with deterministic IPv6 derivation.

### Design Specifications

**IPv6 ULA Prefix**: `fd69:726f::/32` (iron-branded)
- Encodes "iron" in hex: `0x69` = 'i', `0x72` = 'r', `0x6f` = 'o'
- Unlikely to conflict with other ULA networks
- Memorable and project-specific

**Derivation Algorithm**: Direct hash from EndpointId
```rust
fn derive_ip(endpoint_id: &EndpointId) -> Ipv6Addr {
    let bytes = endpoint_id.as_bytes(); // 32 bytes
    let suffix = &bytes[24..32];        // Last 8 bytes (64 bits)
    
    Ipv6Addr::new(
        0xfd69, 0x726f, 0x0000, 0x0000,  // Fixed prefix
        u16::from_be_bytes([suffix[0], suffix[1]]),
        u16::from_be_bytes([suffix[2], suffix[3]]),
        u16::from_be_bytes([suffix[4], suffix[5]]),
        u16::from_be_bytes([suffix[6], suffix[7]]),
    )
}
```

**Properties**:
- Deterministic: Same EndpointId always produces same IPv6
- Fast: O(1) derivation, no cryptographic hashing needed
- Collision-resistant: 64-bit space, acceptable for local networks
- Bidirectional: DashMap provides O(1) reverse lookup

### Implementation Tasks

- ✅ **2.1** Implement `Registry::new()` with empty DashMaps
- ✅ **2.2** Implement `derive_ip()` helper function
  - Extract last 8 bytes from EndpointId
  - Construct IPv6 with `fd69:726f::/32` prefix
- ✅ **2.3** Implement `get_or_assign_ip()`
  - Check if EndpointId exists in cache
  - If not, derive IPv6 and insert into both maps
  - Return IPv6 address
- ✅ **2.4** Implement `get_endpoint_id()`
  - Lookup IPv6 in reverse map
  - Return `Option<EndpointId>`
- ✅ **2.5** Write unit tests following AGENTS.md pattern
  - 11 comprehensive tests implemented
  - Test deterministic derivation ✅
  - Test bidirectional lookup consistency ✅
  - Test concurrent access (DashMap thread-safety) ✅
  - Test with 1000+ endpoints ✅
  - Test array pattern as per AGENTS.md ✅
- ✅ **2.6** Add documentation comments to all public methods
- ✅ **2.7** Run `cargo fmt` before committing

**Success Criteria**: ✅ ALL MET
- ✅ All tests pass (11/11 tests passing)
- ✅ `cargo test` succeeds
- ✅ Registry can map 1000+ EndpointIds without issues (tested)
- ✅ Forward and reverse lookups are consistent (tested)
- ✅ Concurrent access safe (tested with 10 threads)

---

## Phase 3: DNS Resolver ⏳ NOT STARTED

### Overview
Implement DNS server using `hickory-server` to resolve `.iron` domains.

### Design Specifications

**Query Handling**:
- Listen on `127.0.0.1:5333` (nonstandard port to avoid root)
- Handle AAAA queries for `<endpoint_id>.iron` domains
- Parse EndpointId from domain name
- Use Registry to get/generate IPv6 address
- Return AAAA record with mapped address

**Domain Format**:
```
<endpoint_id_base32>.iron
```
- EndpointId encoded in base32 (or hex) for DNS compatibility
- Example: `abc123def456...xyz.iron` → `fd69:726f::xxxx:xxxx:xxxx:xxxx`

### Implementation Tasks

- [ ] **3.1** Create DNS authority for `.iron` TLD
- [ ] **3.2** Implement request handler
  - Parse incoming DNS query
  - Check if domain ends with `.iron`
  - Extract EndpointId from domain name
  - Call `registry.get_or_assign_ip()`
  - Construct AAAA response record
- [ ] **3.3** Implement DnsResolver::new()
  - Accept `Arc<Registry>` for shared state
  - Configure hickory-server listener
- [ ] **3.4** Implement DnsResolver::run()
  - Start hickory-server async task
  - Listen for DNS queries
  - Handle graceful shutdown
- [ ] **3.5** Test with `dig` command
  - Manual testing: `dig @127.0.0.1 -p 5333 <endpoint>.iron AAAA`
  - Verify correct IPv6 returned
  - Test multiple queries return consistent results

**Success Criteria**:
- DNS server starts and listens on configured port
- `.iron` queries return correct IPv6 addresses
- Non-`.iron` queries are rejected or forwarded
- Server handles concurrent queries correctly

---

## Phase 4: TUN Interface ⏳ NOT STARTED

### Overview
Setup TUN device for packet interception and forwarding.

### Design Specifications (MVP)

**Architecture**: Single-threaded async loop
```rust
pub async fn run(&self) -> Result<()> {
    let dev = tun::create_as_async(&config)?;
    let mut framed = dev.into_framed();
    
    loop {
        tokio::select! {
            Some(packet) = framed.next() => {
                self.handle_packet(packet?).await?;
            }
        }
    }
}
```

**Packet Flow**:
1. **Inbound (OS → Network)**:
   - Read IPv6 packet from TUN device
   - Parse destination IPv6 address
   - Lookup EndpointId via `registry.get_endpoint_id()`
   - Forward packet over iroh connection to peer
   
2. **Outbound (Network → OS)**:
   - Receive packet from iroh connection
   - Parse/reconstruct IPv6 packet
   - Write to TUN device
   - OS routes to application

**TUN Configuration**:
- IPv6 only (no IPv4 support in MVP)
- Address: `fd69:726f::1` (gateway address)
- Route: `fd69:726f::/32` (entire iron network)
- MTU: 1420 bytes (WireGuard standard, accounts for QUIC overhead)

### Implementation Tasks

- [ ] **4.1** Implement TUN device creation
  - Platform-specific configuration (macOS utun)
  - Set IPv6 address and routing
  - Requires root/sudo privileges
- [ ] **4.2** Implement packet reader loop
  - Use `AsyncDevice::into_framed()` for clean stream API
  - Parse IPv6 headers with `etherparse`
  - Extract destination address
- [ ] **4.3** Implement `handle_packet()`
  - Lookup destination EndpointId in registry
  - Forward to iroh connection (Phase 5 integration)
  - Handle errors gracefully
- [ ] **4.4** Implement outbound packet writer
  - Receive packets from iroh
  - Write to TUN device
  - Handle backpressure
- [ ] **4.5** Test with ping
  - Manual test: `ping6 fd69:726f::xxxx:xxxx:xxxx:xxxx`
  - Verify packets are received by TUN interface
  - Verify packet forwarding to iroh (Phase 5)

**Future Optimization (Phase 4.1)**:
- Pipeline architecture (reader → processor pool → writer)
- Configurable worker count
- Better CPU utilization for high-throughput scenarios

**Success Criteria**:
- TUN device created successfully
- IPv6 packets intercepted from OS
- Destination addresses resolved to EndpointIds
- Packets forwarded to iroh (integration with Phase 5)

---

## Phase 5: Iroh Integration ⏳ NOT STARTED

### Overview
Initialize iroh `Endpoint` and implement packet transport protocol.

### Design Specifications

**ALPN Protocol**: `b"iron/packet/0"`
- Version 0 for initial implementation
- Identifies iron packet traffic on QUIC connections

**Connection Management**:
- One QUIC connection per remote EndpointId
- Use bi-directional streams for packet forwarding
- Leverage iroh's NAT traversal and relay servers

**Packet Format** (over QUIC streams):
- Simple framing: `[length: u16][packet_data: bytes]`
- Or: Use raw packet data (stream-per-packet)

### Implementation Tasks

- [ ] **5.1** Initialize iroh Endpoint in `IronNode::new()`
  - Configure ALPN for iron traffic
  - Generate or load secret key
  - Start endpoint listening
- [ ] **5.2** Implement connection establishment
  - Accept incoming connections with `iron/packet/0` ALPN
  - Create connection handler task per peer
- [ ] **5.3** Implement packet forwarding
  - TUN → Iroh: Send packets over QUIC stream
  - Iroh → TUN: Receive packets from stream, write to TUN
- [ ] **5.4** Integrate with TUN interface
  - Connect TUN's `handle_packet()` to iroh send
  - Connect iroh receive to TUN write
- [ ] **5.5** Test end-to-end connectivity
  - Start two iron nodes on localhost
  - Ping from node A to node B
  - Verify packet round-trip

**Success Criteria**:
- Iroh endpoint starts successfully
- Connections established between peers
- Packets flow bidirectionally through QUIC
- End-to-end ping works between two iron nodes

---

## Phase 6: CLI & Orchestration ⏳ NOT STARTED

### Overview
Create main entry point with lifecycle management and configuration.

### Implementation Tasks

- [ ] **6.1** Enhance `IronNode::new()`
  - Load configuration (or use defaults)
  - Initialize all components in correct order
  - Share Registry via `Arc<Registry>`
- [ ] **6.2** Implement `IronNode::start()`
  - Spawn DNS resolver task
  - Spawn TUN interface task
  - Start iroh endpoint
  - Wait for shutdown signal
- [ ] **6.3** Implement graceful shutdown
  - Listen for SIGINT/SIGTERM (tokio signal feature)
  - Stop all components cleanly
  - Close connections
  - Clean up TUN device
- [ ] **6.4** Add basic logging
  - Log startup sequence
  - Log EndpointId and connection info
  - Log errors and warnings
- [ ] **6.5** Add command-line interface (optional)
  - Options: `--dns-port`, `--relay-url`, etc.
  - Help text and usage information
- [ ] **6.6** Create README.md with usage instructions
  - Installation
  - Running iron
  - Connecting two nodes
  - Troubleshooting

**Success Criteria**:
- `iron` binary can be run from command line
- All components start correctly
- Ctrl-C shuts down gracefully
- User documentation is clear and complete

---

## Development Notes

### Current Status (as of Phase 1 completion)
- All dependencies updated to latest compatible versions
- Project builds successfully on Rust 1.91.1
- Edition 2024 configured
- Module structure established

### Dependencies
```toml
iroh = "0.95.1"           # P2P QUIC networking
tun = "0.8.5"              # TUN device creation
tokio = "1"                # Async runtime (full features)
dashmap = "6"              # Concurrent HashMap
hickory-server = "0.25"    # DNS server
anyhow = "1.0"             # Error handling
thiserror = "2.0"          # Custom error types
etherparse = "0.19"        # Packet parsing
tracing = "0.1"            # Logging
tracing-subscriber = "0.3" # Log formatting
```

### Testing Strategy
- **Phase 2**: Unit tests with test case arrays (AGENTS.md pattern)
- **Phase 3-6**: Manual testing with two local nodes
- **Future**: Automated integration tests

### Platform Development Priority
1. macOS (primary development environment)
2. Linux (performance optimizations available)
3. Windows (tertiary, requires wintun driver)

---

## Future Enhancements (Post-MVP)

### Performance Optimizations
- TUN pipeline architecture (parallel packet processing)
- Linux multi-queue TUN support
- Buffer pooling with `bytes::BytesMut`
- Zero-copy optimizations

### Features
- Configuration file support
- Multiple relay server support
- Peer discovery mechanisms
- Connection statistics and monitoring
- IPv4 tunneling support (optional)
- Onion routing integration (original vision)

### Platform Support
- Native Windows support with wintun
- Android/iOS mobile support (long-term)
- Embedded systems (if no-std becomes requirement)

### Security
- Connection authentication
- Traffic encryption verification
- Rate limiting
- DDoS protection
