//! Ownership claim management commands
//!
//! Generate, show, and verify device ownership claims.

use anyhow::{Context, Result};
use iron::firewall::{FirewallConfig, OwnershipClaim, PersonSecretKey};
use iron::keys;
use std::fs;

const ONE_YEAR_SECONDS: u64 = 365 * 24 * 60 * 60;

/// Generate an ownership claim for this device
pub fn generate(person_secret_hex: Option<String>, output: Option<String>) -> Result<()> {
    // Load person secret key (auto-load from default path or use provided hex)
    let person_secret = if let Some(hex) = person_secret_hex {
        PersonSecretKey::from_hex(&hex).context("Invalid person secret key format")?
    } else {
        // Try to load from default location
        PersonSecretKey::load_from_default_path()
            .context("No person secret key provided and none found at ~/.config/iron/person_key.secret\n\nGenerate one with: iron person generate --save")?
    };

    // Load this device's endpoint ID
    let device_secret = keys::load_or_generate_key().context("Failed to load device key")?;
    let device_endpoint_id = device_secret.public();

    println!("Generating ownership claim for this device...");
    println!();
    println!("Device ID:  {}", device_endpoint_id);
    println!("Person key: {}", person_secret.public_key().to_hex());
    println!();

    // Create the claim with 1 year validity
    let claim = OwnershipClaim::new(&person_secret, device_endpoint_id, ONE_YEAR_SECONDS);

    // Determine if this is for "this device" or for signing another device
    let is_for_this_device = claim.device_key == device_endpoint_id;

    if let Some(path) = output {
        // Explicit output path provided - save to that file
        let json = serde_json::to_string_pretty(&claim).context("Failed to serialize claim")?;
        fs::write(&path, &json).with_context(|| format!("Failed to write claim to {}", path))?;
        println!("✓ Claim saved to: {}", path);
    } else if is_for_this_device {
        // No output specified and this is for our own device - auto-save to claims directory
        FirewallConfig::save_claim(&claim).context("Failed to save claim to claims directory")?;

        let claim_path = FirewallConfig::claim_path(&claim.person_key, &claim.device_key)?;
        println!("✓ Claim saved to: {}", claim_path.display());
        println!();
        println!("This claim will be automatically used when connecting to peers");
        println!("with firewall enabled.");
    } else {
        // No output specified but signing for another device - print to stdout
        let json = serde_json::to_string_pretty(&claim).context("Failed to serialize claim")?;
        println!("Ownership claim (JSON):");
        println!();
        println!("{}", json);
        println!();
        println!("Note: To save this claim, use --output <file.json>");
    }

    println!();
    println!("Claim details:");
    println!("  Device:  {}", device_endpoint_id);
    println!("  Person:  {}", person_secret.public_key().to_hex());
    println!("  Expires: in 1 year");
    println!();
    println!("Keep this claim secure - share it only with peers who need to authenticate you");

    Ok(())
}

/// Show the current device's claim (if one exists)
pub fn show(claim_file: String) -> Result<()> {
    // Read claim file
    let json = fs::read_to_string(&claim_file)
        .with_context(|| format!("Failed to read claim file: {}", claim_file))?;

    // Parse claim
    let claim: OwnershipClaim =
        serde_json::from_str(&json).context("Failed to parse claim file")?;

    println!("Ownership claim:");
    println!();
    println!("Device ID:  {}", claim.device_key);
    println!("Person key: {}", claim.person_key.to_hex());
    println!();
    println!("Created:    {} (Unix timestamp)", claim.created_at);
    println!("Expires:    {} (Unix timestamp)", claim.expires_at);
    println!();

    // Verify the claim
    match claim.verify() {
        Ok(()) => {
            println!("✓ Claim is valid (signature OK, not expired)");
        }
        Err(e) => {
            println!("✗ Claim verification failed: {}", e);
        }
    }

    Ok(())
}

/// Verify an ownership claim file
pub fn verify(claim_file: String) -> Result<()> {
    // Read claim file
    let json = fs::read_to_string(&claim_file)
        .with_context(|| format!("Failed to read claim file: {}", claim_file))?;

    // Parse claim
    let claim: OwnershipClaim =
        serde_json::from_str(&json).context("Failed to parse claim file")?;

    println!("Verifying ownership claim...");
    println!();
    println!("Device ID:  {}", claim.device_key);
    println!("Person key: {}", claim.person_key.to_hex());
    println!();

    // Verify the claim
    match claim.verify() {
        Ok(()) => {
            println!("✓ Claim is valid");
            println!();
            println!("Signature: OK");
            println!("Expiry:    Not expired");
            Ok(())
        }
        Err(e) => {
            println!("✗ Claim verification failed");
            println!();
            println!("Error: {}", e);
            std::process::exit(1);
        }
    }
}
