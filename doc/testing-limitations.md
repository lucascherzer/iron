# Testing Limitations and Requirements

## Single-Node Testing ✅ (What Works)

With a single iron node, you can test:

### ✅ Component Functionality
- TUN device creation
- IPv6 address configuration  
- Routing table setup
- DNS server startup
- DNS resolution (`.iron` domains)
- Packet capture from OS
- Registry lookups (IPv6 ↔ EndpointId)
- Channel-based communication

### ✅ Verification Steps

```bash
# 1. Start iron
sudo iron --log-level debug

# 2. Verify TUN interface
ifconfig utun13

# 3. Test DNS resolution
dig @127.0.0.1 -p 5333 <your-base32-node-id>.iron AAAA

# 4. Check packet capture
# The logs should show packets being captured from the TUN interface
# when you try to ping your own address
```

---

## What REQUIRES Two Nodes ❌

### ❌ Loopback / Self-Ping Does NOT Work

**You cannot ping yourself** through iron. This is **by design**, not a bug.

**Why?**
1. **P2P requires two endpoints** - iroh cannot connect to itself
2. **Protocol-specific logic needed** - Loopback would require:
   - ICMP echo request → echo reply conversion
   - TCP SYN → SYN-ACK handshake simulation
   - UDP datagram reflection with modified headers
   - Every protocol needs custom handling
3. **Not the real use case** - iron is for connecting to OTHER nodes

**What you'll see when trying to ping yourself:**
```
DEBUG iron::protocol: Loopback detected: cannot connect to self (P2P requires two nodes)
```

This is **correct behavior**. The packet is intentionally dropped.

### ❌ Features Requiring Two Nodes

- **P2P connectivity** - Establishing iroh connections
- **Packet delivery** - End-to-end packet transmission
- **NAT traversal** - Testing hole punching
- **Real applications** - HTTP, SSH, file transfer, etc.
- **Performance testing** - Latency, throughput measurements
- **Connection recovery** - Reconnection after network changes

---

## Two-Node Testing ✅ (Real Testing)

To properly test iron, you need **two separate machines** (or VMs).

### Setup

**Machine A:**
```bash
sudo iron
# Note the base32 Node ID
```

**Machine B:**
```bash
sudo iron
# Note the base32 Node ID
```

### Testing Connectivity

**From Machine B → Machine A:**

```bash
# 1. Test DNS resolution
dig @127.0.0.1 -p 5333 <MACHINE_A_NODE_ID>.iron AAAA

# 2. Ping Machine A
ping6 <MACHINE_A_IPV6_ADDRESS>

# 3. Run service on Machine A
# (on Machine A)
python3 -m http.server 8080 --bind ::

# 4. Access from Machine B
curl http://[<MACHINE_A_IPV6>]:8080/
```

### Expected Results

With two nodes, you should see:

```
# Machine B logs:
DEBUG iron::protocol: Sending packet to <machine-a-endpoint-id>
DEBUG iron::protocol: Successfully sent packet to <machine-a-endpoint-id>

# Machine A logs:
INFO iron::protocol: Accepted connection from endpoint <machine-b-endpoint-id>
DEBUG iron::protocol: Received packet from <machine-b-endpoint-id>
DEBUG iron::tun: Received packet from network, writing to TUN
```

---

## Testing Strategies

### Development / Quick Checks (1 Node)
✅ Test that iron starts correctly  
✅ Verify TUN and DNS components work  
✅ Check logs for errors  
✅ Validate configuration

### Integration / Real Testing (2+ Nodes)
✅ Test P2P connectivity  
✅ Verify packet delivery  
✅ Test with real applications  
✅ Measure performance  
✅ Test NAT traversal  

### Continuous Integration (Automated)
✅ Unit tests (30 tests)  
✅ Integration tests (component interaction)  
❌ E2E tests (require multiple machines - manual only)

---

## Common Testing Scenarios

### Scenario 1: "Does iron work?"

**Single Node Test:**
```bash
sudo iron --log-level debug
# Look for:
# - "TUN device created"
# - "IPv6 address configured"
# - "DNS server listening"
# - No errors in logs
```

**Verdict:** Components work ✅

### Scenario 2: "Can I connect to peers?"

**Requires:** Two nodes on the network

**Test:**
```bash
# Node A: Start iron
# Node B: Start iron
# Node B: ping6 <node-a-ipv6>
```

**Verdict:** P2P connectivity works ✅

### Scenario 3: "Can applications use iron?"

**Requires:** Two nodes + real application

**Test:**
```bash
# Node A: Run HTTP server
# Node B: curl to Node A via iron
```

**Verdict:** Application-level traffic works ✅

---

## Why Self-Ping Doesn't Work (Technical)

### The Problem

When you ping yourself:
```
ping6 fd69:726f::5893:a44a:ffa4:309b
```

1. OS sends ICMP Echo Request packet to TUN
2. iron sees destination = your own EndpointId
3. iron cannot establish QUIC connection to itself (iroh limitation)
4. Packet is dropped

### Why We Don't Fix It

To make self-ping work, we'd need:

```rust
// ICMP Echo Request → Echo Reply
if packet.is_icmp_echo_request() {
    packet.swap_src_dst();
    packet.change_icmp_type(ECHO_REPLY);
    send_to_tun(packet);
}

// TCP SYN → SYN-ACK
if packet.is_tcp_syn() {
    packet.swap_src_dst();
    packet.create_syn_ack();
    send_to_tun(packet);
    // ... and then handle the full TCP handshake ...
}

// UDP
if packet.is_udp() {
    packet.swap_src_dst();
    // But UDP is connectionless - what response to send?
}

// And so on for every protocol...
```

This is:
- ❌ Complex
- ❌ Error-prone
- ❌ Protocol-specific
- ❌ Not the purpose of iron
- ❌ Doesn't test real P2P anyway

### The Right Approach

**Use two nodes for real testing.** That's what iron is designed for.

---

## Troubleshooting

### "I can't ping myself"
→ **Expected behavior**. Use two nodes for ping testing.

### "Logs show 'Loopback detected'"
→ **Correct**. The packet is being properly detected and dropped.

### "All my packets fail with 'cannot connect to self'"
→ Check if you're trying to connect to your own Node ID. Use a different peer's Node ID.

### "How do I test without a second machine?"
→ Use a VM, Docker container, or cloud instance as the second node.

---

## Summary

| Test Type | Nodes Required | What It Tests |
|-----------|---------------|---------------|
| Component functionality | 1 | TUN, DNS, Registry, Logging |
| DNS resolution | 1 | DNS server, base32 encoding |
| Packet capture | 1 | TUN packet reading |
| P2P connectivity | 2+ | iroh connections, packet delivery |
| Application traffic | 2+ | Real-world usage |
| Performance | 2+ | Latency, throughput |

**Bottom line:** iron works, but **P2P requires peers** - you need at least two nodes for real testing.
