//! Exercise the production CLI without starting TUN or touching system DNS.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_iron"));
    command.env_clear().env("IRON_LOG_LEVEL", "error");
    command
}

fn succeeds(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn write_identity(path: &Path, byte: u8) -> String {
    let key = iroh::SecretKey::from_bytes(&[byte; 32]);
    fs::write(path, key.to_bytes()).unwrap();
    key.public().to_string()
}

#[test]
fn identity_commands_use_file_environment_and_cli_precedence() {
    let dir = tempfile::tempdir().unwrap();
    let file_key = dir.path().join("file.key");
    let env_key = dir.path().join("env.key");
    let cli_key = dir.path().join("cli.key");
    let file_id = write_identity(&file_key, 1);
    let env_id = write_identity(&env_key, 2);
    let cli_id = write_identity(&cli_key, 3);
    let config = dir.path().join("config.toml");
    fs::write(
        &config,
        format!("key_file = {}\n", serde_json::to_string(&file_key).unwrap()),
    )
    .unwrap();

    assert_eq!(
        succeeds(
            command()
                .arg("--config")
                .arg(&config)
                .args(["self", "--hex"])
                .output()
                .unwrap()
        )
        .trim(),
        file_id
    );
    assert_eq!(
        succeeds(
            command()
                .env("IRON_CONFIG", &config)
                .env("IRON_KEY_FILE", &env_key)
                .args(["self", "--hex"])
                .output()
                .unwrap()
        )
        .trim(),
        env_id
    );
    let output = succeeds(
        command()
            .env("IRON_CONFIG", &config)
            .env("IRON_KEY_FILE", &env_key)
            .arg("--key-file")
            .arg(&cli_key)
            .args(["self", "--format", "json"])
            .output()
            .unwrap(),
    );
    let info: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(info["node_id"]["hex"], cli_id);
    assert_eq!(info["key_file"], cli_key.to_str().unwrap());
    let output = command()
        .env("IRON_KEY_FILE", &env_key)
        .env("IRON_LOG_LEVEL", "debug")
        .args(["--no-config", "self", "--format", "json"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stderr).contains("iron_core::keys"));
    let info: serde_json::Value = serde_json::from_str(&succeeds(output)).unwrap();
    assert_eq!(info["node_id"]["hex"], env_id);
    let output = succeeds(
        command()
            .env("IRON_CONFIG", &config)
            .env("IRON_KEY_FILE", &env_key)
            .args(["key", "info"])
            .output()
            .unwrap(),
    );
    assert!(output.contains(&env_id));
}

#[test]
fn no_config_skips_file_but_preserves_environment_and_read_only_behavior() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("identity.key");
    let missing_config = dir.path().join("missing.toml");
    let output = command()
        .env("IRON_CONFIG", &missing_config)
        .env("IRON_KEY_FILE", &key)
        .args(["--no-config", "self", "--exists"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!key.exists());
    succeeds(
        command()
            .env("IRON_CONFIG", &missing_config)
            .env("IRON_KEY_FILE", &key)
            .env("RUST_LOG", "invalid legacy filter")
            .args(["--no-config", "key", "generate", "--save"])
            .output()
            .unwrap(),
    );
    assert_eq!(fs::read(&key).unwrap().len(), 32);
    succeeds(
        command()
            .env("IRON_CONFIG", &missing_config)
            .env("IRON_KEY_FILE", &key)
            .args(["--no-config", "self", "--exists"])
            .output()
            .unwrap(),
    );
    let output = command()
        .env("IRON_CONFIG", &missing_config)
        .env("IRON_KEY_FILE", &key)
        .args(["self", "--hex"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    fs::write(&key, b"corrupt").unwrap();
    assert!(
        !command()
            .env("IRON_KEY_FILE", &key)
            .args(["--no-config", "self", "--hex"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(fs::read(&key).unwrap(), b"corrupt");
}

#[test]
fn migration_and_global_flags_work_through_real_binary() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("legacy");
    fs::create_dir(&source).unwrap();
    let id = write_identity(&source.join("secret.key"), 4);
    let peer = iroh::SecretKey::from_bytes(&[9; 32]).public();
    fs::write(
        source.join("known_peers.json"),
        serde_json::to_vec(&vec![
            iron_core::id::to_base32(&peer),
            iron_core::id::to_base32(&peer),
        ])
        .unwrap(),
    )
    .unwrap();
    let key = dir.path().join("state/key");
    let peers = dir.path().join("state/peers");
    let run_migration = || {
        command()
            .args(["--no-config", "--key-file"])
            .arg(&key)
            .arg("--state-file")
            .arg(&peers)
            .args(["migrate", "--from"])
            .arg(&source)
            .output()
            .unwrap()
    };
    succeeds(run_migration());
    assert!(!run_migration().status.success());
    assert!(source.join("secret.key").exists());
    assert!(fs::read(&peers).unwrap().starts_with(b"SQLite format 3"));
    let output = succeeds(
        command()
            .args(["self", "--no-config", "--key-file"])
            .arg(&key)
            .args([
                "--hex",
                "--default-relays=false",
                "--relay-server",
                "https://relay.example.test",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(output.trim(), id);
    assert!(
        !command()
            .args(["--no-config", "--default-relays=false", "self"])
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn environment_provider_preserves_field_names() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("identity.key");
    let identity = write_identity(&key, 5);
    let run = |relays: &str| {
        command()
            .env("IRON_KEY_FILE", &key)
            .env("IRON_DNS_UPSTREAM", r#"["[::1]:53","1.1.1.1:53"]"#)
            .env("IRON_IROH_DEFAULT_RELAYS", "false")
            .env(
                "IRON_IROH_PKARR_SERVER",
                "http://discovery.example.test/pkarr",
            )
            .env("IRON_IROH_RELAY_SERVERS", relays)
            .args(["--no-config", "self", "--hex"])
            .output()
            .unwrap()
    };
    assert_eq!(
        succeeds(run(r#"["http://relay.example.test"]"#)).trim(),
        identity
    );
    succeeds(
        command()
            .env("IRON_KEY_FILE", &key)
            .env("IRON_IROH_DEFAULT_RELAYS", "false")
            .env(
                "IRON_IROH_RELAY_SERVERS",
                r#"["http://relay.example.test"]"#,
            )
            .args(["--no-config", "--default-relays", "self", "--hex"])
            .output()
            .unwrap(),
    );
    assert!(
        !command()
            .env("IRON_KEY_FILE", &key)
            .args(["--no-config", "--default-relays=false", "self", "--hex"])
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn commands_without_settings_ignore_broken_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    fs::write(&config, "invalid [").unwrap();
    let hex_id = "01".repeat(32);
    let output = succeeds(
        command()
            .arg("--config")
            .arg(&config)
            .env("IRON_DNS_PORT", "not-a-port")
            .args(["convert", &hex_id, "--to", "ipv6"])
            .output()
            .unwrap(),
    );
    assert!(!output.trim().is_empty());
    // Commands that do need settings still fail loudly.
    assert!(
        !command()
            .arg("--config")
            .arg(&config)
            .args(["self", "--hex"])
            .output()
            .unwrap()
            .status
            .success()
    );
}
