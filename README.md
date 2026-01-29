# iron

> Peer-to-peer network interface based on iroh

**iron** creates a virtual IPv6 network over peer-to-peer QUIC connections, enabling direct connectivity between endpoints using `.iron` DNS names.

## Features

- 🌐 **Virtual Overlay Network**: Each peer is addressed by their public key
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

## Building

Prerequisites:
- nix

```bash
# build for your system
nix build .#default
# build for an explicit target
nix build .#packages.x86_64-linux.default
```

The binary will be at `result/bin/iron`.

## Testing

```bash
nix flake check
```

## Usage

### Starting iron

**Basic usage (requires root):**
```bash
sudo iron serve
```

On first run, iron will automatically configure DNS for `.iron` domains (macOS and Linux with systemd-resolved only). This configuration only affects `.iron` domains - all other DNS resolution remains unchanged.

**With custom log level:**
```bash
sudo iron serve --log-level debug
```

**With custom DNS port:**
```bash
sudo iron serve --dns-port 5353
```

When iron starts, you'll see:
```
Node ID (hex):    74df87cccf7e0fead1370fc39f65be3de44f5069f5db87f3b08435ccdaf3b5b9
Node ID (base32): ot36ptgm67yp5vjt6b6dtz2l4ppejtggt5w3y64lqqrvztpl2wnq
DNS name:         ot36ptgm67yp5vjt6b6dtz2l4ppejtggt5w3y64lqqrvztpl2wnq.iron
```

The **base32 Node ID** is what you use for DNS queries.

### Identity Persistence

**Your Node ID and `.iron` domain name are persistent across restarts.** 

Iron automatically generates and saves a secret key on first run:
- **Key location:** `~/.config/iron/secret.key`
- **Permissions:** 0600 (owner read/write only)
- **Important:** Keep this key secure - it's your node's identity!

This means:
- ✅ Your `.iron` domain name stays the same across restarts
- ✅ You can share your domain name with others reliably
- ✅ Peers can always find you at the same address

**To reset your identity** (get a new Node ID and domain):
```bash
iron key reset
# or manually:
rm ~/.config/iron/secret.key
sudo iron serve
```

### DNS Configuration

Iron automatically configures DNS on first run for supported platforms:
- ✅ **macOS**: Creates `/etc/resolver/iron` 
- ✅ **Linux with systemd-resolved**: Creates `/etc/systemd/resolved.conf.d/iron.conf`
- ⚠️ **Other Linux**: See [DNS Setup Guide](doc/dns-setup.md) for manual configuration

**Manual DNS commands:**
```bash
# Remove DNS configuration (cleanup if iron crashed)
sudo iron --cleanup-dns
```

**What this does:**
- Routes **only** `.iron` domains to iron's DNS server (127.0.0.1:5333)
- All other domains (google.com, github.com, etc.) use your normal DNS
- **Works alongside Tailscale, VPNs, and other DNS systems**

**For advanced configuration or troubleshooting**, see [DNS Setup Guide](doc/dns-setup.md).

### Command Line Options

For complete CLI documentation, see [CLI Reference](doc/cli.md).

```
Usage: iron [COMMAND]

Commands:
  serve    Start the iron daemon (TUN interface and DNS server)
  self     Show information about your node
  convert  Convert between node ID formats
  key      Key management utilities
  resolve  Test DNS resolution
  vanity   Generate vanity address with desired prefix
  help     Print this message or the help of the given subcommand(s)

Global Options:
  -l, --log-level <LEVEL>  Set the log level [default: info]
                           (trace, debug, info, warn, error)
      --dns-port <PORT>    DNS server port [default: 5333]
      --cleanup-dns        Remove DNS configuration for .iron domains
  -h, --help               Print help
  -V, --version            Print version
```

### Environment Variables

- `RUST_LOG`: Control log levels per module (overrides `--log-level`)
  ```bash
  RUST_LOG=iron::protocol=trace,iron=info sudo iron serve
  ```

## Connecting Two Nodes

### Prerequisites
- Two machines with iron installed
- Both machines can reach each other (same network, or internet with NAT traversal)

### Important: IPv6-Only Network

**Iron uses IPv6 exclusively.** When connecting to `.iron` domains, you must use IPv6:

```bash
nc -6 <node>.iron 1234          
curl -6 http://<node>.iron:8080
ping6 <node>.iron
```

Most applications will automatically fall back to IPv6, but using the `-6` flag explicitly ensures immediate connection.

### Step 1: Start iron on both machines

**Machine A:**
```bash
sudo iron serve
# DNS configured automatically on first run
# Note the base32 Node ID displayed
```

**Machine B:**
```bash
sudo iron serve
# DNS configured automatically on first run
# Note the base32 Node ID displayed
```

### Step 2: Test connectivity from Machine B to Machine A

**Test DNS resolution:**
```bash
iron resolve <MACHINE_A_BASE32_ID>.iron
# or with dig:
dig <MACHINE_A_BASE32_ID>.iron AAAA
```

**Ping Machine A:**
```bash
ping6 <MACHINE_A_BASE32_ID>.iron
```

**Run a service on Machine A and access it from Machine B:**

```bash
# On Machine A - start HTTP server (bind to IPv6)
python3 -m http.server 8080 --bind ::

# On Machine B - access the server (use -6 flag)
curl -6 http://[<MACHINE_A_BASE32_ID>.iron]:8080/

# Or with netcat
# Machine A (server, listen on IPv6):
nc -6 -l 1234

# Machine B (client, connect to IPv6):
nc -6 <MACHINE_A_BASE32_ID>.iron 1234
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
sudo iron serve
```

### "Failed to create TUN device"

**macOS:**
- Ensure you have permission to create TUN devices
- Check system integrity protection settings

**Linux:**
- Verify TUN kernel module is loaded: `lsmod | grep tun`
- Load if needed: `sudo modprobe tun`

### DNS Not Resolving

Iron automatically configures DNS on first run for macOS and Linux with systemd-resolved.

**If DNS is not working:**

1. Verify iron configured DNS:
   - **macOS**: Check if `/etc/resolver/iron` exists
   - **Linux**: Check if `/etc/systemd/resolved.conf.d/iron.conf` exists

2. Manually setup DNS if needed:
   ```bash
   # DNS is auto-configured on first run of 'iron serve'
   # If cleanup is needed:
   sudo iron --cleanup-dns
   ```

3. Test DNS directly:
   ```bash
   iron resolve <node-id>.iron
   # or with dig:
   dig @127.0.0.1 -p 5333 <node-id>.iron AAAA
   ```

4. Verify you're using the **base32** Node ID (52 chars), not hex (64 chars)

5. For advanced configuration, see [DNS Setup Guide](doc/dns-setup.md)

### "I can't ping myself" / "Loopback detected"

**This is expected behavior.** iron is a P2P network - you cannot connect to yourself.

Self-ping requires protocol-specific packet rewriting (ICMP echo reply, TCP handshake, etc.) which would add unnecessary complexity for a feature that doesn't test real P2P connectivity.

**Solution:** Use two separate machines/nodes for testing. See [Testing Limitations](doc/testing-limitations.md) for details.

### No Connection to Peer

1. **Verify Node ID**: Ensure you're using the correct EndpointId
2. **Check logs**: Look for connection attempts with `--log-level debug`
   ```bash
   sudo iron serve --log-level debug
   ```
3. **Firewall**: Ensure UDP traffic is allowed (iroh uses QUIC over UDP)
4. **Relay server**: Check if iroh can reach relay servers

### Performance Issues

1. **Direct connection**: Check if direct connection established (vs relay)
   ```bash
   RUST_LOG=iron::protocol=debug sudo iron serve
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
RUST_LOG=iron::dns=debug,iron::tun=trace,iron=info sudo iron serve
```

## Development

A development shell with all build dependencies is provided via
```sh
nix develop
```

### Running Tests

```bash
# via nix:
nix flake check # this includes further checks like linting and CVE checks
# via cargo (unit and integration tests only):
cargo test
```

### Building Documentation

```bash
cargo doc --open --no-deps
```


## Technical Details

### Components

- **Key Management** (`keys.rs`): Persistent identity storage and generation
- **Registry** (`mapping.rs`): Bidirectional EndpointId ↔ IPv6 mapping
- **DNS Resolver** (`dns.rs`): Hickory-server based resolver for `.iron` domains
- **DNS Config** (`dns_config.rs`): Auto-configuration for system DNS
- **TUN Interface** (`tun.rs`): Virtual network device for packet interception
- **Protocol Handler** (`protocol.rs`): Iroh QUIC transport with connection pooling
- **Orchestrator** (`node.rs`): Component lifecycle management

### Specifications

- **IPv6 ULA Prefix**: `fd69:726f::/32` (iron-branded)
- **IPv6 Only**: Network operates exclusively over IPv6
- **MTU**: 1420 bytes (accounts for QUIC overhead)
- **ALPN**: `iron/packet/0` (protocol identifier)
- **DNS Encoding**: Base32 (no padding), 52 characters
- **Key Storage**: `~/.config/iron/secret.key` (0600 permissions)
- **Platform**: macOS (utun), Linux (iron0)

### Security

- **Encryption**: All traffic encrypted via iroh's QUIC (TLS 1.3)
- **Authentication**: Public key cryptography (EndpointId = PublicKey)
- **Identity Persistence**: Cryptographic keys stored securely (0600 permissions)
- **Source Verification**: Prevents IP spoofing between peers
- **NAT Traversal**: Secure hole punching with relay fallback

## License

MIT OR Apache-2.0

## Contributing

Contributions welcome! Please follow the coding guidelines in `AGENTS.md`.

## Acknowledgments

Built on [iroh](https://iroh.computer) - a Rust library for peer-to-peer networking.
