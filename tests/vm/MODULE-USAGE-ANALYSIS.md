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

**Hybrid approach implemented:**

1. **Smoke Test (Module)** - `tests/vm/smoke-test-module.nix`
   - Uses `nixosModules.iron` to validate the production module
   - Tests what users would actually deploy
   - Validates module configuration and systemd integration
   - **Purpose:** Module validation and "happy path" testing

2. **Smoke Test (Binary)** - `tests/vm/smoke-test.nix`
   - Manual service definition for direct binary testing
   - Tests iron binary functionality independently
   - **Purpose:** Binary validation and basic functionality

3. **Reliability/Chaos Tests** - `tests/vm/reliability-test.nix`, etc.
   - Manual service definitions with custom restart policies
   - Full control for chaos engineering (packet loss, disconnects)
   - **Purpose:** Edge cases, fault injection, stress testing

**Rationale:**
- **Module validation is important** - we ship `nixosModules.iron`, so we should test it
- **Flexibility still needed** - chaos tests require fine-grained control
- **Best of both worlds** - validate module + maintain test flexibility

**Add comment in chaos tests explaining why they don't use the module:**
```nix
# Note: We don't use nixosModules.iron in reliability tests because:
# - Need Restart = "always" with 2s delay (faster recovery for chaos tests)
# - Module uses Restart = "on-failure" with 5s delay (production setting)
# - Tests require direct control for fault injection scenarios
# The module itself is validated in smoke-test-module.nix
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

**Hybrid Approach Adopted:**
- **smoke-test-module.nix**: Uses `nixosModules.iron` to validate the module ✅
- **smoke-test.nix**: Manual definition for binary testing ✅
- **reliability-test.nix**: Manual definition for chaos testing ✅
- Production: Use nixosModules.iron (already documented) ✅

This gives us:
- ✅ Module validation (ensures `nixosModules.iron` actually works)
- ✅ Binary validation (tests iron independently)
- ✅ Test flexibility (chaos tests can control service behavior)
- ✅ Real-world testing (module test uses production config)

The small amount of duplication (two smoke tests) is worthwhile for comprehensive coverage.