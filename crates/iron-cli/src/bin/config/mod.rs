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

#[derive(Args, Debug, Default, Serialize)]
struct IrohArgs {
    /// Override pkarr publication and lookup server
    #[arg(long, global = true)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pkarr_server: Option<url::Url>,
    /// Use iroh's default relays (disable with --default-relays=false)
    #[arg(long, global = true, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    #[serde(skip_serializing_if = "Option::is_none")]
    default_relays: Option<bool>,
    /// Custom relay URL (repeatable; replaces lower-layer list)
    #[arg(long = "relay-server", global = true)]
    #[serde(skip_serializing_if = "Option::is_none")]
    relay_servers: Option<Vec<iroh::RelayUrl>>,
}

/// Read the real environment only at the application boundary. Individual
/// names derive from field names; only the iroh namespace needs translation.
pub fn environment() -> Env {
    Env::prefixed("IRON_").ignore(&["CONFIG"]).map(|key| {
        let key = key.as_str().to_ascii_lowercase();
        key.strip_prefix("iroh_").map_or_else(
            || key.clone().into(),
            |field| format!("iroh.{field}").into(),
        )
    })
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
        Err(error) => Err(error).context("reading default configuration"),
    }
}

/// Layers are merged untyped and only the final result is checked, so a bad
/// value in a lower layer is ignored if a higher layer overrides it.
pub fn resolve(
    config_file: Option<&Path>,
    env: impl Provider,
    cli: CliSettings,
) -> Result<Settings> {
    let mut sources = Figment::from(Serialized::defaults(Settings::default()));
    if let Some(path) = config_file {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("reading configuration {}", path.display()))?;
        sources = sources.merge(Toml::string(&contents));
    }
    let settings: Settings = sources
        .merge(env)
        .merge(Serialized::defaults(cli))
        .extract()
        .context("invalid configuration")?;
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
    fn invalid_environment_values_are_rejected_unless_overridden() {
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
            let result = resolve(None, Serialized::defaults(&env), cli);
            assert_eq!(result.is_ok(), overridden, "{env} overridden={overridden}");
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
    fn related_settings_can_be_completed_by_a_higher_layer() {
        let settings = resolve(
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
}
