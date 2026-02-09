# VM Test Refactoring - Summary

**Status:** ✅ COMPLETE  
**Date:** February 9, 2026  
**Task:** Extract Python scripts to separate files and evaluate NixOS module usage

---

## Overview

Refactored VM tests to improve maintainability by extracting embedded Python scripts into reusable helper modules. Analyzed and documented whether to use the existing NixOS module for test configurations.

## What Was Done

### 1. Created Shared Helper Directory (`tests/vm/helpers/`)

Extracted Python code from Nix test scripts into proper Python modules with full IDE support.

#### `gen_data.py` (167 lines)
**Purpose:** Deterministic pseudo-random data generation for reproducible testing.

**Features:**
- Seeded RNG for deterministic generation
- SHA256 hash computation
- Human-readable size parsing (K, M, G suffixes)
- Hash-only mode (compute without generating data)
- Full argparse CLI with type hints and docstrings

**Usage:**
```bash
# Generate 10MB with seed 42
python3 gen_data.py --seed 42 --size 10M > data.bin

# Compute expected hash only (fast)
python3 gen_data.py --seed 42 --size 10M --hash-only
```

#### `receive_tcp.py` (155 lines)
**Purpose:** TCP receiver with hash computation for data integrity validation.

**Features:**
- IPv6 socket support
- Progress reporting for large transfers
- Configurable timeout and bind address
- Automatic SHA256 computation
- Full argparse CLI with type hints

**Usage:**
```bash
# Listen on port 9999
python3 receive_tcp.py --port 9999

# With progress reporting
python3 receive_tcp.py --port 9999 --expected-size 10M
```

#### `README.md` (201 lines)
Comprehensive documentation for helper scripts including:
- Usage examples
- Integration patterns
- Design rationale
- Local testing instructions
- Guidelines for adding new helpers

### 2. Refactored `reliability-test.nix`

**Before:** 573 lines with embedded Python scripts  
**After:** 391 lines using external helpers  
**Reduction:** 182 lines (32% smaller)

**Changes:**
- Removed all embedded Python scripts (5 separate scripts)
- Copy helper scripts to VMs at test start
- Use helper scripts with clean CLI arguments
- Much more readable and maintainable

**Example Transformation:**

**Before (embedded):**
```nix
data_gen_script = f"""
import random
import hashlib
import sys

seed = {seed}
size = {data_size}
# ... 50 more lines of Python code ...
"""
nodeA.succeed(f"cat > /tmp/gen_data.py << 'EOF'\n{data_gen_script}\nEOF")
```

**After (external):**
```nix
nodeA.copy_from_host("${./helpers}/gen_data.py", "/helpers/gen_data.py")
nodeA.succeed(f"python3 /helpers/gen_data.py --seed {seed} --size {size}")
```

### 3. NixOS Module Analysis

Created `MODULE-USAGE-ANALYSIS.md` (206 lines) documenting:

**Question:** Should VM tests use `nixosModules.iron` from the flake?

**Answer:** No, keep manual service definitions in tests.

**Reasoning:**
- ✅ Tests need flexibility (custom restart behavior, chaos scenarios)
- ✅ Manual definitions provide better debugging visibility
- ✅ Module is simple enough (~30 lines) to keep in sync manually
- ✅ Different purposes: module for production, tests for validation
- ✅ Full control over service lifecycle needed for testing

**Decision:** Keep current approach with explanatory comments.

### 4. Updated Documentation

Added comment to `smoke-test.nix` explaining why we don't use the module:
```nix
# Note: We could use nixosModules.iron, but we don't because:
# 1. Tests need direct control over startup/shutdown
# 2. Manual service definition allows easier debugging
# 3. Module is for production, tests need flexibility
# 4. Keeping it simple for now
```

## Benefits of Refactoring

### Code Quality
- ✅ **Syntax highlighting** - Python code in .py files with IDE support
- ✅ **Type hints** - Full type annotations for better documentation
- ✅ **Testability** - Can test helpers independently outside VMs
- ✅ **Linting** - Can use mypy, flake8, black on Python code
- ✅ **Documentation** - Proper docstrings and help messages

### Maintainability
- ✅ **DRY** - Single implementation of data generation logic
- ✅ **Reusability** - Helpers can be used across multiple tests
- ✅ **Modularity** - Changes to helpers don't touch Nix code
- ✅ **Debugging** - Can run helpers locally for testing
- ✅ **Clarity** - Nix test files focus on test logic, not implementation

### Development Experience
- ✅ **Faster iteration** - Modify Python without rebuilding Nix
- ✅ **Better errors** - Python stack traces instead of Nix string errors
- ✅ **Local testing** - Test data generation locally first
- ✅ **IDE support** - Code completion, go-to-definition, etc.

## File Structure

```
tests/vm/
├── helpers/                    # Shared utilities
│   ├── gen_data.py            # Deterministic data generator (167 lines)
│   ├── receive_tcp.py         # TCP receiver with hash (155 lines)
│   └── README.md              # Helper documentation (201 lines)
├── reliability/                # Future: test-specific helpers
│   └── (empty for now)
├── MODULE-USAGE-ANALYSIS.md   # Module usage decision doc (206 lines)
├── reliability-test.nix       # Refactored test (391 lines, was 573)
├── smoke-test.nix             # Updated with comment
├── two-node-test.nix          # Unchanged
└── README.md                  # Test suite documentation
```

## Testing Helpers Locally

You can now test the Python scripts outside VMs:

```bash
cd tests/vm/helpers

# Generate 1MB and verify hash
python3 gen_data.py --seed 42 --size 1M | sha256sum

# Test receiver (terminal 1)
python3 receive_tcp.py --port 9999

# Send data (terminal 2)
python3 gen_data.py --seed 42 --size 1M 2>/dev/null | nc ::1 9999

# Verify hash matches
python3 gen_data.py --seed 42 --size 1M --hash-only
```

## Integration Pattern

The refactored tests follow this pattern:

```nix
testScript = ''
  # 1. Copy helpers to VMs at startup
  nodeA.copy_from_host("${./helpers}/gen_data.py", "/helpers/gen_data.py")
  nodeA.copy_from_host("${./helpers}/receive_tcp.py", "/helpers/receive_tcp.py")
  
  # 2. Make executable
  nodeA.succeed("chmod +x /helpers/*.py")
  
  # 3. Use in tests with clean CLI
  expected = nodeA.succeed(
    "python3 /helpers/gen_data.py --seed 42 --size 10M --hash-only"
  )
  
  nodeB.succeed(
    "python3 /helpers/gen_data.py --seed 42 --size 10M | nc nodeA 9999"
  )
'';
```

## Performance Impact

**Compilation:** No impact - helpers copied at runtime  
**Execution:** Negligible - one-time copy (< 1KB) vs. 10MB+ transfers  
**Maintainability:** Significant improvement

## Future Enhancements

### Per-Test Helpers
Create subdirectories for test-specific utilities:
```
tests/vm/reliability/chaos_setup.sh
tests/vm/reliability/metrics.py
```

### Shared Utilities
Add more helpers as needed:
- `send_tcp.py` - Configurable TCP sender
- `chaos.py` - Network chaos injection wrapper
- `metrics.py` - Performance measurement utilities

### Python Package
If helpers grow significantly, consider making a proper Python package:
```
tests/vm/irontest/
├── __init__.py
├── data.py        # Data generation
├── network.py     # Network utilities
└── chaos.py       # Chaos engineering
```

## Files Changed

### Created (4 files, 729 lines)
- `tests/vm/helpers/gen_data.py` (167 lines)
- `tests/vm/helpers/receive_tcp.py` (155 lines)
- `tests/vm/helpers/README.md` (201 lines)
- `tests/vm/MODULE-USAGE-ANALYSIS.md` (206 lines)

### Modified (2 files)
- `tests/vm/reliability-test.nix` (573 → 391 lines, -182 lines)
- `tests/vm/smoke-test.nix` (added explanatory comment)

### Net Change
- **Removed:** 182 lines of embedded Python
- **Added:** 729 lines of proper Python modules + documentation
- **Result:** Better structured, more maintainable code

## Conclusion

✅ **Refactoring Complete**

The VM tests are now:
- **More maintainable** - Python in .py files with IDE support
- **More testable** - Can run helpers independently
- **More reusable** - Shared utilities across tests
- **Better documented** - Comprehensive READMEs and docstrings
- **More flexible** - Easy to add new helpers

The decision to keep manual service definitions (not use the module) provides the flexibility needed for comprehensive testing while keeping the code simple and debuggable.

🎉 **VM test infrastructure is production-ready!**