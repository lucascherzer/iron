# Android: Things to Consider

## Status

📝 **Collection** - A running list, not a design.

When a change makes a desktop-only assumption or leaves an open question for
Android, add a bullet here. Keep each entry to a line or two and link the
code or doc it came from. The actual design work happens when the Android
port starts. See the "Android Plan" in
[platform-abstraction.md](./platform-abstraction.md) for the TUN and DNS
approach.

## Configuration

- The TOML file, `IRON_*` env vars, CLI flags and `iron config show` are
  `iron-cli` only. On Android the app's settings screen and storage take
  their place and build `NodeConfig` directly.
- `Settings` (`crates/iron-cli/src/bin/config/mod.rs`) mixes portable
  options (`iroh.*`, `dns_upstream`, `log_level`) with desktop-only ones
  (`key_file`, `state_file`, `dns_port`). Split out a portable part only if
  the app needs to share its validation.
- Validation the app must keep: `IrohConfig::relay_mode()` (already in core).
  The key file ≠ state file check only matters where users pick paths.
- Later: Android Enterprise managed configurations (pushed by an MDM) could
  act as the counterpart of the config file: defaults < managed < user.

## DNS

- `NodeConfig.dns_listen` is required, and `IronNode::run` always starts the
  UDP DNS server (`crates/iron-core/src/node.rs`). Android answers DNS
  through the TUN, so this should become `Option<SocketAddr>`.

## Storage and keys

- State paths come from `Context.getFilesDir()`. The app sandbox makes files
  private, so the `PrivateFileWriter` implementation can be a plain
  create-new write.
- Keep the key in a file, or use the Android Keystore?
- Turso only has a cross-compile check so far. Test it on a device: write,
  reopen, the app being killed mid-write, low storage (see
  [doc/state.md](../state.md)).

## Logging

- `init_tracing` and the filter that covers `iron`, `iron_core` and
  `iron_desktop` live in `iron-cli`. Android needs a logcat subscriber with
  the same filter. Consider moving the filter into a shared helper.

## Lifecycle

- Always-on VPN, Doze and background limits, battery use.
- Network changes (Wi-Fi ↔ mobile data) and how iroh connections recover.
