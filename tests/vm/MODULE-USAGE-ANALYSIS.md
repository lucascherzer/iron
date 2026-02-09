# NixOS Module Usage in VM Tests - Analysis

## Question

Should we use the `nixosModules.iron` module defined in `flake.nix` for VM test node definitions?

## Current Approach

Tests manually define systemd services:

```nix
systemd.services.iron = {
  description = "iron P2P Network Interface";
  after = [ "network.target" ];
  wantedBy = [ "multi-user.target" ];
  
  serviceConfig = {
    ExecStart = "${ironPackage}/bin/iron serve --log-level debug --dns-port 5333";
    Restart = "always";
    RestartSec = 2;
    AmbientCapabilities = [ "CAP_NET_ADMIN" ];
    CapabilityBoundingSet = [ "CAP_NET_ADMIN" ];
  };
};
```

## Module Approach

```nix
imports = [ self.nixosModules.iron ];

services.iron = {
  enable = true;
  logLevel = "debug";
  dnsPort = 5333;
};
```

## Analysis

### ✅ Pros of Using Module

1. **DRY (Don't Repeat Yourself)**
   - Single source of truth for service definition
   - Changes to production config automatically propagate to tests

2. **Consistency**
   - Tests use exact same config as production deployments
   - Validates the module actually works

3. **Less Boilerplate**
   - ~15 lines → ~5 lines per node
   - Cleaner, more readable test definitions

4. **Security Settings**
   - Module includes hardening (ProtectSystem, ProtectHome, etc.)
   - Tests verify these work correctly

5. **Module Testing**
   - VM tests become integration tests for the module itself
   - Catches module configuration errors

### ❌ Cons of Using Module

1. **Less Test Control**
   - Can't easily tweak service for specific test scenarios
   - Harder to test edge cases (wrong permissions, etc.)

2. **Restart Behavior**
   - Module uses `Restart = "on-failure"` (5s delay)
   - Tests need `Restart = "always"` (2s delay) for chaos testing
   - Connection drop tests require specific restart behavior

3. **Debugging Complexity**
   - Module adds indirection - harder to see what's actually configured
   - Test failures might be module issues vs. iron issues

4. **Flexibility**
   - Some tests need non-standard configurations
   - Reliability test: faster restart, different capabilities
   - Smoke test: might want to test startup failure modes

5. **Dependency**
   - Tests now depend on module implementation
   - Module changes could break tests unintentionally

6. **Import Complexity**
   - Need to pass `self` to test functions
   - More complex flake.nix integration

## Recommendation

### **Short Answer: No, don't use the module in tests (yet)**

### Reasoning

1. **Tests Need Flexibility**
   - Reliability test requires `Restart = "always"` with 2s delay
   - Smoke test might want to test failure modes
   - Manual control is important for testing edge cases

2. **Module is Simple**
   - Only ~30 lines of configuration
   - Not enough complexity to justify abstraction
   - Easy to keep in sync manually

3. **Different Purposes**
   - **Module**: Production deployment (stable, hardened, user-friendly)
   - **Tests**: Validation and chaos testing (flexible, observable, controlled)

4. **Current Approach Works**
   - Tests are clear and explicit
   - Easy to debug when something fails
   - Full control over service lifecycle

### When to Reconsider

Use the module in tests if:

1. **Module Gets Complex**
   - Multiple options, conditional config
   - Hard to keep tests in sync manually

2. **Module Testing Becomes Priority**
   - Want to validate module in real deployments
   - Create dedicated "module validation" test suite

3. **Tests Become Repetitive**
   - Many tests with identical service configs
   - Boilerplate outweighs flexibility needs

## Hybrid Approach (Future)

If we need both, we could:

```nix
# Most tests: use module for consistency
imports = [ self.nixosModules.iron ];
services.iron.enable = true;

# Specific tests: override for flexibility
systemd.services.iron.serviceConfig.Restart = lib.mkForce "always";
systemd.services.iron.serviceConfig.RestartSec = lib.mkForce 2;
```

This gets complex quickly and defeats the purpose.

## Decision

**Keep current approach:**
- Manual service definitions in tests
- Full control for testing scenarios
- Clear, explicit configuration
- Easy to understand and debug

**Add comment in tests explaining why:**
```nix
# Note: We don't use nixosModules.iron because tests need:
# - Direct control over restart behavior (chaos testing)
# - Flexibility for edge case scenarios
# - Explicit configuration for debugging
# The module is tested separately via integration checks.
```

## Related Considerations

### Module Improvements

The module could be enhanced for better testability:

```nix
options.services.iron = {
  enable = mkEnableOption "iron P2P network interface";
  
  # For production
  restart = mkOption {
    type = types.str;
    default = "on-failure";
    description = "Restart policy";
  };
  
  restartSec = mkOption {
    type = types.int;
    default = 5;
    description = "Restart delay in seconds";
  };
  
  # For testing
  extraServiceConfig = mkOption {
    type = types.attrs;
    default = {};
    description = "Extra systemd service configuration";
  };
};
```

But this adds complexity for a rare use case.

## Conclusion

**Status Quo is Best:**
- Tests: Manual service definitions (current approach) ✅
- Production: Use nixosModules.iron (already documented) ✅
- Keep them separate with clear purposes

The 15 lines of boilerplate per test is acceptable for the flexibility and clarity it provides.