{ pkgs, ironPackage, relayPackage }:

pkgs.testers.runNixOSTest {
  name = "iron-lossy-network";

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

    # --- Bootstrap relay + iron on both nodes ---
    relay_ip = relay.succeed("hostname -I | awk '{print $1}'").strip()
    relay_url = f"http://{relay_ip}:3340"
    print(f"Relay URL: {relay_url}")

    relay.succeed('echo \'http_bind_addr = "0.0.0.0:3340"\' > /tmp/relay-config.toml')
    relay.succeed("iroh-relay --dev --config-path /tmp/relay-config.toml >& /tmp/relay.log &")
    relay.sleep(2)
    relay.succeed(f"curl -s http://{relay_ip}:3340/health 2>&1")

    nodeA.succeed("iron key generate --save --force")
    nodeB.succeed("iron key generate --save --force")

    nodeB.succeed(f"IROH_RELAY_URL={relay_url} iron serve --log-level debug >& /tmp/iron-b.log &")
    nodeB.sleep(3)
    b_info = json.loads(nodeB.succeed("iron self --format json"))
    b_base32 = b_info["node_id"]["base32"]
    b_ipv6 = b_info["network"]["ipv6"]
    print(f"Node B: id={b_base32} ipv6={b_ipv6}")

    nodeA.succeed(
        f"IROH_RELAY_URL={relay_url} "
        f"IROH_PEER_B={b_base32}@{relay_url} "
        "iron serve --log-level debug >& /tmp/iron-a.log &"
    )
    nodeA.sleep(3)
    a_info = json.loads(nodeA.succeed("iron self --format json"))
    a_base32 = a_info["node_id"]["base32"]
    print(f"Node A: id={a_base32}")

    # Restart B with A as peer (bidirectional)
    nodeB.succeed("pkill -f 'iron serve' 2>/dev/null; sleep 2")
    nodeB.succeed(
        f"IROH_RELAY_URL={relay_url} "
        f"IROH_PEER_A={a_base32}@{relay_url} "
        "iron serve --log-level debug >& /tmp/iron-b.log &"
    )
    nodeB.sleep(3)

    # --- Apply packet loss on nodeA's VM network interface ---
    # eth0 is the VM's interface on the shared virtual network.
    # iron's TUN packets get wrapped in QUIC by iroh and sent out via eth0.
    # tc netem drops packets at the kernel level before they leave eth0,
    # simulating a lossy network link. The iron process stays running.
    print("=== Applying 10% packet loss on nodeA's eth0 ===")
    nodeA.succeed("tc qdisc add dev eth0 root netem loss 10%")

    # --- Generate deterministic test data ---
    nodeA.succeed("python3 -c \"\n"
        "import hashlib, random\n"
        "random.seed(42)\n"
        "data = bytes(random.randint(0,255) for _ in range(10*1024*1024))\n"
        "h = hashlib.sha256(data).hexdigest()\n"
        "with open('/tmp/testdata.bin', 'wb') as f: f.write(data)\n"
        "print(f'Expected hash: {h}')\n"
        "\" 2>&1 | tee /tmp/data-hash.txt")

    expected_hash = nodeA.succeed("cat /tmp/data-hash.txt").strip()
    print(f"Expected hash: {expected_hash}")

    # --- Start TCP receiver on node B (listening on iron's TUN IPv6) ---
    nodeB.succeed(
        "python3 -c \"\n"
        "import socket, hashlib\n"
        "sock = socket.socket(socket.AF_INET6, socket.SOCK_STREAM)\n"
        "sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)\n"
        f"sock.bind(('{b_ipv6}', 9999))\n"
        "sock.listen(1)\n"
        "conn, addr = sock.accept()\n"
        "hasher = hashlib.sha256()\n"
        "while True:\n"
        "    data = conn.recv(65536)\n"
        "    if not data: break\n"
        "    hasher.update(data)\n"
        "conn.close()\n"
        "sock.close()\n"
        "print(hasher.hexdigest())\n"
        "\" >& /tmp/receiver-output.txt &\n"
        "RECEIVER_PID=$!\n"
        "echo $RECEIVER_PID > /tmp/receiver.pid"
    )
    nodeB.sleep(1)

    # --- Send data from A to B via TCP over iron's lossy tunnel ---
    nodeA.succeed(
        "python3 -c \"\n"
        "import socket\n"
        "sock = socket.socket(socket.AF_INET6, socket.SOCK_STREAM)\n"
        f"sock.connect(('{b_ipv6}', 9999))\n"
        "with open('/tmp/testdata.bin', 'rb') as f:\n"
        "    while True:\n"
        "        chunk = f.read(65536)\n"
        "        if not chunk: break\n"
        "        sock.sendall(chunk)\n"
        "sock.close()\n"
        "print('Sender done')\n"
        "\" >& /tmp/sender-output.txt &\n"
        "SENDER_PID=$!\n"
        "echo $SENDER_PID > /tmp/sender.pid"
    )

    # Wait for transfer to complete under lossy conditions
    # QUIC retransmits dropped packets, TCP handles the rest
    nodeA.sleep(30)

    # Clean up
    nodeA.succeed("kill $(cat /tmp/sender.pid 2>/dev/null) 2>/dev/null || true")
    nodeB.succeed("kill $(cat /tmp/receiver.pid 2>/dev/null) 2>/dev/null || true")
    nodeB.sleep(1)

    # Remove packet loss
    nodeA.succeed("tc qdisc del dev eth0 root 2>/dev/null || true")

    # Verify results
    sender_output = nodeA.succeed("cat /tmp/sender-output.txt 2>/dev/null || echo 'not found'")
    print(f"Sender output: {sender_output}")

    recv_output = nodeB.succeed("cat /tmp/receiver-output.txt 2>/dev/null || echo 'no output'")
    print(f"Receiver output: {recv_output}")

    if expected_hash in recv_output:
        print("=== Lossy network test PASSED - data integrity verified ===")
    else:
        print(f"FAILED: Hash mismatch! Expected: {expected_hash}, Got: {recv_output}")
        assert False, f"Data integrity check failed: expected {expected_hash}, got {recv_output}"
  '';
}
