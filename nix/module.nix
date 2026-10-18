# NixOS module for the system-wide iron service.
{ self }:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.iron;
  stateDirectory = "iron";
  stateRoot = "/var/lib/${stateDirectory}";
  # Lexical checks give useful deployment errors; systemd remains the actual
  # isolation boundary. No host filesystem probing or implicit write grants.
  validStatePath = path:
    lib.hasPrefix "${stateRoot}/" path
    && lib.all (part: part != "" && part != "." && part != "..")
      (lib.tail (lib.splitString "/" path));
  settings = {
    key_file = cfg.settings.keyFile;
    state_file = cfg.settings.stateFile;
    log_level = cfg.settings.logLevel;
    dns_port = cfg.settings.dnsPort;
    dns_upstream = cfg.settings.dnsUpstream;
    iroh = {
      default_relays = cfg.settings.iroh.defaultRelays;
      relay_servers = cfg.settings.iroh.relayServers;
    } // lib.optionalAttrs (cfg.settings.iroh.pkarrServer != null) {
      pkarr_server = cfg.settings.iroh.pkarrServer;
    };
  };
  configFile = (pkgs.formats.toml { }).generate "iron-config.toml" settings;
  types = lib.types;
  mkOption = lib.mkOption;
in {
  options.services.iron = {
    enable = lib.mkEnableOption "iron P2P network interface";
    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.iron;
      description = "The iron package to run.";
    };
    settings = mkOption {
      default = { };
      description = "Typed iron daemon settings written to TOML.";
      type = types.submodule {
        options = {
          keyFile = mkOption {
            type = types.str;
            default = "${stateRoot}/secret.key";
            description = "Private identity file below ${stateRoot}/, without traversal components.";
          };
          stateFile = mkOption {
            type = types.str;
            default = "${stateRoot}/state.db";
            description = "Application-state file below ${stateRoot}/, without traversal components.";
          };
          logLevel = mkOption { type = types.enum [ "trace" "debug" "info" "warn" "error" ]; default = "info"; };
          dnsPort = mkOption { type = types.port; default = 5333; };
          dnsUpstream = mkOption { type = types.listOf types.str; default = [ ]; };
          iroh = mkOption {
            default = { };
            type = types.submodule {
              options = {
                pkarrServer = mkOption { type = types.nullOr types.str; default = null; };
                defaultRelays = mkOption { type = types.bool; default = true; };
                relayServers = mkOption { type = types.listOf types.str; default = [ ]; };
              };
            };
          };
        };
      };
    };
    logLevel = mkOption {
      type = types.nullOr (types.enum [ "trace" "debug" "info" "warn" "error" ]);
      default = null;
      description = "Compatibility alias for settings.logLevel.";
    };
    dnsPort = mkOption {
      type = types.nullOr types.port;
      default = null;
      description = "Compatibility alias for settings.dnsPort.";
    };
  };
  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.settings.iroh.defaultRelays || cfg.settings.iroh.relayServers != [ ];
        message = "services.iron.settings.iroh must enable defaultRelays or configure relayServers.";
      }
      {
        assertion = validStatePath cfg.settings.keyFile;
        message = "services.iron.settings.keyFile must be an absolute file path below ${stateRoot}/, with no empty, '.' or '..' components.";
      }
      {
        assertion = validStatePath cfg.settings.stateFile;
        message = "services.iron.settings.stateFile must be an absolute file path below ${stateRoot}/, with no empty, '.' or '..' components.";
      }
      {
        assertion = cfg.settings.keyFile != cfg.settings.stateFile;
        message = "services.iron.settings.keyFile and stateFile must refer to different files.";
      }
    ];
    systemd.tmpfiles.rules = [ "d /etc/systemd/resolved.conf.d 0755 root root -" ];
    systemd.services.iron = {
      description = "iron P2P Network Interface";
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];
      wantedBy = [ "multi-user.target" ];
      path = [ pkgs.iproute2 ];
      serviceConfig = {
        ExecStart = "${lib.getExe cfg.package} --config ${configFile} serve";
        Restart = "on-failure";
        RestartSec = 5;
        StateDirectory = stateDirectory;
        StateDirectoryMode = "0700";
        CapabilityBoundingSet = [ "CAP_NET_ADMIN" ];
        AmbientCapabilities = [ "CAP_NET_ADMIN" ];
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ "/etc/systemd/resolved.conf.d" ];
      };
    };
    services.iron.settings.logLevel = lib.mkIf (cfg.logLevel != null) (lib.mkDefault cfg.logLevel);
    services.iron.settings.dnsPort = lib.mkIf (cfg.dnsPort != null) (lib.mkDefault cfg.dnsPort);
  };
}
