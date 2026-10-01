# iron architecture planning

## Introduction

`iron` is a project that aims to provide a "flat" network based on iroh's
dial-by-public-key approach.
We do this because it has become burdensome for end users to communicate in a
peer-to-peer fashion, as workarounds like NAT in combination with public
infrastructure like DNS and CAs place hard restrictions on users.

[iroh](https://www.iroh.computer/) already solves a lot of these problems by
addressing the routing issue, but it is so far only accessible as a library for
application developers. 

`iron` extends iroh's sphere of influence to the operating system level, like
`i2p` does, by providing a network interface and a resolver to route addresses
natively under the `.iron` TLD. 

The name `iron` is a shorthand for "iroh-onion" as this project is intended to
also support onion routing, which has been implemented in a separate crate.
We leave it out during the initial prototype, but plan to add support later. It 
also makes for a good TLD.

# Components

We require the following components for this to work:
1. A `.iron` resolver which can map `<endpoint_id>.iron` to a deterministic IPv6 address
  in the Unique Local Address (ULA) space. The mapping is deterministic based on
  the EndpointId to ensure consistency. For close integration with existing software,
  it needs to be able to resolve `.iron` DNS queries.
2. A platform TUN backend that facilitates communication to the outside network,
  advertising to route addresses within the ULA address spaces. It uses the
  resolver in reverse, taking the IPv6 address and getting its associated iroh
  EndpointId. And sending the data off.
3. A key management system that persists the node's private key across restarts,
  ensuring consistent EndpointId (using the injected `StatePaths`).
4. A DNS auto-configuration system that sets up system-level DNS resolution for
  `.iron` domains on supported desktop platforms
  (`iron_desktop::dns_config`).
5. A CLI interface providing utilities for node management, key operations, and
  format conversions.

# Packet Flow Architecture

## Overview
The platform TUN backend supplies **bidirectional** packet flow between the OS
and the portable `PacketRouter`:

### OS → Network (Outbound to Peers)
When an OS application wants to communicate with a peer:

1. **Application sends data** to destination IPv6 `fd69:726f::xxxx:xxxx:xxxx:xxxx`
2. **OS routes packet to TUN device** (because we advertise routes for `fd69:726f::/32`)
3. **TunIo reads packet from device**
4. **Parse IPv6 header** to extract destination address
5. **Registry lookup**: Destination IPv6 → EndpointId
6. **Send to iroh** via channel: `(EndpointId, packet_bytes)`
7. **Iroh transmits** packet to peer over QUIC

### Network → OS (Inbound from Peers)
When a peer sends data to us:

1. **Iroh receives packet** from peer (iroh knows sender's EndpointId)
2. **Registry lookup**: Sender EndpointId → Source IPv6
3. **Packet already has correct headers** (peer constructed it properly)
4. **Send to PacketRouter** via channel: `packet_bytes`
5. **TunIo writes packet to device**
6. **OS routes to listening application** based on destination IPv6

## Key Insight
**We do NOT read from network hardware** - instead, we actively poll iroh's bidirectional
endpoint for incoming packets. Iroh handles all the network complexity (NAT traversal,
relay coordination, QUIC connections). We simply:
- Read packets FROM `TunIo` (OS wants to send)
- Write packets TO `TunIo` (peer sent to us)

## Channel Architecture
```rust
// OS → Network
let (to_network_tx, to_network_rx) = mpsc::unbounded_channel::<(EndpointId, Vec<u8>)>();

// Network → OS  
let (from_network_tx, from_network_rx) = mpsc::unbounded_channel::<Vec<u8>>();

// Portable packet router; platform code supplies TunIo when it runs.
PacketRouter::new(registry, to_network_tx, from_network_rx);

// Iroh integration (Phase 5)
IronProtocol::new(registry, to_network_rx, from_network_tx);
```

# IPv6 Address Space

**ULA Prefix**: `fd69:726f::/32` (iron-branded)
- `0xfd69` = First 16 bits (ULA marker + 'i')
- `0x726f` = Second 16 bits ('r' + 'o')
- Remaining 96 bits derived from EndpointId

**Derivation Strategy**: Direct hash from EndpointId
- EndpointId is 32 bytes (256 bits)
- Take last 8 bytes (64 bits) for IPv6 suffix
- Provides deterministic, collision-resistant mapping for local-only networks
- Fast O(1) derivation without additional hashing overhead

Example:
```
EndpointId: <32-byte public key>
IPv6: fd69:726f:0000:0000:xxxx:xxxx:xxxx:xxxx
       ^^^^^^^^^^^^^^^^^^^^^ ^^^^^^^^^^^^^^^^^^^
       ULA prefix (fixed)    Derived from EndpointId
```

# Third Party Software
- iroh 0.95.1 (https://docs.rs/iroh/0.95.1/iroh/)
- tun 0.8.5 (https://docs.rs/tun/0.8.5/tun)
- hickory-proto 0.25 (DNS message parsing; the UDP server is a small tokio loop in `dns.rs`)
- tokio 1.x (async runtime with full features)
- dashmap 6 (concurrent hash maps for Registry)
- etherparse 0.19 (IPv6 packet parsing)

# Architecture Decisions

## Process Model
**Decision**: Single process with multiple tokio tasks

**Rationale**:
- Simpler architecture for MVP
- Shared `Arc<Registry>` between DNS and packet-router components
- No IPC overhead
- Easier debugging and state management
- Can migrate to multi-process later if needed

**Task Structure**:
- Main task: Orchestration and lifecycle management
- DNS task: Hickory-server DNS resolver
- Packet-router task: Packet processing loop
- Iroh task: Connection management (integrated into IronNode)

## Platform Support
**Development Priority**: macOS-first, then Linux, then Windows

**Rationale**:
- Primary development environment is macOS
- Establish correctness baseline on one platform
- Add Linux support (with multi-queue TUN optimization)
- Windows support as tertiary target
- Architecture allows platform-specific optimizations via conditional compilation

The platform layer keeps OS concerns out of the portable engine. `TunBackend`
provisions a device and returns raw packet I/O as `TunIo`; `PacketRouter`
handles packets without knowing whether that I/O came from a desktop TUN
device or a file descriptor supplied by a future Android `VpnService`.
`StatePaths` injects persistent storage, and desktop DNS system configuration
lives in `crates/iron-desktop/src/dns_config/`, separate from the portable
`DnsResolver` (message-level; shared by the UDP frontend and DNS-over-TUN).

## Packet Router Architecture
**MVP Approach**: Single-threaded async loop

**Implementation**:
```rust
let tun_io = backend.open(node_ipv6)?;
// PacketRouter consumes raw I/O supplied by a TunBackend.
router.run(tun_io).await?;
```

**Future Optimization Path**: Pipeline architecture (reader → processor pool → writer)
- Designed to allow upgrade without major refactoring
- Separate I/O from processing
- Configurable worker pool for parallel packet processing
- Can be implemented in Phase 4.1 if performance requires

**Latency Considerations**:
- Network latency (1-50ms) dominates total time
- Async context switching adds ~1-5μs (negligible compared to network)
- Local-only deployment means collision risk is acceptably low

## Iroh Integration
**ALPN Protocol**: `b"iron/packet/0"`
- Version 0 for initial implementation
- Can add versioned protocols later (e.g., `b"iron/packet/1"`)

**Connection Strategy**:
- One QUIC connection per remote EndpointId
- Bi-directional streams for packet forwarding
- Leverage iroh's built-in NAT traversal and relay support
- **Connection pooling**: Cached connections reused to avoid repeated handshakes

**Security Features**:
- Source address rewriting: Packets have source IPv6 rewritten to match authenticated sender
- Prevents source address spoofing by trusting iroh's crypto instead of packet headers

**Key Persistence**:
- Private keys stored below the injected `StatePaths` directory (0600 permissions)
- Automatically generated on first run
- Ensures consistent EndpointId across restarts
- Ownership auto-fixed when run with sudo (prevents root-owned files in user directory)

## Testing Strategy
**Initial Approach**: Manual testing with two local iron nodes
- Start two instances on localhost
- Test DNS resolution: `dig @localhost -p 5333 <endpoint_id>.iron AAAA`
- Test connectivity: `ping6 fd69:726f::...`
- Simple, fast iteration during development

**Current**: Automated unit and integration tests
- 30+ unit tests across all components
- Integration tests for packet flow
- CLI utilities for testing (`iron resolve`, `iron convert`)

**NixOS VM tests** (`tests/vm/`, Linux-only `nix flake check` outputs
`iron-vm-*`): three VMs on a virtual LAN, an `infra` VM running
`iroh-relay` and `iroh-dns-server` (so no internet is needed) and two iron
nodes running `nixosModules.iron`. The harness and its conventions are
documented in `tests/vm/lib.nix`. Link faults are injected with `tc netem` on
the VM LAN interface (`eth1`), below iroh:
- `two-node`: `.iron` name resolution and TCP in both directions
- `lossy-network`: a 10 MiB TCP transfer stays intact with 5% loss per direction
- `outage`: an idle TCP connection survives a 45s blackout (longer than QUIC's
  30s idle timeout) and carries on afterwards

Run one with `nix build .#checks.x86_64-linux.iron-vm-two-node -L`; for an
interactive session use its `.driverInteractive` attribute.

## DNS Auto-Configuration
**Supported Platforms**:
- macOS: Uses `/etc/resolver/iron` (domain-specific DNS)
- Linux (systemd-resolved): Uses `/etc/systemd/resolved.conf.d/iron.conf`

**Features**:
- Automatic detection of platform and DNS system
- Sets up DNS on daemon startup (`sudo iron`)
- Only routes `.iron` domains to iron DNS server
- Coexists with VPNs, Tailscale, and other DNS configurations
- Automatic cleanup on shutdown (Ctrl-C or SIGTERM)
- Manual cleanup available: `sudo iron --cleanup-dns`

**Unsupported Platforms**:
- Linux without systemd-resolved: Manual DNS configuration required
- See `doc/dns-setup.md` for manual setup instructions

## CLI Interface
Iron provides a comprehensive CLI with the following commands:

**Daemon Mode** (default):
```bash
sudo iron serve             # Start daemon with auto DNS setup
sudo iron --dns-port 5353   # Use custom DNS port
sudo iron --log-level debug # Enable debug logging
sudo iron --cleanup-dns     # Cleanup DNS config and exit
```

**Utility Commands**:
```bash
iron convert <value>                  # Convert between formats (hex, base32, .iron, IPv6)
iron self                             # Show node information
iron vanity <prefix>                  # Generate vanity address with prefix
iron key info                         # Show key information
iron key generate --save              # Generate new key
iron key export --format hex          # Export key
iron resolve <domain>                 # Test DNS resolution
```

See `doc/cli.md` for detailed command documentation.

# No-std Considerations
**Decision**: Use std for MVP

**Rationale**:
- TUN devices require OS interaction (file descriptors, system calls)
- Tokio runtime requires std
- iroh requires std
- no-std is not a requirement for the target platforms (desktop/server)
- Embedded/mobile support can be reconsidered if needed

# Platform-Specific Notes

## macOS
- TUN device creation requires root privileges
- Use `utun` devices (user-space TUN, no kernel extension needed)
- No multi-queue support (single-threaded TUN sufficient)

## Linux
- Supports multi-queue TUN (`IFF_MULTI_QUEUE`)
- Can optimize with multiple queues in future (Phase 4.1)
- Better performance characteristics for high-throughput scenarios

## Windows
- Uses wintun driver
- Different TUN API (abstracted by tun crate)
- Tertiary priority for initial development
