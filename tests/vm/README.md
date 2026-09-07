# VM Integration Tests

This directory contains NixOS VM-based integration tests for iron.

## Overview

These tests use the NixOS test framework to create isolated VM environments where multiple iron nodes can communicate with each other over a real network. This allows us to verify actual P2P connectivity without manual setup.

## Test Suites

### `smoke-test.nix`
Single-node test verifying basic iron functionality:
- Key generation and persistence
- Node identity retrieval
- TUN interface creation
- DNS server startup
- Self DNS resolution

**Runtime:** ~30-60 seconds

**Run:**
```bash
nix build ..#checks.x86_64-linux.iron-vm-smoke-test
```

### `two-node-test.nix`
Multi-node test verifying P2P connectivity:
- Two independent iron nodes
- Cross-node DNS resolution
- Actual P2P packet delivery (HTTP traffic)
- Bidirectional connectivity
- Connection establishment verification

**Runtime:** ~2-5 minutes

**Run:**
```bash
nix build ..#checks.x86_64-linux.iron-vm-two-node-test
```

### `reliability-test.nix`
Comprehensive reliability and chaos testing:
- **Large data transfer:** 10MB with SHA256 verification
- **Concurrent transfers:** 5x 2MB simultaneous connections
- **Chaos testing:** Packet loss, latency, jitter, connection drops
- **Deterministic verification:** Seeded RNG for reproducible data
- **TCP reliability:** Ensures data integrity under adverse conditions

**Tests include:**
1. 10MB transfer with hash verification
2. 5 concurrent 2MB transfers
3. 5% packet loss test
4. Connection drop and reconnect
5. 100ms latency + 20ms jitter

**Runtime:** ~5-10 minutes

**Run:**
```bash
nix build ..#checks.x86_64-linux.iron-vm-reliability-test
```

## Platform Support

- ✅ **Linux**: Full support with QEMU
- ⚠️ **macOS**: Tests automatically skipped (no TAP networking)
- ⚠️ **Windows**: Untested

## Running Tests

### All VM Tests
```bash
cd ../..  # Go to project root
nix flake check
```

### Individual Tests
```bash
# Smoke test
nix build .#checks.x86_64-linux.iron-vm-smoke-test

# Two-node test
nix build .#checks.x86_64-linux.iron-vm-two-node-test

# Reliability test (chaos testing)
nix build .#checks.x86_64-linux.iron-vm-reliability-test
```

### With Verbose Output
```bash
nix build .#checks.x86_64-linux.iron-vm-smoke-test --show-trace -L
```

## Test Structure

Each test file exports a NixOS test configuration with:

1. **Node definitions**: VM configuration (packages, services, networking)
2. **Test script**: Python code that runs commands and assertions

Example:
```nix
{ pkgs, ironPackage }:

pkgs.testers.runNixOSTest {
  name = "iron-my-test";
  
  nodes = {
    machine = { config, pkgs, ... }: {
      environment.systemPackages = [ ironPackage ];
    };
  };
  
  testScript = ''
    machine.start()
    machine.succeed("iron self --exists")
  '';
}
```

## Writing New Tests

1. Create a new `.nix` file in this directory
2. Follow the structure of existing tests
3. Add to `flake.nix` checks section:
   ```nix
   iron-my-test = if pkgs.stdenv.isLinux then
     import ./tests/vm/my-test.nix {
       inherit pkgs;
       ironPackage = iron;
     }
   else
     pkgs.runCommand "iron-my-test-skipped" {} ''
       echo "Test skipped (Linux only)" > $out
     '';
   ```

## Available Test Methods

```python
# VM lifecycle
machine.start()
machine.shutdown()
machine.wait_for_unit("service-name")

# Command execution
machine.succeed("command")  # Must exit 0
machine.fail("command")     # Must exit non-0
machine.execute("command")  # Returns (status, output)

# Timing
machine.sleep(seconds)
machine.wait_until_succeeds("command", timeout=60)
machine.wait_until_fails("command", timeout=60)
```

## Troubleshooting

### Tests don't run
**Check platform:** VM tests only run on Linux.

### VM boot timeout
**Increase timeout:**
```python
machine.wait_for_unit("multi-user.target", timeout=120)
```

### Network issues
**Verify network in VM:**
```python
machine.succeed("ping -c 1 1.1.1.1")
```

### Iron fails to start
**Check logs:**
```python
machine.execute("journalctl -u iron.service")
```

### CI failures
**Debug with trace:**
```bash
nix build .#checks.x86_64-linux.iron-vm-smoke-test --show-trace -L
```

## Documentation

For more details, see:
- [VM Testing Documentation](../../doc/vm-testing.md)
- [Architecture Documentation](../../doc/arch.md)
- [Testing Limitations](../../doc/testing-limitations.md)

## CI/CD

These tests run automatically in GitHub Actions on every push:
- `.github/workflows/test.yml`
- Runs on Linux runners (ubuntu-latest)
- KVM-accelerated for faster execution