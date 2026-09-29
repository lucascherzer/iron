//! Desktop TUN backend: creates the device with the `tun` crate, then
//! assigns the node's IPv6 address and the `fd69:726f::/32` route with system
//! commands (`ip` on Linux, `ifconfig`/`route` on macOS), because the `tun`
//! crate only configures IPv4.
//!
//! The device is IPv6-only: no IPv4 address is configured at all (the `tun`
//! crate treats address, destination and netmask as optional).
//!
//! OS-specific commands live in one module per OS with the same
//! `configure_ipv6` function, like `crate::dns_config`. Both use only std, so
//! test builds compile both on every host.

use anyhow::{Context, Result, bail};
use futures::StreamExt;
use iron_core::platform::{TunBackend, TunIo};
use std::net::Ipv6Addr;
use std::process::Command;
use tracing::{debug, info};
use tun::{AsyncDevice, Configuration, Layer};

#[cfg(any(target_os = "linux", test))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod linux;
#[cfg(target_os = "linux")]
use linux::configure_ipv6;

#[cfg(any(target_os = "macos", test))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod macos;
#[cfg(target_os = "macos")]
use macos::configure_ipv6;

/// The prefix routed into the TUN device.
const IRON_PREFIX: &str = "fd69:726f::/32";

/// MTU of the TUN device; leaves room for QUIC/UDP/IP overhead on a
/// 1500-byte path.
const MTU: u16 = 1420;

/// TUN backend that creates and configures the device itself.
///
/// Requires root/sudo: TUN device creation and route setup are privileged
/// operations on both Linux and macOS.
pub struct DesktopTun;

impl TunBackend for DesktopTun {
    fn open(&self, node_ipv6: Ipv6Addr) -> Result<TunIo> {
        let device = create_device(node_ipv6)?;
        let framed = device.into_framed();
        let (outgoing, incoming) = framed.split();
        Ok(TunIo {
            incoming: Box::pin(incoming),
            outgoing: Box::pin(outgoing),
        })
    }
}

/// Creates an IPv6-only layer-3 TUN device (`utun*` on macOS, `tun*` on
/// Linux) with the node's address and the iron route.
fn create_device(node_ipv6: Ipv6Addr) -> Result<AsyncDevice> {
    info!("Creating TUN device (requires root/sudo)");
    let mut config = Configuration::default();
    config.layer(Layer::L3).mtu(MTU).up();

    #[cfg(target_os = "linux")]
    config.platform_config(|platform_config| {
        platform_config.ensure_root_privileges(true);
    });

    debug!("TUN configuration: {:?}", config);
    let device = tun::create_as_async(&config).with_context(|| {
        format!("Failed to create TUN device (are you root?); config: {config:?}")
    })?;
    let tun_name = device
        .as_ref()
        .tun_name()
        .context("Failed to get TUN device name")?;
    info!("TUN device created: {}", tun_name);

    info!(
        "Configuring IPv6 on {} with address {}",
        tun_name, node_ipv6
    );
    configure_ipv6(node_ipv6, &tun_name).context("IPv6 configuration failed")?;
    Ok(device)
}

/// Runs a system command; errors include its stderr.
fn run(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("Failed to execute {program}"))?;
    if !output.status.success() {
        bail!(
            "`{program} {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}
