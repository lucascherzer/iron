//! Manual check of the desktop TUN setup, without the rest of iron:
//!
//! ```sh
//! sudo cargo run -p iron-desktop --example test_tun
//! ```
//!
//! Creates the device exactly as `iron serve` does (IPv6-only, node address,
//! `fd69:726f::/32` route) and keeps it up until Enter is pressed, so it can
//! be inspected with `ifconfig` / `ip addr` and `netstat -rn` / `ip -6 route`.

use iron_core::platform::TunBackend;
use iron_desktop::DesktopTun;
use std::net::Ipv6Addr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("debug").init();
    let node_ipv6 = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
    let _io = DesktopTun.open(node_ipv6)?;
    println!("TUN device is up with {node_ipv6}. Inspect it, then press Enter to remove it.");
    std::io::stdin().read_line(&mut String::new())?;
    Ok(())
}
