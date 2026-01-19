# iron

> Peer-to-peer network interface based on iroh

**iron** creates a virtual IPv6 network over peer-to-peer QUIC connections, enabling direct connectivity between endpoints using `.iron` DNS names.

## Features

- 🌐 **Virtual IPv6 Network**: Each peer gets a unique IPv6 address in the `fd69:726f::/32` space
- 🔒 **Encrypted P2P**: All traffic encrypted via iroh's QUIC protocol
- 🏷️ **DNS Resolution**: `.iron` domain names resolve to peer IPv6 addresses
- 🔌 **TUN Interface**: Standard network interface, works with any application
- 🚀 **NAT Traversal**: Automatic hole punching with relay fallback
- 📡 **Direct Connections**: Establishes direct peer connections when possible

## Architecture

```
┌─────────────┐
│ Application │  (browser, curl, any network app)
└──────┬──────┘
       │ Standard IPv6 socket
┌──────▼────────────────────────────────┐
│ Operating System (IPv6 stack)         │
└──────┬────────────────────────────────┘
       │ fd69:726f::/32 routes to TUN
┌──────▼────────────────────────────────┐
│ iron (TUN Interface)                  │
│  • DNS: .iron → IPv6                  │
│  • Registry: IPv6 ↔ EndpointId        │
│  • Protocol: Packets over QUIC        │
└──────┬────────────────────────────────┘
       │ Encrypted QUIC (iroh)
       ▼
  Internet / LAN
       ▼
┌────────────────────────────────────────┐
│ Peer iron node                         │
└────────────────────────────────────────┘
```

## Installation

### Prerequisites

- Rust 1.70+ (2024 edition)
- Root/sudo privileges (required for TUN device creation)

### Build from Source

```bash
git clone https://github.com/yourusername/iron.git
cd iron
cargo build --release
```

The binary will be at `target/release/iron`.

## Usage

### Starting iron

**Basic usage (requires root):**
```bash
sudo iron
```

**With custom log level:**
```bash
sudo iron --log-level debug
```

**With custom DNS port:**
```bash
sudo iron --dns-port 5353
```

When iron starts, you'll see:
```
Node ID (hex):    74df87cccf7e0fead1370fc39f65be3de44f5069f5db87f3b08435ccdaf3b5b9
Node ID (base32): ot36ptgm67yp5vjt6b6dtz2l4ppejtggt5w3y64lqqrvztpl2wnq
DNS name:         ot36ptgm67yp5vjt6b6dtz2l4ppejtggt5w3y64lqqrvztpl2wnq.iron
```

The **base32 Node ID** is what you use for DNS queries.

### Configuring DNS Resolution

To resolve `.iron` domains, you need to configure your system to query iron's DNS server.

**See [DNS Setup Guide](doc/dns-setup.md) for detailed instructions.**

**Quick options:**
- **Testing:** Use `dig @127.0.0.1 -p 5333 <node-id>.iron AAAA`
- **macOS:** `/etc/resolver/iron` method (see guide)
- **Linux:** systemd-resolved configuration (see guide)
- **Advanced:** dnsmasq forwarding (see guide)

We provide multiple methods to accommodate different setups (VPNs, Tailscale, etc.).

### Command Line Options

```
Usage: iron [OPTIONS]

Options:
  -l, --log-level <LOG_LEVEL>  Set the log level [default: info]
                               (trace, debug, info, warn, error)
      --dns-port <DNS_PORT>    DNS server port [default: 5333]
  -h, --help                   Print help
  -V, --version                Print version
```

### Environment Variables

- `RUST_LOG`: Control log levels per module (overrides `--log-level`)
  ```bash
  RUST_LOG=iron::protocol=trace,iron=info sudo iron
  ```

## Connecting Two Nodes

### Prerequisites
- Two machines with iron installed
- Both machines can reach each other (same network, or internet with NAT traversal)
- DNS configured on at least one machine (see [DNS Setup Guide](doc/dns-setup.md))

### Step 1: Start iron on both machines

**Machine A:**
```bash
sudo iron
# Note the base32 Node ID displayed
```

**Machine B:**
```bash
sudo iron
# Note the base32 Node ID displayed
```

### Step 2: Configure DNS (on Machine B)

Choose a DNS configuration method from [doc/dns-setup.md](doc/dns-setup.md).

**Quick test without DNS configuration:**
```bash
# On Machine B, resolve Machine A's Node ID manually
dig @127.0.0.1 -p 5333 <MACHINE_A_BASE32_ID>.iron AAAA
```

### Step 3: Connect from Machine B to Machine A

**Test DNS resolution:**
```bash
dig <MACHINE_A_BASE32_ID>.iron AAAA
```

**Ping Machine A:**
```bash
ping6 <MACHINE_A_BASE32_ID>.iron
```

**Run a service on Machine A and access it from Machine B:**

```bash
# On Machine A - start HTTP server
python3 -m http.server 8080 --bind ::

# On Machine B - access the server
curl http://[<MACHINE_A_BASE32_ID>.iron]:8080/
```

If you see Machine A's directory listing, it works! 🎉

### Troubleshooting

See [DNS Setup Guide](doc/dns-setup.md#troubleshooting) for DNS issues.

For P2P connection issues:
- Check both nodes show "TUN interface running" in logs
- Verify iroh endpoint is initialized on both
- Check firewalls allow UDP (QUIC uses UDP)
- Watch logs with `--log-level debug` to see connection attempts

## How It Works

### DNS Resolution

When you access `<endpoint_id>.iron`:

1. DNS query sent to iron's resolver (port 5333)
2. EndpointId parsed from base32-encoded domain
3. IPv6 address derived: `fd69:726f::xxxx:xxxx:xxxx:xxxx`
4. Application connects to IPv6 address

### Packet Flow (Send)

1. Application sends to IPv6 address
2. OS routes to TUN interface (iron)
3. iron looks up EndpointId from IPv6
4. Packet sent to peer via iroh QUIC connection
5. NAT traversal handled automatically

### Packet Flow (Receive)

1. Peer sends packet via iroh
2. iron receives on QUIC stream
3. Source address verified (anti-spoofing)
4. Packet written to TUN interface
5. OS routes to listening application

### IPv6 Address Derivation

Each EndpointId (32 bytes) maps to a unique IPv6:

```
EndpointId: 197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61
                                                    └──────────┘
                                                    Last 8 bytes
                                                         ▼
IPv6: fd69:726f:0000:0000:039b:fa8b:3d36:8b3d
      └─ULA prefix─┘           └─from EndpointId─┘
```

## Troubleshooting

### "ERROR: iron must be run as root"

TUN device creation requires elevated privileges. Use `sudo`:
```bash
sudo iron
```

### "Failed to create TUN device"

**macOS:**
- Ensure you have permission to create TUN devices
- Check system integrity protection settings

**Linux:**
- Verify TUN kernel module is loaded: `lsmod | grep tun`
- Load if needed: `sudo modprobe tun`

### DNS Not Resolving

See the comprehensive [DNS Setup Guide](doc/dns-setup.md) for configuration options and troubleshooting.

**Quick checks:**
1. Verify iron is running: `sudo lsof -i :5333`
2. Test DNS directly: `dig @127.0.0.1 -p 5333 <node-id>.iron AAAA`
3. Verify you're using the **base32** Node ID (52 chars), not hex (64 chars)
4. Check DNS configuration method from [doc/dns-setup.md](doc/dns-setup.md)

### "I can't ping myself" / "Loopback detected"

**This is expected behavior.** iron is a P2P network - you cannot connect to yourself.

Self-ping requires protocol-specific packet rewriting (ICMP echo reply, TCP handshake, etc.) which would add unnecessary complexity for a feature that doesn't test real P2P connectivity.

**Solution:** Use two separate machines/nodes for testing. See [Testing Limitations](doc/testing-limitations.md) for details.

### No Connection to Peer

1. **Verify Node ID**: Ensure you're using the correct EndpointId
2. **Check logs**: Look for connection attempts with `--log-level debug`
3. **Firewall**: Ensure UDP traffic is allowed (iroh uses QUIC over UDP)
4. **Relay server**: Check if iroh can reach relay servers

### Performance Issues

1. **Direct connection**: Check if direct connection established (vs relay)
   ```
   RUST_LOG=iron::protocol=debug sudo iron
   ```

2. **MTU**: Verify MTU is set correctly (default 1420)

3. **Network congestion**: Monitor with `--log-level trace` (very verbose)

## Log Levels

- `error`: Only critical failures
- `warn`: Recoverable issues (failed sends, unknown destinations)
- `info`: High-level events (startup, connections, shutdown)
- `debug`: Packet flow, DNS queries, mappings
- `trace`: Very detailed (stream operations, individual packets)

**Example per-module filtering:**
```bash
RUST_LOG=iron::dns=debug,iron::tun=trace,iron=info sudo iron
```

## Development

### Running Tests

```bash
cargo test
```

All 30 tests should pass (unit + integration tests).

### Helper Scripts

Located in `scripts/`:
- `node-id-to-dns.sh` - Convert hex Node ID to base32 DNS name
- `test-dns.sh` - Test DNS resolution interactively
- `test-interactive.sh` - Comprehensive interactive tests

### Building Documentation

```bash
cargo doc --open --no-deps
```

### Code Structure

```
iron/
├── src/
│   ├── lib.rs           # Library exports
│   ├── mapping.rs       # EndpointId ↔ IPv6 registry
│   ├── dns.rs           # DNS resolver for .iron
│   ├── tun.rs           # TUN interface packet handling
│   ├── protocol.rs      # Iroh QUIC packet transport
│   ├── node.rs          # Component orchestration
│   └── bin/
│       └── iron.rs      # Main binary entry point
├── tests/
│   └── integration.rs   # Integration tests
└── doc/
    ├── arch.md          # Architecture decisions
    ├── plan.md          # Implementation plan
    ├── packet-flow.md   # Detailed packet flow
    └── networking.md    # Network specifications
```

## Technical Details

### Components

- **Registry** (`mapping.rs`): Bidirectional EndpointId ↔ IPv6 mapping
- **DNS Resolver** (`dns.rs`): Hickory-server based resolver for `.iron` domains
- **TUN Interface** (`tun.rs`): Virtual network device for packet interception
- **Protocol Handler** (`protocol.rs`): Iroh QUIC transport for packets
- **Orchestrator** (`node.rs`): Component lifecycle management

### Specifications

- **IPv6 ULA Prefix**: `fd69:726f::/32` (iron-branded)
- **MTU**: 1420 bytes (accounts for QUIC overhead)
- **ALPN**: `iron/packet/0` (protocol identifier)
- **DNS Encoding**: Base32 (no padding), 52 characters
- **Platform**: macOS (utun), Linux (iron0)

### Security

- **Encryption**: All traffic encrypted via iroh's QUIC (TLS 1.3)
- **Authentication**: Public key cryptography (EndpointId = PublicKey)
- **Source Verification**: Prevents IP spoofing between peers
- **NAT Traversal**: Secure hole punching with relay fallback

## License

MIT OR Apache-2.0

## Contributing

Contributions welcome! Please follow the coding guidelines in `AGENTS.md`.

## Acknowledgments

Built on [iroh](https://iroh.computer) - a Rust library for peer-to-peer networking.
