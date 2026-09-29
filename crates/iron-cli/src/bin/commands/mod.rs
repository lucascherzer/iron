pub mod convert;
pub mod key;
pub mod resolve;
pub mod self_;
pub mod vanity;

use anyhow::Result;
use iroh::SecretKey;
use std::path::PathBuf;

/// Default secret key location on this machine (`~/.config/iron/secret.key`).
pub fn key_path() -> Result<PathBuf> {
    Ok(iron_desktop::default_state_paths()?.key_file())
}

/// Loads the secret key from its default location.
pub fn load_key() -> Result<SecretKey> {
    iron_core::keys::load_key(&key_path()?)
}
