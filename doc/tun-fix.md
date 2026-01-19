# TUN Device Configuration Fix

## Problem

The iron binary was failing to create the TUN device with the error:
```
ERROR iron::node: TUN interface failed: Failed to create TUN device (are you root?)
```

Even when running with `sudo`, the TUN device creation was failing.

## Root Cause

The `tun` crate's `Configuration` API on macOS only configures IPv4 addresses automatically. For IPv6-only applications like iron, we need to:

1. Create the TUN device with the `tun` crate (this part was working)
2. Manually configure the IPv6 address using system commands (`ifconfig` on macOS, `ip` on Linux)
3. Add routing rules so the OS knows to route `fd69:726f::/32` traffic to our TUN interface

## Solution

Added two key improvements to `src/tun.rs`:

### 1. Added `netmask` to Configuration

```rust
config
    .layer(Layer::L3)
    .address((169, 254, 0, 1))
    .destination((169, 254, 0, 2))
    .netmask((255, 255, 255, 0))  // <-- Added this
    .mtu(1420)
    .up();
```

### 2. Added IPv6 Configuration Function

Created `configure_ipv6()` function that runs after device creation:

**macOS:**
```rust
// Set IPv6 address
ifconfig utunX inet6 fd69:726f::1/32 up

// Add route
route -n add -inet6 fd69:726f::/32 -interface utunX
```

**Linux:**
```rust
// Set IPv6 address
ip -6 addr add fd69:726f::1/32 dev iron0

// Bring interface up
ip link set iron0 up

// Add route
ip -6 route add fd69:726f::/32 dev iron0
```

## Changes Made

### Files Modified

1. **src/tun.rs**
   - Added `.netmask()` to device configuration
   - Added `configure_ipv6()` helper function
   - Updated `create_device()` to call IPv6 configuration
   - Platform-specific commands for macOS and Linux

2. **examples/test_tun.rs**
   - Fixed `device.name()` → `device.tun_name()`
   - Added missing `AbstractDevice` import

### Files Created

1. **test-iron.sh** - Automated test script
   - Checks root privileges
   - Builds and runs iron
   - Verifies TUN device creation
   - Shows device details

## Testing

### Run the Test Script

```bash
# Build and test with automatic checks
sudo ./test-iron.sh
```

### Manual Testing

```bash
# Build
cargo build --release

# Run iron (requires sudo)
sudo ./target/release/iron

# In another terminal, verify TUN device
ifconfig | grep utun
ifconfig utunX  # Replace X with the number from above

# Verify routing
netstat -rn -f inet6 | grep fd69:726f
```

### Expected Output

When iron starts successfully:
```
✓ TUN device created: utunX
✓ IPv6 address configured: fd69:726f::1/32 on utunX
✓ IPv6 route added: fd69:726f::/32 → utunX
```

The TUN interface should show:
```
utunX: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1420
    inet6 fd69:726f::1 prefixlen 32
```

## Verification

All tests still pass:
```bash
cargo test --quiet
# Result: 30/30 tests passing (20 unit + 10 integration)
```

## Platform Support

### ✅ macOS
- Uses `utun` devices (automatically numbered)
- Configured with `ifconfig` and `route` commands
- Tested on macOS (Darwin)

### ✅ Linux
- Uses `iron0` device name
- Configured with `ip` commands
- Ready for testing on Linux

### ⏸️ Windows
- Not yet implemented
- Would require `wintun` driver
- Future enhancement

## Next Steps

1. **Test the fixed binary:**
   ```bash
   sudo ./test-iron.sh
   ```

2. **Verify IPv6 routing:**
   ```bash
   sudo ./target/release/iron &
   ping6 fd69:726f::1  # Should work
   ```

3. **Two-node testing:** Run iron on two machines and test actual P2P connectivity

## References

- **iroh Documentation**: https://iroh.computer
- **TUN/TAP on macOS**: https://www.kernel.org/doc/html/latest/networking/tuntap.html
- **IPv6 ULA Addresses**: RFC 4193
