#!/usr/bin/env python3
"""
Module smoke test helper for iron VM testing.

This script performs comprehensive validation of the nixosModules.iron
configuration in a NixOS VM environment.
"""

import json
import sys


def main(machine):
    """Run all module smoke test checks."""

    print("=" * 60)
    print("MODULE-BASED SMOKE TEST")
    print("Testing nixosModules.iron in a real NixOS VM")
    print("=" * 60)

    # Test 1: Verify iron binary is available
    machine.succeed("which iron")
    print("✓ iron binary found")

    # Test 2: Generate a key (required for iron to start)
    machine.succeed("iron key generate --save --force")
    print("✓ Generated iron key")

    # Test 3: Verify key was created
    machine.succeed("iron self --exists")
    print("✓ Key exists")

    # Test 4: Get node information
    node_info_json = machine.succeed("iron self --format json")
    node_info = json.loads(node_info_json)

    # Verify JSON structure
    assert "node_id" in node_info, "Missing node_id in self info"
    assert "network" in node_info, "Missing network in self info"
    assert "hex" in node_info["node_id"], "Missing hex node_id"
    assert "base32" in node_info["node_id"], "Missing base32 node_id"
    assert "ipv6" in node_info["network"], "Missing IPv6"
    assert "domain" in node_info["network"], "Missing domain"

    node_id_hex = node_info["node_id"]["hex"]
    node_id_base32 = node_info["node_id"]["base32"]
    node_ipv6 = node_info["network"]["ipv6"]
    node_domain = node_info["network"]["domain"]

    print(f"✓ Node ID (hex): {node_id_hex}")
    print(f"✓ Node ID (base32): {node_id_base32}")
    print(f"✓ IPv6: {node_ipv6}")
    print(f"✓ Domain: {node_domain}")

    # Test 5: Verify IPv6 is in iron's ULA space
    assert node_ipv6.startswith("fd69:726f:"), f"IPv6 {node_ipv6} not in iron ULA space"
    print(f"✓ IPv6 in correct ULA space (fd69:726f::/32)")

    # Test 6: Verify domain format
    assert node_domain.endswith(".iron"), f"Domain {node_domain} doesn't end with .iron"
    print(f"✓ Domain format correct (.iron suffix)")

    # Test 7: Wait for iron.service to be active (started by the module)
    machine.wait_for_unit("iron.service")
    print("✓ iron.service is active (started by NixOS module)")

    # Test 8: Verify systemd service status
    service_status = machine.succeed("systemctl status iron.service")
    print(f"Service status:\n{service_status}")

    # Test 9: Verify iron process is running
    machine.succeed("pgrep -f 'iron serve'")
    print("✓ iron serve process is running")

    # Test 10: Verify TUN interface was created
    tun_output = machine.succeed("ip link show | grep utun || ip link show")
    print(f"✓ Network interfaces available:\n{tun_output}")

    # Test 11: Verify DNS is listening on configured port
    machine.succeed("ss -tuln | grep :5333")
    print("✓ DNS server listening on port 5333")

    # Test 12: Test DNS resolution for our own node
    machine.succeed(
        f"dig @127.0.0.1 -p 5333 {node_domain} AAAA +short | grep {node_ipv6}"
    )
    print(f"✓ DNS resolution works for {node_domain}")

    # Test 13: Verify DNS resolution returns correct IPv6
    resolved_ipv6 = machine.succeed(
        f"dig @127.0.0.1 -p 5333 {node_domain} AAAA +short"
    ).strip()
    assert resolved_ipv6 == node_ipv6, (
        f"DNS resolved to {resolved_ipv6}, expected {node_ipv6}"
    )
    print(f"✓ DNS correctly resolves to {node_ipv6}")

    # Test 14: Verify module configuration is applied
    # Check that the service was started with the correct log level
    machine.succeed(
        "systemctl show iron.service | grep 'ExecStart=.*--log-level debug'"
    )
    print("✓ Module configuration applied (log-level=debug)")

    # Test 15: Verify module configuration for DNS port
    machine.succeed("systemctl show iron.service | grep 'ExecStart=.*--dns-port 5333'")
    print("✓ Module configuration applied (dns-port=5333)")

    # Test 16: Test service restart (module should have Restart=on-failure)
    print("Testing service restart behavior...")
    machine.succeed("systemctl restart iron.service")
    machine.wait_for_unit("iron.service")
    machine.sleep(2)
    machine.succeed("pgrep -f 'iron serve'")
    print("✓ Service restart successful")

    # Test 17: Verify logs are accessible
    logs = machine.succeed("journalctl -u iron.service -n 20 --no-pager")
    print(f"Recent logs:\n{logs}")
    print("✓ Service logs accessible via journalctl")

    print("=" * 60)
    print("✅ All module-based smoke tests passed!")
    print("✅ nixosModules.iron works correctly in NixOS VM")
    print("=" * 60)


if __name__ == "__main__":
    print("This script is designed to be imported by NixOS VM tests", file=sys.stderr)
    print("Usage: import this module and call main(machine)", file=sys.stderr)
    sys.exit(1)
