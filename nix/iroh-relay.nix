{
  craneLib,
  iroh-repo,
  pkgs,
}:

rec {

  # Build the iroh relay server binary for VM integration tests.
  # Uses the iroh source from the flake input to build just the
  # iroh-relay crate with the server feature enabled.
  irohRelayDeps = craneLib.buildDepsOnly {
    src = iroh-repo;
    pname = "iroh-relay";
    cargoExtraArgs = "--package iroh-relay --features server";
    strictDeps = true;
    nativeBuildInputs =
      with pkgs;
      [ clang ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [ pkgs.libiconv ];
  };
  irohRelay = craneLib.buildPackage {
    src = iroh-repo;
    pname = "iroh-relay";
    # we get the version from the cargo.toml in the subdir
    version = fromTOML (builtins.readFile "${iroh-repo}/iroh-relay/Cargo.toml"."version");
    cargoExtraArgs = "--package iroh-relay --features server";
    cargoArtifacts = irohRelayDeps;
    doCheck = false;
    strictDeps = true;
    nativeBuildInputs =
      with pkgs;
      [ clang ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [ pkgs.libiconv ];
  };
}
