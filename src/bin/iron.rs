use anyhow::Result;
use clap::Parser;
use iron::IronNode;
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
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments
    let args = Args::parse();

    // Initialize tracing subscriber
    init_tracing(&args.log_level)?;

    // Display banner
    print_banner(&args);

    // Check if running as root (required for TUN device)
    #[cfg(unix)]
    check_root();

    // Initialize and start iron node
    info!("Initializing iron node...");
    let node = IronNode::new().await?;

    // Display node information
    info!("Iron node initialized successfully");
    info!("Node ID: {}", node.endpoint_id());
    info!("DNS server will run on: 127.0.0.1:{}", args.dns_port);
    info!("");
    info!("To connect to this node, other peers need:");
    info!("  - Node ID: {}", node.endpoint_id());
    info!("  - Relay server or direct addresses");
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
