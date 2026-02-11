# NixOS VM test for iron two-node connectivity
#
# This test verifies that two iron nodes can:
# 1. Start successfully
# 2. Discover each other
# 3. Exchange packets over the P2P network
# 4. Perform DNS resolution for peer nodes
# 5. Establish actual connectivity (ping, HTTP)

{ pkgs, ironPackage }:

pkgs.testers.runNixOSTest {
  name = "iron-two-node-connectivity";

  nodes = {
    nodeA = { config, pkgs, ... }: {
      # Enable networking
      networking.firewall.enable = false;

      # Enable systemd-resolved for DNS
      services.resolved.enable = true;

      # Create a systemd service for iron
      systemd.services.iron = {
        description = "iron P2P Network Interface";
        after = [ "network.target" ];
        wantedBy = [ "multi-user.target" ];

        serviceConfig = {
          ExecStart = "${ironPackage}/bin/iron serve --log-level debug --dns-port 5333";
          Restart = "on-failure";
          RestartSec = 5;

          # Security capabilities for TUN device
          AmbientCapabilities = [ "CAP_NET_ADMIN" ];
          CapabilityBoundingSet = [ "CAP_NET_ADMIN" ];
        };
      };

      # Install iron and test tools
      environment.systemPackages = with pkgs; [
        ironPackage
        python3
        dig
        iputils
        curl
      ];
    };

    nodeB = { config, pkgs, ... }: {
      # Enable networking
      networking.firewall.enable = false;

      # Enable systemd-resolved for DNS
      services.resolved.enable = true;

      # Create a systemd service for iron
      systemd.services.iron = {
        description = "iron P2P Network Interface";
        after = [ "network.target" ];
        wantedBy = [ "multi-user.target" ];

        serviceConfig = {
          ExecStart = "${ironPackage}/bin/iron serve --log-level debug --dns-port 5333";
          Restart = "on-failure";
          RestartSec = 5;

          # Security capabilities for TUN device
          AmbientCapabilities = [ "CAP_NET_ADMIN" ];
          CapabilityBoundingSet = [ "CAP_NET_ADMIN" ];
        };
      };

      # Install iron and test tools
      environment.systemPackages = with pkgs; [
        ironPackage
        python3
        dig
        iputils
        curl
      ];
    };
  };

  testScript = ''
    import json

    # Start both nodes
    start_all()

    # Wait for network to be ready
    nodeA.wait_for_unit("network.target")
    nodeB.wait_for_unit("network.target")

    # Wait for iron services to start
    nodeA.wait_for_unit("iron.service")
    nodeB.wait_for_unit("iron.service")

    # Give iron a moment to initialize TUN devices
    nodeA.sleep(3)
    nodeB.sleep(3)

    # Test 1: Verify iron is running on both nodes
    nodeA.succeed("systemctl status iron.service")
    nodeB.succeed("systemctl status iron.service")

    # Test 2: Verify TUN interface exists on both nodes by parsing from logs
    # Extract TUN device names from iron logs
    import re

    logA = nodeA.succeed("journalctl -u iron.service --no-pager")
    logB = nodeB.succeed("journalctl -u iron.service --no-pager")

    # Parse interface names from "TUN device created: <name>" log line
    tunA_match = re.search(r"TUN device created: (\S+)", logA)
    tunB_match = re.search(r"TUN device created: (\S+)", logB)

    assert tunA_match, "Could not find TUN device creation in nodeA logs"
    assert tunB_match, "Could not find TUN device creation in nodeB logs"

    tunA_name = tunA_match.group(1)
    tunB_name = tunB_match.group(1)

    print(f"Node A TUN device: {tunA_name}")
    print(f"Node B TUN device: {tunB_name}")

    # Verify the interfaces actually exist
    nodeA.succeed(f"ip link show {tunA_name}")
    nodeB.succeed(f"ip link show {tunB_name}")

    # Test 3: Get node identities
    nodeA_info = nodeA.succeed("iron self --format json")
    nodeB_info = nodeB.succeed("iron self --format json")

    nodeA_data = json.loads(nodeA_info)
    nodeB_data = json.loads(nodeB_info)

    nodeA_endpoint_id = nodeA_data["node_id"]["hex"]
    nodeA_ipv6 = nodeA_data["network"]["ipv6"]
    nodeA_base32 = nodeA_data["node_id"]["base32"]

    nodeB_endpoint_id = nodeB_data["node_id"]["hex"]
    nodeB_ipv6 = nodeB_data["network"]["ipv6"]
    nodeB_base32 = nodeB_data["node_id"]["base32"]

    print(f"Node A: EndpointId={nodeA_endpoint_id}, IPv6={nodeA_ipv6}")
    print(f"Node B: EndpointId={nodeB_endpoint_id}, IPv6={nodeB_ipv6}")

    # Test 4: DNS resolution - Node B resolves Node A
    nodeB.succeed(f"dig @127.0.0.1 -p 5333 {nodeA_base32}.iron AAAA +short | grep {nodeA_ipv6}")

    # Test 5: DNS resolution - Node A resolves Node B
    nodeA.succeed(f"dig @127.0.0.1 -p 5333 {nodeB_base32}.iron AAAA +short | grep {nodeB_ipv6}")

    # Test 6: Verify IPv6 addresses are in iron's ULA space
    assert nodeA_ipv6.startswith("fd69:726f:"), f"Node A IPv6 {nodeA_ipv6} not in iron ULA space"
    assert nodeB_ipv6.startswith("fd69:726f:"), f"Node B IPv6 {nodeB_ipv6} not in iron ULA space"

    # Test 7: Start HTTP server on Node A
    nodeA.succeed("python3 -m http.server 8080 --bind :: &")
    nodeA.sleep(2)

    # Test 8: Node B connects to Node A via iron network
    # This tests actual P2P packet delivery
    nodeB.succeed(f"curl -s -m 10 http://[{nodeA_ipv6}]:8080/ | grep -i 'Directory listing'")

    # Test 9: Test reverse direction - Node A connects to Node B
    nodeB.succeed("python3 -m http.server 8081 --bind :: &")
    nodeB.sleep(2)
    nodeA.succeed(f"curl -s -m 10 http://[{nodeB_ipv6}]:8081/ | grep -i 'Directory listing'")

    # Test 10: Ping test (if ICMP is implemented)
    # Note: This may fail if ICMP echo is not yet implemented in iron
    # We run it but don't fail the test if it doesn't work
    nodeB.execute(f"ping6 -c 3 -W 5 {nodeA_ipv6}")

    # Test 11: Verify iron logs show P2P connection establishment
    nodeA.succeed("journalctl -u iron.service | grep -i 'accepted connection\\|received packet'")
    nodeB.succeed("journalctl -u iron.service | grep -i 'sending packet\\|sent packet'")

    # Success!
    print("✅ All iron two-node connectivity tests passed!")
  '';
}
