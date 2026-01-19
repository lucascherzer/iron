# DNS Configuration for .iron Domains

iron runs a DNS server on `127.0.0.1:5333` that resolves `.iron` domains to IPv6 addresses. To use `.iron` domains, you need to configure your system to query iron's DNS server.

**We provide multiple methods - choose what works best for your setup.**

---

## Method 1: Per-Application DNS (Recommended for Testing)

Use DNS directly in applications without modifying system DNS.

### dig (DNS testing)
```bash
dig @127.0.0.1 -p 5333 <node-id>.iron AAAA
```

### curl
```bash
curl --dns-servers 127.0.0.1:5333 http://[<node-id>.iron]:8080/
```

### ping6
```bash
# First resolve manually
IPV6=$(dig @127.0.0.1 -p 5333 +short <node-id>.iron AAAA)
ping6 $IPV6
```

### Custom applications
Most applications that support custom DNS servers can be configured to use `127.0.0.1:5333`.

**Pros:**
- ✅ No system-wide changes
- ✅ Doesn't interfere with other DNS configurations
- ✅ Easy to test and debug

**Cons:**
- ❌ Must specify DNS server per application
- ❌ Not all applications support custom DNS

---

## Method 2: systemd-resolved (Linux)

If you use systemd-resolved (most modern Linux distros), add iron as a DNS server for `.iron` domains.

### Configuration

```bash
# Create drop-in configuration
sudo mkdir -p /etc/systemd/resolved.conf.d/
sudo tee /etc/systemd/resolved.conf.d/iron.conf <<EOF
[Resolve]
DNS=127.0.0.1:5333
Domains=~iron
EOF

# Restart systemd-resolved
sudo systemctl restart systemd-resolved

# Verify
resolvectl status
```

### Testing

```bash
# Should now work without specifying DNS server
dig <node-id>.iron AAAA
ping6 <node-id>.iron
```

### Cleanup

```bash
sudo rm /etc/systemd/resolved.conf.d/iron.conf
sudo systemctl restart systemd-resolved
```

**Pros:**
- ✅ Works system-wide
- ✅ Only routes `.iron` queries to iron
- ✅ Coexists with other DNS configurations (Tailscale, VPN, etc.)

**Cons:**
- ❌ Requires systemd-resolved
- ❌ Requires root to configure

---

## Method 3: dnsmasq (Advanced)

Use dnsmasq to forward `.iron` queries to iron while keeping other DNS unchanged.

### Installation

```bash
# macOS
brew install dnsmasq

# Ubuntu/Debian
sudo apt install dnsmasq

# Arch
sudo pacman -S dnsmasq
```

### Configuration

```bash
# Add iron DNS configuration
echo "server=/iron/127.0.0.1#5333" | sudo tee -a /etc/dnsmasq.conf

# Restart dnsmasq
sudo systemctl restart dnsmasq  # Linux
sudo brew services restart dnsmasq  # macOS
```

### System DNS Setup

Configure your system to use dnsmasq as primary DNS (usually `127.0.0.1:53`).

**Linux (systemd-resolved):**
```bash
sudo systemctl disable systemd-resolved
sudo systemctl stop systemd-resolved
echo "nameserver 127.0.0.1" | sudo tee /etc/resolv.conf
```

**macOS:**
```
System Settings → Network → [Interface] → DNS Servers → Add 127.0.0.1
```

**Pros:**
- ✅ Works system-wide transparently
- ✅ Coexists with other local DNS services
- ✅ Can handle complex routing rules

**Cons:**
- ❌ Requires installing and configuring dnsmasq
- ❌ More complex setup
- ❌ May conflict with VPNs or other DNS managers

---

## Method 4: /etc/hosts (Quick & Dirty)

For a small number of known peers, add static entries.

### Configuration

```bash
# Get the IPv6 address for a peer
IPV6=$(dig @127.0.0.1 -p 5333 +short <node-id>.iron AAAA)

# Add to /etc/hosts (choose a friendly name)
echo "$IPV6 peer-alice.iron" | sudo tee -a /etc/hosts

# Now you can use the friendly name
ping6 peer-alice.iron
```

**Pros:**
- ✅ Simple and reliable
- ✅ No daemon required
- ✅ Works everywhere

**Cons:**
- ❌ Manual entry for each peer
- ❌ Doesn't work for dynamic discovery
- ❌ Must update if IPv6 mappings change

---

## Method 5: macOS Resolver Directory (macOS only)

Use macOS's built-in domain-specific DNS resolution.

### Configuration

```bash
# Create resolver directory
sudo mkdir -p /etc/resolver

# Configure .iron domain
sudo tee /etc/resolver/iron <<EOF
nameserver 127.0.0.1
port 5333
EOF

# Test
scutil --dns | grep iron
```

### Testing

```bash
# Should work without specifying DNS server
dig <node-id>.iron AAAA
ping6 <node-id>.iron
```

### Cleanup

```bash
sudo rm /etc/resolver/iron
```

**Pros:**
- ✅ Works system-wide on macOS
- ✅ Only affects `.iron` domains
- ✅ Coexists with VPNs and other DNS (including Tailscale MagicDNS!)

**Cons:**
- ❌ macOS only
- ❌ Requires root to configure

---

## Method 6: Custom Resolver Library (Advanced)

For programmatic access, use a custom resolver library in your application.

### Example (Rust with trust-dns)

```rust
use trust_dns_resolver::config::*;
use trust_dns_resolver::TokioAsyncResolver;

let mut config = ResolverConfig::new();
config.add_name_server(NameServerConfig {
    socket_addr: "127.0.0.1:5333".parse().unwrap(),
    protocol: Protocol::Udp,
    tls_dns_name: None,
    trust_nx_responses: false,
});

let resolver = TokioAsyncResolver::tokio(config, ResolverOpts::default())?;
let response = resolver.lookup_ip("<node-id>.iron").await?;
```

**Pros:**
- ✅ Full control in application
- ✅ No system changes required
- ✅ Portable across platforms

**Cons:**
- ❌ Requires code changes
- ❌ Application-specific

---

## Recommendations by Use Case

### Testing / Development
→ **Method 1 (Per-Application)** - Quick and isolated

### Linux Desktop / Server
→ **Method 2 (systemd-resolved)** - Clean and integrated

### macOS Desktop
→ **Method 5 (Resolver Directory)** - Native and coexists with everything

### Advanced / Complex Setups
→ **Method 3 (dnsmasq)** - Maximum flexibility

### Static Known Peers
→ **Method 4 (/etc/hosts)** - Simple and reliable

### Programmatic Use
→ **Method 6 (Custom Resolver)** - Full control

---

## Verifying DNS Configuration

After configuring DNS, verify it works:

```bash
# Test 1: Direct query (should always work)
dig @127.0.0.1 -p 5333 <node-id>.iron AAAA

# Test 2: System-wide query (tests your DNS configuration)
dig <node-id>.iron AAAA

# Test 3: Actual connectivity
ping6 <node-id>.iron
```

---

## Troubleshooting

### DNS queries return NXDOMAIN
- Check iron is running: `lsof -i :5333`
- Test direct query: `dig @127.0.0.1 -p 5333 <node-id>.iron AAAA`
- Verify Node ID is in base32 format (52 chars, lowercase)

### DNS works but ping fails
- Verify IPv6 is enabled: `sysctl net.ipv6.conf.all.disable_ipv6`
- Check routing: `ip -6 route` (Linux) or `netstat -rn -f inet6` (macOS)
- Verify TUN interface: `ifconfig | grep utun`

### Conflicts with VPN/Tailscale
- Use **Method 2** (systemd-resolved) or **Method 5** (macOS resolver) - they coexist well
- These methods only route `.iron` queries to iron, everything else uses normal DNS

### "Port 5333 already in use"
- Another iron instance is running: `pkill iron`
- Another service on 5333: `lsof -i :5333`
- Change iron's DNS port: `iron --dns-port 5353`

---

## Security Considerations

### Listen Address
By default, iron's DNS server listens on `127.0.0.1:5333` (localhost only).

**Do NOT expose to network** unless you understand the implications:
- Anyone who can query your DNS can discover which peers you know
- DNS queries are unauthenticated

If you need network access, use firewall rules to restrict access.

### DNS Spoofing
iron's DNS server is only accessible locally by default, mitigating DNS spoofing risks. The actual P2P connection uses iroh's encrypted QUIC with endpoint ID verification.

---

## Integration with Other Tools

### Tailscale MagicDNS
✅ Compatible - Use **Method 2** or **Method 5** to coexist

### Local VPN
✅ Compatible - Domain-specific methods (2, 5) coexist with VPN DNS

### Pi-hole
✅ Compatible - Configure Pi-hole to forward `.iron` to `127.0.0.1:5333`

### Corporate DNS
✅ Compatible - Use per-application DNS (Method 1) or domain-specific routing

---

## Example Workflow

**1. Start iron**
```bash
sudo iron
```

**2. Note your Node ID (base32 format)**
```
Node ID (base32): ot36ptgm67yp5vjt6b6dtz2l4ppejtggt5w3y64lqqrvztpl2wnq
DNS name:         ot36ptgm67yp5vjt6b6dtz2l4ppejtggt5w3y64lqqrvztpl2wnq.iron
```

**3. Configure DNS (choose one method above)**

**4. Test**
```bash
dig ot36ptgm67yp5kjt6b6dtz2l4ppejtggt5w3y64lqqrvztpl2wnq.iron AAAA
```

**5. Connect to peer**
```bash
# Get peer's Node ID (they share their base32 Node ID with you)
ping6 <peer-node-id>.iron
```

Done! 🎉
