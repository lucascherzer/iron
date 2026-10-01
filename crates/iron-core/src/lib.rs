//! Portable engine of iron: packet routing, iroh transport, `.iron` DNS
//! resolution, identity, and node orchestration.
//!
//! # This crate must stay platform-independent
//!
//! iron-core is shared by every platform (Linux, macOS, and the planned
//! Android port, where it is embedded in the app's `VpnService` process).
//! Therefore it must NOT:
//!
//! - create or configure network devices (TUN creation, `ip`/`ifconfig`/
//!   `route`, netlink), or depend on crates that do (`tun`, `nix`, ...);
//! - configure the OS resolver (resolver files, systemd-resolved);
//! - spawn system commands, or check for root/privileges;
//! - derive filesystem locations itself (use the injected
//!   [`StatePaths`]);
//! - use `#[cfg(target_os = ...)]` to switch behavior.
//!
//! OS-specific behavior goes behind a trait in [`platform`] (e.g.
//! [`platform::TunBackend`]) with the implementation in a platform crate
//! (`iron-desktop` today, `iron-android` later), injected through
//! [`NodeConfig`]. If a feature seems to need an OS call in here, add or
//! extend a seam instead.
//!
//! This is enforced by the `iron-core-android` flake check, which builds
//! this crate for `aarch64-linux-android`. Rationale and the Android plan:
//! `doc/proposals/platform-abstraction.md`.

pub mod dns;
pub mod id;
pub mod keys;
pub mod mapping;
pub mod node;
pub mod packet;
pub mod paths;
pub mod platform;
pub mod protocol;
pub mod router;
pub mod test_utils;

pub use node::{IrohInfra, IronNode, NodeConfig};
pub use packet::Packet;
pub use paths::StatePaths;
