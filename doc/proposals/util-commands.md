# Proposal: Utility Commands for iron CLI

## Problem Statement

Currently, the iron CLI serves only as an entry point to start the node (TUN device + DNS resolver). Users need offline utilities for:
1. Converting between different node ID formats (hex, base32, .iron domain, IPv6)
2. Viewing information about their own node without starting the full network stack
3. Generating vanity addresses (keys with desired prefix patterns)
4. Managing and inspecting keys
5. Testing connectivity and DNS resolution

These utilities should work **without requiring root privileges** or starting the network daemon.

---

## Current CLI Structure

```bash
# Main daemon (requires root)
sudo iron [OPTIONS]

# DNS management (requires root)
sudo iron --cleanup-dns
```

**Options:**
- `-l, --log-level <LEVEL>` - Set log level (trace, debug, info, warn, error)
- `--dns-port <PORT>` - DNS server port (default: 5333)

---

## Proposed CLI Structure

```bash
# Main daemon (existing, requires root)
sudo iron [OPTIONS]

# Utility subcommands (new, NO root required)
iron convert <value>              # Convert between formats
iron self                         # Show info about own node
iron vanity <prefix> [OPTIONS]    # Generate vanity address
iron key <subcommand>             # Key management utilities
iron ping <target>                # Test connectivity (requires running daemon)
iron resolve <domain>             # Test DNS resolution
```

---

## 1. Format Conversion: `iron convert`

**Purpose:** Convert between hex, base32, .iron domain, and IPv6 formats

### CLI API

```bash
# Auto-detect input format and show all representations
iron convert <value>

# Examples
iron convert df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq
iron convert df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron
iron convert 74df87cccf7e0fead1370fc39f65be3de44f5069f5db87f3b08435ccdaf3b5b9
iron convert fd69:726f::039b:fa8b:3d36:8d61

# Specific output format
iron convert <value> --to hex
iron convert <value> --to base32
iron convert <value> --to iron
iron convert <value> --to ipv6
```

### Output Format

```bash
$ iron convert df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq

Node ID formats:
  Hex:     74df87cccf7e0fead1370fc39f65be3de44f5069f5db87f3b08435ccdaf3b5b9
  Base32:  df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq
  Domain:  df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron
  IPv6:    fd69:726f::0842:35cc:daf3:b5b9
```

### Implementation Details

**Input detection logic:**
1. If ends with `.iron` → strip and treat as base32
2. If 52 chars, all valid base32 → base32 Node ID
3. If 64 chars, all hex → hex Node ID
4. If contains `:` → IPv6 address (reverse lookup via registry derivation)
5. Otherwise → error with helpful message

**Special case: IPv6 to Node ID**
- Can derive IPv6 from Node ID (deterministic)
- **Cannot** reverse IPv6 to Node ID (one-way function)
- Error message: "IPv6 addresses cannot be converted back to Node IDs (use iron's DNS or registry)"

---

## 2. Self-Info: `iron self`

**Purpose:** Display information about the current node without starting the daemon

### CLI API

```bash
# Show all info about own node
iron self

# Show specific format
iron self --format hex
iron self --format base32
# NOTE: I find this weird. It conflates output format and node display variants
iron self --format json

# Short formats (single line output)
iron self --hex        # Just hex
iron self --base32     # Just base32
iron self --ipv6       # Just IPv6
iron self --domain     # Just .iron domain

# Check if key exists
iron self --exists     # Exit code 0 if key exists, 1 otherwise
# NOTE: the above could be left out and the exit code behaviour implemented in iron self
```

### Output Format

```bash
$ iron self

Iron Node Identity:
  Key file:  /home/user/.config/iron/secret.key
  Status:    ✓ Key found

Node ID:
  Hex:       74df87cccf7e0fead1370fc39f65be3de44f5069f5db87f3b08435ccdaf3b5b9
  Base32:    df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq

Network Identity:
  Domain:    df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron
  IPv6:      fd69:726f::0842:35cc:daf3:b5b9

Share this with peers to connect:
  df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron
```

```bash
$ iron self --format json
{
  "key_file": "/home/user/.config/iron/secret.key",
  "key_exists": true, # this is an aggregate function of key_file...
  "node_id": {
    "hex": "74df87cccf7e0fead1370fc39f65be3de44f5069f5db87f3b08435ccdaf3b5b9",
    "base32": "df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq"
  },
  "network": {
    "domain": "df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron",
    "ipv6": "fd69:726f::0842:35cc:daf3:b5b9"
  }
}
```

```bash
$ iron self --base32
df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq
```

### Error Cases

```bash
$ iron self
Error: No key file found at /home/user/.config/iron/secret.key

Run 'iron' once to generate a key, or use 'iron vanity' to create a custom key.
```

---

## 3. Vanity Address Generator: `iron vanity`

**Purpose:** Generate keys with desired prefix in base32 representation (like Tor onion services)

### CLI API

```bash
# Generate key with prefix (case-insensitive)
iron vanity <prefix>

# Options
iron vanity <prefix> --threads <N>        # Use N threads (default: num_cpus)
iron vanity <prefix> --max-attempts <N>   # Give up after N attempts
iron vanity <prefix> --save               # Save to ~/.config/iron/secret.key
iron vanity <prefix> --output <file>      # Save to custom file
iron vanity <prefix> --quiet              # Only output the result, no progress

# Examples
iron vanity alice                         # Find key starting with "alice"
iron vanity bob --threads 16              # Use 16 threads
iron vanity iron --save                   # Generate and save as default key
iron vanity test --max-attempts 1000000   # Try up to 1M keys
```
it would be great if iron vanity could estimate how long the computation would
take (how many on avg we need to compute, how many we can compute per sec)

### Output Format

```bash
$ iron vanity alice

Searching for vanity address with prefix "alice"...
Threads: 8
Difficulty: ~32^5 = 33,554,432 attempts (estimated)

Searching... (1,234,567 attempts, 15s elapsed)
Searching... (2,468,134 attempts, 30s elapsed)
Searching... (3,701,801 attempts, 45s elapsed)

✓ Found matching key!

Node ID:
  Base32:  alicewi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq
  Hex:     a1ce87cccf7e0fead1370fc39f65be3de44f5069f5db87f3b08435ccdaf3b5b9
  Domain:  alicewi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron
  IPv6:    fd69:726f::0842:35cc:daf3:b5b9

Attempts:  4,123,456
Time:      52.3 seconds
Rate:      78,843 keys/second

To use this key:
  iron vanity alice --save
# NOTE: this is dangerous. We may search for a long time and then when finding
# something, realize we did not have --save. printing the key or somehow
# persisting it.
```

### Implementation Details

**Difficulty estimation:**
- Each base32 character = 32 possibilities (5 bits)
- Prefix "alice" (5 chars) = 32^5 = 33,554,432 attempts (expected)
- Show estimated difficulty before starting

**Multi-threading:**
- Each thread generates keys independently with different random seeds
- First thread to find a match wins
- Default: use all CPU cores

**Progress updates:**
- Update every second with attempt count and elapsed time
- Show keys/second rate
- Estimate time remaining (after first 10 seconds)

**Safety:**
NOTE: I like this: we could have multiple keys in the key dir and just use the
one with the designated name.
- **Never** overwrite existing key without `--force` flag
- Confirm before saving with `--save`

**Constraints:**
- Base32 alphabet: `abcdefghijklmnopqrstuvwxyz234567` (lowercase)
- Warn if prefix contains invalid characters (0, 1, 8, 9)
- Max prefix length: 8 characters (reasonable difficulty)

---

## 4. Key Management: `iron key`

**Purpose:** Utilities for managing cryptographic keys

### CLI API
NOTE: scrap the iroh key subcommand. We do not need this (its too much overhead
for too little use)

```bash
# Show key information
iron key info [--path <file>]

# Export key to different formats
iron key export [--format <format>] [--output <file>]

# Import key from file
iron key import <file> [--save]

# Generate new random key
iron key generate [--save] [--force]

# Validate key file
iron key validate [--path <file>]

# Reset (delete) current key
iron key reset [--confirm]
```

### Examples

```bash
# View key info
$ iron key info
Key file: /home/user/.config/iron/secret.key
Valid:    ✓
Created:  2026-01-19 14:30:00
Size:     32 bytes
Node ID:  74df87cccf7e0fead1370fc39f65be3de44f5069f5db87f3b08435ccdaf3b5b9

# Export for backup
$ iron key export --format hex --output backup.key
Key exported to: backup.key

# Import from backup
$ iron key import backup.key --save
✓ Key imported and saved to /home/user/.config/iron/secret.key

# Generate new key
$ iron key generate
WARNING: This will generate a new identity.
You will get a new Node ID and .iron domain.
Generate new key? (y/N) n
Cancelled.

$ iron key generate --force --save
✓ New key generated and saved
Node ID: df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron

# Reset/delete key
$ iron key reset
WARNING: This will delete your key file permanently.
You will lose your current Node ID: df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron
Delete key? (y/N) n
Cancelled.

$ iron key reset --confirm
✓ Key deleted: /home/user/.config/iron/secret.key
```

---

## 5. Connectivity Testing: `iron ping`

**Purpose:** Test connectivity to another node (requires iron daemon running)

### CLI API

```bash
# Ping a node
iron ping <target>

# Options
iron ping <target> --count <N>           # Send N pings (default: 4)
iron ping <target> --timeout <seconds>   # Timeout per ping (default: 5)
iron ping <target> --interval <seconds>  # Time between pings (default: 1)
iron ping <target> --size <bytes>        # Payload size (default: 64)

# Examples
iron ping alice.iron
iron ping df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron
iron ping fd69:726f::0842:35cc:daf3:b5b9
```

### Output Format

```bash
$ iron ping alice.iron

PING alice.iron (df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq / fd69:726f::0842:35cc:daf3:b5b9)
64 bytes from alice.iron: seq=1 time=23.4ms
64 bytes from alice.iron: seq=2 time=21.8ms
64 bytes from alice.iron: seq=3 time=22.1ms
64 bytes from alice.iron: seq=4 time=23.0ms

--- alice.iron ping statistics ---
4 packets transmitted, 4 received, 0% packet loss, time 3012ms
rtt min/avg/max/mdev = 21.8/22.6/23.4/0.7 ms
```

### Implementation Details

**Requires daemon:**
- Must connect to running iron daemon (via local API socket)
- Error if daemon not running: "Error: iron daemon not running. Start with: sudo iron"

**Protocol:**
- Use ICMP6 Echo Request/Reply over iron network
- Alternative: Custom ping protocol over QUIC (if ICMP not available)

**Statistics:**
- Min/avg/max round-trip time
- Packet loss percentage
- Standard deviation (mdev)

---

## 6. DNS Resolution Testing: `iron resolve`

**Purpose:** Test DNS resolution without starting full daemon

### CLI API

```bash
# Resolve .iron domain to IPv6
iron resolve <domain>

# Options
iron resolve <domain> --server <addr>    # Custom DNS server (default: 127.0.0.1:5333)
iron resolve <domain> --timeout <sec>    # Query timeout (default: 5)
iron resolve <domain> --json             # JSON output

# Examples
iron resolve alice.iron
iron resolve df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron
iron resolve test.iron --server 192.168.1.100:5333
```

### Output Format

```bash
$ iron resolve alice.iron

Querying 127.0.0.1:5333 for alice.iron...

✓ Resolved:
  Domain:   alice.iron
  IPv6:     fd69:726f::0842:35cc:daf3:b5b9
  TTL:      300 seconds
  Time:     12ms

Node ID:
  Base32:   alicewi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq
  Hex:      a1ce87cccf7e0fead1370fc39f65be3de44f5069f5db87f3b08435ccdaf3b5b9
```

```bash
$ iron resolve test.iron --json
{
  "domain": "test.iron",
  "ipv6": "fd69:726f::0842:35cc:daf3:b5b9",
  "ttl": 300,
  "query_time_ms": 12,
  "node_id": {
    "hex": "a1ce87cccf7e0fead1370fc39f65be3de44f5069f5db87f3b08435ccdaf3b5b9",
    "base32": "alicewi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq"
  }
}
```

### Error Cases

```bash
$ iron resolve test.iron
Error: DNS query failed: connection refused (is iron running?)

$ iron resolve invalid.com
Error: Not a .iron domain
```

---

## Additional Utility Ideas

### 7. `iron peers` (Future)
NOTE: we leave that out for now, like all others with (Future)
List connected peers and connection status

```bash
iron peers                    # List all connected peers
iron peers --verbose          # Show detailed connection info
iron peers --json             # JSON output
```

### 8. `iron status` (Future)
Show daemon status and statistics

```bash
iron status                   # Show running status
iron status --stats           # Include network statistics
iron status --json            # JSON output
```

### 9. `iron logs` (Future)
View daemon logs without restarting with different log level

```bash
iron logs                     # Tail logs
iron logs --follow            # Follow logs (like tail -f)
iron logs --level debug       # Filter by level
```

### 10. `iron config` (Future)
Manage configuration file

```bash
iron config init              # Create default config
iron config show              # Show current config
iron config edit              # Open in editor
iron config validate          # Check config validity
```

---

## Implementation Plan

### Phase 1: Offline Utilities (No daemon required)
1. ✅ `iron convert` - Format conversion
2. ✅ `iron self` - Show own node info
3. ✅ `iron vanity` - Vanity address generator
4. ✅ `iron key` - Key management (leave out)

### Phase 2: DNS Utilities
5. ✅ `iron resolve` - DNS resolution testing

### Phase 3: Network Utilities (Requires daemon)
6. ⏸️ `iron ping` - Connectivity testing
7. ⏸️ `iron peers` - List connected peers
8. ⏸️ `iron status` - Daemon status

### Phase 4: Advanced Features
9. ⏸️ `iron logs` - Log viewing
10. ⏸️ `iron config` - Configuration management

---

## CLI Framework

**Use `clap` with subcommands:**

```rust
#[derive(Parser)]
enum Command {
    /// Start iron daemon (default, no subcommand)
    #[command(flatten)]
    Start(StartArgs),
    
    /// Convert between node ID formats
    Convert(ConvertArgs),
    
    /// Show information about your node
    r#Self(SelfArgs),  // Self is keyword, use r#Self
    
    /// Generate vanity address
    Vanity(VanityArgs),
    
    /// Test DNS resolution
    Resolve(ResolveArgs),
    
    /// Test connectivity (requires daemon)
    Ping(PingArgs),
}
```

---

## User Experience Goals

1. **No root for utilities** - All utility commands work without sudo
2. **Consistent output** - Similar format across commands
3. **JSON support** - Where useful for scripting
4. **Helpful errors** - Clear messages with suggestions
5. **Progress feedback** - Show progress for long operations (vanity)
6. **Safety** - Confirm destructive operations (key reset)
7. **Offline-first** - Most utilities work without network

---

## Examples in Documentation

### Quick Start Guide

```bash
# Generate a vanity address
iron vanity alice --save

# View your node info
iron self

# Start the daemon
sudo iron

# (In another terminal) Test DNS
iron resolve alice.iron

# Test connectivity
iron ping bob.iron
```

### Troubleshooting Workflow

```bash
# Check if key exists
iron self --exists

# Verify DNS is working
iron resolve $(iron self --domain)

# Test connectivity to peer
iron ping peer.iron

# Check daemon status
iron status
```

---

## Benefits

1. **Developer Experience** - Easy to inspect and debug
2. **User Onboarding** - Clear utilities for common tasks
3. **Vanity Addresses** - Memorable, brandable node IDs
4. **Scripting** - JSON output enables automation
5. **Offline Tools** - Most work without network/daemon
6. **Consistency** - Familiar CLI patterns (like git, docker)

---

## Open Questions

1. **Daemon communication** - For `iron ping`, `iron peers`, `iron status`:
   - Use Unix socket? HTTP API? gRPC?
   - Where should socket live? `/tmp/iron.sock` or `~/.config/iron/daemon.sock`?

2. **Vanity difficulty warnings** - What prefix length requires a warning?
   - 6+ chars = ~1 billion attempts (warn user)
   - 8+ chars = ~1 trillion attempts (strongly warn)

3. **Key export formats** - What formats to support?
   - hex (raw bytes)
   - base64
   - PEM (if compatible with iroh)
   - JSON (with metadata)

4. **Backward compatibility** - Can we keep `iron` (no subcommand) starting daemon?
   - Yes - `clap` supports default subcommand or flatten pattern
   - `iron` = `iron start` (backward compatible)

---

## Summary

This proposal adds **offline-first utility commands** to iron's CLI:

**Core utilities (Phase 1):**
- `iron convert` - Format conversions
- `iron self` - Node information
- `iron vanity` - Vanity address generation
- `iron key` - Key management
- `iron resolve` - DNS testing

**Network utilities (Phase 2+):**
- `iron ping` - Connectivity testing
- `iron peers` - Peer listing
- `iron status` - Daemon status

**All utilities work without root privileges**, making iron more user-friendly for inspection, debugging, and daily use.
