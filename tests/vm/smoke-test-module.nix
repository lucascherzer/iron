# NixOS VM smoke test for iron using the flake's nixosModules.iron
#
# This test validates that the NixOS module works correctly in a real VM.
# Unlike smoke-test.nix (which tests the binary directly), this tests
# the production module configuration that users would actually deploy.

{ pkgs, ironPackage, nixosModule }:

pkgs.testers.runNixOSTest {
  name = "iron-smoke-test-module";

  nodes = {
    machine = { config, pkgs, lib, ... }: {
      imports = [ nixosModule ];

      # Enable iron using the module
      services.iron = {
        enable = true;
        logLevel = "debug";
        dnsPort = 5333;
      };

      # Enable networking
      networking.firewall.enable = false;

      # Install test tools
      environment.systemPackages = with pkgs; [
        ironPackage  # For iron CLI commands (key generation, self info)
        dig
        iputils
        iproute2
        jq
      ];

      # Enable systemd-resolved for DNS
      services.resolved.enable = true;
    };
  };

  testScript = ''
    # Start the machine
    machine.start()
    machine.wait_for_unit("multi-user.target")

    # Import the helper module
    ${builtins.readFile ./helpers/smoke_test_module.py}

    # Run the test
    main(machine)
  '';
}
