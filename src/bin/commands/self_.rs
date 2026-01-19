use anyhow::{Context, Result};
use iron::keys;
use serde_json::json;

pub fn run(
    format: Option<String>,
    hex: bool,
    base32: bool,
    domain: bool,
    ipv6: bool,
    exists: bool,
) -> Result<()> {
    // Handle --exists flag (just check and exit)
    if exists {
        match keys::load_key() {
            Ok(_) => std::process::exit(0),
            Err(_) => std::process::exit(1),
        }
    }

    // Load the key
    let secret_key = keys::load_key().context(
        "No key file found\n\n\
        Run 'iron' once to generate a key, or use 'iron vanity' to create a custom key.",
    )?;

    let endpoint_id = secret_key.public();
    let endpoint_bytes = endpoint_id.as_bytes();

    // Calculate formats
    let hex_id = hex::encode(endpoint_bytes);
    let base32_id = data_encoding::BASE32_NOPAD
        .encode(endpoint_bytes)
        .to_lowercase();
    let domain_name = format!("{}.iron", base32_id);
    let ipv6_addr = iron::mapping::Registry::derive_ip(endpoint_id);

    // Single field outputs
    if hex {
        println!("{}", hex_id);
        return Ok(());
    }
    if base32 {
        println!("{}", base32_id);
        return Ok(());
    }
    if domain {
        println!("{}", domain_name);
        return Ok(());
    }
    if ipv6 {
        println!("{}", ipv6_addr);
        return Ok(());
    }

    // Format-specific output
    if let Some(fmt) = format {
        match fmt.to_lowercase().as_str() {
            "json" => {
                let key_path = keys::key_path();
                let output = json!({
                    "key_file": key_path.to_string_lossy(),
                    "key_exists": true,
                    "node_id": {
                        "hex": hex_id,
                        "base32": base32_id
                    },
                    "network": {
                        "domain": domain_name,
                        "ipv6": ipv6_addr.to_string()
                    }
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
            "hex" => println!("{}", hex_id),
            "base32" => println!("{}", base32_id),
            _ => {
                anyhow::bail!("Invalid format '{}'. Valid formats: json, hex, base32", fmt)
            }
        }
        return Ok(());
    }

    // Default: pretty output
    let key_path = keys::key_path();

    println!("\nIron Node Identity:");
    println!("  Key file:  {}", key_path.display());
    println!("  Status:    ✓ Key found");
    println!();
    println!("Node ID:");
    println!("  Hex:       {}", hex_id);
    println!("  Base32:    {}", base32_id);
    println!();
    println!("Network Identity:");
    println!("  Domain:    {}", domain_name);
    println!("  IPv6:      {}", ipv6_addr);
    println!();
    println!("Share this with peers to connect:");
    println!("  {}", domain_name);
    println!();

    Ok(())
}
