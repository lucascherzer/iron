use anyhow::{Context, Result};
use std::path::Path;
use tracing::{debug, info, warn};

/// Platform-specific DNS configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOS,
    LinuxSystemd,
    LinuxOther,
}

/// Detect the current platform and DNS system
pub fn detect_platform() -> Platform {
    #[cfg(target_os = "macos")]
    {
        return Platform::MacOS;
    }

    #[cfg(target_os = "linux")]
    {
        // Check if systemd-resolved is available
        if Path::new("/run/systemd/resolve/resolv.conf").exists()
            || Path::new("/etc/systemd/resolved.conf").exists()
        {
            return Platform::LinuxSystemd;
        }
        return Platform::LinuxOther;
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Platform::LinuxOther // Fallback for unsupported platforms
    }
}

/// Check if DNS is already configured for .iron domains
pub fn is_dns_configured() -> bool {
    match detect_platform() {
        Platform::MacOS => Path::new("/etc/resolver/iron").exists(),
        Platform::LinuxSystemd => Path::new("/etc/systemd/resolved.conf.d/iron.conf").exists(),
        Platform::LinuxOther => false, // Can't auto-detect for other systems
    }
}

/// Setup DNS configuration for .iron domains
///
/// This configures the system to route only .iron domains to iron's DNS server,
/// leaving all other DNS resolution unchanged.
pub fn setup_dns() -> Result<()> {
    info!("Setting up DNS configuration for .iron domains");

    match detect_platform() {
        Platform::MacOS => setup_dns_macos(),
        Platform::LinuxSystemd => setup_dns_linux_systemd(),
        Platform::LinuxOther => {
            warn!("Automatic DNS setup not available for your system");
            println!("\n⚠️  Automatic DNS setup not available for your Linux distribution");
            println!("\nPlease configure DNS manually. See: doc/dns-setup.md");
            println!("\nSupported automatic setup:");
            println!("  - macOS (using /etc/resolver/)");
            println!("  - Linux with systemd-resolved");
            Ok(())
        }
    }
}

/// Setup DNS on macOS using /etc/resolver/
fn setup_dns_macos() -> Result<()> {
    debug!("Setting up macOS resolver for .iron domains");

    // Create /etc/resolver directory if it doesn't exist
    std::fs::create_dir_all("/etc/resolver")
        .context("Failed to create /etc/resolver directory (are you root?)")?;

    // Write resolver configuration
    let config = "# iron DNS resolver - routes .iron domains to iron's DNS server
# Created by iron
nameserver 127.0.0.1
port 5333
";

    std::fs::write("/etc/resolver/iron", config)
        .context("Failed to write /etc/resolver/iron (are you root?)")?;

    info!("DNS configured: /etc/resolver/iron created");
    println!("\n✓ DNS configured successfully!");
    println!("\n  .iron domains will now resolve automatically");
    println!("  All other domains use your normal DNS");
    println!("\n  To verify: scutil --dns | grep -A3 iron");
    println!("  To remove: sudo rm /etc/resolver/iron\n");

    Ok(())
}

/// Setup DNS on Linux with systemd-resolved
fn setup_dns_linux_systemd() -> Result<()> {
    debug!("Setting up systemd-resolved for .iron domains");

    // Create drop-in directory if it doesn't exist
    std::fs::create_dir_all("/etc/systemd/resolved.conf.d")
        .context("Failed to create /etc/systemd/resolved.conf.d (are you root?)")?;

    // Write resolver configuration
    let config = "# iron DNS resolver - routes .iron domains to iron's DNS server
# Created by iron
[Resolve]
DNS=127.0.0.1:5333
Domains=~iron
";

    std::fs::write("/etc/systemd/resolved.conf.d/iron.conf", config)
        .context("Failed to write /etc/systemd/resolved.conf.d/iron.conf (are you root?)")?;

    // Restart systemd-resolved
    info!("Restarting systemd-resolved");
    let status = std::process::Command::new("systemctl")
        .args(&["restart", "systemd-resolved"])
        .status()
        .context("Failed to restart systemd-resolved")?;

    if !status.success() {
        warn!("systemd-resolved restart returned non-zero status");
        println!("\n⚠️  Warning: systemd-resolved restart failed");
        println!(
            "   You may need to restart it manually: sudo systemctl restart systemd-resolved\n"
        );
    }

    info!("DNS configured: /etc/systemd/resolved.conf.d/iron.conf created");
    println!("\n✓ DNS configured successfully!");
    println!("\n  .iron domains will now resolve automatically");
    println!("  All other domains use your normal DNS");
    println!("\n  To verify: resolvectl status");
    println!(
        "  To remove: sudo rm /etc/systemd/resolved.conf.d/iron.conf && sudo systemctl restart systemd-resolved\n"
    );

    Ok(())
}

/// Clean up DNS configuration
pub fn cleanup_dns() -> Result<()> {
    info!("Cleaning up DNS configuration");

    match detect_platform() {
        Platform::MacOS => cleanup_dns_macos(),
        Platform::LinuxSystemd => cleanup_dns_linux_systemd(),
        Platform::LinuxOther => {
            println!("\n⚠️  Automatic DNS cleanup not available for your system");
            println!("   Please remove DNS configuration manually if you added it.\n");
            Ok(())
        }
    }
}

/// Clean up macOS DNS configuration
fn cleanup_dns_macos() -> Result<()> {
    let path = Path::new("/etc/resolver/iron");
    if !path.exists() {
        println!("\n✓ DNS configuration not found (already clean)\n");
        return Ok(());
    }

    std::fs::remove_file(path).context("Failed to remove /etc/resolver/iron (are you root?)")?;

    info!("DNS configuration removed: /etc/resolver/iron deleted");
    println!("\n✓ DNS configuration removed successfully!\n");

    Ok(())
}

/// Clean up Linux systemd-resolved DNS configuration
fn cleanup_dns_linux_systemd() -> Result<()> {
    let path = Path::new("/etc/systemd/resolved.conf.d/iron.conf");
    if !path.exists() {
        println!("\n✓ DNS configuration not found (already clean)\n");
        return Ok(());
    }

    std::fs::remove_file(path)
        .context("Failed to remove /etc/systemd/resolved.conf.d/iron.conf (are you root?)")?;

    // Restart systemd-resolved
    info!("Restarting systemd-resolved");
    let status = std::process::Command::new("systemctl")
        .args(&["restart", "systemd-resolved"])
        .status()
        .context("Failed to restart systemd-resolved")?;

    if !status.success() {
        warn!("systemd-resolved restart returned non-zero status");
        println!("\n⚠️  Warning: systemd-resolved restart failed");
        println!(
            "   You may need to restart it manually: sudo systemctl restart systemd-resolved\n"
        );
    }

    info!("DNS configuration removed: /etc/systemd/resolved.conf.d/iron.conf deleted");
    println!("\n✓ DNS configuration removed successfully!\n");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_platform() {
        let platform = detect_platform();
        // Just verify it doesn't panic and returns something
        assert!(matches!(
            platform,
            Platform::MacOS | Platform::LinuxSystemd | Platform::LinuxOther
        ));
    }

    #[test]
    fn test_is_dns_configured() {
        // Should not panic
        let _ = is_dns_configured();
    }
}
