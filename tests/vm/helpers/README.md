# VM Test Helpers

This directory contains shared Python utilities for iron VM tests.

## Overview

These helpers provide reusable functionality for testing iron's network reliability and data integrity across VM nodes.

## Files

### `gen_data.py`

Deterministic pseudo-random data generator for reproducible testing.

**Purpose:** Generate data that both sender and receiver can independently verify without transferring reference data.

**Features:**
- Seeded RNG for deterministic generation
- SHA256 hash computation
- Human-readable size parsing (K, M, G suffixes)
- Hash-only mode (compute without generating output)
- Configurable chunk size

**Usage:**

```bash
# Generate 10MB with seed 42
python3 gen_data.py --seed 42 --size 10M > data.bin

# Compute expected hash only (fast, no output)
python3 gen_data.py --seed 42 --size 10M --hash-only

# Generate and pipe to netcat
python3 gen_data.py --seed 42 --size 10M 2>/dev/null | nc host 9999
```

**In VM tests:**

```python
# Both nodes compute expected hash independently
expected_hash = nodeA.succeed(
    "python3 /helpers/gen_data.py --seed 42 --size 10M --hash-only"
).strip()

# Sender generates and transmits
nodeB.succeed(
    "python3 /helpers/gen_data.py --seed 42 --size 10M 2>/dev/null | "
    "nc receiver_ipv6 9999"
)
```

### `receive_tcp.py`

TCP server that receives data and computes SHA256 hash.

**Purpose:** Accept TCP connections, receive data, and verify integrity via hash.

**Features:**
- IPv6 socket support
- Progress reporting for large transfers
- Configurable timeout
- Automatic hash computation
- Bind to specific addresses

**Usage:**

```bash
# Listen on port 9999
python3 receive_tcp.py --port 9999

# With expected size for progress reporting
python3 receive_tcp.py --port 9999 --expected-size 10M

# Bind to specific IPv6 address
python3 receive_tcp.py --port 9999 --bind fd69:726f::1
```

**In VM tests:**

```python
# Start receiver in background
nodeA.succeed(
    "python3 /helpers/receive_tcp.py --port 9999 > /tmp/hash.txt 2>&1 &"
)

# Send data
nodeB.succeed("python3 /helpers/gen_data.py --seed 42 --size 10M | nc nodeA 9999")

# Verify hash
received_hash = nodeA.succeed("cat /tmp/hash.txt").strip()
assert received_hash == expected_hash
```

## Design Rationale

### Why Deterministic Generation?

**Problem:** How to verify large data transfers without storing reference data?

**Solution:** Use seeded RNG so both nodes compute the same expected hash:

```python
# Both nodes do this independently
random.seed(42)
data = generate(10MB)
hash = sha256(data)  # Always the same for seed=42
```

**Benefits:**
- No reference data storage needed
- Reproducible across test runs
- Both ends verify independently
- Catches any bit flips or corruption

### Why Separate Files?

1. **Syntax highlighting** - Proper Python IDE support
2. **Testability** - Can run scripts independently
3. **Reusability** - Share between multiple test suites
4. **Maintainability** - Easier to modify and debug
5. **Type hints** - Can use mypy for type checking
6. **Documentation** - Proper docstrings and examples

## Integration with VM Tests

### Copying Helpers to VMs

In Nix test scripts:

```nix
testScript = ''
  # Copy helpers to both nodes
  nodeA.succeed("mkdir -p /helpers")
  nodeB.succeed("mkdir -p /helpers")
  
  nodeA.copy_from_host("${./helpers}", "/helpers")
  nodeB.copy_from_host("${./helpers}", "/helpers")
  
  # Now use them
  nodeB.succeed("python3 /helpers/gen_data.py --seed 42 --size 10M | ...")
'';
```

### Alternative: Include in VM Image

```nix
environment.systemPackages = [ ... ];
environment.etc."iron-test-helpers".source = ./helpers;
```

Then access at `/etc/iron-test-helpers/gen_data.py`

## Testing Helpers Locally

You can test these scripts outside of VMs:

```bash
cd tests/vm/helpers

# Generate 1MB and verify hash
python3 gen_data.py --seed 42 --size 1M | sha256sum

# Test receiver (in one terminal)
python3 receive_tcp.py --port 9999

# Send data (in another terminal)
python3 gen_data.py --seed 42 --size 1M 2>/dev/null | nc ::1 9999
```

## Adding New Helpers

When adding new shared helpers:

1. Create Python file with proper shebang and docstring
2. Add argparse for CLI usage
3. Include type hints
4. Add usage examples in docstring
5. Document in this README
6. Test locally before using in VM tests

## Per-Test Helpers

For test-specific scripts that aren't shared, create a subdirectory:

```
tests/vm/
├── helpers/              # Shared across all tests
│   ├── gen_data.py
│   └── receive_tcp.py
├── reliability/          # Specific to reliability-test.nix
│   ├── chaos_setup.sh
│   └── metrics.py
└── reliability-test.nix
```

## See Also

- `../reliability-test.nix` - Uses these helpers extensively
- `../../doc/vm-testing.md` - Overall VM testing architecture
- `gen_data.py` docstring - Detailed API documentation
- `receive_tcp.py` docstring - TCP receiver API