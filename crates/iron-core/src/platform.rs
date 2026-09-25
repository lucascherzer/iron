//! Platform abstraction layer.
//!
//! Everything OS-specific in iron sits behind the small interfaces in this
//! module; the portable engine ([`crate::router`], [`crate::protocol`],
//! [`crate::dns`], [`crate::mapping`]) never touches the OS directly.
//!
//! The seams are:
//!
//! - [`TunBackend`]: provisions a TUN device (creation, addresses, routes)
//!   and hands back raw packet I/O as [`TunIo`]. On desktop this creates the
//!   device itself and configures it with system commands
//!   (`iron_desktop::DesktopTun`). On
//!   Android the device comes pre-provisioned as a file descriptor from
//!   `VpnService.Builder.establish()`, so a future backend only wraps the fd
//!   (the `tun` crate supports this via `Configuration::raw_fd`).
//! - [`crate::paths::StatePaths`]: where persistent state lives, injected
//!   instead of derived from `$HOME`.
//! - DNS system integration (`iron_desktop::dns_config`): how the OS is told to
//!   send `.iron` queries to iron's resolver. Desktop uses resolver files /
//!   systemd-resolved; Android will instead intercept DNS packets arriving
//!   over the TUN device (see doc/proposals/platform-abstraction.md).

use anyhow::Result;
use futures::{Sink, Stream};
use std::net::Ipv6Addr;
use std::pin::Pin;

/// Raw IP packet stream coming *from* the OS network stack.
pub type PacketStream = Pin<Box<dyn Stream<Item = std::io::Result<Vec<u8>>> + Send>>;

/// Raw IP packet sink going *to* the OS network stack.
pub type PacketSink = Pin<Box<dyn Sink<Vec<u8>, Error = std::io::Error> + Send>>;

/// Bidirectional raw IP packet I/O with the OS network stack.
///
/// This is what a [`TunBackend`] produces and what
/// [`crate::router::PacketRouter::run`] consumes: the router neither knows
/// nor cares whether the packets come from a self-created `utun`/`tun`
/// device or from a file descriptor handed over by the OS.
pub struct TunIo {
    /// Packets written by the OS to the TUN device (outbound to peers).
    pub incoming: PacketStream,
    /// Packets iron writes back to the OS (inbound from peers).
    pub outgoing: PacketSink,
}

/// Provisions the platform's TUN device and exposes its packet I/O.
///
/// Implementations own all device-level concerns: creation, IPv6 address
/// assignment, routes, and MTU. `node_ipv6` is this node's derived address
/// in the `fd69:726f::/32` range.
pub trait TunBackend: Send {
    fn open(&self, node_ipv6: Ipv6Addr) -> Result<TunIo>;
}
