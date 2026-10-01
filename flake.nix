{
  description = "iron - P2P network interface based on iroh";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, crane, rust-overlay, flake-utils, advisory-db, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
        toolchain = pkgs.rust-bin.stable.latest.default.override {
          targets = [ "aarch64-linux-android" ];
        };
        craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;

        # Common arguments for all crane builds
        # Changes here will rebuild all dependency crates
        commonArgs = {
          src = craneLib.cleanCargoSource ./.;
          strictDeps = true;
          pname = "iron";

          buildInputs = [ ]
            ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
              # Additional darwin specific inputs can be set here
              pkgs.libiconv
            ];
        };

        # Build *just* the cargo dependencies, so we can reuse them
        # This is the key to incremental builds with crane
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        pingPong = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          pname = "iron-vm-ping-pong";
          cargoBuildCommand = "cargo build --release -p iron-vm-ping-pong --bin iron-vm-ping-pong";
          meta.mainProgram = "iron-vm-ping-pong";
        });
        # Android C toolchain (NDK clang + bionic sysroot) for the
        # iron-core-android guardrail. `cargo check` doesn't link, but ring
        # (iroh -> rustls) compiles C in its build script, so an Android
        # sysroot is needed even for a check. The NDK is unfree, so it gets
        # its own nixpkgs instance instead of relaxing the main one.
        androidCc = (import nixpkgs {
          inherit system;
          config = {
            allowUnfree = true;
            android_sdk.accept_license = true;
          };
        }).pkgsCross.aarch64-android-prebuilt.stdenv.cc;
        androidCcPrefix = "${androidCc}/bin/aarch64-unknown-linux-android";

        androidCommonArgs = commonArgs // {
          CARGO_BUILD_TARGET = "aarch64-linux-android";
          CC_aarch64_linux_android = "${androidCcPrefix}-clang";
          AR_aarch64_linux_android = "${androidCcPrefix}-ar";
          CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = "${androidCcPrefix}-clang";
          # Desktop-only build inputs (darwin libiconv) must not leak into
          # the Android target.
          buildInputs = [ ];
          # Only iron-core has to build for Android; don't pull desktop/cli
          # deps (or test binaries) into the Android dependency build.
          cargoExtraArgs = "--locked -p iron-core";
          doCheck = false;
        };
        androidCargoArtifacts = craneLib.buildDepsOnly androidCommonArgs;

        # Build the actual binary
        # Additional args can be added here without rebuilding dependencies
        iron = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          cargoBuildCommand = "cargo build --release -p iron-cli --bin iron";

          pname = "iron";
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

          # Guardrail: the portable engine must keep compiling for Android
          # (`cargo build -p iron-core --target aarch64-linux-android`, an
          # rlib, nothing linked), so OS-specific deps can't creep into
          # iron-core.
          iron-core-android = craneLib.cargoBuild (androidCommonArgs // {
            cargoArtifacts = androidCargoArtifacts;
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
        }
        # NixOS VM tests (tests/vm/). They boot Linux VMs, so they need a
        # Linux builder with the `nixos-test` and `kvm` features.
        // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux (
          let
            vm = import ./tests/vm/lib.nix { inherit pkgs self; };
          in
          {
            iron-vm-two-node = import ./tests/vm/two-node.nix { lib = vm; };
            iron-vm-lossy-network = import ./tests/vm/lossy-network.nix { lib = vm; };
            iron-vm-outage = import ./tests/vm/outage.nix { lib = vm; };
            iron-vm-ping-pong = import ./tests/vm/ping-pong.nix { inherit pkgs self pingPong; };
          }
        );

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
      nixosModules.iron = import ./nix/module.nix { inherit self; };
    };
}
