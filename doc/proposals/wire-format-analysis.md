# Proposal: Wire Format Analysis and Benchmarking

## Status
📊 **Analysis Required** - Needs benchmarking data

## Summary

Compare wire format sizes and performance between `iron/packet/0` (raw bytes) and `iron/packet/1` (serialized `Packet` enum) to ensure the abstraction layer doesn't introduce unacceptable overhead.

## Background

The [packet abstraction proposal](./packet-abstraction.md) introduces an enum-based packet system that requires serialization. We need to measure:
1. **Size overhead**: How much larger are serialized packets?
2. **Performance overhead**: How much slower is serialization/deserialization?
3. **MTU considerations**: Do we exceed 1500-byte limit?

## Packet Size Considerations

### Current MTU Limit

```rust
const MAX_PACKET_SIZE: usize = 1500;
```

**Question**: Is this self-imposed or will exceeding it cause issues?

### Investigation Needed

1. **QUIC/TLS overhead**
   - QUIC packet header: ~20 bytes (short header) to ~60 bytes (long header)
   - TLS record layer: ~30-50 bytes (record header + MAC)
   - Total QUIC/TLS overhead: ~50-100 bytes per packet?
   
2. **Network MTU**
   - Ethernet MTU: 1500 bytes (standard)
   - IPv6 minimum MTU: 1280 bytes
   - QUIC path MTU discovery: Automatically handles fragmentation
   
3. **Fragmentation behavior**
   - Does QUIC automatically fragment large packets?
   - What's the performance impact of fragmentation?
   - Should we enforce MTU at application layer or rely on QUIC?

**TODO**: Test with varying packet sizes and measure:
- When does QUIC start fragmenting?
- What's the overhead of fragmentation?
- Does it affect latency/throughput?

## Serialization Framework

### Decision: `postcard` ✅

We're using [`postcard`](https://docs.rs/postcard/) for all packet serialization.

**Rationale**:
- ✅ **Smallest wire format** - Highly optimized for Rust enums and structs
- ✅ **Fastest performance** - Minimal serialization overhead
- ✅ **No schema required** - Works directly with serde-derived types
- ✅ **Embedded-friendly** - `no_std` compatible (future-proofs iron for IoT/embedded)
- ✅ **Good evolution support** - Can add fields with `#[serde(default)]`
- ✅ **Rust-native** - Designed specifically for Rust's type system

**Tradeoffs**:
- ❌ Not cross-platform (Rust-only)
  - **Acceptable**: Iron is a Rust-only project with no plans for non-Rust clients
- ❌ No official spec (format may change between versions)
  - **Mitigation**: Pin version, update deliberately
- ❌ Smaller ecosystem than MessagePack
  - **Acceptable**: We only need basic serialization

**Migration path**: If cross-platform support becomes necessary, switching to MessagePack (`rmp-serde`) is straightforward since both use the same serde traits. The `Packet` enum definition stays identical.

**Dependencies**:
```toml
[dependencies]
postcard = { version = "1.0", features = ["alloc"] }
serde = { version = "1", features = ["derive"] }
```

**Comparison with alternatives**:

| Framework | Size (1500B packet) | Serialize | Deserialize | Notes |
|-----------|---------------------|-----------|-------------|-------|
| **postcard** ✅ | ~1502 bytes | ~200ns | ~200ns | **CHOSEN** - Best for Rust-only |
| bincode | ~1509 bytes | ~180ns | ~180ns | Slightly larger due to fixed-length encoding |
| rmp-serde | ~1503 bytes | ~300ns | ~300ns | Good if cross-platform needed later |

*Note: Performance numbers are estimates, see benchmark results below*

## Benchmark Plan

### Test Cases

```rust
// Test 1: Minimal Raw packet (40 bytes IPv6 header, no payload)
let raw_minimal = Packet::Raw(vec![0u8; 40]);

// Test 2: Small Raw packet (40 byte header + 100 byte payload)
let raw_small = Packet::Raw(vec![0u8; 140]);

// Test 3: Large Raw packet (40 byte header + 1460 byte payload = MTU)
let raw_large = Packet::Raw(vec![0u8; 1500]);

// Test 4: Onion Relay packet
let onion_relay = Packet::Onion(OnionMessage::Relay {
    next_hop: endpoint_id,  // 32 bytes
    encrypted_payload: vec![0u8; 1400],  // encrypted next hop
});

// Test 5: Onion Data packet
let onion_data = Packet::Onion(OnionMessage::Data(vec![0u8; 1460]));
```

### Metrics to Measure

```rust
#[bench]
fn bench_serialize_raw_minimal(b: &mut Bencher) {
    let packet = Packet::Raw(vec![0u8; 40]);
    b.iter(|| {
        let serialized = postcard::to_vec(&packet).unwrap();
        black_box(serialized);
    });
}

#[bench]
fn bench_deserialize_raw_minimal(b: &mut Bencher) {
    let packet = Packet::Raw(vec![0u8; 40]);
    let serialized = postcard::to_vec(&packet).unwrap();
    b.iter(|| {
        let deserialized: Packet = postcard::from_bytes(&serialized).unwrap();
        black_box(deserialized);
    });
}

// Repeat for all test cases...
```

### Expected Results

Using `postcard` serialization:

| Packet Type | Raw Bytes | Serialized | Overhead | Notes |
|-------------|-----------|------------|----------|-------|
| Raw minimal (40B) | 40 | ~42 | +2B (+5%) | Enum tag (1B) + varint length (1B) |
| Raw small (140B) | 140 | ~142 | +2B (+1.4%) | Same overhead, better ratio |
| Raw large (1500B) | 1500 | ~1502 | +2B (+0.1%) | Negligible percentage |
| Onion Relay | N/A | ~1436 | N/A | 32B EndpointId + 2B overhead + 1400B payload |
| Onion Data | N/A | ~1462 | N/A | 2B overhead + 1460B payload |

**Overhead breakdown** (postcard):
- Enum discriminant: 1 byte (for `Packet::Raw` vs `Packet::Onion`)
- Length prefix (varint): 1-2 bytes for typical packet sizes
- Struct field tags: 0 bytes (postcard uses positional encoding)

**Total overhead**: ~2-3 bytes per packet (negligible)

### Performance Estimates

Using `postcard` serialization:

| Operation | Raw Bytes | postcard | Notes |
|-----------|-----------|----------|-------|
| Serialize (small, 40B) | 0ns (no-op) | ~100ns | Enum tag + varint + memcpy |
| Deserialize (small, 40B) | 0ns (no-op) | ~100ns | Parse tag + length + memcpy |
| Serialize (large, 1500B) | 0ns (no-op) | ~300ns | Mostly just memcpy |
| Deserialize (large, 1500B) | 0ns (no-op) | ~300ns | Mostly just memcpy |

**Throughput impact**:
- At 1Gbps: Serialization adds ~0.03% overhead (300ns per 1500B packet)
- At 100Mbps: ~0.003% overhead
- At 10Mbps: Negligible

**Latency impact**:
- Additional ~0.3μs per packet (serialize + deserialize)
- Network latency: 1-50ms (1,000-50,000μs)
- **Serialization is 0.001-0.03% of total latency** (negligible)

**Conclusion**: `postcard` serialization overhead is negligible compared to network latency.

## MTU Violation Scenarios

### Worst Case: Onion Relay with Max Payload

```
IPv6 header:              40 bytes
Max IP payload:           1460 bytes
Enum discriminant:        1 byte
next_hop (EndpointId):    32 bytes
encrypted_payload length: 2 bytes (varint)
encrypted_payload:        1460 bytes (nested packet)
AES-GCM tag:              16 bytes (if using authenticated encryption)
----------------------------------------
Total:                    2011 bytes (exceeds 1500 MTU!)
```

**Problem**: Onion routing with max-size inner packets exceeds MTU.

### Solutions

#### Option 1: Reduce Inner Packet Size
```rust
// For onion packets, limit inner payload
const MAX_ONION_PAYLOAD: usize = 1400; // leaves room for overhead
```

**Pros**: Simple, no fragmentation
**Cons**: Reduces usable payload for onion-routed packets

#### Option 2: Rely on QUIC Fragmentation
QUIC automatically fragments large packets across multiple QUIC packets.

**Pros**: No application-level logic needed
**Cons**: Performance impact, additional latency

#### Option 3: Application-Level Chunking
Split large packets into multiple smaller packets.

**Pros**: Full control over chunking strategy
**Cons**: Complex, need reassembly logic

**Recommendation**: **Option 1** for MVP (simpler), with **Option 2** as fallback (QUIC handles it).

## Action Items

### Immediate (Before Implementing Packet Abstraction)

- [ ] Create benchmark suite with test cases above
- [ ] Measure serialization overhead with `postcard` and `rmp-serde`
- [ ] Measure packet size for all test cases
- [ ] Document results in this file

### Short-term (During Implementation)

- [ ] Test QUIC behavior with >1500 byte packets
- [ ] Measure actual network throughput with serialized packets
- [ ] Implement MTU enforcement for onion packets if needed

### Long-term (Post-MVP)

- [ ] Evaluate cross-platform serialization needs
- [ ] Consider zero-copy deserialization (Cap'n Proto, FlatBuffers)
- [ ] Optimize hot paths if profiling shows serialization bottleneck

## Benchmark Results

**TODO**: Run benchmarks and fill in actual results

```
Benchmark results (criterion):

serialize_raw_minimal:    ___ ns/iter
deserialize_raw_minimal:  ___ ns/iter
serialize_raw_small:      ___ ns/iter
deserialize_raw_small:    ___ ns/iter
serialize_raw_large:      ___ ns/iter
deserialize_raw_large:    ___ ns/iter

Packet size measurements:

Raw minimal:     40 bytes → ___ bytes serialized
Raw small:       140 bytes → ___ bytes serialized
Raw large:       1500 bytes → ___ bytes serialized
Onion relay:     N/A → ___ bytes serialized
Onion data:      N/A → ___ bytes serialized
```

## Related Proposals

- [packet-abstraction.md](./packet-abstraction.md) - Parent proposal
- [0rtt.md](./0rtt.md) - Onion routing (affects packet size calculations)

## Open Questions

1. **QUIC MTU discovery**: How does iroh handle path MTU discovery?
2. **Fragmentation overhead**: What's the actual cost of QUIC fragmentation?
3. **Compression**: Should we compress large payloads before serialization?
4. **Zero-copy**: Can we avoid copying packet bytes during serialization?
5. **Backward compatibility**: Should `/0` and `/1` peers interoperate during transition?

## Success Criteria

- ✅ Serialization overhead < 1% for typical packet sizes
- ✅ No MTU violations for common use cases
- ✅ Serialization latency < 1μs (negligible vs network latency)
- ✅ Clear documentation of packet size limits per type
- ✅ Benchmarks confirm performance is acceptable
