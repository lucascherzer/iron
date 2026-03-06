use std::path::PathBuf;

use iroh::RelayUrl;
use serde::Deserialize;

pub const DEFAULT_CONFIG_DIR: &str = ".config/iron/";
pub const DEFAULT_CONFIG_FILENAME: &str = "iron.toml";
pub const DEFAULT_FIREWALL_FILENAME: &str = "firewall.json";
pub const DEFAULT_KEY_FILENAME: &str = "secret.key";
pub const DEFAULT_KNOWN_PEERS_FILENAME: &str = "known_peers.json";

enum ConfigParseError {
    /// The config could not be parsed.
    InvalidConfig,
    /// The config file does not exist or we lack the permission to open it.
    CouldNotOpen,
}

#[derive(Deserialize)]
struct IronConfig {
    key_file: PathBuf,
    firewall_config_file: PathBuf,
    known_peers_file: PathBuf,
    relays: Option<Vec<RelayUrl>>,
}

impl IronConfig {
    /// Attempts to parse the config file in its default location `DEFAULT_CONFIG_DIR`
    fn parse() -> Result<Self, ConfigParseError> {
        let p = PathBuf::from(DEFAULT_CONFIG_DIR).join(DEFAULT_CONFIG_FILENAME);
        IronConfig::parse_file(p)
    }
    /// Attempts to parse the config file, reading from `f`
    fn parse_file(f: PathBuf) -> Result<Self, ConfigParseError> {
        std::fs::read_to_string(f)
            .map_err(|_| ConfigParseError::CouldNotOpen)
            .and_then(|c| {
                toml::from_str::<IronConfig>(&*c).map_err(|_| ConfigParseError::InvalidConfig)
            })
    }
}

impl Default for IronConfig {
    fn default() -> Self {
        IronConfig {
            key_file: PathBuf::from(DEFAULT_CONFIG_DIR).join(DEFAULT_KEY_FILENAME),
            firewall_config_file: PathBuf::from(DEFAULT_CONFIG_DIR).join(DEFAULT_FIREWALL_FILENAME),
            known_peers_file: PathBuf::from(DEFAULT_CONFIG_DIR).join(DEFAULT_KNOWN_PEERS_FILENAME),
            relays: None,
        }
    }
}
