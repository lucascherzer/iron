use anyhow::{Context, Result};
use iroh::SecretKey;
use iron_core::platform::PrivateFileWriter;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

pub fn info(key_path: &Path) -> Result<()> {
    if !key_path.exists() {
        anyhow::bail!("Key file not found: {}", key_path.display());
    }

    // Load key to validate it
    let secret_key = iron_core::keys::load_key(key_path)?;
    let endpoint_id = secret_key.public();

    // Get file metadata
    let metadata = fs::metadata(key_path)?;
    let size = metadata.len();

    println!("\nKey file: {}", key_path.display());
    println!("Valid:    ✓");
    println!("Size:     {} bytes", size);
    println!("Node ID:  {}", endpoint_id);
    println!();

    Ok(())
}

pub fn export(format: String, output: Option<String>, configured_path: &Path) -> Result<()> {
    let secret_key = iron_core::keys::load_key(configured_path)
        .context("No key found. Generate one with: iron key generate")?;
    let bytes = secret_key.to_bytes();

    let encoded = match format.to_lowercase().as_str() {
        "hex" => hex::encode(bytes),
        "base64" => data_encoding::BASE64.encode(&bytes),
        _ => anyhow::bail!("Invalid format '{}'. Valid formats: hex, base64", format),
    };

    if let Some(output_path) = output {
        iron_desktop::DesktopPrivateFileWriter
            .create_new(output_path.as_ref(), encoded.as_bytes())?;
        println!("✓ Key exported to: {}", output_path);
    } else {
        println!("{}", encoded);
    }

    Ok(())
}

pub fn import(file: String, save: bool, configured_path: &Path) -> Result<()> {
    let content = fs::read_to_string(&file)
        .context(format!("Failed to read file: {}", file))?
        .trim()
        .to_string();

    // Try to decode as hex or base64
    let bytes = if content.len() == 64 && content.chars().all(|c| c.is_ascii_hexdigit()) {
        hex::decode(&content)?
    } else if let Ok(decoded) = data_encoding::BASE64.decode(content.as_bytes()) {
        decoded
    } else {
        anyhow::bail!("Invalid key format. Expected hex (64 chars) or base64.");
    };

    if bytes.len() != 32 {
        anyhow::bail!("Invalid key length: {} bytes (expected 32)", bytes.len());
    }

    // Validate it's a valid secret key - safe because we validated length
    let mut bytes_array = [0u8; 32];
    bytes_array.copy_from_slice(&bytes);
    let secret_key = SecretKey::from_bytes(&bytes_array);
    let endpoint_id = secret_key.public();

    println!("✓ Valid key imported");
    println!("  Node ID: {}", endpoint_id);

    if save {
        let key_path = configured_path.to_path_buf();
        let replace_existing = key_path.exists();
        if replace_existing {
            print!("\nWarning: This will overwrite your existing key. Continue? (y/N) ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                println!("Cancelled.");
                return Ok(());
            }
        }

        // Save key (using the internal save function)
        save_key_bytes(&key_path, &bytes, replace_existing)?;
        println!("✓ Key saved to: {}", key_path.display());
    }

    Ok(())
}

pub fn generate(save: bool, force: bool, configured_path: &Path) -> Result<()> {
    let secret_key = SecretKey::generate();
    let endpoint_id = secret_key.public();

    println!("✓ New key generated");
    println!("  Node ID (hex): {}", endpoint_id);

    let base32_id = iron_core::id::to_base32(&endpoint_id);
    println!("  Domain:        {}.iron", base32_id);

    if save {
        let key_path = configured_path.to_path_buf();
        let replace_existing = key_path.exists();

        if replace_existing && !force {
            println!("\nWARNING: This will overwrite your existing key.");
            println!("You will get a new Node ID and .iron domain.");
            print!("Generate new key? (y/N) ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                println!("Cancelled.");
                return Ok(());
            }
        }

        save_key_bytes(&key_path, &secret_key.to_bytes(), replace_existing)?;
        println!("\n✓ Key saved to: {}", key_path.display());
    }

    Ok(())
}

pub fn validate(key_path: &Path) -> Result<()> {
    if !key_path.exists() {
        println!("✗ Key file not found: {}", key_path.display());
        std::process::exit(1);
    }

    match iron_core::keys::load_key(key_path) {
        Ok(secret_key) => {
            let endpoint_id = secret_key.public();
            println!("✓ Valid key");
            println!("  Path:    {}", key_path.display());
            println!("  Node ID: {}", endpoint_id);
        }
        Err(e) => {
            println!("✗ Invalid key file: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}

pub fn reset(confirm: bool, configured_path: &Path) -> Result<()> {
    let key_path = configured_path.to_path_buf();

    if !key_path.exists() {
        println!("✓ No key file found (already clean)");
        return Ok(());
    }

    // Show warning and get current node ID
    let current_node_id = if let Ok(key) = iron_core::keys::load_key(configured_path) {
        let endpoint_id = key.public();
        let base32_id = iron_core::id::to_base32(&endpoint_id);
        format!("{}.iron", base32_id)
    } else {
        "unknown".to_string()
    };

    if !confirm {
        print!("\nWARNING: This will delete your key file permanently.\n");
        println!("You will lose your current Node ID: {}", current_node_id);
        print!("Delete key? (y/N) ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    fs::remove_file(&key_path)?;
    println!("✓ Key deleted: {}", key_path.display());

    Ok(())
}

// Helper functions

fn save_key_bytes(path: &Path, bytes: &[u8], replace_existing: bool) -> Result<()> {
    if replace_existing {
        iron_desktop::DesktopPrivateFileWriter.replace(path, bytes)
    } else {
        iron_desktop::DesktopPrivateFileWriter.create_new(path, bytes)
    }
}
