# Manual Testing Guide for iron

## ✅ Basic Functionality Tests (Single Node)

### Test 1: Verify TUN Interface

```bash
# In another terminal while iron is running
ifconfig utun13  # Use the actual interface name from logs

# Expected output:
# utun13: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1420
#     inet6 fd69:726f::1 prefixlen 32
```

### Test 2: Check Routing Table

```bash
# Verify the route was added
netstat -rn -f inet6 | grep fd69:726f

# Expected output:
# fd69:726f::/32         link#XX            UGSc           utun13
```

### Test 3: Test DNS Resolution

```bash
# Get the Node ID from iron's startup logs (it's displayed as "Node ID: ...")
# Example: 96bf76b94c2b7b0d5f2965bd11c45fe90de02e0441509a4e2a9ea8e00a7dfef6

# Test DNS resolution (replace with your actual Node ID)
dig @127.0.0.1 -p 5333 96bf76b94c2b7b0d5f2965bd11c45fe90de02e0441509a4e2a9ea8e00a7dfef6.iron AAAA

# Expected output should include:
# ;; ANSWER SECTION:
# 96bf76b94c2b7b0d5f2965bd11c45fe90de02e0441509a4e2a9ea8e00a7dfef6.iron. 300 IN AAAA fd69:726f::xxxx:xxxx:xxxx:xxxx
```

### Test 4: Ping the Local TUN Interface

```bash
# Ping the TUN interface's own address
ping6 -c 3 fd69:726f::1

# Expected: Should respond (you're pinging yourself via the TUN interface)
```

### Test 5: Test Self-Connection

```bash
# Get your own derived IPv6 address from DNS
NODE_ID="96bf76b94c2b7b0d5f2965bd11c45fe90de02e0441509a4e2a9ea8e00a7dfef6"  # Replace with your actual Node ID
MY_IPV6=$(dig @127.0.0.1 -p 5333 +short $NODE_ID.iron AAAA)

echo "My IPv6: $MY_IPV6"

# Try to ping your own derived address
ping6 -c 3 $MY_IPV6
```

Watch the iron debug logs - you should see packet activity!

---

## 🔗 Two-Node Testing (Real P2P)

This requires two separate machines (or VMs).

### Setup: Machine A

```bash
# Terminal 1 on Machine A
sudo ./target/release/iron --log-level debug

# Note the Node ID displayed, for example:
# Node ID: 96bf76b94c2b7b0d5f2965bd11c45fe90de02e0441509a4e2a9ea8e00a7dfef6
```

Save this Node ID - you'll need it on Machine B.

### Setup: Machine B

```bash
# Terminal 1 on Machine B
sudo ./target/release/iron --log-level debug

# Note this Node ID too
```

### Test 1: DNS Resolution from B to A

```bash
# On Machine B, test if you can resolve Machine A's Node ID
dig @127.0.0.1 -p 5333 <MACHINE_A_NODE_ID>.iron AAAA

# Should return Machine A's IPv6 address (fd69:726f::xxxx)
```

### Test 2: Attempt Ping from B to A

```bash
# On Machine B
ping6 <MACHINE_A_NODE_ID>.iron

# Watch the logs on both machines for:
# - Machine B: Should show "Attempting to connect to endpoint..."
# - Machine A: Should show "Accepting connection from..."
```

**Note:** For this to work, iroh needs to establish a connection. This might require:
- Both machines on the same network, OR
- iroh's relay servers (should work automatically), OR
- NAT traversal (automatic with iroh)

### Test 3: HTTP Server Test

**On Machine A:**
```bash
# Terminal 2 (while iron is running in Terminal 1)
# Start a simple HTTP server
python3 -m http.server 8080 --bind ::
```

**On Machine B:**
```bash
# Try to access Machine A's web server via iron
NODE_A_ID="<MACHINE_A_NODE_ID>"  # Replace with actual ID
curl -6 "http://[$NODE_A_ID.iron]:8080/"

# Or with the resolved IPv6:
MACHINE_A_IPV6=$(dig @127.0.0.1 -p 5333 +short $NODE_A_ID.iron AAAA)
curl -6 "http://[$MACHINE_A_IPV6]:8080/"
```

If successful, you should see the directory listing from Machine A!

---

## 🐛 Advanced Debugging Tests

### Test 1: Packet Capture

```bash
# In another terminal while iron is running
sudo tcpdump -i utun13 -n -vv

# Generate some traffic and watch packets fly by
```

### Test 2: Send Test Packet

```bash
# Install netcat6 if needed
# brew install netcat  (macOS)

# Terminal 1: Start a UDP listener
nc -6 -u -l 9999

# Terminal 2: Send data to your own iron address
NODE_ID="<YOUR_NODE_ID>"
echo "Hello iron!" | nc -6 -u $NODE_ID.iron 9999
```

Watch the iron logs for packet processing!

### Test 3: Verify Registry Mapping

The registry should automatically map Node IDs to IPv6 addresses. Check the logs for:

```
DEBUG iron::mapping: Registering endpoint ... → fd69:726f::...
```

### Test 4: Stress Test (Multiple Connections)

```bash
# On Machine B, try multiple simultaneous pings to Machine A
for i in {1..5}; do
    ping6 -c 10 <MACHINE_A_NODE_ID>.iron &
done
wait
```

Watch both machines' logs for connection handling.

---

## 📊 What to Look For in Logs

### Successful TUN Device Creation
```
INFO iron::tun: TUN device created: utun13
INFO iron::tun: IPv6 address configured: fd69:726f::1/32 on utun13
INFO iron::tun: IPv6 route added: fd69:726f::/32 → utun13
INFO iron::tun: TUN interface running, ready to process packets
```

### Successful DNS Query
```
DEBUG iron::dns: Received query for: <node_id>.iron
DEBUG iron::mapping: Registering endpoint ... → fd69:726f::...
```

### Outbound Packet (OS → Network)
```
DEBUG iron::tun: TUN received OS→Network: <src> -> <dst>, X bytes
DEBUG iron::mapping: Endpoint lookup: fd69:726f::... → <endpoint_id>
DEBUG iron::protocol: Sending packet to endpoint <id> (X bytes)
```

### Inbound Connection
```
INFO iron::protocol: Accepted connection from endpoint <id>
DEBUG iron::protocol: Received packet from <endpoint_id> (X bytes)
```

### Normal Warnings to Ignore
```
WARN iron::tun: No EndpointId found for destination ff02::16, dropping packet
```
This is IPv6 multicast traffic (MLD) - totally normal and safe to drop.

---

## 🔧 Troubleshooting

### "No EndpointId found for destination"

**If it's `ff02::...`**: Normal IPv6 multicast, ignore it.

**If it's `fd69:726f::...`**: The destination wasn't registered yet.
- Make sure you did a DNS lookup first: `dig @127.0.0.1 -p 5333 <node_id>.iron AAAA`
- The DNS lookup registers the endpoint in the registry

### No Connection Between Peers

1. **Check iroh logs** for connection attempts
2. **Verify both nodes can reach relay servers**
3. **Check firewalls** - iroh uses UDP for QUIC
4. **Try on same LAN first** - simpler than NAT traversal testing

### DNS Not Resolving

```bash
# Verify DNS server is running
lsof -i :5333

# Should show the iron process listening on port 5333

# Test with verbose output
dig @127.0.0.1 -p 5333 +trace <node_id>.iron AAAA
```

---

## 🎯 Success Criteria

### ✅ Basic Tests (Single Node)
- [ ] TUN interface created and shows `fd69:726f::1`
- [ ] Route added to routing table
- [ ] DNS resolves `.iron` domains to IPv6 addresses
- [ ] Can ping `fd69:726f::1`
- [ ] Logs show packet activity

### ✅ Advanced Tests (Two Nodes)
- [ ] DNS resolves peer Node IDs to IPv6
- [ ] Ping reaches the peer (via iron logs)
- [ ] HTTP/TCP traffic flows between nodes
- [ ] Multiple simultaneous connections work
- [ ] No crashes or errors in logs

---

## 📝 Next Steps After Testing

1. **Document any issues** you find
2. **Measure performance** (latency, throughput)
3. **Test on different networks** (same LAN, different LANs, behind NAT)
4. **Try real applications** (SSH, HTTP server, etc.)

---

## 🚀 Quick Test Script

Save this as `quick-test.sh`:

```bash
#!/bin/bash
set -e

echo "=== Quick iron Test ==="
echo ""

# Get Node ID from iron logs (assumes iron is running)
NODE_ID=$(sudo lsof -i :5333 | grep iron | head -1 | awk '{print $2}')

if [ -z "$NODE_ID" ]; then
    echo "❌ iron is not running on port 5333"
    exit 1
fi

echo "✓ iron is running (PID: $NODE_ID)"

# Check TUN interface
TUN_DEV=$(ifconfig | grep utun | tail -1 | awk '{print $1}' | sed 's/://')
if [ -z "$TUN_DEV" ]; then
    echo "❌ No TUN device found"
    exit 1
fi

echo "✓ TUN device: $TUN_DEV"

# Check IPv6 address
IPV6=$(ifconfig $TUN_DEV | grep "inet6 fd69:726f" | awk '{print $2}')
if [ -z "$IPV6" ]; then
    echo "❌ No IPv6 address on TUN device"
    exit 1
fi

echo "✓ IPv6 address: $IPV6"

# Check route
ROUTE=$(netstat -rn -f inet6 | grep "fd69:726f::/32")
if [ -z "$ROUTE" ]; then
    echo "⚠️  No route found (might still work)"
else
    echo "✓ Route configured"
fi

# Test DNS (you need to provide a valid Node ID here)
echo ""
echo "To test DNS, run:"
echo "  dig @127.0.0.1 -p 5333 <NODE_ID>.iron AAAA"
echo ""
echo "=== All basic checks passed! ==="
```

Run it with: `./quick-test.sh`
