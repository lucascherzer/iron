# Baseline: two nodes reach each other over iron, by address and by name.
{ lib }:
lib.mkTest {
  name = "two-node";
  testScript = ''
    with subtest("TCP in both directions, addressed by .iron name"):
        a.succeed("echo from-a > /tmp/index.html")
        b.succeed("echo from-b > /tmp/index.html")
        for node in [a, b]:
            node.succeed("systemd-run --unit=http-server --collect --no-block python3 -m http.server 8000 --bind :: --directory /tmp")
            node.wait_for_open_port(8000, addr="::1", timeout=timedelta(seconds=30))
        assert a.succeed(f"curl -sf -m 15 http://{b_domain}:8000/").strip() == "from-b"
        assert b.succeed(f"curl -sf -m 15 http://{a_domain}:8000/").strip() == "from-a"
  '';
}
