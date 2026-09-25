# Proposal: Platform Abstraction

## Status

✅ **Platform seam extracted** on `feat/platform-seam`  
✅ **Workspace split** (`iron-core` / `iron-desktop` / `iron-cli`)  
🚧 **Android backend and application: Future work**

## Motivation

Iron is a dial-by-key (`<pubkey>.iron`) QUIC E2EE P2P VPN. Its packet
forwarding and `.iron` resolution logic should be reusable on Android, with
iOS and Windows as later targets, without making the portable engine aware of
how a particular operating system provisions a VPN.

The seam is deliberately in-process rather than a split into a privileged
helper and an unprivileged daemon. Android's `VpnService` owns the TUN file
descriptor and expects the VPN datapath to remain in the service process;
background daemons are liable to be killed. An IPC boundary would also copy
every packet in the datapath. A privileged Linux helper can still be added
later as another `TunBackend` implementation if it is useful, without making
the portable router depend on a process model.

## Seams

The platform-independent engine now has four explicit boundaries:

- `TunBackend` provisions a platform TUN device and returns `TunIo`, a raw
  packet stream and sink. `PacketRouter` consumes that I/O and does not create
  devices or run OS commands. The desktop implementation is
  `iron_desktop::DesktopTun`.
- `StatePaths` supplies the persistent-state directory (the key file and
  known-peers cache). Desktop callers can use the OS default, while Android
  supplies the app data directory instead of relying on `$HOME`.
- `DnsResolver::run(SocketAddr)` is the desktop UDP frontend. The
  `IronDnsHandler` resolution logic is portable and can be driven by a
  different frontend on platforms where loopback DNS cannot be configured.
- `iron_desktop::dns_config` contains desktop system DNS integration:
  configuring resolver files or `systemd-resolved` to send `.iron` queries to
  the desktop UDP frontend. This is intentionally separate from DNS
  resolution itself.

## Crate Split

The seams alone are just a convention: nothing stops a later change from
calling `ip route` or adding `#[cfg(target_os = "linux")]` inside the
router, and a single crate would still compile on desktop. The workspace
turns the convention into a build-time boundary:

| Crate | Contents | May depend on OS? |
|---|---|---|
| `iron-core` | router, iroh protocol, DNS resolver, registry, keys, `StatePaths`, `NodeConfig`, the `platform` traits | **No** |
| `iron-desktop` | `DesktopTun`, `dns_config`, `desktop_node_config()` | Yes (Linux/macOS) |
| `iron-cli` | the `iron` binary, root checks, CLI commands | Yes |
| `iron-android` (planned) | fd-backed `TunBackend`, DNS-over-TUN wiring, UniFFI bindings | Yes (Android) |

`iron-core` cannot depend on the platform crates (they depend on it), and
OS-bound dependencies such as `tun` and `nix` are simply not in its
`Cargo.toml`. The `iron-core-android` flake check builds `iron-core` for
`aarch64-linux-android` on every `nix flake check`, so anything that only
works on desktop fails CI instead of surfacing months later during the
Android port.

**Rule:** when a feature needs OS behavior, add a trait to
`iron_core::platform`, implement it per platform crate, and inject it via
`NodeConfig`. If the Android check breaks, fix the layering; do not weaken
or gate the check. The same rule is stated in `AGENTS.md` and in the
`iron-core` crate docs so agents and contributors see it where they work.

## Android Plan

The following is future work, not part of the seam extraction.

The Kotlin `VpnService` will use `VpnService.Builder` to provision the device:

- assign the iron address space and route `fd69:726f::/32`;
- set the MTU (Rayfish uses 1280); and
- call `addDnsServer` with an in-tunnel address such as `fd69:726f::53`.

`Builder.establish()` hands the raw TUN file descriptor to Rust. The Android
backend can wrap that descriptor using the `tun` crate's
`Configuration::raw_fd`, then return a `TunIo` to the same portable router.

Android has no split-DNS API, and `addDnsServer` cannot point at loopback.
Therefore DNS must be intercepted over the TUN device at the magic in-tunnel
address (for example, `fd69:726f::53`). Because that captures all device DNS,
queries outside `.iron` must be forwarded upstream through Android's platform
resolver so Private DNS / DoT remains honored.

[Rayfish](https://github.com/rayfish/rayfish) is an iroh-based mesh VPN that
ships this pattern: `addDnsServer("200::53")`, Rust-side DNS interception,
and `DnsResolver.rawQuery` as a loopback proxy for upstream queries. Two
details are especially worth carrying over:

- use MTU 1280; and
- add a `/128` route to a global IPv6 address so Chrome's IPv6 reachability
  probe succeeds on IPv4-only networks. Without it, Chrome stops asking for
  AAAA records and mesh names fail in browsers.

## Suggested Next Steps

1. Add the Android backend and application crate (future work).
2. ✅ Add a cross-compilation check for `aarch64-linux-android`
   (`nix build .#checks.<system>.iron-core-android`). Note that even without
   linking this needs the NDK's C toolchain: ring compiles C in its build
   script. The flake takes it from `pkgsCross.aarch64-android-prebuilt`.
3. Implement DNS-over-TUN interception and upstream forwarding.
4. Build the Android `cdylib` (using UniFFI) and Kotlin `VpnService`
   application.

The portable seam is implemented in `crates/iron-core/src/platform.rs`; desktop
implementations live in `crates/iron-desktop/src/`.
