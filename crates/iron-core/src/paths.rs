//! Filesystem locations for iron's persistent state.
//!
//! All persistent state (secret key, known-peers cache) lives under one
//! configurable directory. The directory is *injected* rather than derived
//! inside each component so that platforms without a usable `$HOME` (e.g.
//! Android, where the app must supply its private data directory) can point
//! iron at the right place.
//!
//! Desktop callers use [`StatePaths::default_os`] (`$HOME/.config/iron`);
//! everything else constructs [`StatePaths::new`] with an explicit directory.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Secret key file name inside the config directory.
const KEY_FILE: &str = "secret.key";

/// Known-peers cache file name inside the config directory.
const KNOWN_PEERS_FILE: &str = "known_peers.json";

/// Locations of iron's persistent state on disk.
#[derive(Debug, Clone)]
pub struct StatePaths {
    config_dir: PathBuf,
}

impl StatePaths {
    /// State rooted at an explicit directory.
    ///
    /// This is the constructor for platforms that manage app storage
    /// themselves (Android data dir, tests with a temp dir, ...).
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }

    /// Desktop default: `$HOME/.config/iron`.
    pub fn default_os() -> Result<Self> {
        let home = std::env::var("HOME").context("HOME environment variable not set")?;
        Ok(Self::new(PathBuf::from(home).join(".config/iron")))
    }

    /// The directory all state files live in.
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Path of the node's secret key file.
    pub fn key_file(&self) -> PathBuf {
        self.config_dir.join(KEY_FILE)
    }

    /// Path of the known-peers cache file.
    pub fn known_peers_file(&self) -> PathBuf {
        self.config_dir.join(KNOWN_PEERS_FILE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paths_are_rooted_in_config_dir() {
        let paths = StatePaths::new("/data/iron");
        assert_eq!(paths.config_dir(), Path::new("/data/iron"));
        assert_eq!(paths.key_file(), PathBuf::from("/data/iron/secret.key"));
        assert_eq!(
            paths.known_peers_file(),
            PathBuf::from("/data/iron/known_peers.json")
        );
    }

    #[test]
    fn test_default_os_uses_home() {
        // HOME is set in test environments on desktop.
        if let Ok(paths) = StatePaths::default_os() {
            assert!(paths.config_dir().ends_with(".config/iron"));
        }
    }
}
