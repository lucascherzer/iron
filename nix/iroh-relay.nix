{
  craneLib,
  iroh-repo,
  pkgs,
}:

rec {

  # The iroh repo's .cargo/config.toml forces -fuse-ld=lld which may not be
  # available on all builders. Strip it so the system default linker is used.
  relaySrc = pkgs.runCommand "iroh-relay-source" { } ''
    cp -r ${iroh-repo} $out
    chmod -R +w $out
    rm -f $out/.cargo/config.toml
  '';

  # Build the iroh relay server binary for VM integration tests.
  # Uses the iroh source from the flake input to build just the
  # iroh-relay crate with the server feature enabled.
  irohRelayDeps = craneLib.buildDepsOnly {
    src = relaySrc;
    pname = "iroh-relay";
    cargoExtraArgs = "--package iroh-relay --features server";
    strictDeps = true;
    nativeBuildInputs =
      with pkgs;
      [ clang ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [ pkgs.libiconv ];
  };
  irohRelay = craneLib.buildPackage {
    src = relaySrc;
    pname = "iroh-relay";
    # we get the version from the cargo.toml in the subdir
    version = (fromTOML (builtins.readFile "${iroh-repo}/iroh-relay/Cargo.toml")).package.version;
    cargoExtraArgs = "--package iroh-relay --features server";
    cargoArtifacts = irohRelayDeps;
    doCheck = false;
    strictDeps = true;
    nativeBuildInputs =
      with pkgs;
      [ clang ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [ pkgs.libiconv ];
  };
}
