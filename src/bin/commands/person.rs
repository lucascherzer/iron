//! Person management commands for the firewall
//!
//! Manages trusted persons in the firewall configuration.

use anyhow::{Context, Result};
use iron::firewall::{FirewallConfig, PersonKey, PersonSecretKey, TrustedPerson};
use std::fs;
use std::path::PathBuf;

/// Get the person key file path
fn person_key_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    Ok(PathBuf::from(home).join(".config/iron/person_key.secret"))
}

/// Generate a new person keypair
pub fn generate(save: bool) -> Result<()> {
    let secret = PersonSecretKey::generate();
    let public = secret.public_key();

    println!("Generated new person keypair:");
    println!();
    println!("Public key (share this with others):");
    println!("{}", public.to_hex());
    println!();
    println!("Secret key (keep this private!):");
    println!("{}", secret.to_hex());
    println!();

    if save {
        let key_path = person_key_path()?;

        // Create directory if it doesn't exist
        if let Some(dir) = key_path.parent() {
            fs::create_dir_all(dir).context("Failed to create config directory")?;

            // Set directory permissions to 0700 (owner only)
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(dir)?.permissions();
                perms.set_mode(0o700);
                fs::set_permissions(dir, perms)?;
            }
        }

        // Save secret key to file
        fs::write(&key_path, secret.to_hex()).context("Failed to write person key")?;

        // Set file permissions to 0600 (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&key_path)?.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&key_path, perms)?;
        }

        println!("✓ Secret key saved to: {}", key_path.display());
        println!();
        println!("To generate ownership claims for your devices:");
        println!("  iron claim generate <person-secret>");
        println!();
        println!("Or load from file:");
        println!("  iron claim generate $(cat {})", key_path.display());
    } else {
        println!("IMPORTANT: Store the secret key securely!");
        println!("You will need it to create device ownership claims.");
        println!();
        println!("To save the key, use: iron person generate --save");
    }

    Ok(())
}

/// Add a trusted person to the firewall
pub fn add(name: String, key_hex: String, comment: Option<String>) -> Result<()> {
    // Parse the person key
    let key = PersonKey::from_hex(&key_hex).context("Invalid person key format")?;

    // Load firewall config
    let mut config = FirewallConfig::load().unwrap_or_else(|_| {
        println!("No existing firewall config found, creating new one");
        FirewallConfig::new()
    });

    // Check if person already exists
    if config.get_person(&name).is_some() {
        anyhow::bail!("Person '{}' already exists in firewall config", name);
    }

    // Add the person
    let person = TrustedPerson {
        name: name.clone(),
        comment,
        key,
    };
    config.add_person(person);

    // Save config
    config.save().context("Failed to save firewall config")?;

    println!("✓ Added trusted person: {}", name);
    Ok(())
}

/// Remove a trusted person from the firewall
pub fn remove(name: String) -> Result<()> {
    // Load firewall config
    let mut config =
        FirewallConfig::load().context("Failed to load firewall config (does it exist?)")?;

    // Remove the person
    if config.remove_person(&name) {
        // Save config
        config.save().context("Failed to save firewall config")?;

        println!("✓ Removed trusted person: {}", name);
        Ok(())
    } else {
        anyhow::bail!("Person '{}' not found in firewall config", name);
    }
}

/// List all trusted persons
pub fn list() -> Result<()> {
    // Load firewall config
    let config = FirewallConfig::load().unwrap_or_else(|_| {
        println!("No firewall config found");
        FirewallConfig::new()
    });

    if config.trusted_persons.is_empty() {
        println!("No trusted persons configured");
        return Ok(());
    }

    println!("Trusted persons:");
    println!();
    for person in &config.trusted_persons {
        println!("Name:    {}", person.name);
        println!("Key:     {}", person.key.to_hex());
        if let Some(comment) = &person.comment {
            println!("Comment: {}", comment);
        }
        println!(
            "Devices: {}",
            config
                .verified_devices
                .iter()
                .filter(|(_, pk)| *pk == &person.key)
                .count()
        );
        println!();
    }

    Ok(())
}
