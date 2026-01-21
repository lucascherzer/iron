# Proposal: Packet Abstraction Layer

## Status
📝 **Proposal** - Detailed design phase

## Summary
Refactor the current raw byte packet handling (`Vec<u8>`) to use a type-safe `Packet` enum that can represent different types of messages. This enables future features like onion routing, cryptographic operations, and firewall functionality while maintaining backward compatibility.

## Motivation

Currently, iron sends raw IPv6 packets as `Vec<u8>` directly over QUIC. This works for simple packet forwarding but limits our ability to add advanced features:

1. **Onion routing** - Multi-hop encrypted routing with relay functionality
2. **Cryptographic operations** - Signing, verification, key exchange
3. **Firewall/Access control** - Device ownership claims and whitelisting (see [firewall.md](./firewall.md))
4. **Protocol evolution** - No way to add new message types without breaking changes

## Design

### Core Types

```rust
/// Top-level packet abstraction
/// 
/// This enum allows us to support multiple packet types over the same QUIC connection
/// while maintaining type safety and extensibility.
#[non_exhaustive]
pub enum Packet {
    /// Raw IPv6 packet (backward compatible with iron/packet/0)
    /// 
    /// This is a raw packet from the OS TUN interface that should be
    /// forwarded directly to the destination peer.
    Raw(Vec<u8>),
    
    /// Onion-routed message (see onion routing proposal)
    /// 
    /// Multi-hop encrypted message that gets relayed through intermediate
    /// nodes before reaching the final destination.
    Onion(OnionMessage),
    
    // Future additions (examples):
    // Crypto(CryptoMessage),  // Cryptographic operations
    // Control(ControlMessage), // Protocol control messages
}

/// Onion routing message structure (see onion.md for full details)
pub enum OnionMessage {
    /// Relay to the next hop with encrypted payload
    /// 
    /// The encrypted_payload contains the next OnionMessage (encrypted with
    /// the next hop's key). Each relay peels one layer of encryption.
    Relay {
        /// The EndpointId of the next hop in the circuit
        next_hop: EndpointId,
        
        /// Encrypted payload containing the next OnionMessage
        /// Format: encrypt(serialize(OnionMessage), next_hop_public_key)
        encrypted_payload: Vec<u8>,
        
        // Note: MAC may not be necessary since QUIC provides authenticated encryption
        // and the sender's identity is cryptographically verified by iroh
    },
    
    /// Final destination - the actual payload data
    /// 
    /// This is delivered to the application layer (TUN interface)
    Data(Vec<u8>),
    
    /// Error response (propagated backward through circuit)
    /// 
    /// NOTE: Error propagation requires tracking message history.
    /// See "Error Handling Complexity" section below.
    Error {
        /// Type of error that occurred
        error_type: OnionError,
        
        /// Checksum of the original message (for routing error back)
        /// This allows nodes to look up which peer sent the message
        message_checksum: [u8; 32],
    },
}

pub enum OnionError {
    RelayUnreachable,
    DecryptionFailed,
    CircuitClosed,
    // More variants as needed
}
```

### Wire Format

**Serialization**: `postcard` with `serde`

**Decision**: We're using [`postcard`](https://docs.rs/postcard/) for serialization.

**Why `postcard`**:
- ✅ **Smallest wire format** - Highly optimized for Rust enums (~2 bytes overhead)
- ✅ **Fastest performance** - Minimal serialization overhead (~100-500ns)
- ✅ **No schema required** - Works directly with serde-derived types
- ✅ **Embedded-friendly** - `no_std` compatible (future-proofs iron for IoT/embedded)
- ✅ **Good evolution support** - Can add fields with `#[serde(default)]`
- ✅ **Rust-native** - Designed specifically for Rust's type system

**Tradeoffs**:
- ❌ Not cross-platform (Rust-only, but iron is Rust-only)
- ❌ No official spec (format may change between versions)
- ❌ Smaller ecosystem than MessagePack

**Migration path**: If cross-platform support is needed later, switching to MessagePack (`rmp-serde`) is a one-line change since both use the same serde traits.

**Dependencies**:
```toml
postcard = { version = "1.0", features = ["alloc"] }
serde = { version = "1", features = ["derive"] }
```

**Format comparison**:
- `Packet::Raw(1500 bytes)`: ~1502 bytes serialized (+2 bytes overhead)
- See [wire-format-analysis.md](./wire-format-analysis.md) for detailed benchmarks

### Protocol Version

**ALPN Update**: `b"iron/packet/0"` → `b"iron/packet/1"`

This signals a breaking protocol change:
- Peers using `/0` send raw bytes
- Peers using `/1` send serialized `Packet` enums
- No backward compatibility between versions (clean break)

### Channel Type Changes

#### Current Implementation
```rust
// OS → Network (TUN sends to Protocol)
mpsc::unbounded_channel::<(EndpointId, Vec<u8>)>()

// Network → OS (Protocol sends to TUN)
mpsc::unbounded_channel::<Vec<u8>>()
```

#### After Refactor
```rust
// OS → Network (TUN sends to Protocol)
mpsc::unbounded_channel::<(EndpointId, Packet)>()

// Network → OS (Protocol sends to TUN)
mpsc::unbounded_channel::<Packet>()
```

**TUN Interface Behavior**:
- When reading from OS: wraps raw bytes in `Packet::Raw`
- When writing to OS: extracts bytes from `Packet::Raw` (or: handles it via side effects, e.g. `OnionMessage::Data`)
- Ignores non-data packets (e.g., `Packet::Onion` with `OnionMessage::Relay`)

**Protocol Layer Responsibility**:
- Deserializes incoming `Packet` from QUIC stream
- Routes based on packet type:
  - `Packet::Raw` → forward to destination
  - `Packet::Onion(Relay)` → decrypt and relay to next_hop
  - `Packet::Onion(Data)` → send to TUN for delivery to OS
  - `Packet::Onion(Error)` → route back to previous hop using message history

## Packet Processing Location

**Decision**: Packet type determination happens in the **Protocol layer**

- **TUN layer**: Simple, only deals with `Packet::Raw` and `Packet::Onion(Data)`
- **Protocol layer**: Handles all packet routing logic, relaying, encryption/decryption
- **Separation of concerns**: TUN manages OS interface, Protocol manages network logic

## Error Handling Complexity

### Problem: Delivering Error Messages Backward

When an onion relay fails (e.g., next hop unreachable), we need to send an error back to the **previous hop** in the circuit. But QUIC connections are bidirectional between pairs of peers.

**Challenge**: How does a relay know which peer sent the message?

### Proposed Solution: Message History with Checksums

Each relay keeps a **bounded history** of recent messages:

```rust
struct MessageHistory {
    /// Map: message_checksum → sender_endpoint_id
    /// Used to route error messages backward
    recent_messages: LruCache<[u8; 32], EndpointId>,
    
    /// TTL for entries (e.g., 30 seconds)
    entry_ttl: Duration,
}
```

**Process**:
1. Relay receives `OnionMessage::Relay` from peer A
2. Compute checksum: `hash(encrypted_payload)`
3. Store: `recent_messages[checksum] = peer_A`
4. Attempt to relay to next_hop
5. If relay fails:
   - Create `OnionMessage::Error` with `message_checksum`
   - Look up sender in `recent_messages[checksum]`
   - Send error back to sender

**Complexity Considerations**:
- Memory overhead: storing checksums and EndpointIds
- Timing attacks: could leak information about circuit structure
- Race conditions: message might expire before error is sent
- Replay attacks: need to handle duplicate checksums

**Status**: 🚧 **Needs detailed evaluation** - This adds significant complexity and may not be worth it for MVP. Consider alternatives:
- Drop silently (privacy-preserving, no state required)
- Log only (good for debugging, no network overhead)
- Optional error reporting (configurable per-node)

## Packet Size Considerations

### Current MTU Limit

```rust
const MAX_PACKET_SIZE: usize = 1500;
```

**Questions**:
1. Is this self-imposed or will exceeding it cause issues?
2. How does QUIC handle fragmentation?
3. Will serialization overhead push us over MTU?

### Analysis Required

**TODO**: Investigate packet size boundaries
- Raw IPv6 packet: 40 bytes header + up to 1460 bytes payload = 1500 total
- QUIC overhead: TLS record layer + QUIC headers (~50-100 bytes?)
- Serialization overhead: enum discriminant + length prefix (~2-10 bytes?)
- Onion routing overhead: next_hop EndpointId (32 bytes) + encryption (16 bytes for AES-GCM tag)

**Worst case** (Onion Relay):
```
40 (IPv6 header)
+ 1460 (max payload)
+ 2 (enum discriminant)
+ 32 (next_hop EndpointId)
+ 16 (encryption tag)
= 1550 bytes (exceeds MTU!)
```

**Solutions**:
1. Reduce max payload size for onion packets
2. Use QUIC stream fragmentation (automatic)
3. Implement chunking at application layer
4. Document maximum payload sizes per packet type

**Status**: 📊 **Needs benchmarking** - See separate analysis document

## Implementation Plan

### Phase 1: Core Abstraction (No Breaking Changes)
1. Add `src/packet.rs` with `Packet` enum (only `Packet::Raw` variant initially)
2. Update internal channels to use `Packet` instead of `Vec<u8>`
3. TUN wraps/unwraps `Packet::Raw` at boundaries
4. Protocol layer handles `Packet` serialization
5. All tests updated to use `Packet::Raw`
6. **No ALPN change yet** - still using `iron/packet/0` with raw bytes on wire

### Phase 2: Protocol Version Update
1. Bump ALPN to `iron/packet/1`
2. Implement serialization/deserialization on QUIC streams
3. Benchmark packet size and performance
4. Document wire format differences

### Phase 3: Onion Routing (Future)
1. Add `Packet::Onion` variant
2. Implement `OnionMessage` routing logic
3. Add encryption/decryption layer
4. Evaluate error propagation complexity
5. See [onion.md](./0rtt.md) for full proposal (TODO: rename or create new file)

### Phase 4: Additional Features (Future)
1. Firewall with device ownership claims ([firewall.md](./firewall.md))
2. Cryptographic operations
3. Control messages

## Related Proposals

- [onion.md](./0rtt.md) - Onion routing implementation details (TODO: create/update)
- [firewall.md](./firewall.md) - Whitelist-based access control with device claims
- Wire format analysis (TODO: create document with benchmarks)

## Open Questions

1. **Error propagation**: Is the complexity worth it? Should we implement it at all?
2. **Wire format**: `postcard` vs `bincode` vs `rmp-serde`?
3. **MTU handling**: How to handle packet sizes exceeding 1500 bytes?
4. **Backward compatibility**: Should we support both `/0` and `/1` simultaneously during transition?
5. **Performance impact**: What is the serialization overhead? (needs benchmarking)

## Success Criteria

- ✅ Type-safe packet handling with `Packet` enum
- ✅ No breaking changes to existing `Packet::Raw` behavior
- ✅ Clear path to onion routing implementation
- ✅ All existing tests pass with new abstraction
- ✅ Wire format documented and benchmarked
- ✅ ALPN version updated to signal protocol change

## Timeline

- **Phase 1**: ~1-2 days (core abstraction, internal refactor)
- **Phase 2**: ~1 day (protocol version update, benchmarking)
- **Phase 3**: TBD (depends on onion routing proposal evaluation)
- **Phase 4**: TBD (depends on firewall and crypto proposals)
