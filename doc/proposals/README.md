# Iron Proposals Index

This directory contains design proposals for future iron features. Each proposal is a separate document that can be evaluated and implemented independently.

## Proposal Status Legend

- 📝 **Proposal** - Conceptual design phase, needs review
- 🔍 **Investigation** - Requires technical investigation
- 📊 **Analysis** - Needs benchmarking/measurement
- 🚧 **Needs Evaluation** - High complexity, needs detailed cost/benefit analysis
- ✅ **Approved** - Ready for implementation
- 🚀 **In Progress** - Currently being implemented
- ✅ **Implemented** - Completed and merged

## Active Proposals

### 1. [Packet Abstraction Layer](./packet-abstraction.md) 📝
**Status**: Proposal (detailed design phase)
**Priority**: High (blocking other features)
**Dependencies**: None

Refactor raw byte packets (`Vec<u8>`) to type-safe `Packet` enum. This is the foundation for all advanced features.

**Key points**:
- Introduces `Packet::Raw` for backward compatibility
- Extensible with `#[non_exhaustive]`
- Updates ALPN to `iron/packet/1`
- Changes internal channel types
- Enables onion routing, firewall, crypto features

**Implementation phases**:
1. Core abstraction (no breaking changes) - ~1-2 days
2. Protocol version update - ~1 day
3. Future features (onion, firewall, etc.) - TBD

### 2. [Onion Routing with Error Propagation](./onion-routing.md) 🚧
**Status**: Needs evaluation (high complexity)
**Priority**: Medium (privacy feature)
**Dependencies**: [packet-abstraction.md](./packet-abstraction.md)

Multi-hop encrypted routing similar to Tor. Provides source/destination anonymity through relay nodes.

**Key considerations**:
- Error propagation adds significant complexity
- Requires message history tracking (memory overhead)
- 3× latency increase for 3-hop circuits
- Packet size overhead (~51 bytes per hop)
- Privacy vs. performance tradeoff

**Open question**: Is error propagation worth the complexity?
- **Option 1**: Drop errors silently (simple, privacy-preserving)
- **Option 2**: Checksum-based message history (functional but complex)

**Recommendation**: Start with Option 1 (no errors) for MVP.

### 3. [Firewall with Device Ownership Claims](./firewall.md) 📝
**Status**: Proposal (conceptual design)
**Priority**: Medium (security feature)
**Dependencies**: [packet-abstraction.md](./packet-abstraction.md)

Whitelist-based access control using two-tier key system: person keys and device keys.

**Problem solved**:
- Avoid manually whitelisting every device
- Trust people, not devices
- Friends can add new devices without manual intervention

**Key concepts**:
- Person key: Long-term identity (user exchanges these)
- Device key: Ephemeral identity (current EndpointId)
- Ownership claim: Signed proof that person owns device

**Implementation**: ~5-8 days

**Conflict with onion routing**: Firewall may block relay nodes. Needs investigation.

### 4. [Wire Format Analysis](./wire-format-analysis.md) ✅
**Status**: Decided - Using `postcard` serialization
**Priority**: High (required for packet abstraction)
**Dependencies**: None (but informs packet-abstraction)

Analysis of serialization overhead and packet sizes for `iron/packet/1`.

**Decision**: Using [`postcard`](https://docs.rs/postcard/) for serialization
- Smallest wire format (~2 bytes overhead)
- Fastest performance (~100-300ns per packet)
- Rust-native, `no_std` compatible
- Good schema evolution support

**Key findings**:
- Serialization adds ~2 bytes per packet (negligible)
- Performance overhead: ~0.03% of network latency
- Onion routing may exceed MTU (needs payload size limits)

**Remaining action items**:
- [ ] Run actual benchmarks to confirm estimates
- [ ] Test QUIC fragmentation behavior with large packets
- [ ] Document final packet size limits
- [ ] Create benchmark suite
- [ ] Measure packet sizes for all packet types
- [ ] Test QUIC behavior with >1500 byte packets
- [ ] Document results and recommendations

## Proposal Relationships

```
packet-abstraction.md
  ├─→ onion-routing.md (requires Packet::Onion)
  ├─→ firewall.md (requires Packet::Auth)
  └─→ wire-format-analysis.md (informs design decisions)

onion-routing.md
  ├─→ wire-format-analysis.md (packet size concerns)
  └─→ ⚠️  May conflict with firewall.md (relays need access)

firewall.md
  ├─→ packet-abstraction.md (requires Packet::Auth)
  └─→ ⚠️  May conflict with onion-routing.md (blocks relays)
```

## Implementation Roadmap

### Phase 1: Foundation (Required)
1. ✅ Complete test coverage for protocol module (DONE - Jan 21, 2026)
2. ✅ Decide on serialization framework (DONE - using `postcard`)
3. 📊 Wire format benchmarking (optional, estimates documented)
4. 🚀 Implement packet abstraction layer

**Estimated timeline**: 2-3 days

### Phase 2: Security Features (Optional)
Choose one:
- **Option A**: Firewall (better for private networks)
- **Option B**: Onion routing (better for privacy/anonymity)

**Note**: These may conflict. Need to design interoperability.

**Estimated timeline**: 5-10 days depending on choice

### Phase 3: Advanced Features (Future)
- Onion routing + firewall integration
- Cryptographic operations (signing, verification)
- Control messages
- Performance optimizations

## Design Principles

All proposals follow these principles:

1. **Backward compatibility**: New features don't break existing deployments
2. **Incremental implementation**: Can be added in phases
3. **Type safety**: Leverage Rust's type system
4. **Performance**: Benchmark and measure overhead
5. **Privacy-preserving**: Minimal information leakage
6. **Testability**: All features must be testable in CI
7. **Simplicity**: Start with MVP, add complexity only when needed

## Contributing to Proposals

### Adding a New Proposal

1. Create `doc/proposals/your-feature.md`
2. Use the following template:

```markdown
# Proposal: Your Feature Name

## Status
📝 **Proposal** - Brief status description

## Summary
One-paragraph overview

## Motivation
Why is this needed?

## Design
Detailed design with code examples

## Implementation Complexity
Effort estimates

## Open Questions
Unresolved issues

## Related Proposals
Cross-references

## Success Criteria
How do we know it works?
```

3. Update this index with:
   - Link to your proposal
   - Status and priority
   - Dependencies
   - Brief description

4. Update relationship diagram if applicable

### Reviewing Proposals

When reviewing, consider:
- Is the motivation clear?
- Are edge cases addressed?
- What's the complexity/benefit tradeoff?
- How does it interact with other proposals?
- Is it testable?
- What are the security implications?

## Historical Proposals

None yet - all proposals are active.

## Rejected Proposals

None yet.

## Questions?

See [AGENTS.md](../../AGENTS.md) for general coding guidelines and [doc/arch.md](../arch.md) for project architecture overview.
