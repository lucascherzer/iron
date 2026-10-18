use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use iron_core::IronNode;
use iron_core::platform::PrivateFileWriter;
use iron_desktop::dns_config;
use std::net::SocketAddr;
use tracing::{error, info, warn};

mod commands;
mod config;

/// iron - P2P network interface based on iroh
///
/// Creates a TUN network interface and DNS resolver for .iron domains,
/// enabling peer-to-peer connectivity over iroh's encrypted QUIC protocol.
#[derive(Parser, Debug)]
#[command(name = "iron")]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Configuration file (defaults to desktop config location)
    #[arg(long, global = true)]
    config: Option<std::path::PathBuf>,

    /// Skip loading any configuration file
    #[arg(long, global = true, conflicts_with = "config")]
    no_config: bool,

    #[command(flatten)]
    settings: config::CliSettings,

    /// Remove DNS configuration for .iron domains (manual cleanup)
    #[arg(long)]
    cleanup_dns: bool,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Start the iron daemon (TUN interface and DNS server)
    Serve,

    /// Convert between node ID formats (hex, base32, .iron domain, IPv6)
    Convert {
        /// Value to convert (auto-detects format)
        value: String,

        /// Output format: hex, base32, iron, ipv6
        #[arg(long)]
        to: Option<String>,
    },

    /// Show information about your node
    #[command(name = "self")]
    Self_ {
        /// Output format (json, hex, base32)
        #[arg(short = 'f', long, value_name = "FORMAT")]
        format: Option<String>,

        /// Show only hex Node ID
        #[arg(long)]
        hex: bool,

        /// Show only base32 Node ID
        #[arg(long)]
        base32: bool,

        /// Show only .iron domain
        #[arg(long)]
        domain: bool,

        /// Show only IPv6 address
        #[arg(long)]
        ipv6: bool,

        /// Check if key exists (exit code 0 if exists, 1 otherwise)
        #[arg(long)]
        exists: bool,
    },

    /// Generate vanity address with desired prefix
    Vanity {
        /// Desired prefix (case-insensitive, base32 alphabet)
        prefix: String,

        /// Number of threads to use (default: number of CPUs)
        #[arg(long)]
        threads: Option<usize>,

        /// Maximum attempts before giving up
        #[arg(long)]
        max_attempts: Option<u64>,

        /// Save the generated key as default
        #[arg(long)]
        save: bool,

        /// Output file path
        #[arg(long)]
        output: Option<String>,

        /// Only output the result, no progress
        #[arg(long)]
        quiet: bool,
    },

    /// Key management utilities
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },

    /// Test DNS resolution
    Resolve {
        /// Domain to resolve
        domain: String,

        /// DNS server address
        #[arg(long, default_value = "127.0.0.1:5333")]
        server: String,

        /// Query timeout in seconds
        #[arg(long, default_value = "5")]
        timeout: u64,

        /// Output format (json)
        #[arg(short = 'f', long, value_name = "FORMAT")]
        format: Option<String>,
    },

    /// Import legacy state from a source directory
    Migrate {
        /// Legacy source directory
        #[arg(long)]
        from: std::path::PathBuf,
    },
}

#[derive(Subcommand, Debug)]
enum KeyCommand {
    /// Show key information
    Info,

    /// Export key to file
    Export {
        /// Output format: hex, base64
        #[arg(long, default_value = "hex")]
        format: String,

        /// Output file path
        #[arg(long)]
        output: Option<String>,
    },

    /// Import key from file
    Import {
        /// Input file path
        file: String,

        /// Save as default key
        #[arg(long)]
        save: bool,
    },

    /// Generate new random key
    Generate {
        /// Save as default key
        #[arg(long)]
        save: bool,

        /// Force overwrite existing key
        #[arg(long)]
        force: bool,
    },

    /// Validate key file
    Validate,

    /// Reset (delete) current key
    Reset {
        /// Skip confirmation prompt
        #[arg(long)]
        confirm: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // These don't read configuration, so a broken config file or environment
    // must not prevent them from running (`--cleanup-dns` is the recovery path).
    if cli.cleanup_dns {
        init_tracing(&config::LogLevel::default().to_string())?;
        return dns_config::cleanup_dns();
    }
    let command = match cli.command {
        Some(Command::Convert { value, to }) => return commands::convert::run(value, to),
        Some(Command::Resolve {
            domain,
            server,
            timeout,
            format,
        }) => return commands::resolve::run(domain, server, timeout, format).await,
        command => command,
    };

    let selected_config = config::select_config(
        cli.no_config,
        cli.config.clone(),
        std::env::var_os("IRON_CONFIG").map(std::path::PathBuf::from),
        iron_desktop::default_config_file(),
    )?;
    let settings = config::resolve(
        selected_config.as_deref(),
        config::environment(),
        cli.settings,
    )?;

    // Initialize tracing subscriber (skip for some quiet commands)
    let skip_tracing = matches!(
        command,
        Some(Command::Self_ { exists: true, .. })
            | Some(Command::Key {
                command: KeyCommand::Info
            })
    );

    if !skip_tracing {
        init_tracing(&settings.log_level.to_string())?;
    }

    match command {
        Some(Command::Serve) => {
            // Start daemon
            start_daemon(settings).await?;
        }
        Some(Command::Self_ {
            format,
            hex,
            base32,
            domain,
            ipv6,
            exists,
        }) => {
            commands::self_::run(
                &settings.key_file,
                format,
                hex,
                base32,
                domain,
                ipv6,
                exists,
            )?;
        }
        Some(Command::Vanity {
            prefix,
            threads,
            max_attempts,
            save,
            output,
            quiet,
        }) => {
            commands::vanity::run(
                &settings.key_file,
                prefix,
                threads,
                max_attempts,
                save,
                output,
                quiet,
            )?;
        }
        Some(Command::Key { command }) => match command {
            KeyCommand::Info => {
                commands::key::info(&settings.key_file)?;
            }
            KeyCommand::Export { format, output } => {
                commands::key::export(format, output, &settings.key_file)?;
            }
            KeyCommand::Import { file, save } => {
                commands::key::import(file, save, &settings.key_file)?;
            }
            KeyCommand::Generate { save, force } => {
                commands::key::generate(save, force, &settings.key_file)?;
            }
            KeyCommand::Validate => {
                commands::key::validate(&settings.key_file)?;
            }
            KeyCommand::Reset { confirm } => {
                commands::key::reset(confirm, &settings.key_file)?;
            }
        },
        Some(Command::Convert { .. } | Command::Resolve { .. }) => {
            unreachable!("handled above, before configuration is loaded")
        }
        Some(Command::Migrate { from }) => {
            warn!("`iron migrate` is deprecated and will be removed in a future release");
            #[allow(deprecated)]
            migrate(&from, &settings).await?;
        }
        None => {
            // No subcommand provided - show help
            eprintln!("Error: No subcommand provided");
            eprintln!();
            eprintln!("To start the daemon, use: iron serve");
            eprintln!("For help, use: iron --help");
            std::process::exit(1);
        }
    }

    Ok(())
}

/// Start the iron daemon (default behavior)
async fn start_daemon(settings: config::Settings) -> Result<()> {
    let log_level = settings.log_level.to_string();
    let dns_port = settings.dns_port;
    let dns_upstream = settings.dns_upstream;
    // Display banner
    print_banner(&log_level, dns_port, &dns_upstream);

    // Check if running as root (required for TUN device)
    #[cfg(unix)]
    check_root();

    // Initialize and start iron node with desktop platform defaults
    info!("Initializing iron node...");
    let mut node_config = iron_desktop::desktop_node_config().with_dns_port(dns_port);
    node_config.dns_upstream = dns_upstream;
    node_config.paths = iron_core::StatePaths::from_files(settings.key_file, settings.state_file);
    node_config.iroh = settings.iroh;
    let node = IronNode::new(node_config).await?;

    // Invalid state or identity must fail before changing the OS resolver.
    setup_dns_for_daemon(dns_port);

    // Get a reference to the registry for saving on shutdown
    let registry = node.registry().clone();
    let state = node.state().clone();

    // Display node information
    info!("Iron node initialized successfully");
    info!("");

    let endpoint_id = node.endpoint_id();
    info!("Node ID (hex):    {}", endpoint_id);

    // Convert to base32 for DNS
    let base32_id = iron_core::id::to_base32(&endpoint_id);

    info!("Node ID (base32): {}", base32_id);
    info!("DNS name:         {}.iron", base32_id);
    info!("");
    info!("DNS server will run on: 127.0.0.1:{}", dns_port);
    info!("");
    info!("To connect to this node, other peers need:");
    info!("  - DNS name: {}.iron", base32_id);
    info!(
        "  - Or use dig: dig @127.0.0.1 -p {} {}.iron AAAA",
        dns_port, base32_id
    );
    info!("");
    info!("Press Ctrl-C to shutdown gracefully");

    // Setup graceful shutdown on Ctrl-C and SIGTERM
    let shutdown_result = tokio::select! {
        result = node.start() => {
            result
        }
        _ = tokio::signal::ctrl_c() => {
            info!("");
            info!("Received shutdown signal (Ctrl-C)");
            info!("Shutting down gracefully...");
            Ok(())
        }
        _ = async {
            #[cfg(unix)]
            {
                match tokio::signal::unix::signal(
                    tokio::signal::unix::SignalKind::terminate()
                ) {
                    Ok(mut sigterm) => sigterm.recv().await,
                    Err(e) => {
                        eprintln!("Warning: Failed to setup SIGTERM handler: {}", e);
                        eprintln!("SIGTERM signals will not be handled (Ctrl-C will still work)");
                        std::future::pending::<Option<()>>().await
                    }
                }
            }
            #[cfg(not(unix))]
            {
                // On non-Unix, just wait forever (only Ctrl-C will trigger)
                std::future::pending::<()>().await
            }
        } => {
            info!("");
            info!("Received shutdown signal (SIGTERM)");
            info!("Shutting down gracefully...");
            Ok(())
        }
    };

    // Cleanup DNS configuration on shutdown
    info!("Cleaning up DNS configuration...");
    if let Err(e) = dns_config::cleanup_dns() {
        error!("Failed to cleanup DNS: {}", e);
        error!("You may need to manually cleanup with: sudo iron --cleanup-dns");
    } else {
        info!("✓ DNS configuration removed");
    }

    // Save known peers for next startup
    info!("Saving known peers...");
    if let Err(e) = state.save_peers(registry.endpoint_ids()).await {
        error!("Failed to save peers to state database: {}", e);
    } else {
        info!("✓ Peers cache saved");
    }

    match shutdown_result {
        Ok(_) => {
            info!("Iron node shutdown complete");
            Ok(())
        }
        Err(e) => {
            error!("Iron node encountered an error: {}", e);
            Err(e)
        }
    }
}

/// Points `.iron` resolution at our DNS port; failures are logged, not fatal.
fn setup_dns_for_daemon(dns_port: u16) {
    if dns_config::is_dns_configured(dns_port) {
        info!("DNS already configured for .iron domains");
        return;
    }
    info!("Setting up DNS for .iron domains...");
    match dns_config::setup_dns(dns_port) {
        Ok(()) => info!("✓ DNS configured successfully"),
        // Not fatal: peers stay reachable by address, and DNS can be
        // configured by hand.
        Err(e) => {
            warn!("⚠️  DNS setup failed: {e:#}");
            warn!("iron will keep running, but .iron names may not resolve.");
            warn!("You can clean up later with: sudo iron --cleanup-dns");
        }
    }
}

/// Initialize tracing with resolved iron settings, never implicit RUST_LOG.
fn init_tracing(log_level: &str) -> Result<()> {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    let env_filter = project_log_filter(log_level)?;

    // Initialize subscriber
    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_target(true)
                .with_thread_ids(false)
                .with_file(false)
                .with_line_number(false),
        )
        .init();

    Ok(())
}

fn project_log_filter(level: &str) -> Result<tracing_subscriber::EnvFilter> {
    Ok(tracing_subscriber::EnvFilter::try_new(format!(
        "iron={level},iron_core={level},iron_desktop={level}"
    ))?)
}

#[cfg(test)]
mod logging_tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::SubscriberExt;

    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().write(bytes)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn resolved_level_applies_to_all_project_crates() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let captured = output.clone();
        let subscriber = tracing_subscriber::registry()
            .with(project_log_filter("info").unwrap())
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(move || Capture(captured.clone())),
            );
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "iron", "binary message");
            tracing::error!(target: "iron_core::protocol", "core message");
            tracing::warn!(target: "iron_desktop::dns_config", "desktop message");
            tracing::debug!(target: "iron_core::protocol", "filtered debug");
        });
        let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        for expected in ["binary message", "core message", "desktop message"] {
            assert!(output.contains(expected), "{output}");
        }
        assert!(!output.contains("filtered debug"), "{output}");
    }
}

/// Print startup banner with basic information
fn print_banner(log_level: &str, dns_port: u16, dns_upstream: &[SocketAddr]) {
    println!("┌─────────────────────────────────────────┐");
    println!("│          iron - P2P Network             │");
    println!("│   Peer-to-peer connectivity via iroh    │");
    println!("└─────────────────────────────────────────┘");
    println!();
    println!("Configuration:");
    println!("  Log level:  {}", log_level);
    println!("  DNS port:   {}", dns_port);
    println!("  Tunnel DNS: [{}]:53", iron_core::dns::MAGIC_DNS_ADDR);
    if dns_upstream.is_empty() {
        println!("  Upstream:   none (non-.iron queries refused)");
    } else {
        let upstream: Vec<String> = dns_upstream.iter().map(ToString::to_string).collect();
        println!("  Upstream:   {}", upstream.join(", "));
    }
    println!();
}

/// Check if running with root privileges (required for TUN device creation)
#[cfg(unix)]
fn check_root() {
    if !nix::unistd::Uid::effective().is_root() {
        error!("ERROR: iron must be run as root (or with sudo)");
        error!("Reason: Creating TUN network devices requires elevated privileges");
        error!("");
        error!("Please run with: sudo iron");
        std::process::exit(1);
    }
}

#[cfg(not(unix))]
fn check_root() {
    // Windows/other platforms: just continue
    // TUN device creation will fail with appropriate error message if privileges are insufficient
}

// TODO: Remove this migration function and its subcommand in the next release.
#[deprecated(note = "remove after the next release")]
async fn migrate(source: &std::path::Path, settings: &config::Settings) -> Result<()> {
    let source_key = source.join("secret.key");
    let key = iron_core::keys::load_key(&source_key)
        .with_context(|| format!("invalid legacy key: {}", source_key.display()))?;
    let destination_key = &settings.key_file;
    let destination_state = &settings.state_file;
    let source_peers = source.join("known_peers.json");
    for destination in [destination_key, destination_state] {
        match std::fs::symlink_metadata(destination) {
            Ok(_) => anyhow::bail!("refusing migration: destination state already exists"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("checking migration destination"),
        }
    }
    let key_bytes = key.to_bytes();
    let peer_bytes = match std::fs::read(&source_peers) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("reading legacy known peers"),
    };
    let writer = iron_desktop::DesktopPrivateFileWriter;
    let peer_bytes = peer_bytes.as_deref().unwrap_or(b"[]");
    iron_core::AppState::decode_legacy_peers(peer_bytes).context("invalid legacy known peers")?;
    // Prepare and checkpoint a complete database before publishing either
    // destination. WAL/temporary files stay in the private staging directory.
    let staging = tempfile::tempdir()?;
    let staged_path = staging.path().join("state.db");
    let staged_state = iron_core::AppState::open(&staged_path, &writer).await?;
    staged_state.import_legacy_peers(peer_bytes).await?;
    staged_state.checkpoint().await?;
    drop(staged_state);
    let state_bytes = std::fs::read(staged_path)?;
    writer
        .create_new(destination_key, &key_bytes)
        .with_context(|| {
            format!(
                "failed to create destination key {}",
                destination_key.display()
            )
        })?;
    if let Err(error) = writer.create_new(destination_state, &state_bytes) {
        std::fs::remove_file(destination_key)
            .context("state migration failed and newly created key could not be removed")?;
        return Err(error).context("failed to create destination database; removed new key");
    }
    info!("Legacy state migrated. Original files were left unchanged.");
    Ok(())
}

#[cfg(test)]
#[allow(deprecated)]
mod migration_tests {
    use super::*;
    use iroh::SecretKey;

    fn settings(key_file: std::path::PathBuf, state_file: std::path::PathBuf) -> config::Settings {
        config::Settings {
            key_file,
            state_file,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn migration_copies_key_and_imports_peers_without_removing_sources() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("legacy");
        std::fs::create_dir(&source).unwrap();
        let key = SecretKey::generate().to_bytes();
        std::fs::write(source.join("secret.key"), key).unwrap();
        let peer = SecretKey::from_bytes(&key).public();
        std::fs::write(
            source.join("known_peers.json"),
            serde_json::to_vec(&vec![iron_core::id::to_base32(&peer)]).unwrap(),
        )
        .unwrap();
        let destination_key = dir.path().join("destination/secret.key");
        let destination_state = dir.path().join("destination/state.db");
        migrate(
            &source,
            &settings(destination_key.clone(), destination_state.clone()),
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(destination_key).unwrap(), key);
        let state =
            iron_core::AppState::open(&destination_state, &iron_desktop::DesktopPrivateFileWriter)
                .await
                .unwrap();
        assert_eq!(state.load_peers().await.unwrap(), vec![peer]);
        assert!(source.join("secret.key").exists());
        assert!(source.join("known_peers.json").exists());
    }

    #[tokio::test]
    async fn migration_refuses_existing_destination_state() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("legacy");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("secret.key"), SecretKey::generate().to_bytes()).unwrap();
        let destination_key = dir.path().join("state/secret.key");
        let destination_state = dir.path().join("state/state.db");
        std::fs::create_dir_all(destination_key.parent().unwrap()).unwrap();
        std::fs::write(&destination_key, b"keep").unwrap();
        assert!(
            migrate(
                &source,
                &settings(destination_key.clone(), destination_state)
            )
            .await
            .is_err()
        );
        assert_eq!(std::fs::read(destination_key).unwrap(), b"keep");
    }

    #[tokio::test]
    async fn invalid_legacy_peers_do_not_publish_key_or_database() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("legacy");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("secret.key"), SecretKey::generate().to_bytes()).unwrap();
        std::fs::write(source.join("known_peers.json"), b"not JSON").unwrap();
        let key = dir.path().join("destination/key");
        let database = dir.path().join("destination/state.db");
        assert!(
            migrate(&source, &settings(key.clone(), database.clone()))
                .await
                .is_err()
        );
        assert!(!key.exists());
        assert!(!database.exists());
        assert!(source.join("secret.key").exists());
    }

    #[tokio::test]
    async fn missing_legacy_peers_creates_an_empty_database() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("legacy");
        std::fs::create_dir(&source).unwrap();
        let bytes = SecretKey::generate().to_bytes();
        std::fs::write(source.join("secret.key"), bytes).unwrap();
        let key = dir.path().join("destination/key");
        let database = dir.path().join("destination/state.db");
        migrate(&source, &settings(key.clone(), database.clone()))
            .await
            .unwrap();
        assert_eq!(std::fs::read(key).unwrap(), bytes);
        let state = iron_core::AppState::open(&database, &iron_desktop::DesktopPrivateFileWriter)
            .await
            .unwrap();
        assert!(state.load_peers().await.unwrap().is_empty());
    }
}
