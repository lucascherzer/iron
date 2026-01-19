use anyhow::Result;
use clap::Parser;
use iron::IronNode;
use iron::dns_config;
use tracing::{error, info};

/// iron - P2P network interface based on iroh
///
/// Creates a TUN network interface and DNS resolver for .iron domains,
/// enabling peer-to-peer connectivity over iroh's encrypted QUIC protocol.
#[derive(Parser, Debug)]
#[command(name = "iron")]
#[command(version, about, long_about = None)]
struct Args {
    /// Set the log level (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info")]
    log_level: String,

    /// DNS server port (default: 5333)
    #[arg(long, default_value = "5333")]
    dns_port: u16,

    /// Setup DNS configuration for .iron domains (one-time setup)
    #[arg(long)]
    setup_dns: bool,

    /// Remove DNS configuration for .iron domains
    #[arg(long)]
    cleanup_dns: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments
    let args = Args::parse();

    // Initialize tracing subscriber
    init_tracing(&args.log_level)?;

    // Handle DNS setup/cleanup commands
    if args.setup_dns {
        return dns_config::setup_dns();
    }

    if args.cleanup_dns {
        return dns_config::cleanup_dns();
    }

    // Display banner
    print_banner(&args);

    // Check if running as root (required for TUN device)
    #[cfg(unix)]
    check_root();

    // Check if DNS is configured, offer to set it up if not
    check_and_setup_dns_if_needed()?;

    // Initialize and start iron node
    info!("Initializing iron node...");
    let node = IronNode::new().await?;

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
    info!("DNS server will run on: 127.0.0.1:{}", args.dns_port);
    info!("");
    info!("To connect to this node, other peers need:");
    info!("  - DNS name: {}.iron", base32_id);
    info!(
        "  - Or use dig: dig @127.0.0.1 -p {} {}.iron AAAA",
        args.dns_port, base32_id
    );
    info!("");
    info!("Press Ctrl-C to shutdown gracefully");

    // Setup graceful shutdown on Ctrl-C
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
    };

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

/// Check if DNS is configured and offer to set it up if not
fn check_and_setup_dns_if_needed() -> Result<()> {
    if dns_config::is_dns_configured() {
        return Ok(());
    }

    // DNS not configured - offer to set it up
    println!("\n⚠️  DNS not configured for .iron domains");
    println!("\nWithout DNS configuration, you must use IP addresses directly.");
    println!("Configure DNS to use domain names like: <peer-id>.iron\n");

    match dns_config::detect_platform() {
        dns_config::Platform::MacOS | dns_config::Platform::LinuxSystemd => {
            println!("Setting up DNS automatically...");
            println!("(This will only affect .iron domains, all other DNS stays the same)\n");

            match dns_config::setup_dns() {
                Ok(_) => {
                    println!("\nContinuing with iron startup...\n");
                    Ok(())
                }
                Err(e) => {
                    error!("Failed to setup DNS: {}", e);
                    println!("\n⚠️  DNS setup failed, but iron will continue running.");
                    println!("You can setup DNS manually later with: sudo iron --setup-dns\n");
                    Ok(())
                }
            }
        }
        dns_config::Platform::LinuxOther => {
            println!("Automatic DNS setup not available for your system.");
            println!("See doc/dns-setup.md for manual configuration.\n");
            println!("Continuing with iron startup...\n");
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
fn print_banner(args: &Args) {
    println!("┌─────────────────────────────────────────┐");
    println!("│          iron - P2P Network             │");
    println!("│   Peer-to-peer connectivity via iroh    │");
    println!("└─────────────────────────────────────────┘");
    println!();
    println!("Configuration:");
    println!("  Log level:  {}", args.log_level);
    println!("  DNS port:   {}", args.dns_port);
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
