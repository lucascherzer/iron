# Baseline: two nodes reach each other over iron, by address and by name.
{ lib }:
lib.mkTest {
  name = "two-node";
  testScript = ''
    with subtest(".iron names resolve through systemd-resolved"):
        for node in [a, b]:
            node.wait_until_succeeds(f"getent ahostsv6 {b_domain} | grep -q {b_ip}", timeout=30)
            node.wait_until_succeeds(f"getent ahostsv6 {a_domain} | grep -q {a_ip}", timeout=30)

    with subtest("TCP in both directions, addressed by .iron name"):
        a.succeed(f"echo from-a > /tmp/index.html")
        b.succeed(f"echo from-b > /tmp/index.html")
        for node, ip in [(a, a_ip), (b, b_ip)]:
            node.succeed(f"cd /tmp && python3 -m http.server 8000 --bind {ip} >/dev/null 2>&1 &")
            node.wait_for_open_port(8000, addr=ip)
        assert a.succeed(f"curl -sf -m 15 http://{b_domain}:8000/").strip() == "from-b"
        assert b.succeed(f"curl -sf -m 15 http://{a_domain}:8000/").strip() == "from-a"
  '';
}
