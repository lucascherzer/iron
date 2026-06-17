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
    iroh-repo = {
      url = "github:n0-computer/iroh";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, crane, flake-utils, advisory-db, iroh-repo, ... }:
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

        # Build the iroh relay server binary for VM integration tests.
        # Uses the iroh source from the flake input to build just the
        # iroh-relay crate with the server feature enabled.
        irohRelayDeps = craneLib.buildDepsOnly {
          src = iroh-repo;
          pname = "iroh-relay";
          cargoExtraArgs = "--package iroh-relay --features server";
          strictDeps = true;
          nativeBuildInputs = with pkgs; [ clang ]
            ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [ pkgs.libiconv ];
        };
        irohRelay = craneLib.buildPackage {
          src = iroh-repo;
          pname = "iroh-relay";
          cargoExtraArgs = "--package iroh-relay --features server";
          cargoArtifacts = irohRelayDeps;
          doCheck = false;
          strictDeps = true;
          nativeBuildInputs = with pkgs; [ clang ]
            ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [ pkgs.libiconv ];
        };
      in
      {
        # `nix build`
        packages = {
          default = iron;
          inherit iron;
          iroh-relay = irohRelay;
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
          iron-vm-two-node-test = if pkgs.stdenv.isLinux then
            import ./tests/vm/two-node-test.nix {
              inherit pkgs;
              ironPackage = iron;
              relayPackage = irohRelay;
            }
          else
            pkgs.runCommand "iron-vm-two-node-test-skipped" {} ''
              echo "VM two-node test skipped (Linux only)" > $out
            '';

          iron-vm-reconnect-test = if pkgs.stdenv.isLinux then
            import ./tests/vm/reconnect-test.nix {
              inherit pkgs;
              ironPackage = iron;
              relayPackage = irohRelay;
            }
          else
            pkgs.runCommand "iron-vm-reconnect-test-skipped" {} ''
              echo "VM reconnect test skipped (Linux only)" > $out
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
        in {
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
                ExecStart = "${self.packages.${pkgs.system}.iron}/bin/iron serve --log-level ${cfg.logLevel} --dns-port ${toString cfg.dnsPort}";
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
    };
}
