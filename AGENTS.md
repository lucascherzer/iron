See doc/arch.md for a general project description.

# Coding Guidelines

We do a lot of vibe coding here, so simplicity of code style, appropriate,
in-place documentation with optional references to other docs are imperative in
order to avoid documentation going out-of-date when developing new features.
Check the doc/ directory when making changes to reflect them.

Agents should strive for: clean, sensibly deduplicated, testable code by
implementing unit- and integration tests.

# Tests

Simple tests that can be modeled as "this function should produce these outputs
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

# Code Style

Agents with access to tools that allow them to execute a formatter should use
them before submitting code for review. We use `rustfmt` with the default
configuration, or one that is present in the repository.

# Project Management
Agents persist their status on feature implementation in doc/plan.md in order to track progress.
