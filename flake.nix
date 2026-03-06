{
  description = "iron - P2P network interface based on iroh";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, crane, flake-utils, advisory-db, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        craneLib = crane.mkLib pkgs;

        # Common arguments for all crane builds
        # Changes here will rebuild all dependency crates
        commonArgs = {
          src = craneLib.cleanCargoSource ./.;
          strictDeps = true;

          buildInputs = [ ]
            ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
              # Additional darwin specific inputs can be set here
              pkgs.libiconv
            ];
        };

        # Build *just* the cargo dependencies, so we can reuse them
        # This is the key to incremental builds with crane
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # Build the actual binary
        # Additional args can be added here without rebuilding dependencies
        iron = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;

          meta = with pkgs.lib; {
            description = "P2P network interface based on iroh";
            homepage = "https://github.com/lucascherzer/iron";
            license = with licenses; [ gpl2Plus ];
            mainProgram = "iron";
          };
        });
      in
      {
        # `nix build`
        packages = {
          default = iron;
          inherit iron;
        };

        # `nix run`
        apps.default = flake-utils.lib.mkApp {
          drv = iron;
        };

        # `nix flake check`
        checks = {
          # Build the crate as part of checks
          inherit iron;

          # Run tests
          iron-test = craneLib.cargoTest (commonArgs // {
            inherit cargoArtifacts;
          });

          # Run clippy
          iron-clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          });

          # Check formatting
          iron-fmt = craneLib.cargoFmt {
            src = ./.;
          };

          # Audit dependencies
          iron-audit = craneLib.cargoAudit {
            inherit (commonArgs) src;
            inherit advisory-db;
          };

          # VM-based integration tests (Linux only)
          iron-vm-smoke-test = if pkgs.stdenv.isLinux then
            import ./tests/vm/smoke-test.nix {
              inherit pkgs;
              ironPackage = iron;
            }
          else
            # Skip VM tests on non-Linux platforms
            pkgs.runCommand "iron-vm-smoke-test-skipped" {} ''
              echo "VM smoke test skipped (Linux only)" > $out
            '';

          iron-vm-smoke-test-module = if pkgs.stdenv.isLinux then
            import ./tests/vm/smoke-test-module.nix {
              inherit pkgs;
              ironPackage = iron;
              nixosModule = self.nixosModules.iron;
            }
          else
            # Skip VM tests on non-Linux platforms
            pkgs.runCommand "iron-vm-smoke-test-module-skipped" {} ''
              echo "VM smoke test (module) skipped (Linux only)" > $out
            '';

          iron-vm-two-node-test = if pkgs.stdenv.isLinux then
            import ./tests/vm/two-node-test.nix {
              inherit pkgs;
              ironPackage = iron;
            }
          else
            # Skip VM tests on non-Linux platforms
            pkgs.runCommand "iron-vm-two-node-test-skipped" {} ''
              echo "VM two-node test skipped (Linux only)" > $out
            '';

          iron-vm-reliability-test = if pkgs.stdenv.isLinux then
            import ./tests/vm/reliability-test.nix {
              inherit pkgs;
              ironPackage = iron;
            }
          else
            # Skip VM tests on non-Linux platforms
            pkgs.runCommand "iron-vm-reliability-test-skipped" {} ''
              echo "VM reliability test skipped (Linux only)" > $out
            '';
        };

        # `nix develop`
        devShells.default = craneLib.devShell {
          # Inherit inputs from checks
          checks = self.checks.${system};

          packages = [
            pkgs.rust-analyzer
            pkgs.cargo-watch
            pkgs.cargo-edit
          ];

          # Environment variables for development
          RUST_LOG = "iron=debug";
        };
      }
    ) // {
      # NixOS module for system-wide installation
      nixosModules.iron = { config, lib, pkgs, ... }:
        with lib;
        let
          cfg = config.services.iron;
          pkg = self.packages.${pkgs.system}.iron;

          # Synthesise a TOML config from the declared options.
          # The file is written to the Nix store (read-only) and passed to the
          # daemon via --config.  Mutable runtime state (key, peers cache) lives
          # under StateDirectory (/var/lib/iron) which the binary selects
          # automatically when running as root — so path options are only
          # emitted when the user explicitly overrides the defaults.
          configFile = pkgs.writeText "iron.toml" (
            optionalString (cfg.keyFile != "/var/lib/iron/secret.key") (
              "key_file = \"${cfg.keyFile}\"\n"
            )
            + optionalString (cfg.knownPeersFile != "/var/lib/iron/known_peers.json") (
              "known_peers_file = \"${cfg.knownPeersFile}\"\n"
            )
            + optionalString (cfg.relays != []) (
              "relays = [${concatMapStringsSep ", " (r: ''"${r}"'') cfg.relays}]\n"
            ) + ''

              [firewall]
              enable = ${boolToString cfg.firewall.enable}
            '' + optionalString (cfg.firewall.file != null) (
              "file = \"${cfg.firewall.file}\"\n"
            ));
        in {
          options.services.iron = {
            enable = mkEnableOption "iron P2P network interface";

            logLevel = mkOption {
              type = types.str;
              default = "info";
              description = "Log level (trace, debug, info, warn, error).";
            };

            dnsPort = mkOption {
              type = types.port;
              default = 5333;
              description = "DNS server port.";
            };

            # ── Paths ──────────────────────────────────────────────────────────
            # Both default to /var/lib/iron/ which is created and owned by the
            # service via StateDirectory.  Override only if you want to supply
            # an externally managed key (e.g. from a secrets manager).

            keyFile = mkOption {
              type = types.path;
              default = "/var/lib/iron/secret.key";
              description = ''
                Path to the node secret key file.  The key is generated on
                first startup if absent.  Defaults to
                /var/lib/iron/secret.key.
              '';
            };

            knownPeersFile = mkOption {
              type = types.path;
              default = "/var/lib/iron/known_peers.json";
              description = ''
                Path to the known peers cache file written on shutdown and read
                on startup.  Defaults to
                /var/lib/iron/known_peers.json.
              '';
            };

            # ── Relay servers ──────────────────────────────────────────────────

            relays = mkOption {
              type = types.listOf types.str;
              default = [];
              description = ''
                Relay server URLs.  An empty list (the default) uses iroh's
                built-in relay infrastructure.
              '';
              example = [ "https://relay.example.com" ];
            };

            # ── Firewall ───────────────────────────────────────────────────────

            firewall = {
              enable = mkOption {
                type = types.bool;
                default = true;
                description = "Whether the iron packet firewall is active.";
              };

              file = mkOption {
                type = types.nullOr types.path;
                default = null;
                description = ''
                  Path to the firewall rules JSON file.  When null the default
                  path inside the state directory is used.
                '';
              };
            };
          };

          config = mkIf cfg.enable {
            systemd.services.iron = {
              description = "iron P2P Network Interface";
              after = [ "network.target" ];
              wantedBy = [ "multi-user.target" ];

              serviceConfig = {
                ExecStart = "${pkg}/bin/iron serve"
                  + " --config ${configFile}"
                  + " --log-level ${cfg.logLevel}"
                  + " --dns-port ${toString cfg.dnsPort}";
                Restart = "on-failure";
                RestartSec = 5;

                # /var/lib/iron is created and chowned to the service user
                # automatically by systemd.
                StateDirectory = "iron";
                StateDirectoryMode = "0700";

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
    };
}
