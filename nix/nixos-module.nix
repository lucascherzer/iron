{ self, ... }:

{
  # NixOS module for system-wide installation
  nixosModules.iron =
    {
      config,
      lib,
      pkgs,
      ...
    }:
    with lib;
    let
      cfg = config.services.iron;
    in
    {
      options.services.iron = {
        enable = mkEnableOption "iron P2P network interface";

        logLevel = mkOption {
          type = types.str;
          default = "info";
          description = "Log level (trace, debug, info, warn, error)";
        };

        dnsPort = mkOption {
          type = types.port;
          default = 5333;
          description = "DNS server port";
        };
      };

      config = mkIf cfg.enable {
        systemd.services.iron = {
          description = "iron P2P Network Interface";
          after = [ "network.target" ];
          wantedBy = [ "multi-user.target" ];

          serviceConfig = {
            ExecStart = "${
              self.packages.${pkgs.system}.iron
            }/bin/iron serve --log-level ${cfg.logLevel} --dns-port ${toString cfg.dnsPort}";
            Restart = "on-failure";
            RestartSec = 5;

            # Security hardening
            CapabilityBoundingSet = [ "CAP_NET_ADMIN" ];
            AmbientCapabilities = [ "CAP_NET_ADMIN" ];
            NoNewPrivileges = true;
            PrivateTmp = true;
            ProtectSystem = "strict";
            ProtectHome = true;
          };
        };
      };
    };
}
