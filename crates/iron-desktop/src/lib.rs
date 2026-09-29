//! Desktop (Linux/macOS) platform implementation for iron.
//!
//! Provides [`DesktopTun`], a [`iron_core::platform::TunBackend`] that
//! creates the TUN device itself, [`dns_config`], which points the OS
//! resolver at iron's DNS server for `.iron` domains, and
//! [`desktop_node_config`], the desktop defaults for an
//! [`iron_core::IronNode`].

pub mod dns_config;
mod tun;

pub use tun::DesktopTun;

use anyhow::Result;
use iron_core::{NodeConfig, StatePaths};
use std::net::SocketAddr;

/// Desktop defaults: state in `~/.config/iron`, DNS on `127.0.0.1:5333`,
/// self-provisioned TUN device.
pub fn desktop_node_config() -> Result<NodeConfig> {
    Ok(NodeConfig {
        paths: StatePaths::default_os()?,
        dns_listen: SocketAddr::from(([127, 0, 0, 1], 5333)),
        dns_upstream: vec![],
        tun: Box::new(DesktopTun),
    })
}
