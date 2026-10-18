//! Cryptographic key management for iron
//!
//! Handles persistence and loading of the node's private key.
//! The key file location comes from [`StatePaths`], chosen by the platform
//! crate; the file is written with 0600 permissions.

use crate::paths::StatePaths;
use crate::platform::PrivateFileWriter;
use anyhow::{Context, Result};
use iroh::SecretKey;
use std::fs;
use std::path::Path;
use tracing::{debug, info};

/// Load or generate a persistent secret key
///
/// # Behavior
///
/// 1. If key file exists: Load and return existing key
/// 2. If key file doesn't exist: Generate new key, save it, and return it
///
/// # Security
///
/// - Key file is created with 0600 permissions (owner read/write only)
/// - Directory is created with 0700 permissions (owner access only)
///
/// # Returns
///
/// Returns the loaded or newly generated SecretKey
pub fn load_or_generate_key(
    paths: &StatePaths,
    store: &dyn PrivateFileWriter,
) -> Result<SecretKey> {
    let key_path = paths.key_file();

    match fs::read(&key_path) {
        Ok(bytes) => {
            info!("Loading existing key from {}", key_path.display());
            parse_key(bytes)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            info!("No existing key found, generating new key");
            let key = SecretKey::generate();
            store
                .create_new(&key_path, &key.to_bytes())
                .context("Failed to create key file")?;
            info!("Key saved to {}", key_path.display());
            Ok(key)
        }
        Err(error) => Err(error).context("Failed to read key file"),
    }
}

/// Load a secret key from a file
pub fn load_key(path: &Path) -> Result<SecretKey> {
    let bytes = fs::read(path).context("Failed to read key file")?;
    parse_key(bytes)
}

fn parse_key(bytes: Vec<u8>) -> Result<SecretKey> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::PrivateFileWriter;
    use crate::test_utils::TestPrivateFileWriter;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_save_and_load_key() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test_secret.key");

        // Create key file directly for load validation.
        let original_key = SecretKey::generate();
        fs::write(&key_path, original_key.to_bytes()).unwrap();

        // Load the key
        let loaded_key = load_key(&key_path).unwrap();

        // Verify they're the same
        assert_eq!(
            original_key.to_bytes(),
            loaded_key.to_bytes(),
            "Loaded key should match original"
        );
    }

    #[test]
    fn test_load_or_generate_key_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let paths = StatePaths::new(temp_dir.path());

        // First call generates and persists
        let generated = load_or_generate_key(&paths, &TestPrivateFileWriter).unwrap();
        assert!(paths.key_file().exists());

        // Second call loads the same key
        let loaded = load_or_generate_key(&paths, &TestPrivateFileWriter).unwrap();
        assert_eq!(generated.to_bytes(), loaded.to_bytes());
    }

    #[test]
    fn test_load_nonexistent_key() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("nonexistent.key");

        let result = load_key(&key_path);
        assert!(result.is_err(), "Loading nonexistent key should fail");
    }

    #[test]
    fn test_load_invalid_key() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("invalid.key");

        // Write invalid data (wrong size)
        fs::write(&key_path, [1, 2, 3, 4, 5]).unwrap();

        let result = load_key(&key_path);
        assert!(result.is_err(), "Loading key with invalid size should fail");
    }

    #[test]
    fn corrupt_existing_key_is_not_replaced() {
        let temp_dir = TempDir::new().unwrap();
        let paths = StatePaths::new(temp_dir.path());
        fs::write(paths.key_file(), [1, 2, 3]).unwrap();
        assert!(load_or_generate_key(&paths, &TestPrivateFileWriter).is_err());
        assert_eq!(fs::read(paths.key_file()).unwrap(), [1, 2, 3]);
    }

    #[test]
    fn create_only_store_does_not_clobber() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("key");
        TestPrivateFileWriter.create_new(&path, &[7; 32]).unwrap();
        assert!(TestPrivateFileWriter.create_new(&path, &[8; 32]).is_err());
        assert_eq!(fs::read(path).unwrap(), [7; 32]);
    }

    #[test]
    fn unreadable_path_is_not_replaced() {
        let temp_dir = TempDir::new().unwrap();
        let paths = StatePaths::new(temp_dir.path());
        fs::create_dir(paths.key_file()).unwrap();
        assert!(load_or_generate_key(&paths, &TestPrivateFileWriter).is_err());
        assert!(paths.key_file().is_dir());
    }
}
