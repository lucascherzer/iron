# Shared harness for iron's NixOS VM tests.
#
# Every test runs three VMs on one virtual LAN:
# - `infra`: iroh relay + iroh-dns-server (pkarr), so the test needs no
#   internet access.
# - `a`, `b`: iron nodes, run via the NixOS module (nix/module.nix).
#
# `mkTest` prepends a Python prelude to the test script that boots
# everything and defines:
# - `a_ip`, `b_ip`: the nodes' iron IPv6 addresses
# - `a_domain`, `b_domain`: their `.iron` names
# - `LAN_IF`: the interface carrying VM-to-VM traffic, for `tc netem`
{ pkgs, self }:
let
  relayPort = 3340;
  pkarrPort = 8080;

  infra = { lib, ... }: {
    networking.firewall.enable = false;

    systemd.services.iroh-relay = {
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      # --dev: plain HTTP on [::]:3340.
      serviceConfig.ExecStart = "${lib.getExe pkgs.iroh-relay} --dev";
      serviceConfig.DynamicUser = true;
    };

    systemd.services.iroh-dns-server = {
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      serviceConfig.ExecStart =
        let
          config = (pkgs.formats.toml { }).generate "iroh-dns-server.toml" {
            # Every node publishes on startup; don't throttle the test.
            pkarr_put_rate_limit = "disabled";
            http = {
              port = pkarrPort;
              bind_addr = "0.0.0.0";
            };
            # Required section; nodes only use the HTTP pkarr endpoint.
            dns = {
              port = 5300;
              bind_addr = "0.0.0.0";
              default_soa = "ns1.iroh.test hostmaster.iroh.test 0 10800 3600 604800 3600";
              default_ttl = 30;
              origins = [ "iroh.test." ];
              rr_a = "127.0.0.1";
              rr_ns = "ns1.iroh.test.";
            };
            mainline.enabled = false;
            data_dir = "/var/lib/iroh-dns-server";
          };
        in
        "${lib.getExe pkgs.iroh-dns-server} --config ${config}";
      serviceConfig.DynamicUser = true;
      serviceConfig.StateDirectory = "iroh-dns-server";
    };
  };

  ironNode = { nodes, lib, ... }:
    let
      infraIp = nodes.infra.networking.primaryIPAddress;
    in
    {
      imports = [ self.nixosModules.iron ];
      services.iron = {
        enable = true;
        logLevel = "debug";
        relayUrl = "http://${infraIp}:${toString relayPort}";
        pkarrUrl = "http://${infraIp}:${toString pkarrPort}/pkarr";
      };
      # Started by the prelude once infra is up, so the first pkarr publish
      # and relay connection don't race the infra VM's boot.
      systemd.services.iron.wantedBy = lib.mkForce [ ];
      # iron points `.iron` at itself via a systemd-resolved drop-in.
      services.resolved.enable = true;
      # iroh uses random UDP ports.
      networking.firewall.enable = false;
      environment.systemPackages = [ pkgs.curl pkgs.python3 ];
    };

  prelude = ''
    LAN_IF = "eth1"  # eth0 is QEMU user networking; VMs talk over eth1

    start_all()
    infra.wait_for_open_port(${toString relayPort})
    infra.wait_for_open_port(${toString pkarrPort})
    infra.succeed("curl -sf http://localhost:${toString relayPort}/healthz")
    infra.succeed("curl -sf http://localhost:${toString pkarrPort}/healthcheck")

    def iron_self(node, flag):
        return node.succeed(f"HOME=/var/lib/iron iron self --{flag}").strip()

    for node in [a, b]:
        node.wait_for_unit("multi-user.target")
        node.systemctl("start iron.service")
        node.wait_for_unit("iron.service")
        node.wait_until_succeeds("HOME=/var/lib/iron iron self --exists")

    a_ip, b_ip = iron_self(a, "ipv6"), iron_self(b, "ipv6")
    a_domain, b_domain = iron_self(a, "domain"), iron_self(b, "domain")
    print(f"a: {a_domain} {a_ip}")
    print(f"b: {b_domain} {b_ip}")

    # Both nodes are reachable once pkarr records are published and the
    # first connection is up.
    a.wait_until_succeeds(f"ping -c1 -W2 {b_ip}", timeout=120)
    b.wait_until_succeeds(f"ping -c1 -W2 {a_ip}", timeout=120)
  '';
in
{
  mkTest = { name, testScript }:
    pkgs.testers.runNixOSTest {
      name = "iron-${name}";
      nodes = {
        inherit infra;
        a = ironNode;
        b = ironNode;
      };
      testScript = prelude + testScript;
    };
}
