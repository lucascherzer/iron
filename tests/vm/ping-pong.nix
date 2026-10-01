{ pkgs, self, pingPong }:
let
  infra = (import ./lib.nix { inherit pkgs self; }).infra;
  relayPort = 3340;
  pkarrPort = 8080;
  peer = { nodes, ... }: {
    networking.firewall.enable = false;
    environment.systemPackages = [ pingPong ];
  };
  pong = { nodes, ... }: {
    imports = [ peer ];
    systemd.services.pong = {
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      serviceConfig.ExecStart = "${pkgs.lib.getExe pingPong} server http://${nodes.infra.networking.primaryIPAddress}:${toString relayPort} http://${nodes.infra.networking.primaryIPAddress}:${toString pkarrPort}/pkarr";
    };
  };
  ping = { nodes, ... }: {
    imports = [ peer ];
  };
in
pkgs.testers.runNixOSTest {
  name = "iron-vm-ping-pong";
  nodes = { inherit infra ping pong; };
  testScript = { nodes, ... }: ''
    start_all()
    # Retry health checks: an open socket alone does not establish readiness.
    try:
        infra.wait_until_succeeds(
            "curl -sf --max-time 2 http://localhost:${toString relayPort}/healthz",
            timeout=60,
        )
        infra.wait_until_succeeds(
            "curl -sf --max-time 2 http://localhost:${toString pkarrPort}/healthcheck",
            timeout=60,
        )
    except Exception:
        print(infra.succeed("journalctl -u iroh-relay -u iroh-dns-server --no-pager"))
        raise

    for node in [ping, pong]:
        node.wait_for_unit("multi-user.target")
    pong.wait_for_unit("pong.service")
    pong.wait_until_succeeds("journalctl -u pong.service --no-pager | grep -q ENDPOINT_ID=", timeout=60)
    peer_id = pong.succeed(
        "journalctl -u pong.service --no-pager | sed -n 's/.*ENDPOINT_ID=\\([^ ]*\\).*/\\1/p' | tail -1"
    ).strip()
    assert peer_id, "pong endpoint id not present in journal"
    relay = "http://${nodes.infra.networking.primaryIPAddress}:${toString relayPort}"
    pkarr = "http://${nodes.infra.networking.primaryIPAddress}:${toString pkarrPort}/pkarr"
    result = ping.succeed(
        f"timeout 105 iron-vm-ping-pong client {relay} {pkarr} {peer_id}",
        timeout=110,
    )
    assert "PONG_RECEIVED" in result
    pong.succeed("journalctl -u pong.service --no-pager | grep -q PONG_SENT")
  '';
}
