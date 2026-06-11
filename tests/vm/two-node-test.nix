{ pkgs, ironPackage }:

pkgs.testers.runNixOSTest {
  name = "iron-two-node-connectivity";

  nodes = {
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
    import json, re, time

    start_all()
    nodeA.wait_for_unit("network.target")
    nodeB.wait_for_unit("network.target")

    # Generate keys on both nodes
    nodeA.succeed("iron key generate --save --force")
    nodeB.succeed("iron key generate --save --force")

    # Start node B first with a fixed listen port
    nodeB.succeed("iron serve --listen-port 11222 --log-level debug >& /tmp/iron-b.log &")
    nodeB.sleep(3)

    # Get node B's identity
    b_info = json.loads(nodeB.succeed("iron self --format json"))
    b_base32 = b_info["node_id"]["base32"]
    b_ipv6 = b_info["network"]["ipv6"]
    print(f"Node B: id={b_base32} ipv6={b_ipv6}")

    # Start node A with B as a known peer
    nodeA.succeed(
        f"iron serve --listen-port 11223 "
        f"--add-peer {b_base32}@192.168.1.3:11222 "
        f"--log-level debug >& /tmp/iron-a.log &"
    )
    nodeA.sleep(3)

    # Get node A's identity
    a_info = json.loads(nodeA.succeed("iron self --format json"))
    a_base32 = a_info["node_id"]["base32"]
    a_ipv6 = a_info["network"]["ipv6"]
    print(f"Node A: id={a_base32} ipv6={a_ipv6}")

    # Restart node B with A as known peer (so both know each other)
    nodeB.succeed("pkill -f 'iron serve' 2>/dev/null; sleep 2")
    nodeB.succeed(
        f"iron serve --listen-port 11222 "
        f"--add-peer {a_base32}@192.168.1.2:11223 "
        f"--log-level debug >& /tmp/iron-b.log &"
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
