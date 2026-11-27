//! `iron config show`: the resolved configuration as valid TOML.
//!
//! Explicitly set values are active lines annotated with their source;
//! defaults are commented out, so redirecting `iron --no-config --no-env config show`
//! into a file yields a complete reference that pins no defaults. Keys,
//! values and descriptions all come from `Settings` and the CLI arguments,
//! so there is no separate list of options to keep in sync.
use super::{CliSettings, Origin, Settings, Sources, env_var_name};
use anyhow::{Context, Result};
use clap::Args;
use serde_json::Value;
use std::fmt::Write;

/// Only the file content: anything about this particular run (such as which
/// config file was read) belongs on stderr, so redirected output stays true.
pub fn render(sources: &Sources) -> Result<String> {
    let resolved = serde_json::to_value(sources.settings()?)?;
    let defaults = serde_json::to_value(Settings::default())?;
    let args = CliSettings::augment_args(clap::Command::new("iron"));
    let mut out = String::new();
    writeln!(
        out,
        "# Active lines are set explicitly; commented lines are defaults."
    )?;
    render_table(&mut out, None, &resolved, &defaults, sources, &args)?;
    Ok(out)
}

fn render_table(
    out: &mut String,
    prefix: Option<&str>,
    resolved: &Value,
    defaults: &Value,
    sources: &Sources,
    args: &clap::Command,
) -> Result<()> {
    let Value::Object(fields) = resolved else {
        anyhow::bail!("configuration must serialize to a table");
    };
    let path_of = |key: &str| prefix.map_or_else(|| key.to_owned(), |p| format!("{p}.{key}"));
    // TOML requires plain keys before sub-tables.
    for (key, value) in fields.iter().filter(|(_, v)| !v.is_object()) {
        let path = path_of(key);
        let arg = args
            .get_arguments()
            .find(|arg| arg.get_id() == path.as_str())
            .with_context(|| format!("no CLI argument documents `{path}`"))?;
        writeln!(out)?;
        // Every line is commented, so multi-line help stays valid TOML.
        if let Some(help) = arg.get_help() {
            for help_line in help.to_string().lines() {
                writeln!(out, "# {help_line}")?;
            }
        }
        let line = format!("{key} = {}", toml_value(value)?);
        let source = match sources.origin(&path) {
            Origin::Default => {
                writeln!(out, "# {line}")?;
                continue;
            }
            Origin::File => "config file".to_owned(),
            Origin::Env => format!("env {}", env_var_name(&path)),
            Origin::Cli => format!("flag --{}", arg.get_long().unwrap_or(key)),
        };
        let default = toml_value(&defaults[key.as_str()])?;
        writeln!(out, "{line}  # from {source} (default: {default})")?;
    }
    for (key, value) in fields.iter().filter(|(_, v)| v.is_object()) {
        let path = path_of(key);
        writeln!(out, "\n[{path}]")?;
        render_table(
            out,
            Some(&path),
            value,
            &defaults[key.as_str()],
            sources,
            args,
        )?;
    }
    Ok(())
}

/// TOML has no null: unset optional values render as a placeholder, which
/// only ever appears on a commented-out line.
fn toml_value(value: &Value) -> Result<String> {
    if value.is_null() {
        return Ok("(unset)".into());
    }
    Ok(toml::Value::try_from(value)?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::IrohArgs;
    use figment::providers::Serialized;
    use serde_json::json;

    fn sources(file: Option<&str>, env: Value, cli: CliSettings) -> (tempfile::TempDir, Sources) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        if let Some(contents) = file {
            std::fs::write(&path, contents).unwrap();
        }
        let sources =
            Sources::load(file.map(|_| path.as_path()), Serialized::defaults(env), cli).unwrap();
        (dir, sources)
    }

    /// Every option appears, nothing is active, and the output is a valid
    /// config file that resolves to the defaults.
    #[test]
    fn defaults_render_as_a_complete_inert_config() {
        let (_dir, empty) = sources(None, json!({}), CliSettings::default());
        let out = render(&empty).unwrap();
        let defaults = serde_json::to_value(Settings::default()).unwrap();
        let mut keys = vec![];
        for (key, value) in defaults.as_object().unwrap() {
            match value.as_object() {
                Some(table) => keys.extend(table.keys().cloned()),
                None => keys.push(key.clone()),
            }
        }
        for key in keys {
            assert!(
                out.contains(&format!("\n# {key} = ")),
                "{key} missing:\n{out}"
            );
        }
        let active = out
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('['));
        assert_eq!(active.count(), 0, "{out}");
        let (_dir, reparsed) = sources(Some(&out), json!({}), CliSettings::default());
        assert_eq!(
            serde_json::to_value(reparsed.settings().unwrap()).unwrap(),
            defaults
        );
    }

    /// Arguments are matched to settings by dotted path, so every leaf of
    /// `Settings` needs an argument with exactly that id.
    #[test]
    fn every_setting_has_a_cli_argument() {
        fn leaves(prefix: &str, value: &Value, out: &mut Vec<String>) {
            for (key, value) in value.as_object().unwrap() {
                let path = format!("{prefix}{key}");
                match value {
                    Value::Object(_) => leaves(&format!("{path}."), value, out),
                    _ => out.push(path),
                }
            }
        }
        let mut paths = vec![];
        leaves(
            "",
            &serde_json::to_value(Settings::default()).unwrap(),
            &mut paths,
        );
        let args = CliSettings::augment_args(clap::Command::new("iron"));
        for path in paths {
            assert!(
                args.get_arguments()
                    .any(|arg| arg.get_id() == path.as_str()),
                "setting `{path}` has no CLI argument with that id"
            );
        }
    }

    #[test]
    fn explicit_values_are_active_and_attributed() {
        let (_dir, layered) = sources(
            Some("dns_port = 6000\n[iroh]\nrelay_servers = [\"https://relay.example.test\"]"),
            json!({"log_level": "error"}),
            CliSettings {
                iroh: IrohArgs {
                    default_relays: Some(false),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let out = render(&layered).unwrap();
        let cases = [
            "\ndns_port = 6000  # from config file (default: 5333)\n",
            "\nlog_level = \"error\"  # from env IRON_LOG_LEVEL (default: \"info\")\n",
            "\ndefault_relays = false  # from flag --default-relays (default: true)\n",
            "\nrelay_servers = [\"https://relay.example.test/\"]  # from config file (default: [])\n",
            "\n# pkarr_server = (unset)\n",
            "\n# key_file = ",
        ];
        fn run_test(out: &str, expected: &str) {
            assert!(out.contains(expected), "expected {expected:?} in:\n{out}");
        }
        for case in cases {
            run_test(&out, case)
        }

        // Redirecting the output reproduces the same configuration.
        let (_dir, reparsed) = sources(Some(&out), json!({}), CliSettings::default());
        assert_eq!(
            serde_json::to_value(reparsed.settings().unwrap()).unwrap(),
            serde_json::to_value(layered.settings().unwrap()).unwrap()
        );
    }
}
