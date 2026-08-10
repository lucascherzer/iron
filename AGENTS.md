See doc/arch.md for a general project description.

# Coding Guidelines

We do a lot of vibe coding here, so simplicity of code style, appropriate,
in-place documentation with optional references to other docs are imperative in
order to avoid documentation going out-of-date when developing new features.

Check the doc/ directory when making changes to reflect them.

Agents should strive for: clean, sensibly deduplicated, testable code by
implementing unit- and integration tests.

# Tests

Simple unit tests that can be modeled as "this function should produce these outputs
given these inputs" can and should be implemented following this pattern:

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

### Integration tests

VM-based integration tests live in tests/vm/ and are implemented using the
NixOS test framework (`pkgs.testers.runNixOSTest`). Use these tests for
scenarios that require more than one machine or realistic networking/storage
interactions. This is necessary, as we cannot send data to loopback via the
iron interface.

# Code Style

Agents with access to tools that allow them to execute a formatter should use
them before submitting code for review.
Use `nix flake check` to ensure the repo is in a good state.
Nix may not be available in zsh, try `/nix/var/nix/profiles/default/bin/nix`.

# Project Management
Agents persist their status on feature implementation in doc/plan.md in order to
track progress.

<!-- gortex:communities:start -->
<!-- gortex:skills:start -->
## Community Skills

| Area | Description | Skill |
|------|-------------|-------|
| Src New | 38 symbols | `/gortex-src-new` |
| Src Get Or Assign Ip | 30 symbols | `/gortex-src-get-or-assign-ip` |
| Src 1 Dirs New | 30 symbols | `/gortex-src-1-dirs-new` |
| Bin | 30 symbols | `/gortex-bin` |
| Bin Commands 1 Dirs | 25 symbols | `/gortex-bin-commands-1-dirs` |
| Bin Commands Detect And Convert | 24 symbols | `/gortex-bin-commands-detect-and-convert` |
| Src Raw | 20 symbols | `/gortex-src-raw` |
| Bin Commands Run Vanity | 19 symbols | `/gortex-bin-commands-run-vanity` |
| Src 1 Dirs Platform | 14 symbols | `/gortex-src-1-dirs-platform` |
| Src Run | 12 symbols | `/gortex-src-run` |
| Src Tuninterface | 12 symbols | `/gortex-src-tuninterface` |
| Src Handle Request | 11 symbols | `/gortex-src-handle-request` |
| Src Len | 11 symbols | `/gortex-src-len` |
| Bin Commands Generate | 9 symbols | `/gortex-bin-commands-generate` |
| Src Handle Connection | 9 symbols | `/gortex-src-handle-connection` |
| Src Get Or Connect | 8 symbols | `/gortex-src-get-or-connect` |
| Bin Commands Run Resolve | 5 symbols | `/gortex-bin-commands-run-resolve` |
| Bin Commands Info | 4 symbols | `/gortex-bin-commands-info` |
| Bin Commands Load Key From File | 4 symbols | `/gortex-bin-commands-load-key-from-file` |
<!-- gortex:skills:end -->

<!-- gortex:communities:end -->
