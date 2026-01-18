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

We require two components for this to work:
1. A `.iron` resolver which can map `<endpoint_id>.iron` to a deterministic IPv6 address
  in the Unique Local Address (ULA) space. The mapping is deterministic based on
  the EndpointId to ensure consistency. For close integration with existing software,
  it needs to be able to resolve `.iron` DNS queries.
2. A tun interface that facilitates communication to the outside network,
  advertising to route addresses within the ULA address spaces. It uses the
  resolver in reverse, taking the IPv6 address and getting its associated iroh
  EndpointId. And sending the data off.

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
- hickory-server 0.25 (DNS server, successor to trust-dns)
- tokio 1.x (async runtime with full features)
- dashmap 6 (concurrent hash maps for Registry)
- etherparse 0.19 (IPv6 packet parsing)

# Architecture Decisions

## Process Model
**Decision**: Single process with multiple tokio tasks

**Rationale**:
- Simpler architecture for MVP
- Shared `Arc<Registry>` between DNS and TUN components
- No IPC overhead
- Easier debugging and state management
- Can migrate to multi-process later if needed

**Task Structure**:
- Main task: Orchestration and lifecycle management
- DNS task: Hickory-server DNS resolver
- TUN task: Packet processing loop
- Iroh task: Connection management (integrated into IronNode)

## Platform Support
**Development Priority**: macOS-first, then Linux, then Windows

**Rationale**:
- Primary development environment is macOS
- Establish correctness baseline on one platform
- Add Linux support (with multi-queue TUN optimization)
- Windows support as tertiary target
- Architecture allows platform-specific optimizations via conditional compilation

## TUN Interface Architecture
**MVP Approach**: Single-threaded async loop

**Implementation**:
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

## Testing Strategy
**Initial Approach**: Manual testing with two local iron nodes
- Start two instances on localhost
- Test DNS resolution: `dig @localhost -p 5333 <endpoint_id>.iron AAAA`
- Test connectivity: `ping6 fd69:726f::...`
- Simple, fast iteration during development

**Future**: Automated integration tests with tokio test infrastructure

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
