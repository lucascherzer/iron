# Persistent application state

Iron uses the **local, embedded Turso database engine** for mutable application
state. This is the native Rust engine exposed by the `turso` crate, not Turso
Cloud or the older libSQL client. The current dependency is Turso 0.8.1 with
default features disabled; the committed lockfile fixes the resolved versions.
Cloud sync is not enabled. Engine/SDK dependencies remain transitive dependencies
of the supported high-level API.

## Storage boundaries

| Data | Storage | Authority |
| --- | --- | --- |
| Runtime settings | TOML, environment, CLI | Resolved configuration |
| Secret identity key | Separate private key file | Node identity |
| Learned peer EndpointIds | Local application-state database | Persisted peer snapshot |

Never move the secret key into the database or embed key material in TOML.
The database currently stores learned peers only. Hostname aliases, CLI-managed
mappings, and reconciliation of configuration-owned records are future features.
Those features need explicit ownership and conflict rules rather than a generic
copy of configuration fields into database rows.

## Locations and permissions

`state_file`, `IRON_STATE_FILE`, and `--state-file` select the database file.
`key_file` independently selects the identity key. Defaults are:

- Linux: `/var/lib/iron/state.db` and `/var/lib/iron/secret.key`.
- macOS: `/Library/Application Support/iron/state.db` and `secret.key` in
  that directory.
- Android: the application must supply app-private paths and secure file
  creation; the portable core does not discover platform directories.

The state directory must be private and writable, including permission to
create engine sidecars such as the WAL. Desktop creation uses private files
and new directories, but does not chmod arbitrary existing parent directories.
Administrators choosing custom locations are responsible for their privacy.

The NixOS service manages `/var/lib/iron` with `StateDirectoryMode = "0700"`
and retains `ProtectSystem = "strict"` and `ProtectHome = true`. A different
filename or subdirectory within that managed directory does not require a
sandbox exception. The NixOS module rejects paths outside `/var/lib/iron/`,
the managed directory itself, trailing or repeated separators, `.`/`..`
components, and identical key/database paths during configuration evaluation.
It does not broaden the service's write access. These checks are lexical, not
runtime filesystem or symlink inspection; systemd remains the isolation boundary.
Standalone CLI and core callers retain their own paths. The rule that key and
database must differ is deliberately checked in both places: the CLI rejects it
at startup, the Nix module at evaluation time.

Automated VM tests use these paths inside disposable guests, not the host's
iron state. State survives a service restart within one guest but is not shared
between independent automated test executions.

## Schema and lifecycle

Schema version 1 has two tables:

```sql
CREATE TABLE schema_version (version INTEGER NOT NULL);
CREATE TABLE peers (endpoint_id BLOB PRIMARY KEY NOT NULL);
```

The version table must contain exactly one supported version. EndpointIds are
stored as 32-byte BLOBs and parsed as native EndpointIds when loaded. IPv6
addresses are derived from the IDs, not stored as a second source of truth.

`AppState` is the domain API in `iron-core`: callers load or replace a peer
snapshot and do not receive a SQL connection. One mutex owns the connection.
Writes and initial schema creation use Turso's native transaction guards,
including rollback-on-drop before a subsequent operation. Cancellation tests
cover controlled boundaries after beginning a transaction and after a write;
cancelling an operation cannot promise to undo an already completed commit.

Startup opens state and restores the in-memory registry before generating a
missing identity or starting the endpoint. Corrupt or unsupported-version state
causes an error, not an automatic reset. OS DNS configuration is changed only
after node initialization succeeds.

Learned peers are saved as a transactionally replaced snapshot on shutdown,
not synchronously after every lookup or connection. An abrupt process exit can
lose peers learned since the last saved snapshot; these are rediscoverable.
The separate persisted identity is unaffected.

Startup checks the supported version, table/column layout, declared types,
explicit nullability and primary-key status through the engine's schema
metadata, then validates decoded peer values. Incompatible schemas are rejected
without reset; declared type names are matched case-insensitively. These are
checks of our small schema contract, not a proof of physical database integrity
or every possible SQL constraint. For a future schema change, implement and
test a versioned migration; do not merely increment the version constant.

## Legacy migration and backups

The deprecated `iron migrate --from OLD_DIR` command copies the legacy identity
and imports the old `known_peers.json` array of base32 EndpointIds. Repeated
valid IDs are deduplicated; invalid IDs reject the import.

Migration validates sources, stages the database privately, and checkpoints
committed WAL contents before publishing the complete database file. It refuses
existing destinations and leaves originals intact. It is never called
automatically on startup and is marked for removal in the next release.

Do not copy a live main database file without accounting for its WAL. Stop the
service before an ordinary filesystem backup and preserve any remaining engine
sidecars with the database. Keep identity-key backups private and separate from
application-state handling.

## Verification scope

Application tests cover persistence, transaction rollback, invalid state,
legacy import, cancellation, and checkpoint/copy/reopen behavior. Linux Nix
checks exercise the production binary in isolated VM scenarios, and the
Android core build checks portable dependencies.

Compilation is not on-device validation. Android storage behavior on a device
and simulated power-loss or torn-write behavior remain unverified by these
checks. Full synchronization and passing process-exit tests are not a guarantee
against every filesystem or hardware failure.
