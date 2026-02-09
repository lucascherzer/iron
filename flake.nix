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
    microvm = {
      url = "github:astro/microvm.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, crane, flake-utils, advisory-db, microvm, ... }:
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
