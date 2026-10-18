# Evaluate real NixOS configurations without booting VMs or building iron.
{ pkgs, self }:
let
  inherit (pkgs) lib;
  evaluate = settings: (import "${pkgs.path}/nixos/lib/eval-config.nix" {
    inherit pkgs;
    system = pkgs.stdenv.hostPlatform.system;
    modules = [
      self.nixosModules.iron
      {
        boot.isContainer = true;
        system.stateVersion = "26.05";
        services.iron = {
          enable = true;
          package = pkgs.hello;
          inherit settings;
        };
      }
    ];
  }).config;

  invalidPaths = [
    { name = "outside"; path = "/var/lib/other/state.db"; }
    { name = "prefix-lookalike"; path = "/var/lib/iron-other/state.db"; }
    { name = "relative"; path = "var/lib/iron/state.db"; }
    { name = "empty"; path = ""; }
    { name = "root-directory"; path = "/var/lib/iron"; }
    { name = "directory-only"; path = "/var/lib/iron/nested/"; }
    { name = "parent-traversal"; path = "/var/lib/iron/../state.db"; }
    { name = "nested-traversal"; path = "/var/lib/iron/a/../../state.db"; }
    { name = "dot-component"; path = "/var/lib/iron/./state.db"; }
    { name = "repeated-separator"; path = "/var/lib/iron//state.db"; }
  ];
  cases = [
    { name = "defaults"; settings = { }; failure = null; }
    {
      name = "nested";
      settings = {
        keyFile = "/var/lib/iron/custom/secret";
        stateFile = "/var/lib/iron/custom/data/state.db";
      };
      failure = null;
    }
    {
      name = "identical";
      settings = {
        keyFile = "/var/lib/iron/same";
        stateFile = "/var/lib/iron/same";
      };
      failure = "services.iron.settings.keyFile and stateFile";
    }
  ] ++ lib.concatMap (field: map (case: {
    name = "${field}-${case.name}";
    settings = { ${field} = case.path; };
    failure = "services.iron.settings.${field}";
  }) invalidPaths) [ "keyFile" "stateFile" ];

  check = case:
    let
      config = evaluate case.settings;
      failed = lib.filter (entry: !entry.assertion) config.assertions;
      deployment = builtins.tryEval config.system.build.toplevel.drvPath;
      sandbox = config.systemd.services.iron.serviceConfig;
      expected = if case.failure == null then
        deployment.success && failed == [ ]
      else
        !deployment.success
        && lib.any (entry: lib.hasPrefix case.failure entry.message) failed;
    in
    assert lib.assertMsg expected "iron module evaluation failed: ${case.name}";
    assert lib.assertMsg (
      sandbox.StateDirectory == "iron"
      && sandbox.StateDirectoryMode == "0700"
      && sandbox.ProtectSystem == "strict"
      && sandbox.ProtectHome == true
      && sandbox.ReadWritePaths == [ "/etc/systemd/resolved.conf.d" ]
    ) "iron module changed sandbox isolation: ${case.name}";
    true;
in
assert lib.all check cases;
pkgs.runCommand "iron-module-check" { } ''
  touch "$out"
''
