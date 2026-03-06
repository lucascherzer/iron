use anyhow::{Context, Result};
use iroh::SecretKey;
use iron::keys;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Show key information. If `path` is given it overrides `default_key_path`.
pub fn info(default_key_path: &Path, path: Option<String>) -> Result<()> {
    let key_path = path_or_default(default_key_path, path);

    if !key_path.exists() {
        anyhow::bail!("Key file not found: {}", key_path.display());
    }

    // Load key to validate it
    let secret_key = load_key_from_file(&key_path)?;
    let endpoint_id = secret_key.public();

    // Get file metadata
    let metadata = fs::metadata(&key_path)?;
    let size = metadata.len();

    println!("\nKey file: {}", key_path.display());
    println!("Valid:    ✓");
    println!("Size:     {} bytes", size);
    println!("Node ID:  {}", hex::encode(endpoint_id.as_bytes()));
    println!();

    Ok(())
}

/// Export the default key to stdout or a file.
pub fn export(default_key_path: &Path, format: String, output: Option<String>) -> Result<()> {
    let secret_key = keys::load_key_at(default_key_path)
        .context("No key found. Generate one with: iron key generate")?;
    let bytes = secret_key.to_bytes();

    let encoded = match format.to_lowercase().as_str() {
        "hex" => hex::encode(bytes),
        "base64" => data_encoding::BASE64.encode(&bytes),
        _ => anyhow::bail!("Invalid format '{}'. Valid formats: hex, base64", format),
    };

    if let Some(output_path) = output {
        fs::write(&output_path, &encoded)?;
        println!("Key exported to: {}", output_path);
    } else {
        println!("{}", encoded);
    }

    Ok(())
}

/// Import a key from a hex/base64 file. If `save` is true it overwrites
/// `default_key_path`.
pub fn import(default_key_path: &Path, file: String, save: bool) -> Result<()> {
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

    println!("Valid key imported");
    println!("  Node ID: {}", hex::encode(endpoint_id.as_bytes()));

    if save {
        if default_key_path.exists() {
            print!("\nWarning: This will overwrite your existing key. Continue? (y/N) ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                println!("Cancelled.");
                return Ok(());
            }
        }

        // Save key
        save_key_bytes(default_key_path, &bytes)?;
        println!("Key saved to: {}", default_key_path.display());
    }

    Ok(())
}

/// Generate a new random key. If `save` is true it is written to
/// `default_key_path`.
pub fn generate(default_key_path: &Path, save: bool, force: bool) -> Result<()> {
    let secret_key = SecretKey::generate(&mut rand::rng());
    let endpoint_id = secret_key.public();

    println!("New key generated");
    println!("  Node ID (hex): {}", hex::encode(endpoint_id.as_bytes()));

    let base32_id = data_encoding::BASE32_NOPAD
        .encode(endpoint_id.as_bytes())
        .to_lowercase();
    println!("  Domain:        {}.iron", base32_id);

    if save {
        if default_key_path.exists() && !force {
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

        save_key_bytes(default_key_path, &secret_key.to_bytes())?;
        println!("\nKey saved to: {}", default_key_path.display());
    }

    Ok(())
}

/// Validate a key file. If `path` is given it overrides `default_key_path`.
pub fn validate(default_key_path: &Path, path: Option<String>) -> Result<()> {
    let key_path = path_or_default(default_key_path, path);

    if !key_path.exists() {
        println!("Key file not found: {}", key_path.display());
        std::process::exit(1);
    }

    match load_key_from_file(&key_path) {
        Ok(secret_key) => {
            let endpoint_id = secret_key.public();
            println!("Valid key");
            println!("  Path:    {}", key_path.display());
            println!("  Node ID: {}", hex::encode(endpoint_id.as_bytes()));
        }
        Err(e) => {
            println!("Invalid key file: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}

/// Delete the key at `default_key_path`.
pub fn reset(default_key_path: &Path, confirm: bool) -> Result<()> {
    if !default_key_path.exists() {
        println!("No key file found (already clean)");
        return Ok(());
    }

    // Show warning and get current node ID
    let current_node_id = if let Ok(key) = keys::load_key_at(default_key_path) {
        let endpoint_id = key.public();
        let base32_id = data_encoding::BASE32_NOPAD
            .encode(endpoint_id.as_bytes())
            .to_lowercase();
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

    fs::remove_file(default_key_path)?;
    println!("Key deleted: {}", default_key_path.display());

    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Return `path` as a `PathBuf` if provided, otherwise `default`.
fn path_or_default(default: &Path, path: Option<String>) -> PathBuf {
    match path {
        Some(p) => PathBuf::from(p),
        None => default.to_owned(),
    }
}

fn load_key_from_file(path: &Path) -> Result<SecretKey> {
    let bytes = fs::read(path).context("Failed to read key file")?;

    if bytes.len() != 32 {
        anyhow::bail!("Invalid key file: expected 32 bytes, got {}", bytes.len());
    }

    // Safe because we validated length
    let mut byte_array = [0u8; 32];
    byte_array.copy_from_slice(&bytes);
    Ok(SecretKey::from_bytes(&byte_array))
}

fn save_key_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    // Create parent directory if it doesn't exist
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Write key file
    fs::write(path, bytes)?;

    // Set permissions to 0600 (owner read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::Permissions::from_mode(0o600);
        fs::set_permissions(path, permissions)?;
    }

    Ok(())
}
