//! Linux: a systemd-resolved drop-in. Systems without systemd-resolved need
//! manual setup (doc/dns-setup.md).

use anyhow::{Context, Result, bail};
use std::path::Path;
use tracing::{info, warn};

const RESOLVER_DIR: &str = "/etc/systemd/resolved.conf.d";
const RESOLVER_FILE: &str = "/etc/systemd/resolved.conf.d/iron.conf";

fn has_systemd_resolved() -> bool {
    Path::new("/run/systemd/resolve/resolv.conf").exists()
        || Path::new("/etc/systemd/resolved.conf").exists()
}

/// Drop-in sending `.iron` to `127.0.0.1:port`.
fn config(port: u16) -> String {
    format!(
        "# iron DNS resolver - routes .iron domains to iron's DNS server
# Created by iron
[Resolve]
DNS=127.0.0.1:{port}
Domains=~iron
"
    )
}

/// True if `.iron` queries already go to `127.0.0.1:port`. Compares file
/// contents, so a config left over from another `--dns-port` is rewritten.
pub fn is_dns_configured(port: u16) -> bool {
    has_systemd_resolved()
        && std::fs::read_to_string(RESOLVER_FILE).is_ok_and(|current| current == config(port))
}

/// Writes the drop-in and restarts systemd-resolved. Fails without
/// systemd-resolved.
pub fn setup_dns(port: u16) -> Result<()> {
    if !has_systemd_resolved() {
        bail!(
            "automatic DNS setup needs systemd-resolved; configure .iron manually \
             (see doc/dns-setup.md)"
        );
    }
    std::fs::create_dir_all(RESOLVER_DIR)
        .with_context(|| format!("Failed to create {RESOLVER_DIR} (are you root?)"))?;
    std::fs::write(RESOLVER_FILE, config(port))
        .with_context(|| format!("Failed to write {RESOLVER_FILE} (are you root?)"))?;
    restart_resolved()?;

    info!("DNS configured: {RESOLVER_FILE} created");
    println!("\n✓ DNS configured successfully!");
    println!("\n  .iron domains will now resolve automatically");
    println!("  All other domains use your normal DNS");
    println!("\n  To verify: resolvectl status");
    println!("  To remove: sudo iron --cleanup-dns\n");
    Ok(())
}

pub fn cleanup_dns() -> Result<()> {
    if !has_systemd_resolved() {
        println!("\n⚠️  Automatic DNS cleanup not available for your system");
        println!("   Please remove DNS configuration manually if you added it.\n");
        return Ok(());
    }
    if !Path::new(RESOLVER_FILE).exists() {
        println!("\n✓ DNS configuration not found (already clean)\n");
        return Ok(());
    }
    std::fs::remove_file(RESOLVER_FILE)
        .with_context(|| format!("Failed to remove {RESOLVER_FILE} (are you root?)"))?;
    restart_resolved()?;

    info!("DNS configuration removed: {RESOLVER_FILE} deleted");
    println!("\n✓ DNS configuration removed successfully!\n");
    Ok(())
}

/// systemd-resolved only reads drop-ins on (re)start.
fn restart_resolved() -> Result<()> {
    info!("Restarting systemd-resolved");
    let status = std::process::Command::new("systemctl")
        .args(["restart", "systemd-resolved"])
        .status()
        .context("Failed to restart systemd-resolved")?;
    if !status.success() {
        warn!("systemd-resolved restart returned non-zero status");
        println!("\n⚠️  Warning: systemd-resolved restart failed");
        println!(
            "   You may need to restart it manually: sudo systemctl restart systemd-resolved\n"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_uses_port() {
        for port in [5333, 5454] {
            assert!(config(port).contains(&format!("DNS=127.0.0.1:{port}\n")));
            assert!(config(port).contains("Domains=~iron\n"));
        }
    }
}
