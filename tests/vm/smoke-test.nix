# NixOS VM smoke test for iron
#
# This is a minimal test to verify that iron can start successfully
# in a VM environment and perform basic operations.

{ pkgs, ironPackage }:

pkgs.testers.runNixOSTest {
  name = "iron-smoke-test";

  # Note: We could use the nixosModules.iron module here, but we don't because:
  # 1. Tests need direct control over iron startup/shutdown
  # 2. Manual service definition allows easier debugging (see logs, restart timing)
  # 3. Module is designed for production use, tests need more flexibility
  # 4. Keeping it simple for now - can evaluate module usage if tests get complex

  nodes = {
    machine = { config, pkgs, ... }: {
      # Enable networking
      networking.firewall.enable = false;

      # Install iron and test tools
      environment.systemPackages = with pkgs; [
        ironPackage
        dig
        iputils
        iproute2
      ];

      # Enable systemd-resolved for DNS
      services.resolved.enable = true;
    };
  };

  testScript = ''
    # Import the helper module
    ${builtins.readFile ./helpers/smoke_test_binary.py}

    # Run the test
    main(machine)
  '';
}
