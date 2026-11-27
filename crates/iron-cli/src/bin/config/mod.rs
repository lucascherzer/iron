//! Configuration sources supply sparse values; Figment merges them and
//! Serde extracts the resolved native types. The loader never reads the
//! process environment: production supplies an Env provider, tests supply data.
use anyhow::{Context, Result};
use clap::Args;
use figment::{
    Figment, Provider,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};

pub mod show;
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub key_file: PathBuf,
    pub state_file: PathBuf,
    pub log_level: LogLevel,
    pub dns_port: u16,
    pub dns_upstream: Vec<SocketAddr>,
    pub iroh: iron_core::IrohConfig,
}

impl Default for Settings {
    fn default() -> Self {
        let paths = iron_desktop::default_state_paths();
        Self {
            key_file: paths.key_file(),
            state_file: paths.state_file(),
            log_level: LogLevel::default(),
            dns_port: 5333,
            dns_upstream: vec![],
            iroh: Default::default(),
        }
    }
}

// A small boundary type: unlike tracing::Level it derives both Serde and
// Clap's value parser, keeping config and CLI spellings identical without
// a custom serialization adapter. It is not a separate logging mechanism.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        })
    }
}

/// These are the CLI arguments themselves, not another intermediate overlay
/// schema. Missing arguments are omitted when supplied to Figment.
#[derive(Args, Debug, Default, Serialize)]
pub struct CliSettings {
    /// Key file location
    #[arg(long, global = true)]
    #[serde(skip_serializing_if = "Option::is_none")]
    key_file: Option<PathBuf>,
    /// Application-state database location
    #[arg(long, global = true)]
    #[serde(skip_serializing_if = "Option::is_none")]
    state_file: Option<PathBuf>,
    /// Log level: trace, debug, info, warn, error
    #[arg(short, long, global = true, value_enum)]
    #[serde(skip_serializing_if = "Option::is_none")]
    log_level: Option<LogLevel>,
    /// DNS server port
    #[arg(long, global = true)]
    #[serde(skip_serializing_if = "Option::is_none")]
    dns_port: Option<u16>,
    /// Upstream DNS server (repeatable; replaces lower-layer list)
    #[arg(long, global = true, value_name = "ADDR")]
    #[serde(skip_serializing_if = "Option::is_none")]
    dns_upstream: Option<Vec<SocketAddr>>,
    #[command(flatten)]
    iroh: IrohArgs,
}

/// Nested arguments take their dotted config path as clap id, so
/// `config show` can match every setting to its argument by path.
#[derive(Args, Debug, Default, Serialize)]
struct IrohArgs {
    /// Override pkarr publication and lookup server
    #[arg(
        id = "iroh.pkarr_server",
        long = "pkarr-server",
        value_name = "URL",
        global = true
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    pkarr_server: Option<url::Url>,
    /// Use iroh's default relays (disable with --default-relays=false)
    #[arg(
        id = "iroh.default_relays",
        long = "default-relays",
        value_name = "BOOL",
        global = true,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true"
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    default_relays: Option<bool>,
    /// Custom relay URL (repeatable; replaces lower-layer list)
    #[arg(
        id = "iroh.relay_servers",
        long = "relay-server",
        value_name = "URL",
        global = true
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    relay_servers: Option<Vec<iroh::RelayUrl>>,
}

const ENV_PREFIX: &str = "IRON_";

/// Read the real environment only at the application boundary. Individual
/// names derive from field names; only the iroh namespace needs translation.
/// Disabled (`--no-env`), the layer is present but empty.
pub fn environment(enabled: bool) -> Env {
    Env::prefixed(ENV_PREFIX)
        .ignore(&["CONFIG"])
        .filter(move |_| enabled)
        .map(|key| env_key_to_path(key.as_str()).into())
}

/// `DNS_PORT` -> `dns_port`, `IROH_RELAY_SERVERS` -> `iroh.relay_servers`.
fn env_key_to_path(key: &str) -> String {
    let key = key.to_ascii_lowercase();
    key.strip_prefix("iroh_")
        .map_or_else(|| key.clone(), |field| format!("iroh.{field}"))
}

/// Inverse of [`env_key_to_path`], including the prefix.
pub fn env_var_name(path: &str) -> String {
    format!(
        "{ENV_PREFIX}{}",
        path.replace('.', "_").to_ascii_uppercase()
    )
}

pub fn select_config(
    no_config: bool,
    explicit: Option<PathBuf>,
    env_path: Option<PathBuf>,
    default_path: PathBuf,
) -> Result<Option<PathBuf>> {
    if no_config {
        return Ok(None);
    }
    if let Some(path) = explicit.or(env_path) {
        std::fs::metadata(&path)
            .with_context(|| format!("reading explicit configuration {}", path.display()))?;
        return Ok(Some(path));
    }
    match std::fs::metadata(&default_path) {
        Ok(_) => Ok(Some(default_path)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            let hint = if error.kind() == std::io::ErrorKind::PermissionDenied {
                " (run as root, or pass --no-config)"
            } else {
                ""
            };
            Err(error).with_context(|| {
                format!(
                    "reading default configuration {}{hint}",
                    default_path.display()
                )
            })
        }
    }
}

/// Human-readable note on which configuration file was used, for
/// `config show` and the daemon's startup log.
pub fn describe_selection(no_config: bool, selected: Option<&Path>, default: &Path) -> String {
    match selected {
        Some(path) => path.display().to_string(),
        None if no_config => "none (--no-config)".into(),
        None => format!("none ({} not found, using defaults)", default.display()),
    }
}

/// Which layer supplied a value. Later variants take precedence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Default,
    File,
    Env,
    Cli,
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Default => "defaults",
            Self::File => "config file",
            Self::Env => "environment",
            Self::Cli => "command-line flags",
        })
    }
}

/// The configuration layers, kept apart so `config show` can attribute each
/// value to the layer it came from.
pub struct Sources {
    layers: Vec<(Origin, Figment)>,
    /// Named in errors: the TOML is parsed from a string, so Figment's own
    /// errors don't carry the path.
    config_file: Option<PathBuf>,
}

impl Sources {
    pub fn load(config_file: Option<&Path>, env: impl Provider, cli: CliSettings) -> Result<Self> {
        let mut layers = vec![(
            Origin::Default,
            Figment::from(Serialized::defaults(Settings::default())),
        )];
        if let Some(path) = config_file {
            let contents = std::fs::read_to_string(path)
                .with_context(|| format!("reading configuration {}", path.display()))?;
            layers.push((Origin::File, Figment::from(Toml::string(&contents))));
        }
        layers.push((Origin::Env, Figment::from(env)));
        layers.push((Origin::Cli, Figment::from(Serialized::defaults(cli))));
        Ok(Self {
            layers,
            config_file: config_file.map(Path::to_path_buf),
        })
    }

    /// Strict: the configuration must be valid after every layer, so an
    /// error always names the layer that broke it, even if a higher layer
    /// would have overridden the bad value.
    pub fn settings(&self) -> Result<Settings> {
        let mut merged = Figment::new();
        let mut settings = None;
        for (origin, layer) in &self.layers {
            merged = merged.merge(layer.clone());
            let valid = validate(&merged).with_context(|| match (origin, &self.config_file) {
                (Origin::File, Some(path)) => {
                    format!("invalid configuration in {origin} {}", path.display())
                }
                _ => format!("invalid configuration in {origin}"),
            })?;
            settings = Some(valid);
        }
        settings.context("no configuration layers")
    }

    /// The highest layer that sets `key` (dotted path, e.g. `iroh.relay_servers`).
    pub fn origin(&self, key: &str) -> Origin {
        self.layers
            .iter()
            .rev()
            .find(|(_, layer)| layer.find_value(key).is_ok())
            .map_or(Origin::Default, |(origin, _)| *origin)
    }
}

fn validate(merged: &Figment) -> Result<Settings> {
    let settings: Settings = merged.extract()?;
    settings.iroh.relay_mode()?;
    if settings.key_file == settings.state_file {
        anyhow::bail!("key_file and state_file must be different files");
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn resolve(
        config_file: Option<&Path>,
        env: impl Provider,
        cli: CliSettings,
    ) -> Result<Settings> {
        Sources::load(config_file, env, cli)?.settings()
    }

    fn empty_env() -> impl Provider {
        Serialized::defaults(json!({}))
    }

    #[test]
    fn layers_preserve_unrelated_fields_and_replace_lists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
key_file = "/toml/key"
dns_port = 6000
dns_upstream = ["1.1.1.1:53"]
[iroh]
default_relays = false
relay_servers = ["https://relay.example.test"]
pkarr_server = "https://discovery.example.test"
"#,
        )
        .unwrap();
        let settings = resolve(
            Some(&path),
            Serialized::defaults(json!({"dns_port": 7000, "dns_upstream": []})),
            CliSettings {
                dns_port: Some(8000),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(settings.dns_port, 8000);
        assert_eq!(settings.key_file, PathBuf::from("/toml/key"));
        assert!(settings.dns_upstream.is_empty());
        assert!(!settings.iroh.default_relays);
        assert_eq!(settings.iroh.relay_servers.len(), 1);
        assert!(settings.iroh.pkarr_server.is_some());
        let settings = resolve(
            Some(&path),
            empty_env(),
            CliSettings {
                iroh: IrohArgs {
                    default_relays: Some(true),
                    relay_servers: Some(vec![]),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        assert!(settings.iroh.default_relays);
        assert!(settings.iroh.relay_servers.is_empty());
        assert!(settings.iroh.pkarr_server.is_some());
    }

    #[test]
    fn invalid_environment_values_are_rejected_even_if_overridden() {
        let cases = [
            (json!({"dns_port": 65536}), false),
            (json!({"dns_port": 65536}), true),
            (json!({"dns_upstream": "not-json"}), false),
            (json!({"log_level": "nonsense"}), false),
            (json!({"iroh": {"pkarr_server": "bad"}}), false),
            (json!({"iroh": {"relay_servers": ["bad"]}}), false),
            (json!({"iroh": {"default_relays": "maybe"}}), false),
            // Cross-field rule: no relay left at all.
            (json!({"iroh": {"default_relays": false}}), false),
        ];
        fn run_test(case: (serde_json::Value, bool)) {
            let (env, overridden) = case;
            let cli = CliSettings {
                dns_port: overridden.then_some(5333),
                ..Default::default()
            };
            let error = resolve(None, Serialized::defaults(&env), cli)
                .expect_err(&format!("{env} overridden={overridden} should be rejected"));
            assert_eq!(
                error.to_string(),
                "invalid configuration in environment",
                "{env} should blame the environment"
            );
        }
        for case in cases {
            run_test(case)
        }
    }

    #[test]
    fn defaults_apply_without_any_source() {
        let settings = resolve(None, empty_env(), CliSettings::default()).unwrap();
        assert_eq!(settings.dns_port, 5333);
    }

    #[test]
    fn invalid_config_files_are_rejected() {
        let cases = [
            ("unknown = 1", "unknown key"),
            ("[iroh]\nunknown = 1", "unknown iroh key"),
            ("dns_port = -1", "negative port"),
            ("invalid [", "malformed toml"),
        ];
        fn run_test(case: (&str, &str)) {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("bad.toml");
            std::fs::write(&path, case.0).unwrap();
            let result = resolve(Some(&path), empty_env(), CliSettings::default());
            assert!(result.is_err(), "{} should be rejected", case.1);
        }
        for case in cases {
            run_test(case)
        }
    }

    #[test]
    fn every_layer_must_be_valid_on_its_own() {
        // Disabling default relays in one layer and adding custom relays in
        // a higher one is rejected: the lower layer alone leaves no relay.
        let error = resolve(
            None,
            Serialized::defaults(json!({"iroh": {"default_relays": false}})),
            CliSettings {
                iroh: IrohArgs {
                    relay_servers: Some(vec!["http://relay.example.test".parse().unwrap()]),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "invalid configuration in environment");
        // The same settings within one layer are fine.
        let settings = resolve(
            None,
            Serialized::defaults(json!({"iroh": {
                "default_relays": false,
                "relay_servers": ["http://relay.example.test"],
            }})),
            CliSettings::default(),
        )
        .unwrap();
        assert!(!settings.iroh.default_relays);
        assert_eq!(settings.iroh.relay_servers.len(), 1);
    }

    #[test]
    fn select_config_precedence() {
        // (no_config, explicit, env, default) -> expected, using file names
        // that all exist, so only the precedence is under test.
        type Case = (
            bool,
            Option<&'static str>,
            Option<&'static str>,
            Option<&'static str>,
        );
        let cases: [(Case, Option<&str>); 5] = [
            (
                (false, Some("explicit"), Some("env"), Some("default")),
                Some("explicit"),
            ),
            ((false, None, Some("env"), Some("default")), Some("env")),
            ((false, None, None, Some("default")), Some("default")),
            ((false, None, None, None), None),
            ((true, None, Some("env"), Some("default")), None),
        ];
        fn run_test(case: (Case, Option<&str>)) {
            let dir = tempfile::tempdir().unwrap();
            for name in ["explicit", "env", "default"] {
                std::fs::write(dir.path().join(name), "").unwrap();
            }
            let path = |name: &str| dir.path().join(name);
            let (no_config, explicit, env, default) = case.0;
            // A missing default file is spelled as a path that was never written.
            let default = default.map_or(path("missing"), path);
            let selected =
                select_config(no_config, explicit.map(path), env.map(path), default).unwrap();
            assert_eq!(selected, case.1.map(path), "selecting from {:?}", case.0);
        }
        for case in cases {
            run_test(case)
        }
    }

    #[test]
    fn missing_or_unreadable_config_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        assert!(select_config(false, Some(missing.clone()), None, missing.clone()).is_err());
        assert!(resolve(Some(&missing), empty_env(), CliSettings::default()).is_err());
        assert!(resolve(Some(dir.path()), empty_env(), CliSettings::default()).is_err());
    }

    #[test]
    fn env_names_round_trip() {
        let cases = [
            ("dns_port", "IRON_DNS_PORT"),
            ("state_file", "IRON_STATE_FILE"),
            ("iroh.relay_servers", "IRON_IROH_RELAY_SERVERS"),
        ];
        fn run_test(case: (&str, &str)) {
            let (path, var) = case;
            assert_eq!(env_var_name(path), var, "naming {path}");
            let key = var.strip_prefix(ENV_PREFIX).unwrap();
            assert_eq!(env_key_to_path(key), path, "mapping {var}");
        }
        for case in cases {
            run_test(case)
        }
    }
}
