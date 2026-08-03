# iron

Peer-to-peer network interface based on [iroh](https://iroh.computer).

iron creates a virtual IPv6 network over peer-to-peer QUIC connections, letting
endpoints reach each other directly through `.iron` DNS names.

## Features

- Virtual overlay network: each peer is addressed by its public key
- Encrypted P2P: all traffic is encrypted via iroh's QUIC protocol
- DNS resolution: `.iron` domains resolve to peer IPv6 addresses
- TUN interface: a standard network device, so any application can use it
- NAT traversal: automatic hole punching with relay fallback

## Architecture

An application opens a normal IPv6 socket. The operating system routes traffic
for `fd69:726f::/32` to iron's TUN interface, which maps the destination IPv6
address to a peer EndpointId and forwards the packet over an encrypted iroh QUIC
connection. The receiving peer verifies the source address and writes the packet
to its own TUN interface, where the OS delivers it to the listening application.

## Building

Prerequisite: nix

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

Basic usage (requires root):

```bash
sudo iron serve
```

On first run, iron automatically configures DNS for `.iron` domains on macOS and
Linux with systemd-resolved. This only affects `.iron` domains; all other DNS
resolution is untouched.

Custom log level and DNS port:

```bash
sudo iron serve --log-level debug
sudo iron serve --dns-port 5353
```

When iron starts, it prints your node identity:

```
Node ID (hex):    74df87cccf7e0fead1370fc39f65be3de44f5069f5db87f3b08435ccdaf3b5b9
Node ID (base32): ot36ptgm67yp5vjt6b6dtz2l4ppejtggt5w3y64lqqrvztpl2wnq
DNS name:         ot36ptgm67yp5vjt6b6dtz2l4ppejtggt5w3y64lqqrvztpl2wnq.iron
```

Use the base32 Node ID for DNS queries.

### Identity persistence

Your Node ID and `.iron` domain are persistent across restarts. iron generates
and saves a secret key on first run:

- Key location: `~/.config/iron/secret.key`
- Permissions: 0600 (owner read/write only)

Keep this key secure; it is your node's identity. Your domain name stays the
same across restarts, so peers can always reach you at the same address.

To reset your identity:

```bash
iron key reset
# or manually:
rm ~/.config/iron/secret.key
sudo iron serve
```

### DNS configuration

iron configures DNS automatically on supported platforms:

- macOS: creates `/etc/resolver/iron`
- Linux with systemd-resolved: creates `/etc/systemd/resolved.conf.d/iron.conf`
- Other Linux: see [DNS Setup Guide](doc/dns-setup.md) for manual configuration

Only `.iron` domains are routed to iron's DNS server (`127.0.0.1:5333`); all
other domains use your normal DNS. This works alongside Tailscale, VPNs, and
other DNS setups.

To remove iron's DNS configuration (for example, after a crash):

```bash
sudo iron --cleanup-dns
```

See [DNS Setup Guide](doc/dns-setup.md) for advanced configuration and
troubleshooting.

### Command line options

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

### Environment variables

`RUST_LOG` controls log levels per module and overrides `--log-level`:

```bash
RUST_LOG=iron::protocol=trace,iron=info sudo iron serve
```

## Connecting two nodes

Both machines need iron installed and reachable from each other (same network,
or internet with NAT traversal).

iron uses IPv6 exclusively, so always use IPv6 when connecting to `.iron`
domains:

```bash
nc -6 <node>.iron 1234
curl -6 http://<node>.iron:8080
ping6 <node>.iron
```

### Step 1: start iron on both machines

```bash
sudo iron serve
```

Note the base32 Node ID displayed on each machine.

### Step 2: test connectivity

From Machine B, resolve and reach Machine A:

```bash
iron resolve <MACHINE_A_BASE32_ID>.iron
# or with dig:
dig <MACHINE_A_BASE32_ID>.iron AAAA
ping6 <MACHINE_A_BASE32_ID>.iron
```

To verify end-to-end traffic, run a service on Machine A and access it from
Machine B:

```bash
# Machine A: start an HTTP server bound to IPv6
python3 -m http.server 8080 --bind ::

# Machine B: access the server
curl -6 http://[<MACHINE_A_BASE32_ID>.iron]:8080/

# Or with netcat
# Machine A (listen on IPv6):
nc -6 -l 1234
# Machine B (connect over IPv6):
nc -6 <MACHINE_A_BASE32_ID>.iron 1234
```

## How it works

### DNS resolution

When you access `<endpoint_id>.iron`:

1. A DNS query is sent to iron's resolver (port 5333)
2. The EndpointId is parsed from the base32-encoded domain
3. An IPv6 address is derived: `fd69:726f::xxxx:xxxx:xxxx:xxxx`
4. The application connects to that IPv6 address

### Sending packets

1. The application sends to the IPv6 address
2. The OS routes the packet to the TUN interface
3. iron looks up the EndpointId from the IPv6 address
4. The packet is sent to the peer over the iroh QUIC connection
5. NAT traversal happens automatically

### Receiving packets

1. The peer sends the packet via iroh
2. iron receives it on a QUIC stream
3. The source address is verified (anti-spoofing)
4. The packet is written to the TUN interface
5. The OS routes it to the listening application

### IPv6 address derivation

Each EndpointId (32 bytes) maps to a unique IPv6 address. The last 8 bytes of the
EndpointId form the host part of the IPv6 address:

```
EndpointId: 197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61
                                                        Last 8 bytes: 039b fa8b 3d36 8b3d
IPv6: fd69:726f:0000:0000:039b:fa8b:3d36:8b3d
      |--ULA prefix--|         |--from EndpointId--|
```

## Troubleshooting

### "ERROR: iron must be run as root"

TUN device creation requires elevated privileges. Use `sudo`:

```bash
sudo iron serve
```

### "Failed to create TUN device"

macOS:

- Ensure you have permission to create TUN devices
- Check system integrity protection settings

Linux:

- Verify the TUN kernel module is loaded: `lsmod | grep tun`
- Load it if needed: `sudo modprobe tun`

### DNS not resolving

1. Verify iron configured DNS:
   - macOS: check that `/etc/resolver/iron` exists
   - Linux: check that `/etc/systemd/resolved.conf.d/iron.conf` exists
2. Test DNS directly:

   ```bash
   iron resolve <node-id>.iron
   # or with dig:
   dig @127.0.0.1 -p 5333 <node-id>.iron AAAA
   ```

3. Make sure you are using the base32 Node ID (52 chars), not the hex one
   (64 chars).
4. For advanced configuration, see [DNS Setup Guide](doc/dns-setup.md)

### "I can't ping myself" / "Loopback detected"

This is expected. iron is a P2P network; you cannot connect to yourself.
Self-ping would require protocol-specific packet rewriting (ICMP echo reply, TCP
handshake, etc.), which is unnecessary complexity for a feature that does not
test real P2P connectivity.

Use two separate nodes for testing. See [Testing
Limitations](doc/testing-limitations.md) for details.

### No connection to peer

1. Verify you are using the correct EndpointId
2. Check the logs for connection attempts with `--log-level debug`
3. Ensure firewalls allow UDP (QUIC runs over UDP)
4. Check whether iroh can reach its relay servers

### Performance

1. Check whether a direct connection was established instead of a relay
2. Verify the MTU (default 1420)
3. Monitor with `--log-level trace` (very verbose)

## Log levels

- `error`: only critical failures
- `warn`: recoverable issues (failed sends, unknown destinations)
- `info`: high-level events (startup, connections, shutdown)
- `debug`: packet flow, DNS queries, mappings
- `trace`: very detailed (stream operations, individual packets)

Per-module filtering:

```bash
RUST_LOG=iron::dns=debug,iron::tun=trace,iron=info sudo iron serve
```

## Development

A development shell with all build dependencies is provided via:

```sh
nix develop
```

### Running tests

```bash
nix flake check   # includes linting and CVE checks
cargo test        # unit and integration tests only
```

### Building documentation

```bash
cargo doc --open --no-deps
```

## Technical details

### Components

- `keys.rs`: persistent identity storage and generation
- `mapping.rs`: bidirectional EndpointId to IPv6 mapping
- `dns.rs`: hickory-server based resolver for `.iron` domains
- `dns_config.rs`: auto-configuration for system DNS
- `tun.rs`: virtual network device for packet interception
- `protocol.rs`: iroh QUIC transport with connection pooling
- `node.rs`: component lifecycle management

### Specifications

- IPv6 ULA prefix: `fd69:726f::/32`
- IPv6 only: the network operates exclusively over IPv6
- MTU: 1420 bytes (accounts for QUIC overhead)
- ALPN: `iron/packet/0`
- DNS encoding: base32 (no padding), 52 characters
- Key storage: `~/.config/iron/secret.key` (0600 permissions)
- Platform: macOS (utun), Linux (iron0)

### Security

- Encryption: all traffic is encrypted via iroh's QUIC (TLS 1.3)
- Authentication: public key cryptography (EndpointId = PublicKey)
- Identity persistence: cryptographic keys stored with 0600 permissions
- Source verification: prevents IP spoofing between peers
- NAT traversal: hole punching with relay fallback

## License

MIT OR Apache-2.0

## Contributing

Contributions welcome. Follow the coding guidelines in `AGENTS.md`.
