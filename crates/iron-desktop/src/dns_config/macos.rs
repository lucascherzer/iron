//! macOS: `/etc/resolver/iron`, which the system resolver consults for the
//! `iron` domain.

use anyhow::{Context, Result};
use std::path::Path;
use tracing::info;

const RESOLVER_DIR: &str = "/etc/resolver";
const RESOLVER_FILE: &str = "/etc/resolver/iron";

/// Resolver file sending `.iron` to `127.0.0.1:port`.
fn config(port: u16) -> String {
    format!(
        "# iron DNS resolver - routes .iron domains to iron's DNS server
# Created by iron
nameserver 127.0.0.1
port {port}
"
    )
}

/// True if `.iron` queries already go to `127.0.0.1:port`. Compares file
/// contents, so a config left over from another `--dns-port` is rewritten.
pub fn is_dns_configured(port: u16) -> bool {
    std::fs::read_to_string(RESOLVER_FILE).is_ok_and(|current| current == config(port))
}

pub fn setup_dns(port: u16) -> Result<()> {
    std::fs::create_dir_all(RESOLVER_DIR)
        .with_context(|| format!("Failed to create {RESOLVER_DIR} (are you root?)"))?;
    std::fs::write(RESOLVER_FILE, config(port))
        .with_context(|| format!("Failed to write {RESOLVER_FILE} (are you root?)"))?;

    info!("DNS configured: {RESOLVER_FILE} created");
    println!("\n✓ DNS configured successfully!");
    println!("\n  .iron domains will now resolve automatically");
    println!("  All other domains use your normal DNS");
    println!("\n  To verify: scutil --dns | grep -A3 iron");
    println!("  To remove: sudo iron --cleanup-dns\n");
    Ok(())
}

pub fn cleanup_dns() -> Result<()> {
    if !Path::new(RESOLVER_FILE).exists() {
        println!("\n✓ DNS configuration not found (already clean)\n");
        return Ok(());
    }
    std::fs::remove_file(RESOLVER_FILE)
        .with_context(|| format!("Failed to remove {RESOLVER_FILE} (are you root?)"))?;

    info!("DNS configuration removed: {RESOLVER_FILE} deleted");
    println!("\n✓ DNS configuration removed successfully!\n");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_uses_port() {
        for port in [5333, 5454] {
            assert!(config(port).contains(&format!("port {port}\n")));
            assert!(config(port).contains("nameserver 127.0.0.1\n"));
        }
    }
}
