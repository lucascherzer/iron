# Implementation Plan - iron

## Status Summary
- ✅ Phase 1: Complete - Foundation & scaffolding established
- ✅ Phase 2: Complete - Registry implementation with full test coverage
- ✅ Phase 3: Complete - DNS Resolver with base32 encoding
- ✅ Phase 4: Complete - TUN interface with IPv6 packet handling
- ✅ Phase 5: Complete - Iroh integration with packet transport protocol
- ✅ Integration Tests: Complete - 10 comprehensive integration tests
- ⏳ Phase 6: Not Started - CLI & Orchestration

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

## Phase 3: DNS Resolver ✅ COMPLETE

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
<endpoint_id_base32>.iron  (52 chars, fits in single label)
```
- EndpointId encoded in base32 (no padding) for DNS compatibility
- Base32: 52 chars fits in one label (e.g., `DF7WWI7BNSCTFRVLZA4PVTK6U6E34DDWWKJAGNADTP5IWPJWRVQQ.iron`)
- Case-insensitive encoding avoids user errors
- Example resolved: `fd69:726f::xxxx:xxxx:xxxx:xxxx`

### Implementation Tasks

- ✅ **3.1** Create DNS authority for `.iron` TLD
  - Implemented `IronDnsHandler` with `RequestHandler` trait
- ✅ **3.2** Implement request handler
  - Parse incoming DNS query ✅
  - Check if domain ends with `.iron` ✅
  - Extract EndpointId from domain name (supports multi-label hex) ✅
  - Call `registry.get_or_assign_ip()` ✅
  - Construct AAAA response record ✅
- ✅ **3.3** Implement DnsResolver::new()
  - Accept `Arc<Registry>` for shared state ✅
  - Configure hickory-server listener ✅
- ✅ **3.4** Implement DnsResolver::run()
  - Start hickory-server async task ✅
  - Listen for DNS queries ✅
  - Handle graceful shutdown ✅
- ✅ **3.5** Write unit tests following AGENTS.md pattern
  - 5 comprehensive tests implemented ✅
  - Test DNS resolver construction ✅
  - Test base32 encoding/decoding (single label) ✅
  - Test base32 uppercase handling ✅
  - Test invalid domain handling (multi-label rejection) ✅
  - Test AAAA query handling ✅
- ⏸️ **3.6** Manual test with `dig` command (optional)
  - Would require running server: `dig @127.0.0.1 -p 5333 <endpoint>.iron AAAA`
  - Skipped for now, can be done during integration testing

**Success Criteria**: ✅ ALL MET
- ✅ DNS server implementation complete
- ✅ All unit tests pass (5/5 tests passing)
- ✅ Uses base32 encoding (52 chars, single label)
- ✅ `.iron` queries return correct IPv6 addresses
- ✅ Non-`.iron` queries return NXDOMAIN
- ✅ Non-AAAA queries return empty response
- ✅ Multi-label domains rejected (invalid format)
- ✅ Code formatted with `cargo fmt`

**Key Implementation Notes**:
- Base32 encoding (52 chars) fits in single DNS label (63-char limit)
- Case-insensitive: accepts uppercase/lowercase, stores lowercase
- Simplified parsing: single label before `.iron` (no concatenation needed)
- Uses `data_encoding::BASE32_NOPAD` for consistent encoding

---

## Phase 4: TUN Interface ✅ COMPLETE

### Overview
Setup TUN device for packet interception and forwarding.

### Design Specifications (MVP)

**Architecture**: Single-threaded async loop with tokio::select!
```rust
pub async fn run(mut self) -> Result<()> {
    let device = Self::create_device()?;
    let mut framed = device.into_framed();
    
    loop {
        tokio::select! {
            Some(packet) = framed.next() => {
                self.handle_inbound_packet(&packet?).await?;
            }
            Some(packet) = self.outbound_rx.recv() => {
                framed.send(packet.into()).await?;
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
   - Forward packet over iroh connection to peer (Phase 5 integration point)
   
2. **Outbound (Network → OS)**:
   - Receive packet from iroh connection via channel
   - Write to TUN device
   - OS routes to application

**TUN Configuration**:
- IPv6 only (Layer3)
- Link-local IPv4: `169.254.0.1` (required but unused)
- MTU: 1420 bytes (WireGuard standard, accounts for QUIC overhead)
- Platform-specific naming: `utun` (macOS), `iron0` (Linux)

### Implementation Tasks

- ✅ **4.1** Implement TUN device creation
  - Platform-specific configuration (macOS utun, Linux iron0) ✅
  - Configure Layer3, MTU 1420 ✅
  - Requires root/sudo privileges (documented) ✅
- ✅ **4.2** Implement packet reader loop
  - Use `AsyncDevice::into_framed()` for clean stream API ✅
  - Parse IPv6 headers with `etherparse` ✅
  - Extract destination address ✅
- ✅ **4.3** Implement `handle_inbound_packet()`
  - Lookup destination EndpointId in registry ✅
  - Graceful error handling (log warnings, don't crash) ✅
  - Phase 5 integration point marked with TODO ✅
- ✅ **4.4** Implement outbound packet writer
  - Receive packets via `mpsc::unbounded_channel` ✅
  - Write to TUN device in tokio::select! loop ✅
  - Proper error handling ✅
- ✅ **4.5** Write unit tests following AGENTS.md pattern
  - 4 comprehensive tests implemented ✅
  - Test TUN interface construction ✅
  - Test IPv6 packet handling with valid destination ✅
  - Test IPv6 packet handling with unknown destination ✅
  - Test invalid packet handling ✅
- ⏸️ **4.6** Manual test with ping (deferred to integration)
  - Would require root: `sudo ping6 fd69:726f::xxxx:xxxx:xxxx:xxxx`
  - Skipped for now, will test during full integration

**Future Optimization (Phase 4.1)**:
- Pipeline architecture (reader → processor pool → writer)
- Configurable worker count
- Better CPU utilization for high-throughput scenarios

**Success Criteria**: ✅ ALL MET
- ✅ TUN device creation implementation complete
- ✅ IPv6 packets parsed and destination extracted
- ✅ Destination addresses resolved to EndpointIds via registry
- ✅ Bidirectional packet flow (inbound + outbound via channel)
- ✅ All unit tests pass (4/4 tests passing)
- ✅ Code formatted with `cargo fmt`
- ✅ Integration point with Phase 5 clearly marked

**Key Implementation Notes**:
- Uses `mpsc::unbounded_channel` for outbound packets (from network to OS)
- TUN interface consumes itself in `run()` to take ownership of channel receiver
- Platform-specific device naming via `tun_name()` (macOS: utun, Linux: iron0)
- Requires root/sudo privileges to create TUN device
- IPv6 packet parsing with `etherparse::Ipv6Header::from_slice()`
- Graceful degradation: unknown destinations logged but don't crash the interface

---

## Integration Tests ✅ COMPLETE

### Overview
Comprehensive integration tests that verify all components work together correctly without requiring root privileges or actual network devices.

### Test Coverage

**File**: `tests/integration.rs` (10 tests, all passing)

1. ✅ **test_registry_consistency_across_components**
   - Verifies Registry provides consistent mappings across DNS and TUN
   - Tests forward lookup (EndpointId → IPv6)
   - Tests reverse lookup (IPv6 → EndpointId)
   - Confirms deterministic derivation across multiple Registry instances

2. ✅ **test_dns_resolution_base32_encoding**
   - Tests single-label DNS domain parsing
   - Verifies base32 encoding/decoding (52 chars in one label)
   - Confirms EndpointId reconstruction from domain
   - Validates Registry returns correct IPv6 for decoded EndpointId

3. ✅ **test_tun_os_to_network_packet_flow**
   - Tests TUN packet processing (OS → Network direction)
   - Verifies IPv6 packet parsing
   - Confirms destination lookup in Registry
   - Validates packet sent to `to_network_tx` channel with correct EndpointId

4. ✅ **test_two_node_setup**
   - Simulates two independent nodes with separate registries
   - Confirms deterministic mapping (both nodes agree on IPv6 for same EndpointId)
   - Verifies different nodes get different IPv6 addresses

5. ✅ **test_simulated_packet_flow_node_a_to_b**
   - End-to-end simulation: Node A sends packet to Node B
   - Node A does DNS lookup (registers endpoint_b in registry)
   - Node A's TUN processes packet
   - Verifies correct EndpointId extracted and sent to channel
   - Simulates Node B receiving packet via channel

6. ✅ **test_packet_to_unregistered_destination**
   - Tests graceful handling of unknown destinations
   - Verifies no packet sent for unregistered IPv6
   - Confirms no crashes or errors (logs warning)

7. ✅ **test_dns_resolver_construction**
   - Verifies DnsResolver constructs correctly
   - Basic sanity check for DNS component

8. ✅ **test_ipv6_prefix_consistency**
   - Confirms all derived IPv6 addresses use `fd69:726f::/32` prefix
   - Validates ULA space compliance

9. ✅ **test_concurrent_packet_processing**
   - Tests 10 concurrent packet processing tasks
   - Verifies thread-safety of Registry (DashMap)
   - Confirms all packets processed correctly

10. ✅ **test_tun_interface_public_api**
    - Verifies public API accessibility
    - Ensures TUN interface can be constructed without panicking

### Key Testing Approach

**No Root Required**: Tests use `handle_os_to_network()` directly instead of creating actual TUN devices.

**Channel-Based Verification**: Tests verify behavior by checking messages sent to channels, simulating iroh integration.

**Deterministic Testing**: All tests use fixed seed values for EndpointIds, ensuring reproducible results.

**Two-Node Simulation**: Tests verify cross-node consistency (both nodes independently derive same IPv6 for same peer).

### Success Criteria: ✅ ALL MET
- ✅ All 10 integration tests pass
- ✅ Tests verify DNS, Registry, and TUN work together
- ✅ Two-node communication flow validated
- ✅ Concurrent access tested (10 threads)
- ✅ No root privileges required for testing
- ✅ Graceful error handling verified

### Testing Without Actual Network

These tests validate the logic without requiring:
- Root/sudo privileges (no actual TUN device creation)
- Network connectivity (channels simulate iroh)
- Running DNS server (tests DNS parsing logic directly)

**Ready for Phase 5**: Integration tests provide confidence that components will work together when iroh is integrated.

---

## Phase 5: Iroh Integration ✅ COMPLETE

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
- Raw packet data sent directly over QUIC stream
- Stream-per-packet approach for simplicity
- Source address verification on receive

### Implementation Tasks

- ✅ **5.1** Initialize iroh Endpoint in `IronNode::new()`
  - Configured ALPN protocol `iron/packet/0` ✅
  - Endpoint initialized with default secret key ✅
  - Started endpoint listening ✅
- ✅ **5.2** Implement connection establishment
  - Accept incoming connections with `iron/packet/0` ALPN ✅
  - Create connection handler task per peer ✅
  - Implemented in `IronProtocol::accept_loop()` ✅
- ✅ **5.3** Implement packet forwarding
  - TUN → Iroh: Send packets over QUIC stream ✅
  - Iroh → TUN: Receive packets from stream, write to TUN ✅
  - Implemented in `IronProtocol::send_packet()` and `handle_connection()` ✅
- ✅ **5.4** Integrate with TUN interface
  - Connected TUN's `to_network_tx` to iroh send loop ✅
  - Connected iroh receive to TUN's `from_network_tx` ✅
  - Channel architecture properly implemented ✅
- ⏸️ **5.5** Test end-to-end connectivity
  - Requires actual TUN device (root/sudo) ⏸️
  - Deferred to Phase 6 manual testing ⏸️

**Success Criteria**: ✅ IMPLEMENTATION COMPLETE
- ✅ Iroh endpoint starts successfully
- ✅ Accept loop handles incoming connections
- ✅ Send loop processes outbound packets
- ✅ Packets flow bidirectionally through channels
- ✅ Source address verification implemented
- ✅ All unit tests still passing (30/30)
- ✅ Code formatted with `cargo fmt`
- ⏸️ End-to-end ping testing (requires Phase 6 CLI)

**Key Implementation Notes**:
- **File**: `src/protocol.rs` (new module, 236 lines)
- ALPN constant: `iron/packet/0`
- Two concurrent tasks: send loop and accept loop
- Source address verification prevents IP spoofing
- Graceful error handling (warnings, not crashes)
- Uses `Connection::open_bi()` for sending
- Uses `Connection::accept_bi()` for receiving
- Each connection handled in separate tokio task
- Maximum packet size: 1500 bytes (MTU)

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
