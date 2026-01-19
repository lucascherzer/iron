# Helper Scripts

Utility scripts for testing and development.

## DNS and Testing Scripts

### `node-id-to-dns.sh`
Converts a hex Node ID to base32 format for DNS queries.

**Usage:**
```bash
./scripts/node-id-to-dns.sh <HEX_NODE_ID>
```

**Example:**
```bash
./scripts/node-id-to-dns.sh 74df87cccf7e0fead1370fc39f65be3de44f5069f5db87f3b08435ccdaf3b5b9
```

Output:
- Node ID in both formats
- DNS name
- Automatic DNS resolution test

### `test-dns.sh`
Interactive DNS testing tool.

**Usage:**
```bash
./scripts/test-dns.sh
```

Prompts for base32 Node ID and tests DNS resolution.

### `test-interactive.sh`
Comprehensive interactive test suite.

**Usage:**
```bash
# While iron is running
./scripts/test-interactive.sh
```

Tests:
- TUN interface creation
- IPv6 configuration
- DNS resolution
- Basic connectivity

### `test-iron.sh`
Automated startup and verification test.

**Usage:**
```bash
sudo ./scripts/test-iron.sh
```

Builds, starts, and verifies iron automatically.

## Notes

- Most scripts require iron to be running
- Some scripts need sudo privileges
- See individual scripts for detailed usage
