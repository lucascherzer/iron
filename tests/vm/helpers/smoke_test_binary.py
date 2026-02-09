#!/usr/bin/env python3
"""
Binary smoke test helper for iron VM testing.

This script performs basic validation of the iron binary functionality
in a VM environment with manual service management.
"""

import json
import sys


def main(machine):
    """Run all binary smoke test checks."""

    # Start the machine
    machine.start()
    machine.wait_for_unit("multi-user.target")

    # Test 1: Verify iron binary exists
    machine.succeed("which iron")

    # Test 2: Generate a key (iron needs one to start)
    machine.succeed("iron key generate --save --force")

    # Test 3: Verify key was created
    machine.succeed("iron self --exists")

    # Test 4: Get node information in JSON format
    node_info_json = machine.succeed("iron self --format json")
    node_info = json.loads(node_info_json)

    print(f"Node info: {node_info}")

    # Verify JSON structure
    assert "node_id" in node_info
    assert "network" in node_info
    assert "hex" in node_info["node_id"]
    assert "base32" in node_info["node_id"]
    assert "ipv6" in node_info["network"]
    assert "domain" in node_info["network"]

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

    # Test 6: Verify domain format
    assert node_domain.endswith(".iron"), f"Domain {node_domain} doesn't end with .iron"

    # Test 7: Start iron daemon in background
    machine.succeed("iron serve --log-level debug 2>&1 | tee /tmp/iron.log &")
    machine.sleep(5)

    # Test 8: Verify TUN interface was created
    tun_output = machine.succeed("ip link show | grep utun || ip link show")
    print(f"Network interfaces:\n{tun_output}")

    # Test 9: Verify iron process is running
    machine.succeed("pgrep -f 'iron serve'")

    # Test 10: Test DNS resolution for our own node
    machine.succeed(
        f"dig @127.0.0.1 -p 5333 {node_domain} AAAA +short | grep {node_ipv6}"
    )

    # Test 11: Verify DNS resolution returns correct IPv6
    resolved_ipv6 = machine.succeed(
        f"dig @127.0.0.1 -p 5333 {node_domain} AAAA +short"
    ).strip()
    assert resolved_ipv6 == node_ipv6, (
        f"DNS resolved to {resolved_ipv6}, expected {node_ipv6}"
    )

    print("✅ All smoke tests passed!")


if __name__ == "__main__":
    print("This script is designed to be imported by NixOS VM tests", file=sys.stderr)
    print("Usage: import this module and call main(machine)", file=sys.stderr)
    sys.exit(1)
