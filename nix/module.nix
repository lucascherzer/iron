# NixOS module: runs `iron serve` as a system service.
#
# Exercised by the VM tests in tests/vm/, which use it to run every node.
{ self }:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.iron;
  # iron keeps its state in $HOME/.config/iron.
  home = "/var/lib/iron";
in
{
  options.services.iron = {
    enable = lib.mkEnableOption "iron P2P network interface";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.iron;
      defaultText = lib.literalExpression "iron.packages.\${system}.iron";
      description = "The iron package to run.";
    };

    logLevel = lib.mkOption {
      type = lib.types.str;
      default = "info";
      description = "Log level (trace, debug, info, warn, error)";
    };

    dnsPort = lib.mkOption {
      type = lib.types.port;
      default = 5333;
      description = "DNS server port";
    };

    relayUrl = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "https://relay.example.com";
      description = "Relay server to use instead of n0's public relays.";
    };

    pkarrUrl = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "https://dns.example.com/pkarr";
      description = "pkarr server to publish to and resolve peers from, instead of n0's.";
    };
  };

  config = lib.mkIf cfg.enable {
    # iron writes a systemd-resolved drop-in here so `.iron` names resolve;
    # it must exist to be listed in ReadWritePaths below.
    systemd.tmpfiles.rules = [ "d /etc/systemd/resolved.conf.d 0755 root root -" ];

    systemd.services.iron = {
      description = "iron P2P Network Interface";
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];
      wantedBy = [ "multi-user.target" ];
      # TUN setup shells out to `ip`.
      path = [ pkgs.iproute2 ];
      environment.HOME = home;

      serviceConfig = {
        ExecStart = lib.escapeShellArgs (
          [
            (lib.getExe cfg.package)
            "serve"
            "--log-level"
            cfg.logLevel
            "--dns-port"
            (toString cfg.dnsPort)
          ]
          ++ lib.optionals (cfg.relayUrl != null) [ "--relay-url" cfg.relayUrl ]
          ++ lib.optionals (cfg.pkarrUrl != null) [ "--pkarr-url" cfg.pkarrUrl ]
        );
        Restart = "on-failure";
        RestartSec = 5;
        StateDirectory = "iron";

        # Security hardening
        CapabilityBoundingSet = [ "CAP_NET_ADMIN" ];
        AmbientCapabilities = [ "CAP_NET_ADMIN" ];
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ "/etc/systemd/resolved.conf.d" ];
      };
    };
  };
}
