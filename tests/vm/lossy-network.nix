# A TCP transfer through iron stays intact on a lossy link.
#
# `tc netem` drops packets on the VM LAN interface of both nodes, below
# iroh, so both directions of the tunnel (and the relay path) lose packets.
# TCP inside the tunnel must still deliver every byte in order.
{ lib }:
lib.mkTest {
  name = "lossy-network";
  testScript = ''
    loss = "5%"  # per node and direction

    b.succeed("head -c 10M /dev/urandom > /tmp/blob")
    b.succeed(f"cd /tmp && python3 -m http.server 8000 --bind {b_ip} >/dev/null 2>&1 &")
    b.wait_for_open_port(8000, addr=b_ip)

    for node in [a, b]:
        node.succeed(f"tc qdisc add dev {LAN_IF} root netem loss {loss}")

    with subtest(f"10 MiB over TCP with {loss} loss per direction"):
        a.succeed(f"curl -sf -m 300 -o /tmp/blob http://[{b_ip}]:8000/blob")
        sent = b.succeed("sha256sum < /tmp/blob")
        received = a.succeed("sha256sum < /tmp/blob")
        assert sent == received, f"payload corrupted: sent {sent}, received {received}"

    for node in [a, b]:
        node.succeed(f"tc qdisc del dev {LAN_IF} root")
  '';
}
