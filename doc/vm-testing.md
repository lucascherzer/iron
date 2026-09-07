# VM Testing Infrastructure

This document describes the automated testing infrastructure for iron using NixOS VMs.

## Overview

Iron uses the **NixOS test framework** (`pkgs.testers.runNixOSTest`) to create lightweight QEMU-based NixOS VMs for automated integration testing. This allows us to test real P2P connectivity between iron nodes in isolated environments.

## Test Suites

### 1. Smoke Test - Binary (`tests/vm/smoke-test.nix`)

A minimal test that verifies the iron **binary** can start and perform basic operations in a VM.

**Testing approach:** Direct binary execution with manual service management.

**What it tests:**
- ✅ Binary availability
- ✅ Key generation and persistence
- ✅ Node identity retrieval
- ✅ TUN interface creation
- ✅ DNS server startup
- ✅ Self DNS resolution

**Run time:** ~30-60 seconds

**Usage:**
```bash
nix build .#checks.x86_64-linux.iron-vm-smoke-test
```

### 2. Smoke Test - Module (`tests/vm/smoke-test-module.nix`)

A comprehensive test that validates the **NixOS module** (`nixosModules.iron`) works correctly in a real NixOS VM.

**Testing approach:** Uses the flake's production NixOS module configuration.

**What it tests:**
- ✅ Module imports and configuration
- ✅ Systemd service creation and startup
- ✅ Service configuration (log level, DNS port)
- ✅ Service lifecycle (restart behavior)
- ✅ Security hardening (capabilities, sandboxing)
- ✅ All basic functionality (keys, DNS, TUN, etc.)
- ✅ Log accessibility via journalctl

**Why this matters:** This test validates what users would actually deploy. If the module configuration breaks, this test catches it.

**Run time:** ~30-60 seconds

**Usage:**
```bash
nix build .#checks.x86_64-linux.iron-vm-smoke-test-module
```

**Comparison:**

| Aspect | Binary Test | Module Test |
|--------|-------------|-------------|
| **Tests** | `iron` binary directly | `nixosModules.iron` module |
| **Service** | Manual background process | systemd service via module |
| **Use Case** | Binary functionality | Production deployment config |
| **Restart** | Manual control | systemd Restart=on-failure |
| **Logs** | stdout/stderr to file | journalctl integration |

### 3. Two-Node Test (`tests/vm/two-node-test.nix`)

A comprehensive test that verifies P2P connectivity between two iron nodes.

**What it tests:**
- ✅ Two nodes starting independently
- ✅ TUN interfaces on both nodes
- ✅ DNS resolution across nodes
- ✅ P2P packet delivery (HTTP traffic)
- ✅ Bidirectional connectivity
- ✅ Connection establishment in logs

**Run time:** ~2-5 minutes

**Usage:**
```bash
nix build .#checks.x86_64-linux.iron-vm-two-node-test
```

### 4. Reliability Test (`tests/vm/reliability-test.nix`)

A comprehensive test suite that verifies TCP reliability and data integrity under adverse network conditions.

**What it tests:**
- ✅ Large data transfer (10MB) with SHA256 verification
- ✅ Concurrent connections (5x 2MB simultaneous transfers)
- ✅ Packet loss (5% with 25% correlation)
- ✅ Connection drops and reconnects
- ✅ High latency (100ms + 20ms jitter)
- ✅ Deterministic data generation (seeded RNG)

**Run time:** ~5-10 minutes

**Usage:**
```bash
nix build .#checks.x86_64-linux.iron-vm-reliability-test
```

## Running Tests

### Run All Checks (Including VM Tests)

```bash
nix flake check
```

This will run:
- Cargo build
- Cargo tests (unit + integration)
- Cargo clippy
- Cargo fmt check
- Cargo audit
- VM smoke test - binary (Linux only)
- VM smoke test - module (Linux only)
- VM two-node test (Linux only)
- VM reliability test (Linux only)
```

### Run Individual VM Tests

```bash
# Smoke test (binary) only
nix build .#checks.x86_64-linux.iron-vm-smoke-test

# Smoke test (module) only
nix build .#checks.x86_64-linux.iron-vm-smoke-test-module

# Two-node test only
nix build .#checks.x86_64-linux.iron-vm-two-node-test

# Reliability test only (chaos testing)
nix build .#checks.x86_64-linux.iron-vm-reliability-test
```

### Interactive VM Testing

For debugging, you can run VMs interactively:

```bash
# Build and run the test with verbose output
nix build .#checks.x86_64-linux.iron-vm-smoke-test --show-trace
```

## Platform Support

### Linux ✅
Full support with QEMU and TAP networking. VMs can communicate directly with each other.

**Hypervisors:**
- QEMU (default, best compatibility)
- Firecracker (faster, more isolated)

### macOS ⚠️
VM tests are **skipped** on macOS. The tests will show as passing but won't actually run.

**Why?**
- QEMU on macOS lacks TAP networking support
- VMs can't easily communicate with each other
- Multi-VM testing requires Linux

**Alternative:** Use GitHub Actions (runs on Linux) or a local Linux machine.

### Windows ⚠️
Not currently supported. May work with WSL2 + Linux kernel but untested.

## CI/CD Integration

### GitHub Actions

VM tests run automatically in CI on every push:

```yaml
# .github/workflows/test.yml
name: Test

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: cachix/install-nix-action@v24
      - run: nix flake check
```

This runs all checks including VM tests on Linux runners.

## Test Architecture

### VM Configuration

Each VM in the test suite:
- Runs full NixOS
- Has iron installed from the current build
- Has systemd-resolved enabled for DNS
- Has networking enabled (no firewall)
- Has test tools installed (dig, curl, ping, etc.)

### Network Topology (Two-Node Test)

```
┌─────────────────┐         ┌─────────────────┐
│    Node A       │         │    Node B       │
│                 │         │                 │
│  iron daemon    │◄───────►│  iron daemon    │
│  TUN: utun0     │  P2P    │  TUN: utun0     │
│  DNS: :5333     │  QUIC   │  DNS: :5333     │
│  fd69:726f::... │         │  fd69:726f::... │
└─────────────────┘         └─────────────────┘
```

Nodes communicate via:
1. **Control plane:** Standard network (for test orchestration)
2. **Data plane:** Iron P2P network (via iroh QUIC)

### Test Execution Flow

1. **VM Startup:** Both VMs boot in parallel
2. **Service Start:** Iron daemons start via systemd
3. **Identity Exchange:** Test script extracts node identities
4. **DNS Resolution:** Each node resolves the other's .iron domain
5. **P2P Communication:** HTTP requests over iron network
6. **Verification:** Logs checked for successful P2P connections

## Writing New VM Tests

### Basic Structure

```nix
{ pkgs, ironPackage }:

pkgs.testers.runNixOSTest {
  name = "iron-my-test";

  nodes = {
    node1 = { config, pkgs, ... }: {
      # VM configuration here
      environment.systemPackages = [ ironPackage ];
      # ...
    };
  };

  testScript = ''
    # Python test script here
    node1.start()
    node1.wait_for_unit("multi-user.target")
    node1.succeed("iron self --exists")
    # ...
  '';
}
```

### Available Test Methods

```python
# VM lifecycle
machine.start()
machine.shutdown()
machine.wait_for_unit("service-name")

# Command execution
machine.succeed("command")  # Must succeed (exit 0)
machine.fail("command")     # Must fail (exit non-0)
machine.execute("command")  # Returns (status, output)

# Utilities
machine.sleep(seconds)
machine.wait_until_succeeds("command", timeout=60)
machine.wait_until_fails("command", timeout=60)
```

### Adding to Flake

```nix
# In flake.nix checks section
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

## Troubleshooting

### "VM tests are not running"

**Check platform:**
```bash
uname -s
```

VM tests only run on Linux. On macOS/Windows, they're automatically skipped.

### "Test times out during VM boot"

**Increase timeout in test script:**
```python
machine.wait_for_unit("multi-user.target", timeout=120)
```

### "Network not available in VM"

**Verify VM has network access:**
```python
machine.succeed("ping -c 1 1.1.1.1")
```

### "Iron fails to start in VM"

**Check logs:**
```python
machine.execute("journalctl -u iron.service")
```

**Common issues:**
- Missing CAP_NET_ADMIN capability
- Key file permissions
- Port already in use

### "Tests pass locally but fail in CI"

**Possible causes:**
- Different Nix version
- Different NixOS channel
- Resource constraints (CPU/memory)
- Timing issues (add more sleep statements)

**Debug in CI:**
```yaml
- run: nix build .#checks.x86_64-linux.iron-vm-smoke-test --show-trace -L
```

## Performance Considerations

### Test Duration

| Test | Typical Duration | Maximum Duration |
|------|-----------------|------------------|
| Smoke test | 30-60s | 2 min |
| Two-node test | 2-5 min | 10 min |
| Reliability test | 5-10 min | 15 min |

### Resource Usage

- **Memory:** ~512MB per VM (1GB for two-node test)
- **Disk:** ~500MB for NixOS + iron
- **CPU:** 1-2 cores per VM

### Optimization Tips

1. **Cache builds:** Use Cachix to avoid rebuilding iron
2. **Parallel tests:** Run multiple test suites in parallel
3. **Minimize sleeps:** Use `wait_until_succeeds` instead of `sleep`
4. **Share derivations:** Reuse common VM configurations

## Future Enhancements

### Planned Features

- [x] Reliability and chaos testing (packet loss, latency, drops)
- [ ] Three-node test (triangle topology)
- [ ] NAT traversal test (simulated NAT)
- [ ] Relay server test
- [ ] Performance benchmarks (latency, throughput)
- [ ] Long-running stability test (24+ hours)

### Advanced Testing

- **Network simulation:** ✅ Latency, packet loss (implemented in reliability test)
- **Multiple topologies:** Star, mesh, ring networks
- **Scale testing:** 10+ nodes communicating
- **Failure scenarios:** ✅ Connection drops (implemented in reliability test)
- **Bandwidth limits:** Test with throttled connections
- **Network partitions:** Split-brain scenarios

## References

- [microvm.nix Documentation](https://github.com/astro/microvm.nix)
- [NixOS Test Framework](https://nixos.org/manual/nixos/stable/index.html#sec-nixos-tests)
- [iron Architecture](./arch.md)
- [Testing Limitations](./testing-limitations.md)

## Summary

VM testing provides:
- ✅ Automated multi-node testing
- ✅ Real P2P connectivity verification
- ✅ CI/CD integration
- ✅ Reproducible test environments
- ✅ Platform isolation

**Key Takeaway:** VM tests verify that iron actually works in realistic scenarios, not just unit tests in isolation.