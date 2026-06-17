{ pkgs, ironPackage, relayPackage }:

pkgs.testers.runNixOSTest {
  name = "iron-two-node-connectivity";

  nodes = {
    relay = { config, pkgs, ... }: {
      networking.firewall.enable = false;
      environment.systemPackages = [ relayPackage ];
    };

    nodeA = { config, pkgs, ... }: {
      networking.firewall.enable = false;
      services.resolved.enable = true;
      environment.systemPackages = with pkgs; [
        ironPackage python3 curl dig iputils
      ];
    };

    nodeB = { config, pkgs, ... }: {
      networking.firewall.enable = false;
      services.resolved.enable = true;
      environment.systemPackages = with pkgs; [
        ironPackage python3 curl dig iputils
      ];
    };
  };

  testScript = ''
    import json

    start_all()
    relay.wait_for_unit("network.target")
    nodeA.wait_for_unit("network.target")
    nodeB.wait_for_unit("network.target")

    relay_ip = relay.succeed("hostname -I | awk '{print $1}'").strip()
    relay_url = f"http://{relay_ip}:3340"
    print(f"Relay URL: {relay_url}")

    # Write relay config and start server
    relay.succeed('echo \'http_bind_addr = "0.0.0.0:3340"\' > /tmp/relay-config.toml')
    relay.succeed(f"iroh-relay --dev --config-path /tmp/relay-config.toml >& /tmp/relay.log &")
    relay.sleep(2)
    relay.succeed(f"curl -s http://{relay_ip}:3340/health 2>&1")

    # Generate keys on both nodes
    nodeA.succeed("iron key generate --save --force")
    nodeB.succeed("iron key generate --save --force")

    # Start node B with custom relay and register its info
    nodeB.succeed(f"IROH_RELAY_URL={relay_url} iron serve --log-level debug >& /tmp/iron-b.log &")
    nodeB.sleep(3)
    b_info = json.loads(nodeB.succeed("iron self --format json"))
    b_base32 = b_info["node_id"]["base32"]
    b_ipv6 = b_info["network"]["ipv6"]
    print(f"Node B: id={b_base32} ipv6={b_ipv6}")

    # Start node A with relay and B's peer address (via relay)
    nodeA.succeed(
        f"IROH_RELAY_URL={relay_url} "
        f"IROH_PEER_B={b_base32}@{relay_url} "
        f"iron serve --log-level debug >& /tmp/iron-a.log &"
    )
    nodeA.sleep(3)
    a_info = json.loads(nodeA.succeed("iron self --format json"))
    a_base32 = a_info["node_id"]["base32"]
    a_ipv6 = a_info["network"]["ipv6"]
    print(f"Node A: id={a_base32} ipv6={a_ipv6}")

    # Restart node B with A as peer so both know each other
    nodeB.succeed("pkill -f 'iron serve' 2>/dev/null; sleep 2")
    nodeB.succeed(
        f"IROH_RELAY_URL={relay_url} "
        f"IROH_PEER_A={a_base32}@{relay_url} "
        f"iron serve --log-level debug >& /tmp/iron-b.log &"
    )
    nodeB.sleep(3)

    # Verify TUN interfaces exist
    tun_exists = lambda node, ipv6: node.succeed(f"ip -6 addr show | grep -q {ipv6}")
    tun_exists(nodeA, a_ipv6)
    tun_exists(nodeB, b_ipv6)

    # Verify DNS resolution works on each node
    nodeA.succeed(f"dig @127.0.0.1 -p 5333 {a_base32}.iron AAAA +short | grep -q {a_ipv6}")
    nodeB.succeed(f"dig @127.0.0.1 -p 5333 {b_base32}.iron AAAA +short | grep -q {b_ipv6}")

    # Test 1: A connects to B via iron network
    nodeB.succeed(f"python3 -m http.server 8080 --bind {b_ipv6} >& /tmp/httpserver-b.log &")
    nodeB.sleep(1)
    result = nodeA.succeed(f"curl -s -m 15 http://[{b_ipv6}]:8080/ 2>&1")
    print(f"A→B curl: {result[:200]}")
    assert len(result) > 0, "Empty response from A→B HTTP"

    # Test 2: B connects to A via iron network
    nodeA.succeed(f"python3 -m http.server 8081 --bind {a_ipv6} >& /tmp/httpserver-a.log &")
    nodeA.sleep(1)
    result2 = nodeB.succeed(f"curl -s -m 15 http://[{a_ipv6}]:8081/ 2>&1")
    print(f"B→A curl: {result2[:200]}")
    assert len(result2) > 0, "Empty response from B→A HTTP"

    print("=== Two-node connectivity test PASSED ===")
  '';
}
