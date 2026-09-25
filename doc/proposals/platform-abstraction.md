# Proposal: Platform Abstraction

## Status

✅ **Platform seam extracted** on `feat/platform-seam`  
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
  `platform::desktop::DesktopTun`.
- `StatePaths` supplies the persistent-state directory (the key file and
  known-peers cache). Desktop callers can use the OS default, while Android
  supplies the app data directory instead of relying on `$HOME`.
- `DnsResolver::run(SocketAddr)` is the desktop UDP frontend. The
  `IronDnsHandler` resolution logic is portable and can be driven by a
  different frontend on platforms where loopback DNS cannot be configured.
- `platform::desktop::dns_config` contains desktop system DNS integration:
  configuring resolver files or `systemd-resolved` to send `.iron` queries to
  the desktop UDP frontend. This is intentionally separate from DNS
  resolution itself.

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

1. Split the workspace into `iron-core`, `iron-desktop`, `iron-android`, and
   `iron-cli`.
2. Add a CI cross-compilation check for `aarch64-linux-android`.
3. Implement DNS-over-TUN interception and upstream forwarding.
4. Build the Android `cdylib` (using UniFFI) and Kotlin `VpnService`
   application.
