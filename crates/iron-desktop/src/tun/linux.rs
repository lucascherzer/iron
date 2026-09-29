//! Linux: `ip` from iproute2.

use super::{IRON_PREFIX, run};
use anyhow::Result;
use std::net::Ipv6Addr;
use tracing::{info, warn};

/// Adds `node_ipv6/32`, brings the link up and routes the iron prefix into
/// the device. Only the address is required; the other steps warn on
/// failure (e.g. the route may already exist).
pub fn configure_ipv6(node_ipv6: Ipv6Addr, tun_name: &str) -> Result<()> {
    let address = format!("{node_ipv6}/32");
    run("ip", &["-6", "addr", "add", &address, "dev", tun_name])?;
    info!("IPv6 address configured: {} on {}", address, tun_name);

    if let Err(e) = run("ip", &["link", "set", tun_name, "up"]) {
        warn!("Failed to bring interface up: {e:#}");
    }

    match run("ip", &["-6", "route", "add", IRON_PREFIX, "dev", tun_name]) {
        Ok(()) => info!("IPv6 route added: {} → {}", IRON_PREFIX, tun_name),
        Err(e) => warn!("Failed to add route (might already exist): {e:#}"),
    }
    Ok(())
}
