# Proposal: Onion Routing with Error Propagation

## Status
🚧 **Needs Evaluation** - High complexity, significant architectural implications

## Summary

Implement multi-hop onion routing similar to Tor, where packets are encrypted multiple times (one layer per hop) and relayed through intermediate nodes before reaching the final destination. Each relay peels one layer of encryption and forwards to the next hop.

This proposal extends the [packet abstraction layer](./packet-abstraction.md) with onion routing capabilities.

## Motivation

**Privacy benefits**:
- Hide the final destination from intermediate relays
- Hide the source from the final destination
- Make traffic analysis more difficult
- Enable plausible deniability for relay operators

**Use cases**:
- Accessing services without revealing your identity
- Bypassing network restrictions
- Protecting metadata (who talks to whom)

## Design

### Packet Structure

From [packet-abstraction.md](./packet-abstraction.md):

```rust
#[non_exhaustive]
pub enum Packet {
    Raw(Vec<u8>),
    Onion(OnionMessage),
}

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
        // and iroh cryptographically verifies the sender's identity
    },
    
    /// Final destination - the actual payload data
    /// 
    /// This is delivered to the application layer (TUN interface)
    Data(Vec<u8>),
    
    /// Error response (propagated backward through circuit)
    /// 
    /// See "Error Propagation Complexity" section below
    Error {
        /// Type of error that occurred
        error_type: OnionError,
        
        /// Checksum of the original message (for routing error back)
        /// Allows nodes to look up which peer sent the message
        message_checksum: [u8; 32],
    },
}

pub enum OnionError {
    /// Next hop in circuit is unreachable
    RelayUnreachable,
    
    /// Failed to decrypt the payload (wrong key or corrupted data)
    DecryptionFailed,
    
    /// Circuit was closed by an intermediate node
    CircuitClosed,
    
    /// Relay node is overloaded and refusing new circuits
    OverloadRejection,
}
```

### Encryption Scheme

**Choice**: Authenticated encryption using each relay's public key

**Options**:
1. **Iroh's built-in encryption** (if available)
   - Leverage existing infrastructure
   - Consistent with rest of iron
   - **Needs investigation**: Does iroh expose encryption primitives?

2. **libsodium's sealed boxes** (crypto_box_seal)
   - Anonymous sender (receiver can't identify encryptor)
   - Authenticated encryption
   - Well-audited, fast

3. **Custom ECIES** (Elliptic Curve Integrated Encryption Scheme)
   - More control over parameters
   - More complex to implement correctly

**Recommended**: Use iroh's encryption if available, otherwise libsodium sealed boxes.

### Circuit Construction

**Note**: Serialization uses `postcard` (see [packet-abstraction.md](./packet-abstraction.md))

**Sender's process**:

```rust
// 1. Choose circuit: [relay1, relay2, relay3, destination]
let circuit = vec![relay1_id, relay2_id, relay3_id, dest_id];

// 2. Create innermost message (for destination)
let inner = OnionMessage::Data(payload);

// 3. Encrypt for each hop (working backward)
let mut encrypted = postcard::to_vec(&inner)?;
for hop in circuit.iter().rev() {
    let onion_msg = OnionMessage::Relay {
        next_hop: *hop,
        encrypted_payload: encrypted,
    };
    let serialized = postcard::to_vec(&onion_msg)?;
    encrypted = encrypt(serialized, hop.public_key());
}

// 4. Send to first relay
let first_msg = decrypt_outer_layer(encrypted);
send_packet(circuit[0], Packet::Onion(first_msg));
```

**Relay's process**:

```rust
// 1. Receive OnionMessage::Relay
match message {
    OnionMessage::Relay { next_hop, encrypted_payload } => {
        // 2. Decrypt one layer
        let decrypted = decrypt(encrypted_payload, my_private_key)?;
        
        // 3. Deserialize inner message (using postcard)
        let inner_message: OnionMessage = postcard::from_bytes(&decrypted)?;
        
        // 4. Forward to next hop
        send_packet(next_hop, Packet::Onion(inner_message));
    }
}
```

**Destination's process**:

```rust
match message {
    OnionMessage::Data(payload) => {
        // Deliver to TUN interface (application layer)
        tun.write(payload);
    }
}
```

## Error Propagation

### The Problem

When a relay fails (next hop unreachable, decryption failed, etc.), we want to notify the **sender** so they can:
- Choose a different circuit
- Mark unreliable relays
- Retry with a fresh circuit

**Challenge**: Relays don't know who the original sender is (that's the point of onion routing!).

### Proposed Solution: Message History with Checksums

Each relay maintains a **bounded cache** of recent messages:

```rust
struct MessageHistory {
    /// Map: message_checksum → sender_endpoint_id
    /// Used to route error messages backward through circuit
    recent_messages: LruCache<[u8; 32], EndpointId>,
    
    /// TTL for entries (e.g., 30 seconds)
    entry_ttl: Duration,
    
    /// Maximum number of entries to prevent DoS
    max_entries: usize, // e.g., 10,000
}
```

**Process**:

1. **Relay receives message**:
   ```rust
   let checksum = blake3::hash(&encrypted_payload).into();
   message_history.insert(checksum, sender_endpoint_id, now());
   ```

2. **Relay attempts to forward**:
   ```rust
   match send_to_next_hop(next_hop, inner_message).await {
       Ok(_) => { /* success */ }
       Err(e) => {
           // Create error message
           let error = OnionMessage::Error {
               error_type: OnionError::RelayUnreachable,
               message_checksum: checksum,
           };
           
           // Look up sender
           if let Some(sender) = message_history.get(&checksum) {
               send_packet(sender, Packet::Onion(error));
           }
       }
   }
   ```

3. **Sender receives error**:
   ```rust
   match message {
       OnionMessage::Error { error_type, message_checksum } => {
           // Match error to original request
           if let Some(circuit) = pending_circuits.get(&message_checksum) {
               warn!("Circuit failed: {:?}", error_type);
               // Retry with different circuit
               retry_with_different_circuit(circuit);
           }
       }
   }
   ```

### Complexity Analysis

**Memory overhead**:
- 32 bytes (checksum) + 32 bytes (EndpointId) + 8 bytes (timestamp) = 72 bytes per entry
- 10,000 entries = 720 KB per node
- **Acceptable** for modern systems

**Security concerns**:

1. **Timing attacks**: 
   - Error response timing could leak circuit structure
   - **Mitigation**: Add random delays before sending errors

2. **Replay attacks**:
   - Attacker could resend messages to fill message history
   - **Mitigation**: Use LRU cache with size limits and TTL

3. **Circuit fingerprinting**:
   - Checksums could be used to correlate messages across relays
   - **Mitigation**: Checksums are only stored locally, not sent over wire

4. **Information leakage**:
   - Error messages reveal that relay N couldn't reach relay N+1
   - **Mitigation**: This is inherent to error reporting, accept or drop silently

**Performance concerns**:
- Hash computation: ~1μs per message (Blake3 is fast)
- LRU cache lookup: O(1) amortized
- **Negligible overhead** compared to encryption/network latency

### Alternative Approaches

#### Option 1: Drop Errors Silently
```rust
// No error propagation, just log
Err(e) => {
    warn!("Failed to relay message: {}", e);
    // Drop silently
}
```

**Pros**:
- Simple, no state needed
- Maximum privacy (no information leakage)
- No additional attack surface

**Cons**:
- Sender never knows if circuit failed
- Can't adapt to network conditions
- Poor user experience (silent failures)

#### Option 2: Encrypted Error Messages
Each hop encrypts the error with the previous hop's key, creating an onion-encrypted error message that unwinds back to the sender.

```rust
// Relay N fails, creates error
let error = OnionError::RelayUnreachable;

// Encrypt for previous hop (N-1)
let encrypted = encrypt(error, previous_hop_key);

// Send back
send_to_previous_hop(encrypted);

// Previous hop decrypts, adds its own layer, sends to N-2
// Repeat until reaching sender
```

**Pros**:
- No message history needed
- Privacy-preserving (each hop only sees one layer)
- Similar to Tor's error handling

**Cons**:
- Complex to implement
- Requires tracking "previous hop" for each circuit
- Still leaks some information (error occurred somewhere in circuit)

#### Option 3: Circuit IDs
Assign a unique ID to each circuit and use it to route errors back.

```rust
struct OnionMessage {
    circuit_id: u64,  // Unique per sender-destination pair
    // ... rest of fields
}
```

**Pros**:
- Simple to implement
- Efficient error routing

**Cons**:
- Circuit IDs are linkable across hops (privacy leak!)
- Attacker can track circuits through the network
- **Not recommended** for privacy-focused system

### Recommendation

**For MVP**: **Option 1** (drop errors silently)
- Simplest implementation
- No additional state or complexity
- Can add error propagation later if needed

**For production** (if error reporting is critical): **Checksum-based message history**
- Acceptable memory/performance overhead
- Good balance of functionality and privacy
- Can be enabled/disabled per node

## Implementation Phases

### Phase 1: Basic Onion Routing (No Errors)
1. Implement `OnionMessage::Relay` and `OnionMessage::Data`
2. Add encryption/decryption layer
3. Update protocol handler to route onion messages
4. Test with 3-hop circuits
5. **Skip error propagation** (drop silently)

**Estimated effort**: 5-7 days

### Phase 2: Error Propagation (Optional)
1. Implement `MessageHistory` cache
2. Add `OnionMessage::Error` variant
3. Implement checksum calculation and storage
4. Add error handling in relay logic
5. Test error scenarios (unreachable relay, decryption failure)

**Estimated effort**: 3-5 days

### Phase 3: Circuit Management
1. Circuit selection algorithms (random vs. optimized)
2. Circuit reuse and pooling
3. Performance monitoring (latency, reliability)
4. Circuit timeout and cleanup

**Estimated effort**: 5-7 days

## Security Considerations

### Threat Model

**What onion routing protects against**:
- ✅ Destination anonymity from relays
- ✅ Source anonymity from destination (if enough hops)
- ✅ Traffic correlation (harder but not impossible)

**What it does NOT protect against**:
- ❌ Global passive adversary (can correlate all traffic)
- ❌ Malicious relays colluding (timing attacks, tagging attacks)
- ❌ Intersection attacks (long-term traffic analysis)
- ❌ End-to-end correlation (if attacker controls first and last hop)

### Minimum Circuit Length

**Recommendation**: At least 3 hops
- 1 hop: No anonymity (relay knows source and destination)
- 2 hops: Weak anonymity (first relay knows source, second knows destination)
- 3 hops: Good anonymity (no single relay knows both source and destination)
- 4+ hops: Diminishing returns (more latency, same privacy)

### Relay Selection

**Random selection**: Pick relays uniformly at random
- Simple
- Resistant to manipulation
- May choose slow/unreliable relays

**Reputation-based selection**: Prefer reliable, fast relays
- Better performance
- Vulnerable to Sybil attacks (attacker runs many "good" relays)
- Needs secure reputation system

**Hybrid approach** (recommended):
- Use reputation for performance hints
- Still randomize selection to prevent exploitation
- Monitor and ban malicious relays

## Performance Impact

### Latency

**Formula**: `total_latency = sum(hop_latencies) + sum(crypto_overhead)`

**Example** (3-hop circuit):
- Direct connection: 20ms RTT
- 3-hop circuit: 60ms RTT (3× latency) + 3× encryption overhead (~1ms total)
- **Total**: ~61ms (3× slower than direct)

**Mitigation**:
- Use low-latency relays
- Pre-build circuits (avoid construction latency)
- Parallel circuit construction

### Throughput

**Bottleneck**: Slowest relay in circuit

**Example**:
- Direct connection: 100 Mbps
- Relay 1: 100 Mbps
- Relay 2: 10 Mbps (bottleneck!)
- Relay 3: 100 Mbps
- **Effective throughput**: 10 Mbps

**Mitigation**:
- Measure relay throughput and avoid slow relays
- Use multiple circuits for parallel streams
- Load balancing across circuits

### Packet Size Overhead

**Calculation** (per hop):
- Enum discriminant: 1 byte
- next_hop EndpointId: 32 bytes
- Encrypted payload length: 2 bytes (varint)
- Encryption overhead (AES-GCM tag): 16 bytes
- **Total per hop**: ~51 bytes

**3-hop circuit overhead**:
- Original packet: 1500 bytes
- After 3 hops of encryption: 1500 + 3×51 = ~1653 bytes
- **Exceeds MTU!** (see [wire-format-analysis.md](./wire-format-analysis.md))

**Solution**: Reduce inner payload size to account for onion overhead:
```rust
const MAX_ONION_PAYLOAD: usize = 1400; // leaves room for 2-3 hops
```

## Open Questions

1. **Error propagation**: Is the complexity worth it, or should we drop errors silently?
   - **Status**: 🚧 Needs evaluation, recommend starting without

2. **Encryption scheme**: Use iroh's built-in crypto or libsodium?
   - **Status**: 🔍 Needs investigation of iroh's API

3. **Circuit pooling**: Should we reuse circuits or create new ones per message?
   - **Tradeoff**: Reuse is faster but less privacy

4. **Relay incentives**: How do we incentivize nodes to act as relays?
   - **Options**: Altruism, quid-pro-quo (I relay for you, you relay for me), payment

5. **Directory service**: How do nodes discover available relays?
   - **Options**: Distributed hash table (DHT), centralized directory, gossip protocol

6. **Circuit failures**: What's the user experience when circuits fail?
   - **Options**: Automatic retry, user notification, fallback to direct connection

## Related Proposals

- [packet-abstraction.md](./packet-abstraction.md) - Required base layer
- [wire-format-analysis.md](./wire-format-analysis.md) - Packet size considerations
- [firewall.md](./firewall.md) - May conflict (firewall blocks unknown relays)

## Success Criteria

- ✅ 3-hop onion routing working end-to-end
- ✅ Encryption/decryption at each hop
- ✅ Packet forwarding through intermediate relays
- ✅ Destination receives original payload intact
- ✅ Source anonymity preserved (relays don't know original sender)
- ✅ Destination anonymity preserved (relays don't know final destination)
- ✅ Performance acceptable (< 3× latency overhead)
- ✅ Comprehensive tests with various circuit lengths

## Future Enhancements

1. **Hidden services**: Destination can be anonymous (Tor-style onion addresses)
2. **Rendezvous points**: Source and destination meet in the middle
3. **Guard nodes**: Always use same entry relay (protects against first-hop attacks)
4. **Bandwidth accounting**: Track and limit relay usage
5. **Payment channels**: Micropayments for relay services
6. **Padding**: Add dummy traffic to resist traffic analysis
7. **Directory consensus**: Distributed relay directory with voting

## References

- [Tor design paper](https://svn-archive.torproject.org/svn/projects/design-paper/tor-design.pdf)
- [Tor path selection](https://blog.torproject.org/lifecycle-of-a-new-relay/)
- [Sphinx packet format](https://cypherpunks.ca/~iang/pubs/Sphinx_Oakland09.pdf) - More advanced onion routing
