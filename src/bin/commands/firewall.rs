//! Firewall management commands
//!
//! Enable, disable, and check status of the firewall.

use anyhow::{Context, Result};
use iron::firewall::{FirewallConfig, PersonSecretKey};
use std::io::{self, Write};

/// Setup firewall (generate keys, create claim, enable)
pub fn setup(force: bool) -> Result<()> {
    println!("Setting up firewall...");
    println!();

    // Check if person key exists
    let person_key_exists = PersonSecretKey::exists_at_default_path();

    // Check if firewall config exists
    let config_exists = FirewallConfig::load().is_ok();

    if (person_key_exists || config_exists) && !force {
        println!("⚠️  Warning: Existing firewall configuration detected");
        println!();
        if person_key_exists {
            println!("  • Person key exists at ~/.config/iron/person_key.secret");
        }
        if config_exists {
            println!("  • Firewall config exists");
        }
        println!();
        println!("Running setup will overwrite existing configuration!");
        println!();
        print!("Continue? (y/N): ");
        io::stdout().flush()?;

        let mut response = String::new();
        io::stdin().read_line(&mut response)?;

        if !response.trim().eq_ignore_ascii_case("y") {
            println!("Setup cancelled");
            return Ok(());
        }
        println!();
    }

    // Step 1: Generate person key if needed
    if !person_key_exists || force {
        println!("Step 1/3: Generating person keypair...");
        crate::commands::person::generate(true)?;
    } else {
        println!("Step 1/3: Person key already exists ✓");
    }
    println!();

    // Step 2: Generate claim for this device
    println!("Step 2/3: Generating ownership claim for this device...");
    crate::commands::claim::generate(None, None)?;
    println!();

    // Step 3: Enable firewall
    println!("Step 3/3: Enabling firewall...");
    let mut config = FirewallConfig::load().unwrap_or_else(|_| FirewallConfig::new());
    config.enabled = true;
    config.save().context("Failed to save firewall config")?;
    println!("✓ Firewall enabled");
    println!();

    // Show summary
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Firewall setup complete!");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();

    let person_secret = PersonSecretKey::load_from_default_path()?;
    let public_key = person_secret.public_key();

    println!("Your public key (share with peers):");
    println!("  {}", public_key.to_hex());
    println!();
    println!("Next steps:");
    println!("  1. Share your public key with peers who need to trust you");
    println!("  2. Add trusted persons:");
    println!("     iron person add <name> <their-public-key>");
    println!("  3. Restart the daemon if it's running:");
    println!("     sudo iron serve");
    println!();
    println!("Note: With firewall enabled but no trusted persons,");
    println!("      all incoming connections will be rejected!");

    Ok(())
}

/// Enable the firewall
pub fn enable() -> Result<()> {
    // Load firewall config
    let mut config = FirewallConfig::load().unwrap_or_else(|_| {
        println!("No existing firewall config found, creating new one");
        FirewallConfig::new()
    });

    if config.enabled {
        println!("Firewall is already enabled");
        return Ok(());
    }

    config.enabled = true;

    // Save config
    config.save().context("Failed to save firewall config")?;

    println!("✓ Firewall enabled");
    println!();
    println!("Note: You need to restart the iron daemon for this to take effect");
    Ok(())
}

/// Disable the firewall
pub fn disable() -> Result<()> {
    // Load firewall config
    let mut config =
        FirewallConfig::load().context("Failed to load firewall config (does it exist?)")?;

    if !config.enabled {
        println!("Firewall is already disabled");
        return Ok(());
    }

    config.enabled = false;

    // Save config
    config.save().context("Failed to save firewall config")?;

    println!("✓ Firewall disabled");
    println!();
    println!("Note: You need to restart the iron daemon for this to take effect");
    Ok(())
}

/// Show firewall status
pub fn status() -> Result<()> {
    use iron::firewall::PacketSource;

    // Load firewall config
    let config = FirewallConfig::load().unwrap_or_else(|_| {
        println!("No firewall config found (using defaults)");
        FirewallConfig::new()
    });

    println!("Firewall status:");
    println!();
    println!(
        "Enabled:         {}",
        if config.enabled { "yes" } else { "no" }
    );
    println!("Trusted persons: {}", config.trusted_persons.len());
    println!("Verified devices: {}", config.verified_devices.len());
    println!();

    // Show policies if any
    if !config.policies.is_empty() {
        println!("Policies ({}):", config.policies.len());
        for (idx, policy) in config.policies.iter().enumerate() {
            let src_display = match &policy.source {
                PacketSource::Any => "*".to_string(),
                PacketSource::Person(name) => format!("person:{}", name),
                PacketSource::Peer(id) => format!("peer:{}", id),
            };

            let port_display = policy
                .dst_port
                .as_ref()
                .map(|p| p.to_string())
                .unwrap_or_else(|| "*".to_string());

            println!(
                "  {}. {} from {} on port {}",
                idx, policy.action, src_display, port_display
            );
        }
        println!();
    } else {
        println!("Policies:        0");
        println!();
    }

    if config.enabled && config.trusted_persons.is_empty() {
        println!("⚠️  Warning: Firewall is enabled but no trusted persons configured!");
        println!("All incoming connections will be rejected.");
        println!();
        println!("To add trusted persons:");
        println!("  iron person add <name> <public-key>");
    }

    Ok(())
}

/// Add a new firewall policy
pub fn policy_add(src: String, port: Option<String>) -> Result<()> {
    use iron::firewall::{FirewallAction, FirewallPolicy, PacketSource, PortRange};

    // Parse source
    let source =
        PacketSource::parse(&src).with_context(|| format!("Invalid source format: {}", src))?;

    // Parse port range if provided
    let port_range = if let Some(port_str) = port {
        Some(
            PortRange::parse(&port_str)
                .with_context(|| format!("Invalid port format: {}", port_str))?,
        )
    } else {
        None
    };

    // Load config
    let mut config = FirewallConfig::load().unwrap_or_else(|_| {
        println!("No existing firewall config found, creating new one");
        FirewallConfig::new()
    });

    // Create policy
    let policy = FirewallPolicy {
        action: FirewallAction::Accept,
        source,
        dst_port: port_range,
    };

    // Add policy
    config.policies.push(policy);

    // Save config
    config.save().context("Failed to save firewall config")?;

    println!("✓ Policy added successfully");
    println!();
    println!("Total policies: {}", config.policies.len());
    println!();
    println!("Note: You need to restart the iron daemon for this to take effect");

    Ok(())
}

/// List all firewall policies
pub fn policy_list() -> Result<()> {
    use iron::firewall::PacketSource;

    // Load config
    let config = FirewallConfig::load().unwrap_or_else(|_| {
        println!("No firewall config found (using defaults)");
        FirewallConfig::new()
    });

    if config.policies.is_empty() {
        println!("No policies configured");
        println!();
        println!("To add a policy:");
        println!("  iron firewall policy add <src> [--port <range>]");
        println!();
        println!("Examples:");
        println!("  iron firewall policy add \"*\" --port \"80\"");
        println!("  iron firewall policy add \"person:alice\" --port \"1000-\"");
        println!("  iron firewall policy add \"peer:abc123...\" --port \"8080\"");
        return Ok(());
    }

    println!("Firewall policies ({}):", config.policies.len());
    println!();

    for (idx, policy) in config.policies.iter().enumerate() {
        let src_display = match &policy.source {
            PacketSource::Any => "*".to_string(),
            PacketSource::Person(name) => format!("person:{}", name),
            PacketSource::Peer(id) => format!("peer:{}", id),
        };

        let port_display = policy
            .dst_port
            .as_ref()
            .map(|p| p.to_string())
            .unwrap_or_else(|| "*".to_string());

        println!(
            "  {}. {} from {} on port {}",
            idx, policy.action, src_display, port_display
        );
    }

    println!();

    Ok(())
}

/// Remove a firewall policy by index
pub fn policy_remove(index: usize) -> Result<()> {
    // Load config
    let mut config =
        FirewallConfig::load().context("Failed to load firewall config (does it exist?)")?;

    if config.policies.is_empty() {
        anyhow::bail!("No policies configured");
    }

    if index >= config.policies.len() {
        anyhow::bail!(
            "Invalid policy index: {} (only {} policies exist)",
            index,
            config.policies.len()
        );
    }

    // Remove policy
    let removed_policy = config.policies.remove(index);

    // Save config
    config.save().context("Failed to save firewall config")?;

    println!("✓ Policy removed successfully");
    println!();
    println!("Removed policy:");
    println!(
        "  {} from {:?}",
        removed_policy.action, removed_policy.source
    );
    println!();
    println!("Remaining policies: {}", config.policies.len());
    println!();
    println!("Note: You need to restart the iron daemon for this to take effect");

    Ok(())
}
