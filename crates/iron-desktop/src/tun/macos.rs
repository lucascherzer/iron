//! macOS: `ifconfig` and `route`.

use super::{IRON_PREFIX, run};
use anyhow::Result;
use std::net::Ipv6Addr;
use tracing::{info, warn};

/// Adds `node_ipv6/32` (bringing the interface up) and routes the iron
/// prefix into the device. Only the address is required; a failing route
/// warns (it may already exist).
pub fn configure_ipv6(node_ipv6: Ipv6Addr, tun_name: &str) -> Result<()> {
    let address = format!("{node_ipv6}/32");
    run("ifconfig", &[tun_name, "inet6", &address, "up"])?;
    info!("IPv6 address configured: {} on {}", address, tun_name);

    match run(
        "route",
        &["-n", "add", "-inet6", IRON_PREFIX, "-interface", tun_name],
    ) {
        Ok(()) => info!("IPv6 route added: {} → {}", IRON_PREFIX, tun_name),
        Err(e) => warn!("Failed to add route (might already exist): {e:#}"),
    }
    Ok(())
}
