# Proposal: `iron doctor`

## Status

📝 **Proposal** - Not scheduled. Written down after `iron config show` landed.

## Summary

`iron doctor` is a read-only command that explains why `iron serve` would
fail before you start it. It looks at two things:

1. **Configuration**: each layer (defaults, config file, environment, CLI
   flags), the raw values it supplies, and the first layer that makes the
   configuration invalid.
2. **System**: the conditions the daemon needs at runtime, for example key
   and state file permissions, a free DNS port, and TUN privileges.

## Motivation

The configuration is validated strictly after each layer is applied (see
`Sources::settings` in `crates/iron-cli/src/bin/config/mod.rs`). Errors name
the layer that broke the configuration, and type errors also name the
offending key. The output still leaves gaps:

- **`iron config show` needs a valid configuration.** It renders the
  resolved `Settings`, so when a layer is invalid it prints only the error.
  It does not show the values that layer contributed.
- **Cross-field errors don't show their inputs.** For example, "No relays
  left" in the environment layer doesn't say which `IRON_*` variables
  produced that state.
- **Most startup failures aren't configuration errors at all.** Common causes
  include an unreadable or world-readable key, a state database with a newer
  schema, a DNS port already in use, and missing root or TUN privileges.
  Today you find these one at a time by starting the daemon.

A doctor covers all of these in one run. `config show` keeps one job:
printing a valid configuration as TOML.

## Design

### Output

```
$ iron doctor
configuration
  defaults      ok
  config file   /etc/iron/config.toml: ok
  environment   invalid
    IRON_IROH_DEFAULT_RELAYS = "false"
    error: no relays left (default_relays = false, relay_servers empty)
  cli flags     skipped (an earlier layer failed)

system
  key file      /var/lib/iron/secret.key: ok (0600 root)
  state file    /var/lib/iron/state.db: ok (schema v1)
  dns port      5333/udp: in use (pid 812, iron serve)
  privileges    ok (root)

1 problem found.
```

The output has these rules:

- **One line per check.** Each line has a short status: `ok`, `invalid`,
  `missing`, `skipped` or `warning`. A detail line follows only when it helps
  fix the problem.
- **Layers stop at the first failure**, just as `Sources::settings` does.
  Later layers are shown as `skipped`, so the output never contradicts what
  `iron serve` would do.
- **System checks use the configuration up to the last valid layer** if a
  later layer fails, and say so. Without at least the defaults they are
  skipped.
- **Raw values are printed as the layer supplied them.** For the environment
  layer that is the variable name and its string value. For the file it is
  the TOML key and value. For flags it is the flag as typed.
- **Exit code:** `0` when no problems are found, `1` otherwise. Warnings
  alone do not fail the run.

Output that has to be machine-readable (for example `--json`) is left out
until someone needs it.

### Configuration section

`Sources` already keeps the layers apart, so most of the work is already
done:

- Iterate over `Sources::layers`, merge them one at a time, and call the
  existing `validate` after each one (the same loop as `settings()`).
- For each layer, list the keys its provider supplies (`Provider::data`)
  with their raw values.
- On the first error, print that layer's keys and the error, then mark the
  remaining layers `skipped`.

There is no second validation path. If the doctor reports a layer as `ok`,
`iron serve` must accept it too. A test enforces this: for a table of
layer combinations, the doctor's verdict must match whether
`Sources::settings()` succeeds.

### System section

Each check is a small function from the resolved settings to a `Check`
result:

| Check | What it verifies | Platform code |
|---|---|---|
| key file | exists (or its parent is writable for first-run generation), private permissions, parses as a key | permission check in `iron-desktop` |
| state file | parent directory private and writable, schema version supported | schema check reuses `iron-core` state code |
| dns port | UDP port can be bound, or which process holds it | `iron-desktop` |
| privileges | root, or the capabilities needed for TUN and DNS setup | `iron-desktop` |
| config file | not writable by other users (it can redirect key paths) | `iron-desktop` |

All checks are **read-only**. The doctor never creates a key, database,
directory or config file, and never changes permissions. A check whose
target doesn't exist yet reports what `iron serve` would do (for example
"missing; will be generated on first start").

Platform-specific checks go behind the existing `iron_core::platform` seams
or live in `iron-desktop`, following the crate layout rules in `AGENTS.md`.
`iron-core` gets no OS-specific checks.

### A running daemon

The doctor is most useful before a start, but people will also run it while
the daemon is up:

- **DNS port:** if `iron` itself holds the port, report that as `ok
  (held by running iron)` rather than as a conflict.
- **State database:** opening it from a second process while the daemon
  holds it may not be safe with the embedded engine (see
  [doc/state.md](../state.md)). In that case the doctor should skip the
  schema check and report it as `skipped (daemon running)` rather than open
  the file.

## Implementation Complexity

- **Configuration section:** about 50 lines plus tests. It reuses `Sources`
  and `validate`.
- **System section:** about 1 day. Most of the effort goes into the
  per-platform permission and port checks and their tests.

The two sections can ship separately. The configuration section is useful
on its own.

## Open Questions

- **Name.** `iron doctor` (checks the whole setup) or `iron config doctor`
  (configuration only)? This proposal assumes the former, because the system
  checks are the bigger benefit.
- **Without root.** On macOS the default config and state live in a
  root-only directory. Should a non-root doctor report those checks as
  `skipped (needs root)`, or refuse to run?
- **Secrets in output.** No setting is secret today. If one is ever added,
  the raw-value listing must redact it.
- **Schema check while running.** Does the embedded database allow a safe
  read-only open from a second process, or must the doctor always skip it
  while the daemon runs?

## Related

- [doc/cli.md](../cli.md): configuration layering and `config show`
- [doc/state.md](../state.md): state database and schema versioning
- [platform-abstraction.md](./platform-abstraction.md): where OS-specific
  checks belong

## Success Criteria

- For any layer combination, the doctor's configuration verdict matches
  whether `iron serve` accepts the configuration (enforced by a test).
- A broken environment variable, an unreadable key, a port conflict and
  missing privileges are each reported in a single run, with the failing
  value or path named.
- The doctor makes no changes on disk.
