//! Cryptographic key management for iron
//!
//! Handles persistence and loading of the node's private key.
//! Callers are responsible for providing the key file path (typically from
//! [`crate::config::IronConfig`]). Keys are stored with 0600 permissions.

use anyhow::{Context, Result};
use iroh::SecretKey;
use std::fs;
use std::path::Path;
use tracing::{debug, info};

/// Load an existing key, or generate and persist a new one at `path`.
///
/// # Behavior
///
/// 1. If `path` exists: load and return the key.
/// 2. Otherwise: generate a new key, save it to `path`, and return it.
///
/// # Security
///
/// - Key file is created with 0600 permissions (owner read/write only).
/// - Parent directory is created with 0700 permissions if absent.
pub fn load_or_generate_key_at(path: &Path) -> Result<SecretKey> {
    if path.exists() {
        info!("Loading existing key from {}", path.display());
        load_key_from_path(path)
    } else {
        info!("No existing key found, generating new key");
        let key = SecretKey::generate(&mut rand::rng());
        save_key(path, &key)?;
        info!("Key saved to {}", path.display());
        Ok(key)
    }
}

/// Load an existing key from `path`. Returns an error if the file is absent
/// or malformed.
pub fn load_key_at(path: &Path) -> Result<SecretKey> {
    load_key_from_path(path)
}

/// Load a secret key from a file.
fn load_key_from_path(path: &Path) -> Result<SecretKey> {
    let bytes = fs::read(path).context("Failed to read key file")?;

    if bytes.len() != 32 {
        anyhow::bail!(
            "Invalid key file: expected 32 bytes, got {}. File may be corrupted.",
            bytes.len()
        );
    }

    let key_bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("Failed to convert key bytes"))?;

    let key = SecretKey::from_bytes(&key_bytes);
    debug!("Successfully loaded key (EndpointId: {})", key.public());

    Ok(key)
}

/// Save a secret key to a file with secure permissions.
pub fn save_key(path: &Path, key: &SecretKey) -> Result<()> {
    // Create directory if it doesn't exist
    let dir = path
        .parent()
        .context("Failed to get parent directory of key file")?;

    if !dir.exists() {
        debug!("Creating key directory: {}", dir.display());
        fs::create_dir_all(dir).context("Failed to create key directory")?;

        // Set directory permissions to 0700 (owner only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(dir)?.permissions();
            perms.set_mode(0o700);
            fs::set_permissions(dir, perms)?;
            debug!("Set directory permissions to 0700");
        }
    }

    // Write key to file
    let key_bytes = key.to_bytes();
    fs::write(path, key_bytes).context("Failed to write key file")?;

    // Set file permissions to 0600 (owner read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(path, perms)?;
        debug!("Set key file permissions to 0600");
    }

    #[cfg(not(unix))]
    {
        // On non-Unix systems, we can't set permissions the same way
        // The file system should still provide some default protection
        debug!("File permissions not explicitly set (not on Unix-like system)");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_save_and_load_key() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test_secret.key");

        // Generate and save a key
        let original_key = SecretKey::generate(&mut rand::rng());
        save_key(&key_path, &original_key).unwrap();

        // Load the key
        let loaded_key = load_key_from_path(&key_path).unwrap();

        // Verify they're the same
        assert_eq!(
            original_key.to_bytes(),
            loaded_key.to_bytes(),
            "Loaded key should match original"
        );
    }

    #[test]
    fn test_load_nonexistent_key() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("nonexistent.key");

        let result = load_key_from_path(&key_path);
        assert!(result.is_err(), "Loading nonexistent key should fail");
    }

    #[test]
    fn test_load_invalid_key() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("invalid.key");

        // Write invalid data (wrong size)
        fs::write(&key_path, [1, 2, 3, 4, 5]).unwrap();

        let result = load_key_from_path(&key_path);
        assert!(result.is_err(), "Loading key with invalid size should fail");
    }

    #[test]
    #[cfg(unix)]
    fn test_key_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test_secret.key");

        let key = SecretKey::generate(&mut rand::rng());
        save_key(&key_path, &key).unwrap();

        let metadata = fs::metadata(&key_path).unwrap();
        let mode = metadata.permissions().mode();

        // Check that only owner has read/write permissions
        assert_eq!(mode & 0o777, 0o600, "Key file should have 0600 permissions");
    }

    #[test]
    fn test_load_or_generate_creates_key() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("secret.key");

        assert!(!key_path.exists());
        let key = load_or_generate_key_at(&key_path).unwrap();
        assert!(key_path.exists());

        // Loading again should return the same key
        let key2 = load_or_generate_key_at(&key_path).unwrap();
        assert_eq!(key.to_bytes(), key2.to_bytes());
    }
}
