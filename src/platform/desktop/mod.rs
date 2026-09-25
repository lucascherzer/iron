//! Desktop (Linux/macOS) platform implementation.
//!
//! Provides [`DesktopTun`], a [`TunBackend`] that creates the TUN device
//! itself and configures addresses/routes with system commands, and
//! [`dns_config`], which points the OS resolver at iron's DNS server for
//! `.iron` domains.

pub mod dns_config;

use crate::platform::{TunBackend, TunIo};
use anyhow::{Context, Result};
use futures::StreamExt;
use std::net::Ipv6Addr;
use tracing::{debug, error, info, warn};
use tun::{AsyncDevice, Configuration, Layer};

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

/// Creates and configures the TUN device
///
/// # Platform Notes
///
/// - macOS: Creates a `utun` device (requires root/sudo)
/// - Linux: Creates a `tun` device (requires root/sudo)
///
/// # Configuration
///
/// - IPv6 only (Layer3)
/// - Address: Node's derived IPv6 with /32 prefix
/// - MTU: 1420 bytes (accounts for QUIC overhead)
fn create_device(node_ipv6: Ipv6Addr) -> Result<AsyncDevice> {
    info!("Creating TUN device (requires root/sudo)");
    let mut config = Configuration::default();

    // Configure IPv6-only TUN device (Layer 3)
    // Note: IPv4 addresses are required by tun crate but ignored for IPv6 traffic
    // The actual IPv6 configuration happens in configure_ipv6()
    config
        .layer(Layer::L3)
        .address((169, 254, 0, 1))
        .netmask((255, 255, 255, 0))
        .destination((169, 254, 0, 2))
        .mtu(1420)
        .up();

    #[cfg(target_os = "linux")]
    config.platform_config(|platform_config| {
        platform_config.ensure_root_privileges(true);
    });

    debug!("TUN configuration: {:?}", config);

    let device = match tun::create_as_async(&config) {
        Ok(dev) => {
            debug!("TUN device creation successful");
            dev
        }
        Err(e) => {
            error!("TUN device creation failed: {:?}", e);
            error!("Error kind: {}", e);
            error!("Configuration was: {:?}", config);
            anyhow::bail!("Failed to create TUN device: {} (are you root?)", e);
        }
    };

    let tun_name = match device.as_ref().tun_name() {
        Ok(name) => name,
        Err(e) => {
            error!("Failed to get TUN device name: {:?}", e);
            anyhow::bail!("Failed to get TUN device name: {}", e);
        }
    };
    info!("TUN device created: {}", tun_name);

    // Disable IPv4 on the interface (we only use IPv6)
    match disable_ipv4(&tun_name) {
        Ok(_) => debug!("IPv4 disabled on interface"),
        Err(e) => {
            warn!("Failed to disable IPv4 (non-critical): {}", e);
            // Continue anyway - this is not critical
        }
    }

    // Configure IPv6 address on the interface
    match configure_ipv6(node_ipv6, &tun_name) {
        Ok(_) => debug!("IPv6 configuration successful"),
        Err(e) => {
            error!("IPv6 configuration failed: {:?}", e);
            return Err(e);
        }
    }

    Ok(device)
}

/// Disables IPv4 on the TUN interface
///
/// This ensures the interface only handles IPv6 traffic, preventing
/// the OS from sending any IPv4 packets to the TUN device.
fn disable_ipv4(tun_name: &str) -> Result<()> {
    info!("Disabling IPv4 on {}", tun_name);

    #[cfg(target_os = "macos")]
    {
        // Remove the IPv4 address configuration
        // Format: ifconfig <interface> inet <address> delete
        let output = std::process::Command::new("ifconfig")
            .arg(tun_name)
            .arg("inet")
            .arg("169.254.0.1")
            .arg("delete")
            .output()
            .context("Failed to execute ifconfig")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Might not exist, that's okay
            debug!("IPv4 removal returned: {}", stderr);
        } else {
            info!("IPv4 address removed from {}", tun_name);
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Remove all IPv4 addresses
        let output = std::process::Command::new("ip")
            .arg("-4")
            .arg("addr")
            .arg("flush")
            .arg("dev")
            .arg(tun_name)
            .output()
            .context("Failed to execute ip command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            debug!("IPv4 flush returned: {}", stderr);
        }

        info!("IPv4 disabled on {}", tun_name);
    }

    Ok(())
}

/// Configures IPv6 address and routing on the TUN interface
///
/// Uses system commands to add IPv6 address and route since the tun crate
/// doesn't handle IPv6 configuration automatically on all platforms.
///
/// Sets up:
/// - Interface IPv6 address: Node's derived IPv6 with /32 prefix
/// - Route: fd69:726f::/32 → TUN interface
fn configure_ipv6(node_ipv6: Ipv6Addr, tun_name: &str) -> Result<()> {
    info!(
        "Configuring IPv6 on {} with address {}",
        tun_name, node_ipv6
    );

    #[cfg(target_os = "macos")]
    {
        // Add IPv6 address with /32 prefix
        let ipv6_with_prefix = format!("{}/32", node_ipv6);
        let output = std::process::Command::new("ifconfig")
            .arg(tun_name)
            .arg("inet6")
            .arg(&ipv6_with_prefix)
            .arg("up")
            .output()
            .context("Failed to execute ifconfig")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to configure IPv6 address: {}", stderr);
        }

        info!(
            "IPv6 address configured: {} on {}",
            ipv6_with_prefix, tun_name
        );

        // Add route for the entire fd69:726f::/32 network
        let output = std::process::Command::new("route")
            .arg("-n")
            .arg("add")
            .arg("-inet6")
            .arg("fd69:726f::/32")
            .arg("-interface")
            .arg(tun_name)
            .output()
            .context("Failed to execute route command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Route might already exist, just warn
            warn!("Failed to add route (might already exist): {}", stderr);
        } else {
            info!("IPv6 route added: fd69:726f::/32 → {}", tun_name);
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Add IPv6 address with /32 prefix
        let ipv6_with_prefix = format!("{}/32", node_ipv6);
        let output = std::process::Command::new("ip")
            .arg("-6")
            .arg("addr")
            .arg("add")
            .arg(&ipv6_with_prefix)
            .arg("dev")
            .arg(tun_name)
            .output()
            .context("Failed to execute ip command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to configure IPv6 address: {}", stderr);
        }

        info!(
            "IPv6 address configured: {} on {}",
            ipv6_with_prefix, tun_name
        );

        // Bring the interface up
        let output = std::process::Command::new("ip")
            .arg("link")
            .arg("set")
            .arg(tun_name)
            .arg("up")
            .output()
            .context("Failed to bring interface up")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("Failed to bring interface up: {}", stderr);
        }

        // Add route for the entire fd69:726f::/32 network
        let output = std::process::Command::new("ip")
            .arg("-6")
            .arg("route")
            .arg("add")
            .arg("fd69:726f::/32")
            .arg("dev")
            .arg(tun_name)
            .output()
            .context("Failed to add route")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Route might already exist, just warn
            warn!("Failed to add route (might already exist): {}", stderr);
        } else {
            info!("IPv6 route added: fd69:726f::/32 → {}", tun_name);
        }
    }

    Ok(())
}
