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

### Step 1: Start Node A

```bash
# On machine A
sudo iron
```

Output will show:
```
Node ID: df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq
DNS server will run on: 127.0.0.1:5333
```

### Step 2: Get Node A's Information

The Node ID is the base32-encoded EndpointId. This is what peers need to connect.

### Step 3: Start Node B

```bash
# On machine B
sudo iron
```

### Step 4: Connect from Node B to Node A

First, configure your system to use iron's DNS resolver:

**macOS/Linux:**
```bash
# Add to /etc/resolv.conf (or via network settings)
nameserver 127.0.0.1
port 5333
```

Then, use the `.iron` domain to connect:

```bash
# Node A's domain = <Node_A_ID>.iron
ping6 df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron
```

or

```bash
curl -6 http://[df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron]/
```

### Step 5: Verify Connection

You should see:
- DNS resolution to an IPv6 address (e.g., `fd69:726f::1234:5678`)
- Packet flow in the logs (with debug level)
- Direct connection established via iroh

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

1. Verify iron is running and DNS server started:
   ```
   INFO DNS server listening on 127.0.0.1:5333
   ```

2. Test DNS directly:
   ```bash
   dig @127.0.0.1 -p 5333 <endpoint_id>.iron AAAA
   ```

3. Check system DNS configuration points to 127.0.0.1:5333

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
