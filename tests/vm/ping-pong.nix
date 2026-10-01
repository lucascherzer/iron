{ pkgs, self, pingPong }:
let
  vm = import ./lib.nix { inherit pkgs self; };
  inherit (vm) infra relayPort pkarrPort infraReadyScript;
  peer = { ... }: {
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
  ping = { ... }: {
    imports = [ peer ];
  };
in
pkgs.testers.runNixOSTest {
  name = "iron-vm-ping-pong";
  nodes = { inherit infra ping pong; };
  testScript = { nodes, ... }: ''
    start_all()
  '' + infraReadyScript + ''
    relay = "http://${nodes.infra.networking.primaryIPAddress}:${toString relayPort}"
    pkarr = "http://${nodes.infra.networking.primaryIPAddress}:${toString pkarrPort}/pkarr"
    try:
        with subtest("Pong endpoint starts"):
            for node in [ping, pong]:
                node.wait_for_unit("multi-user.target", timeout=timedelta(seconds=60))
            pong.wait_for_unit("pong.service", timeout=timedelta(seconds=60))
            pong.wait_until_succeeds(
                "journalctl -u pong.service --no-pager | grep -q ENDPOINT_ID=",
                timeout=timedelta(seconds=60),
            )
            journal = pong.succeed("journalctl -u pong.service --no-pager -o cat")
            peer_ids = [
                line.removeprefix("ENDPOINT_ID=")
                for line in journal.splitlines()
                if line.startswith("ENDPOINT_ID=")
            ]
            assert len(peer_ids) == 1, f"expected one pong endpoint ID, got {peer_ids}"
            peer_id = peer_ids[0]

        with subtest("EndpointId-only discovery and ping/pong succeed"):
            result = ping.succeed(
                f"timeout 105 iron-vm-ping-pong client {relay} {pkarr} {peer_id}",
                timeout=timedelta(seconds=110),
            )
            assert "PONG_RECEIVED" in result
            pong.wait_until_succeeds(
                "journalctl -u pong.service --no-pager | grep -q PONG_SENT",
                timeout=timedelta(seconds=10),
            )
    except Exception:
        print(pong.succeed("journalctl -u pong.service --no-pager"))
        raise
  '';
}
