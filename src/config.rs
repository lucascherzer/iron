use std::path::{Path, PathBuf};

use iroh::RelayUrl;
use serde::Deserialize;
use thiserror::Error;

pub const DEFAULT_CONFIG_DIR: &str = ".config/iron";
pub const DEFAULT_CONFIG_FILENAME: &str = "iron.toml";
pub const DEFAULT_FIREWALL_FILENAME: &str = "firewall.json";
pub const DEFAULT_KEY_FILENAME: &str = "secret.key";
pub const DEFAULT_KNOWN_PEERS_FILENAME: &str = "known_peers.json";

/// Returns `~/.config/iron` as an absolute path, using `$HOME`.
fn default_config_dir() -> Result<PathBuf, ConfigError> {
    let home = std::env::var("HOME").map_err(|_| ConfigError::NoHomeDir)?;
    Ok(PathBuf::from(home).join(DEFAULT_CONFIG_DIR))
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("$HOME is not set")]
    NoHomeDir,
    #[error("failed to read config file: {0}")]
    CouldNotOpen(#[source] std::io::Error),
    #[error("failed to parse config file: {0}")]
    InvalidConfig(#[source] toml::de::Error),
}

// ── Raw (deserialization) types ───────────────────────────────────────────────
//
// All fields are Option<T> so that absent keys never cause a parse error.
// These types are private — callers only ever see the resolved public types.

#[derive(Deserialize, Default)]
struct RawFirewallSection {
    /// Path to the firewall rules file.
    #[serde(default)]
    file: Option<PathBuf>,
    /// Whether the firewall is active. Defaults to `true`.
    #[serde(default)]
    enable: Option<bool>,
}

/// Deserialization mirror of [`IronConfig`].
///
/// Add new fields here as `Option<T>` to maintain backward compatibility with
/// older config files.
#[derive(Deserialize)]
struct RawConfig {
    #[serde(default)]
    key_file: Option<PathBuf>,
    #[serde(default)]
    known_peers_file: Option<PathBuf>,
    #[serde(default)]
    relays: Option<Vec<RelayUrl>>,
    /// Corresponds to the `[firewall]` TOML section.
    #[serde(default)]
    firewall: Option<RawFirewallSection>,
}

impl RawConfig {
    fn from_str(s: &str) -> Result<Self, ConfigError> {
        toml::from_str(s).map_err(ConfigError::InvalidConfig)
    }

    fn resolve(self) -> Result<IronConfig, ConfigError> {
        let dir = default_config_dir()?;
        // Absent [firewall] section is equivalent to an empty one.
        let fw = self.firewall.unwrap_or_default();
        Ok(IronConfig {
            key_file: self
                .key_file
                .unwrap_or_else(|| dir.join(DEFAULT_KEY_FILENAME)),
            known_peers_file: self
                .known_peers_file
                .unwrap_or_else(|| dir.join(DEFAULT_KNOWN_PEERS_FILENAME)),
            relays: self.relays,
            firewall: FirewallConfig {
                file: fw
                    .file
                    .unwrap_or_else(|| dir.join(DEFAULT_FIREWALL_FILENAME)),
                enable: fw.enable.unwrap_or(true),
            },
        })
    }
}

// ── Public (resolved) types ───────────────────────────────────────────────────

/// Firewall configuration, resolved from the `[firewall]` TOML section.
#[derive(Debug)]
pub struct FirewallConfig {
    /// Path to the firewall rules file.
    pub file: PathBuf,
    /// Whether the firewall is active.
    pub enable: bool,
}

/// Iron node configuration with all values resolved to their concrete types.
///
/// Obtain via [`IronConfig::parse`], [`IronConfig::parse_file`], or
/// [`IronConfig::parse_str`]. Values absent from the config file fall back to
/// sensible defaults (paths under `~/.config/iron/`, firewall enabled).
#[derive(Debug)]
pub struct IronConfig {
    /// Path to the node's secret key file.
    pub key_file: PathBuf,
    /// Path to the known peers file.
    pub known_peers_file: PathBuf,
    /// Relay servers to use. If absent, iroh's built-in relays are used.
    pub relays: Option<Vec<RelayUrl>>,
    /// Firewall settings.
    pub firewall: FirewallConfig,
}

impl Default for IronConfig {
    /// Returns a default config with all values resolved from `$HOME`.
    ///
    /// Equivalent to parsing an empty TOML string. Panics only if `$HOME` is
    /// not set, which is always a fatal misconfiguration.
    fn default() -> Self {
        Self::parse_str("").expect("default config resolution requires $HOME to be set")
    }
}

impl IronConfig {
    /// Parse the config from its default location (`~/.config/iron/iron.toml`).
    pub fn parse() -> Result<Self, ConfigError> {
        let p = default_config_dir()?.join(DEFAULT_CONFIG_FILENAME);
        Self::parse_file(&p)
    }

    /// Parse the config from an arbitrary file path.
    pub fn parse_file(f: &Path) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(f).map_err(ConfigError::CouldNotOpen)?;
        Self::parse_str(&contents)
    }

    /// Parse the config from a TOML string directly.
    ///
    /// Useful for testing without touching the filesystem.
    pub fn parse_str(s: &str) -> Result<Self, ConfigError> {
        RawConfig::from_str(s)?.resolve()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // ── parse_str ────────────────────────────────────────────────────────────

    #[test]
    fn parse_str_full_config() {
        let toml = r#"
            key_file = "/etc/iron/my.key"
            known_peers_file = "/etc/iron/peers.json"
            [firewall]
            file = "/etc/iron/fw.json"
            enable = false
        "#;
        let cfg = IronConfig::parse_str(toml).expect("should parse");
        assert_eq!(cfg.key_file, PathBuf::from("/etc/iron/my.key"));
        assert_eq!(cfg.known_peers_file, PathBuf::from("/etc/iron/peers.json"));
        assert_eq!(cfg.firewall.file, PathBuf::from("/etc/iron/fw.json"));
        assert!(!cfg.firewall.enable);
        assert!(cfg.relays.is_none());
    }

    #[test]
    fn parse_str_empty_config_uses_defaults() {
        // An empty TOML file must not be an error — all values get defaults.
        let cfg = IronConfig::parse_str("").expect("empty config should parse");
        assert!(cfg.key_file.ends_with(DEFAULT_KEY_FILENAME));
        assert!(cfg.known_peers_file.ends_with(DEFAULT_KNOWN_PEERS_FILENAME));
        assert!(cfg.relays.is_none());
        let fw_ok = cfg.firewall.file.ends_with(DEFAULT_FIREWALL_FILENAME);
        assert!(fw_ok);
        assert!(cfg.firewall.enable);
    }

    #[test]
    fn parse_str_partial_firewall_section_fills_missing_with_defaults() {
        // Only `enable` set — `file` should still get its default.
        let toml = "[firewall]\nenable = false";
        let cfg = IronConfig::parse_str(toml).expect("partial firewall section should parse");
        let fw_ok = cfg.firewall.file.ends_with(DEFAULT_FIREWALL_FILENAME);
        assert!(fw_ok);
        assert!(!cfg.firewall.enable);
    }

    #[test]
    fn parse_str_absent_firewall_section_uses_defaults() {
        let cfg = IronConfig::parse_str("").expect("should parse");
        let fw_ok = cfg.firewall.file.ends_with(DEFAULT_FIREWALL_FILENAME);
        assert!(fw_ok);
        assert!(cfg.firewall.enable);
    }

    #[test]
    fn parse_str_invalid_toml_returns_error() {
        let result = IronConfig::parse_str("key_file = [[[invalid");
        assert!(
            matches!(result, Err(ConfigError::InvalidConfig(_))),
            "malformed TOML should return InvalidConfig"
        );
    }

    // ── parse_file ───────────────────────────────────────────────────────────

    #[test]
    fn parse_file_reads_from_disk() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "[firewall]\nfile = \"/tmp/fw.json\"").unwrap();
        let cfg = IronConfig::parse_file(f.path()).expect("should parse file");
        assert_eq!(cfg.firewall.file, PathBuf::from("/tmp/fw.json"));
    }

    #[test]
    fn parse_file_missing_file_returns_could_not_open() {
        let result = IronConfig::parse_file(Path::new("/nonexistent/iron.toml"));
        assert!(
            matches!(result, Err(ConfigError::CouldNotOpen(_))),
            "missing file should return CouldNotOpen"
        );
    }
}
