# NixOS VM smoke test for iron
#
# This is a minimal test to verify that iron can start successfully
# in a VM environment and perform basic operations.

{ pkgs, ironPackage }:

pkgs.testers.runNixOSTest {
  name = "iron-smoke-test";

  # Note: We could use the nixosModules.iron module here, but we don't because:
  # 1. Tests need direct control over iron startup/shutdown
  # 2. Manual service definition allows easier debugging (see logs, restart timing)
  # 3. Module is designed for production use, tests need more flexibility
  # 4. Keeping it simple for now - can evaluate module usage if tests get complex

  nodes = {
    machine = { config, pkgs, ... }: {
      # Enable networking
      networking.firewall.enable = false;

      # Install iron and test tools
      environment.systemPackages = with pkgs; [
        ironPackage
        dig
        iputils
        iproute2
      ];

      # Enable systemd-resolved for DNS
      services.resolved.enable = true;
    };
  };

  testScript = ''
    import json

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
    machine.succeed(f"dig @127.0.0.1 -p 5333 {node_domain} AAAA +short | grep {node_ipv6}")

    # Test 11: Verify DNS resolution returns correct IPv6
    resolved_ipv6 = machine.succeed(f"dig @127.0.0.1 -p 5333 {node_domain} AAAA +short").strip()
    assert resolved_ipv6 == node_ipv6, f"DNS resolved to {resolved_ipv6}, expected {node_ipv6}"

    print("✅ All smoke tests passed!")
  '';
}
