//! Filesystem locations for iron's persistent state.
//!
//! Persistent state locations (secret key, application database) are
//! caller-supplied rather than derived
//! inside each component so that platforms without a usable `$HOME` (e.g.
//! Android, where the app must supply its private data directory) can point
//! iron at the right place.
//!
//! The platform crate supplies defaults; callers may use [`StatePaths::new`]
//! for one state directory or [`StatePaths::from_files`] for separate files.

use std::path::PathBuf;

/// Secret key file name inside the config directory.
const KEY_FILE: &str = "secret.key";

/// Known-peers cache file name inside the config directory.
const STATE_FILE: &str = "state.db";

/// Locations of iron's persistent state on disk.
#[derive(Debug, Clone)]
pub struct StatePaths {
    key_file: PathBuf,
    state_file: PathBuf,
}

impl StatePaths {
    /// State rooted at an explicit directory.
    ///
    /// This is the constructor for platforms that manage app storage
    /// themselves (Android data dir, tests with a temp dir, ...).
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        let config_dir = config_dir.into();
        Self::from_files(config_dir.join(KEY_FILE), config_dir.join(STATE_FILE))
    }

    /// State at independently selected identity-key and application database files.
    pub fn from_files(key_file: impl Into<PathBuf>, state_file: impl Into<PathBuf>) -> Self {
        Self {
            key_file: key_file.into(),
            state_file: state_file.into(),
        }
    }

    /// Path of the node's secret key file.
    pub fn key_file(&self) -> PathBuf {
        self.key_file.clone()
    }

    /// Path of the application-state database.
    pub fn state_file(&self) -> PathBuf {
        self.state_file.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paths_are_rooted_in_config_dir() {
        let paths = StatePaths::new("/data/iron");
        assert_eq!(paths.key_file(), PathBuf::from("/data/iron/secret.key"));
        assert_eq!(paths.state_file(), PathBuf::from("/data/iron/state.db"));
    }

    #[test]
    fn test_paths_can_be_independent() {
        let paths = StatePaths::from_files("/keys/node.key", "/cache/state.db");
        assert_eq!(paths.key_file(), PathBuf::from("/keys/node.key"));
        assert_eq!(paths.state_file(), PathBuf::from("/cache/state.db"));
    }
}
