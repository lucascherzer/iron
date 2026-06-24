use anyhow::Result;
use clap::{Parser, Subcommand};
use iron::IronNode;
use iron::dns_config;
use iron::node::IronNodeConfig;
use tracing::{error, info};

mod commands;

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

    /// Set the log level (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info", global = true)]
    log_level: String,

    /// DNS server port (default: 5333)
    #[arg(long, default_value = "5333", global = true)]
    dns_port: u16,

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
}

#[derive(Subcommand, Debug)]
enum KeyCommand {
    /// Show key information
    Info {
        /// Path to key file
        #[arg(long)]
        path: Option<String>,
    },

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
    Validate {
        /// Path to key file
        #[arg(long)]
        path: Option<String>,
    },

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

    // Initialize tracing subscriber (skip for some quiet commands)
    let skip_tracing = matches!(
        cli.command,
        Some(Command::Self_ { exists: true, .. })
            | Some(Command::Convert { .. })
            | Some(Command::Key {
                command: KeyCommand::Info { .. }
            })
    );

    if !skip_tracing {
        init_tracing(&cli.log_level)?;
    }

    // Handle DNS cleanup flag (for manual cleanup)
    if cli.cleanup_dns {
        return dns_config::cleanup_dns();
    }

    // Handle subcommands
    match cli.command {
        Some(Command::Serve) => {
            // Start daemon
            start_daemon(cli.log_level, cli.dns_port).await?;
        }
        Some(Command::Convert { value, to }) => {
            commands::convert::run(value, to)?;
        }
        Some(Command::Self_ {
            format,
            hex,
            base32,
            domain,
            ipv6,
            exists,
        }) => {
            commands::self_::run(format, hex, base32, domain, ipv6, exists)?;
        }
        Some(Command::Vanity {
            prefix,
            threads,
            max_attempts,
            save,
            output,
            quiet,
        }) => {
            commands::vanity::run(prefix, threads, max_attempts, save, output, quiet)?;
        }
        Some(Command::Key { command }) => match command {
            KeyCommand::Info { path } => {
                commands::key::info(path)?;
            }
            KeyCommand::Export { format, output } => {
                commands::key::export(format, output)?;
            }
            KeyCommand::Import { file, save } => {
                commands::key::import(file, save)?;
            }
            KeyCommand::Generate { save, force } => {
                commands::key::generate(save, force)?;
            }
            KeyCommand::Validate { path } => {
                commands::key::validate(path)?;
            }
            KeyCommand::Reset { confirm } => {
                commands::key::reset(confirm)?;
            }
        },
        Some(Command::Resolve {
            domain,
            server,
            timeout,
            format,
        }) => {
            commands::resolve::run(domain, server, timeout, format).await?;
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
async fn start_daemon(log_level: String, dns_port: u16) -> Result<()> {
    // Display banner
    print_banner(&log_level, dns_port);

    // Check if running as root (required for TUN device)
    #[cfg(unix)]
    check_root();

    // Fix key directory ownership if needed (before dropping privileges)
    #[cfg(unix)]
    fix_key_directory_ownership()?;

    // Setup DNS configuration automatically
    setup_dns_for_daemon()?;

    // Initialize and start iron node
    info!("Initializing iron node...");
    let config = IronNodeConfig::from_env();
    let has_custom_config = config.relay_url.is_some() || !config.peers.is_empty();
    let node = if has_custom_config {
        info!("Using custom relay/peer configuration from environment");
        IronNode::with_config(&config).await?
    } else {
        IronNode::new().await?
    };

    // Get a reference to the registry for saving on shutdown
    let registry = node.registry().clone();

    // Display node information
    info!("Iron node initialized successfully");
    info!("");

    let endpoint_id = node.endpoint_id();
    info!("Node ID (hex):    {}", endpoint_id);

    // Convert to base32 for DNS
    let endpoint_bytes = endpoint_id.as_bytes();
    let base32_id = data_encoding::BASE32_NOPAD
        .encode(endpoint_bytes)
        .to_lowercase();

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
    if let Err(e) = registry.save_peers() {
        error!("Failed to save peers cache: {}", e);
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

/// Setup DNS configuration for the daemon
/// Auto-configures DNS on supported platforms
fn setup_dns_for_daemon() -> Result<()> {
    if dns_config::is_dns_configured() {
        info!("DNS already configured for .iron domains");
        return Ok(());
    }

    // DNS not configured - set it up automatically
    info!("Setting up DNS for .iron domains...");

    match dns_config::detect_platform() {
        dns_config::Platform::MacOS | dns_config::Platform::LinuxSystemd => {
            match dns_config::setup_dns() {
                Ok(_) => {
                    info!("✓ DNS configured successfully");
                    Ok(())
                }
                Err(e) => {
                    error!("Failed to setup DNS: {}", e);
                    info!("⚠️  DNS setup failed, but iron will continue running.");
                    info!("You can manually cleanup later with: sudo iron --cleanup-dns");
                    Ok(())
                }
            }
        }
        dns_config::Platform::LinuxOther => {
            info!("⚠️  Automatic DNS setup not available for your system.");
            info!("Please configure DNS manually to resolve .iron domains");
            Ok(())
        }
    }
}

/// Initialize tracing subscriber with the specified log level
fn init_tracing(log_level: &str) -> Result<()> {
    use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

    // Build the env filter
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(format!("iron={}", log_level)))?;

    // Initialize subscriber
    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_thread_ids(false)
                .with_file(false)
                .with_line_number(false),
        )
        .init();

    Ok(())
}

/// Print startup banner with basic information
fn print_banner(log_level: &str, dns_port: u16) {
    println!("┌─────────────────────────────────────────┐");
    println!("│          iron - P2P Network             │");
    println!("│   Peer-to-peer connectivity via iroh    │");
    println!("└─────────────────────────────────────────┘");
    println!();
    println!("Configuration:");
    println!("  Log level:  {}", log_level);
    println!("  DNS port:   {}", dns_port);
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

/// Fix key directory ownership if it's owned by root
/// This happens when the daemon creates the directory as root
#[cfg(unix)]
fn fix_key_directory_ownership() -> Result<()> {
    use anyhow::Context;
    use nix::unistd::User;
    use std::env;
    use std::fs;
    use std::os::unix::fs::MetadataExt;

    let key_path = iron::keys::key_path();
    let config_dir = key_path
        .parent()
        .context("Cannot determine config directory")?;

    // Get the original user (before sudo)
    let original_user = env::var("SUDO_USER")
        .ok()
        .and_then(|username| User::from_name(&username).ok().flatten());

    if let Some(user) = original_user {
        let target_uid = user.uid;
        let target_gid = user.gid;

        // Create config directory if it doesn't exist
        if !config_dir.exists() {
            info!("Creating config directory: {}", config_dir.display());
            fs::create_dir_all(config_dir)?;
        }

        // Check if directory is owned by root
        if let Ok(metadata) = fs::metadata(config_dir) {
            let dir_uid = metadata.uid();

            if dir_uid == 0 {
                // Directory is owned by root, fix it
                info!(
                    "Fixing ownership of {} (was root, setting to {} uid={})",
                    config_dir.display(),
                    user.name,
                    user.uid
                );

                // Change ownership using chown command
                use std::process::Command;

                let status = Command::new("chown")
                    .arg("-R")
                    .arg(format!("{}:{}", target_uid, target_gid))
                    .arg(config_dir)
                    .status()?;

                if !status.success() {
                    return Err(anyhow::anyhow!(
                        "Failed to change ownership of config directory"
                    ));
                }

                info!("✓ Key directory ownership fixed");
            }
        }
    }

    Ok(())
}
