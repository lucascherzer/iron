See doc/arch.md for a general project description.

# Coding Guidelines

We do a lot of vibe coding here, so simplicity of code style, appropriate, in-place documentation with optional references to other docs are imperative in order to avoid documentation going out-of-date when developing new features.

Agents should strive for: clean, sensibly deduplicated, testable code by implementing unit- and integration tests.

Before you start work on a change, discuss the high-level details with the user.
Check the doc/ directory, maybe the change you are working on has already been considered or is tangentially related to another change.
In that case, bring it up to the user explicitly and make it clear which information comes from the doc, to ensure that information from stale docs is caught during the design phase.


# Crate Layout (do not violate)

- `crates/iron-core`: portable engine, shared by all platforms including the
  planned Android app. **No platform-specific code here**: no TUN device
  setup, no system commands, no OS resolver config, no `cfg(target_os)`, no
  OS-bound crates (`tun`, `nix`, ...). Rules and rationale: the crate docs in
  `crates/iron-core/src/lib.rs`.
- `crates/iron-desktop`: Linux/macOS implementations of the seams in
  `iron_core::platform` (TUN provisioning, DNS system config).
- `crates/iron-cli`: the `iron` binary.

Need OS behavior in the core? Add a trait to `iron_core::platform`, implement it in the platform crate, inject it via `NodeConfig`. The `iron-core-android` check in `nix flake check` fails if iron-core stops building for Android; fix the layering, never weaken that check. Background: `doc/proposals/platform-abstraction.md`.

# Tests

Simple tests that can be modeled as "this function should produce these outputs given these inputs" can and should be implemented following this pattern:

```rs
fn add(x: usize, y: usize) -> usize { x+y }
fn test_add() {
    let tests = [(1, 1, 2), ...]
    fn run_tests(case: (usize, usize, usize)) {
        assert_eq!(add(case.0, case.1), case.2, format!("adding {} and {} should result in {}", ...))
    }
    for t in test {
        run_tests(t)
    }
}
```

# Code Style

Agents with access to tools that allow them to execute a formatter should use them before submitting code for review.
Use `nix flake check` to ensure the repo is in a good state.
Nix may not be available in zsh, try `/nix/var/nix/profiles/default/bin/nix`.
