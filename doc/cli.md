# iron CLI Reference

Complete reference for all iron command-line interface commands.

---

## Table of Contents

- [Daemon Mode](#daemon-mode)
- [Node Information](#node-information)
- [Format Conversion](#format-conversion)
- [Key Management](#key-management)
- [DNS Testing](#dns-testing)
- [Vanity Address Generation](#vanity-address-generation)

---

## Daemon Mode

The default mode - starts the iron daemon with TUN interface and DNS server.

### Basic Usage

```bash
sudo iron
```

**Requires**: Root/sudo privileges (for TUN device creation)

**What it does**:
1. Creates TUN network interface (e.g., `utun13` on macOS)
2. Configures IPv6 routing for `fd69:726f::/32`
3. Starts DNS server on `127.0.0.1:5333`
4. Auto-configures system DNS for `.iron` domains (macOS, Linux systemd)
5. Starts iroh endpoint for P2P connections
6. Displays node information and waits for connections

### Options

```bash
sudo iron [OPTIONS]
```

**Global Options**:
- `--log-level <LEVEL>`: Set logging level
  - Values: `trace`, `debug`, `info`, `warn`, `error`
  - Default: `info`
  - Example: `sudo iron --log-level debug`

- `--dns-port <PORT>`: Custom DNS server port
  - Default: `5333`
  - Example: `sudo iron --dns-port 5353`

- `--cleanup-dns`: Remove DNS configuration and exit
  - Useful if iron crashed and didn't auto-cleanup
  - Example: `sudo iron --cleanup-dns`

### Example Output

```
┌─────────────────────────────────────────┐
│          iron - P2P Network             │
│   Peer-to-peer connectivity via iroh    │
└─────────────────────────────────────────┘

Configuration:
  Log level:  info
  DNS port:   5333

Initializing iron node...
TUN device created: utun13
IPv6 address configured: fd69:726f::3d36:8b3d:fa9b:0339/32
IPv6 route added: fd69:726f::/32 → utun13

✓ DNS configured successfully!

Node ID (hex):    197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61
Node ID (base32): df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq
DNS name:         df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron

DNS server will run on: 127.0.0.1:5333

Press Ctrl-C to shutdown gracefully
```

### Shutdown

Press `Ctrl-C` to gracefully shutdown. Iron will:
1. Stop accepting new connections
2. Close existing connections
3. Remove TUN device
4. Clean up DNS configuration
5. Exit

---

## Node Information

Show information about your node's identity.

### iron self

Display all node information:

```bash
iron self
```

**Output**:
```
Node Information
────────────────────────────────────────────────────────

Hex (64 chars):
197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61

Base32 (52 chars):
df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq

.iron domain:
df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron

IPv6 address:
fd69:726f::3d36:8b3d:fa9b:0339

Key file: /Users/you/.config/iron/secret.key
```

### Options

**Show specific format only**:
```bash
iron self --hex         # Show only hex Node ID
iron self --base32      # Show only base32 Node ID
iron self --domain      # Show only .iron domain
iron self --ipv6        # Show only IPv6 address
```

**Check if key exists**:
```bash
iron self --exists
# Exit code: 0 if key exists, 1 if not
```

Useful in scripts:
```bash
if iron self --exists; then
  echo "Key exists"
else
  echo "No key found"
fi
```

**JSON output**:
```bash
iron self --format json
```

**Output**:
```json
{
  "hex": "197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61",
  "base32": "df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq",
  "domain": "df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron",
  "ipv6": "fd69:726f::3d36:8b3d:fa9b:0339",
  "key_path": "/Users/you/.config/iron/secret.key"
}
```

---

## Format Conversion

Convert between different Node ID formats.

### iron convert

Auto-detects input format and converts to all other formats:

```bash
iron convert <VALUE>
```

**Supported formats**:
- Hex (64 characters)
- Base32 (52 characters)
- `.iron` domain
- IPv6 address (`fd69:726f::...`)

### Examples

**Convert from hex**:
```bash
iron convert 197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61
```

**Output**:
```
Input format: Hex

Hex (64 chars):
197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61

Base32 (52 chars):
df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq

.iron domain:
df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron

IPv6 address:
fd69:726f::3d36:8b3d:fa9b:0339
```

**Convert from base32**:
```bash
iron convert df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq
```

**Convert from .iron domain**:
```bash
iron convert df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron
```

**Convert from IPv6**:
```bash
iron convert fd69:726f::3d36:8b3d:fa9b:0339
```

### Options

**Convert to specific format**:
```bash
iron convert <VALUE> --to <FORMAT>
```

**Formats**: `hex`, `base32`, `iron`, `ipv6`

**Examples**:
```bash
# Convert hex to base32 only
iron convert 197f6b... --to base32

# Convert base32 to .iron domain only
iron convert df7wwi7... --to iron

# Convert IPv6 to hex only
iron convert fd69:726f::3d36:8b3d:fa9b:0339 --to hex
```

---

## Key Management

Manage your node's private key.

### iron key info

Show information about the current key:

```bash
iron key info
```

**Output**:
```
Key Information
────────────────────────────────────────────────────────

Path: /Users/you/.config/iron/secret.key
Exists: Yes
Size: 32 bytes
Permissions: 0600 (owner read/write only)

Node ID (hex):
197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61

Node ID (base32):
df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq
```

**Show info for different key file**:
```bash
iron key info --path /path/to/other/key
```

### iron key generate

Generate a new random key:

```bash
iron key generate
```

**Output**:
```
Generated new key:
Hex: a7b3c9d2e5f8a1b4c7d0e3f6a9b2c5d8e1f4a7b0c3d6e9f2a5b8c1d4e7f0a3b6
Base32: u6z4ju4l7cubwt6q4p36vi4fxdu4d5npbq6d5pu5lcaq5ly4b5wa

This key has NOT been saved.
```

**Save as default key**:
```bash
iron key generate --save
```

**Force overwrite existing key**:
```bash
iron key generate --save --force
```

**Warning**: This replaces your current key and changes your Node ID!

### iron key export

Export your key to a file or stdout:

```bash
iron key export
```

**Output** (to stdout):
```
197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61
```

**Export to file**:
```bash
iron key export --output backup.key
```

**Export as base64**:
```bash
iron key export --format base64
iron key export --format base64 --output backup.key
```

### iron key import

Import a key from a file:

```bash
iron key import <FILE>
```

**Examples**:
```bash
# Import from backup
iron key import backup.key

# Import and save as default
iron key import backup.key --save
```

**Note**: The file must contain exactly 32 bytes (raw binary) or 64 hex characters.

### iron key validate

Check if a key file is valid:

```bash
iron key validate
```

**Validates default key at** `~/.config/iron/secret.key`

**Validate different file**:
```bash
iron key validate --path /path/to/key
```

**Output** (valid):
```
✓ Key is valid
  Size: 32 bytes
  Format: Valid iroh secret key
```

**Output** (invalid):
```
✗ Key is invalid
  Error: Invalid key file: expected 32 bytes, got 16. File may be corrupted.
```

### iron key reset

Delete the current key:

```bash
iron key reset
```

**Requires confirmation**:
```
Warning: This will DELETE your current key!
  Node ID: df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq
  Path: /Users/you/.config/iron/secret.key

Type 'yes' to confirm: 
```

**Skip confirmation** (dangerous):
```bash
iron key reset --confirm
```

**Warning**: After reset, a new key will be auto-generated on next `iron` start, giving you a different Node ID!

---

## DNS Testing

Test DNS resolution without starting the daemon.

### iron resolve

Query a `.iron` domain and display the result:

```bash
iron resolve <DOMAIN>
```

**Example**:
```bash
iron resolve df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron
```

**Output**:
```
Resolving: df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron
Server:    127.0.0.1:5333
Timeout:   5s

✓ Success

Domain: df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron
IPv6:   fd69:726f::3d36:8b3d:fa9b:0339
TTL:    300s
```

### Options

**Custom DNS server**:
```bash
iron resolve <DOMAIN> --server <ADDR>
```

**Example**:
```bash
iron resolve example.iron --server 127.0.0.1:5353
```

**Custom timeout**:
```bash
iron resolve <DOMAIN> --timeout <SECONDS>
```

**Example**:
```bash
iron resolve example.iron --timeout 10
```

**JSON output**:
```bash
iron resolve <DOMAIN> --json
```

**Example output**:
```json
{
  "domain": "df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron",
  "ipv6": "fd69:726f::3d36:8b3d:fa9b:0339",
  "ttl": 300,
  "server": "127.0.0.1:5333",
  "query_time_ms": 12
}
```

### Error Handling

**DNS server not running**:
```
✗ Failed to resolve

Error: Connection refused
```

**Invalid domain**:
```
✗ Failed to resolve

Error: NXDOMAIN (domain does not exist)
```

**Timeout**:
```
✗ Failed to resolve

Error: Query timeout after 5s
```

---

## Vanity Address Generation

Generate a Node ID with a desired prefix.

### iron vanity

Generate a key with a specific prefix in the base32 encoding:

```bash
iron vanity <PREFIX>
```

**Example**:
```bash
iron vanity alice
```

**Output** (while searching):
```
Generating vanity address with prefix: alice
Using 8 threads
Searching... (attempt 12,451, rate: 2,490/sec)
```

**Output** (found):
```
✓ Found vanity address!

Attempts: 38,742
Time: 15.5s

Node ID (hex):
03a18c9d2e5f8a1b4c7d0e3f6a9b2c5d8e1f4a7b0c3d6e9f2a5b8c1d4e7f0a3b6

Node ID (base32):
alicewi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwr3va
^^^^^ - Your prefix!

.iron domain:
alicewi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwr3va.iron

IPv6 address:
fd69:726f::a5b8:c1d4:e7f0:a3b6

This key has NOT been saved.
To use: iron vanity alice --save
```

### Options

**Save immediately when found**:
```bash
iron vanity alice --save
```

**Warning**: This replaces your current key!

**Custom thread count**:
```bash
iron vanity alice --threads 4
```

Default: Number of CPU cores

**Maximum attempts**:
```bash
iron vanity alice --max-attempts 1000000
```

Stops after N attempts even if not found.

**Quiet mode** (no progress, just result):
```bash
iron vanity alice --quiet
```

**Save to specific file**:
```bash
iron vanity alice --output vanity.key
```

### Performance Notes

**Difficulty increases exponentially**:
- 1 char: ~instant
- 2 chars: ~1 second
- 3 chars: ~30 seconds
- 4 chars: ~15 minutes
- 5 chars: ~8 hours
- 6+ chars: days to weeks

**Base32 alphabet**: `a-z` and `2-7` (case-insensitive)
- Total: 32 characters
- Cannot use: `0`, `1`, `8`, `9` (not in base32)

**Tips**:
- Use short prefixes (2-4 chars) for reasonable generation times
- More CPU cores = faster generation
- Case doesn't matter: `ALICE` = `alice`

### Examples

**Generate "bob" prefix**:
```bash
iron vanity bob
# Result: bob7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwr3va.iron
```

**Generate and save**:
```bash
iron vanity alice --save
# Replaces your current key when found
```

**Use more threads**:
```bash
iron vanity charlie --threads 16
# Uses 16 CPU cores for faster generation
```

**Quiet mode with limit**:
```bash
iron vanity dave --quiet --max-attempts 100000
# Tries up to 100k attempts silently
```

---

## Common Workflows

### First Time Setup

```bash
# 1. Check if you have a key
iron self --exists
# Exit code 1 = no key

# 2. Start iron (will auto-generate key)
sudo iron

# 3. Note your Node ID (share with peers)
iron self --base32
```

### Backup Your Key

```bash
# Export to backup file
iron key export --output ~/Backups/iron-key-backup.txt

# Store safely (this is your identity!)
```

### Restore From Backup

```bash
# Import backup
iron key import ~/Backups/iron-key-backup.txt --save

# Verify
iron self
```

### Share Your Node ID

```bash
# Get base32 format (easiest to share)
iron self --base32

# Or get the .iron domain directly
iron self --domain
```

### Test Connection

```bash
# 1. Get peer's Node ID (base32 format)
# 2. Start iron
sudo iron

# 3. In another terminal, test DNS
iron resolve <peer-base32>.iron

# 4. Test ping
ping6 <resolved-ipv6>
```

### Generate Memorable Address

```bash
# Generate vanity address with your name
iron vanity alice --save

# Restart iron to use new identity
sudo iron

# Your new domain: alice....iron
```

### Troubleshooting

```bash
# Check DNS server is running
lsof -i :5333

# Test DNS resolution
iron resolve <domain> --server 127.0.0.1:5333

# Verify key is valid
iron key validate

# Check node info
iron self

# Cleanup stuck DNS config
sudo iron --cleanup-dns

# Run with debug logging
sudo iron --log-level debug
```

---

## Exit Codes

All commands follow standard Unix exit code conventions:

- `0`: Success
- `1`: General error (invalid arguments, file not found, etc.)
- `2`: Command not found / invalid usage

**Special cases**:
- `iron self --exists`: Returns `0` if key exists, `1` if not

---

## Environment Variables

**RUST_LOG**: Override log level
```bash
RUST_LOG=iron=debug sudo -E iron
```

**HOME**: Key file location is `$HOME/.config/iron/secret.key`

---

## Security Notes

1. **Key file permissions**: Always 0600 (owner read/write only)
2. **Backup your key**: Loss of key = loss of identity
3. **Root privileges**: Only daemon mode needs root (for TUN device)
4. **DNS is unauthenticated**: Real security comes from iroh's crypto
5. **Share base32 ID, not hex key**: Never share your secret key file!

---

## See Also

- `doc/arch.md` - Architecture overview
- `doc/dns-setup.md` - DNS configuration details
- `doc/networking.md` - Networking internals
- `doc/packet-flow.md` - Packet flow walkthrough
