# Proposal: Peer Discovery Web UI

To get my friends online, I'd like to have a very simple web ui that lets anyone
visiting add .iron domains and a short description per domain.

The server should have a simple persistent backend that stores the input domains,
attempts to ping them and shows a list of all alongside their status in color
(online, offline, group by status, online > offline).

This should be done in a separate repo (to not clutter this one), with a
reproducible build based on a nix flake (crane). Deploy natively (not containerized)
since it needs direct access to iron's TUN device to resolve .iron domains.

I want this done as easy as it gets, simple dependencies, no fancy stuff.

Style wise, simple is good, I like the nord theme, so lets go with that.

Requirements:
- Entirely Rust (backend + HTML templates)
- Minimal JavaScript on frontend if necessary (prefer progressive enhancement)
- Native deployment (containers can't access TUN device)

---

# Implementation Guide

## Repository Structure

Create a new repository at the same level as iron:

```
iron/                    (existing repo)
iron-discovery/          (new repo)
├── src/
│   ├── main.rs          # Server entry point
│   ├── handlers.rs      # HTTP request handlers
│   ├── models.rs        # Data structures (Peer, Status)
│   ├── storage.rs       # Persistent storage (JSON or SQLite)
│   ├── pinger.rs        # Background task for pinging peers
│   └── templates.rs     # Inline HTML templates with Nord theme
├── Cargo.toml
├── flake.nix            # Nix flake with crane build
└── README.md
```

## Architecture Overview

### Components

1. **Web Server** (axum)
   - GET `/` - Show peer list (HTML page)
   - POST `/peers` - Add new peer (form submission)
   - DELETE `/peers/:id` - Remove peer (optional)
   - GET `/health` - Health check endpoint

2. **Storage Layer**
   - Simple JSON file persistence (`peers.json`)
   - Struct: `{ id: String, domain: String, description: String, status: Status, last_checked: DateTime }`
   - Status enum: `Online | Offline | Unknown`

3. **Background Pinger**
   - Tokio task that runs every 30-60 seconds
   - Attempts ICMP ping6 to each domain
   - Updates status in storage
   - Uses system `ping6` command or raw sockets

4. **Templates**
   - Server-side rendered HTML using `askama` or inline strings
   - Nord color scheme CSS inline or in `<style>` tag
   - Minimal/zero JavaScript (form submission via standard HTML)
   - Auto-refresh via `<meta http-equiv="refresh">` or SSE (optional)

### Technology Stack

**Required Dependencies:**
```toml
[dependencies]
axum = "0.7"                    # Web framework
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"                # JSON storage
uuid = { version = "1", features = ["v4"] }
tracing = "0.1"
tracing-subscriber = "0.3"
tower-http = { version = "0.5", features = ["fs", "trace"] }

# HTML templating (choose one):
askama = "0.12"                 # Compile-time templates (recommended)
# OR
# tera = "1"                    # Runtime templates (alternative)

# For pinging:
# Option 1: Use system ping6 command
tokio-process = "0.2"
# Option 2: Raw ICMP (more complex, requires root)
# pnet = "0.35"
```

**Nord Color Scheme:**
```css
/* Nord Polar Night */
--nord0: #2e3440;   /* background */
--nord1: #3b4252;   /* darker background */
--nord2: #434c5e;   /* selection bg */
--nord3: #4c566a;   /* comments/disabled */

/* Nord Snow Storm */
--nord4: #d8dee9;   /* text */
--nord5: #e5e9f0;   /* brighter text */
--nord6: #eceff4;   /* brightest text */

/* Nord Frost */
--nord7: #8fbcbb;   /* accent cyan */
--nord8: #88c0d0;   /* bright cyan */
--nord9: #81a1c1;   /* blue */
--nord10: #5e81ac;  /* dark blue */

/* Nord Aurora */
--nord11: #bf616a;  /* red (offline) */
--nord14: #a3be8c;  /* green (online) */
--nord13: #ebcb8b;  /* yellow (unknown) */
```

## Deployment Strategy: Native Only

### Why No Container?

**The TUN Device Problem:**

Containers cannot easily access iron's TUN device:
- `.iron` domains are resolved by iron's DNS server (127.0.0.1:5333)
- Iron routes traffic through a TUN device (`utun` on macOS, `iron0` on Linux)
- Containers are network-isolated by default
- Sharing TUN devices into containers requires complex setups (`--network host`, mounting `/etc/resolver`, etc.)

**Solution: Native Deployment**

Deploy iron-discovery natively on the same machine as iron:
- ✅ Direct access to iron's TUN device
- ✅ Can resolve `.iron` domains immediately
- ✅ Simple deployment (just run binary)
- ✅ No container overhead
- ✅ Nix flake provides reproducible builds (same benefits as containers)

**Setup:**
```bash
# Build with Nix
cd iron-discovery
nix build

# Run directly (iron must be running)
./result/bin/iron-discovery

# Or install as systemd service
sudo cp result/bin/iron-discovery /usr/local/bin/
sudo systemctl enable --now iron-discovery
```

The Nix flake provides the same benefits as containerization (reproducible builds, dependency management) without the networking complexity.

## Implementation Steps

### 1. Project Setup

```bash
# Create repository
mkdir iron-discovery
cd iron-discovery
git init

# Create basic Cargo.toml
cat > Cargo.toml << 'EOF'
[package]
name = "iron-discovery"
version = "0.1.0"
edition = "2024"

[dependencies]
axum = "0.7"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4", "serde"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tower-http = { version = "0.5", features = ["trace"] }
askama = "0.12"
chrono = { version = "0.4", features = ["serde"] }
EOF
```

### 2. Data Models (`src/models.rs`)

```rust
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    pub id: Uuid,
    pub domain: String,        // e.g., "abc123...xyz.iron"
    pub description: String,   // User-provided description
    pub status: Status,
    pub last_checked: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Status {
    Online,
    Offline,
    Unknown,   // Not yet checked
}

impl Peer {
    pub fn new(domain: String, description: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            domain,
            description,
            status: Status::Unknown,
            last_checked: None,
        }
    }
}
```

### 3. Storage Layer (`src/storage.rs`)

Simple JSON file storage:

```rust
use crate::models::Peer;
use anyhow::{Context, Result};
use std::path::Path;
use tokio::sync::RwLock;
use uuid::Uuid;

pub struct Storage {
    peers: RwLock<Vec<Peer>>,
    path: String,
}

impl Storage {
    pub async fn new(path: impl Into<String>) -> Result<Self> {
        let path = path.into();
        let peers = if Path::new(&path).exists() {
            let data = tokio::fs::read_to_string(&path).await?;
            serde_json::from_str(&data).context("Failed to parse peers.json")?
        } else {
            Vec::new()
        };
        
        Ok(Self {
            peers: RwLock::new(peers),
            path,
        })
    }

    pub async fn add_peer(&self, peer: Peer) -> Result<()> {
        let mut peers = self.peers.write().await;
        peers.push(peer);
        self.save(&peers).await
    }

    pub async fn get_peers(&self) -> Vec<Peer> {
        self.peers.read().await.clone()
    }

    pub async fn update_peer_status(&self, id: Uuid, status: Status, checked: DateTime<Utc>) -> Result<()> {
        let mut peers = self.peers.write().await;
        if let Some(peer) = peers.iter_mut().find(|p| p.id == id) {
            peer.status = status;
            peer.last_checked = Some(checked);
        }
        self.save(&peers).await
    }

    async fn save(&self, peers: &[Peer]) -> Result<()> {
        let json = serde_json::to_string_pretty(peers)?;
        tokio::fs::write(&self.path, json).await?;
        Ok(())
    }
}
```

### 4. Pinger (`src/pinger.rs`)

Background task that pings domains:

```rust
use crate::models::Status;
use crate::storage::Storage;
use std::sync::Arc;
use tokio::process::Command;
use tracing::{debug, warn};

pub async fn start_pinger(storage: Arc<Storage>, interval_secs: u64) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(
            tokio::time::Duration::from_secs(interval_secs)
        );
        
        loop {
            interval.tick().await;
            ping_all_peers(&storage).await;
        }
    });
}

async fn ping_all_peers(storage: &Storage) {
    let peers = storage.get_peers().await;
    
    for peer in peers {
        let status = ping_domain(&peer.domain).await;
        let now = chrono::Utc::now();
        
        if let Err(e) = storage.update_peer_status(peer.id, status, now).await {
            warn!("Failed to update peer {}: {}", peer.domain, e);
        }
        
        debug!("Pinged {} -> {:?}", peer.domain, status);
    }
}

async fn ping_domain(domain: &str) -> Status {
    // Use system ping6 command with IPv6 flag
    // ping6 -c 1 -W 2 <domain>
    // -c 1: send 1 packet
    // -W 2: wait 2 seconds max
    
    match Command::new("ping6")
        .args(["-c", "1", "-W", "2", domain])
        .output()
        .await
    {
        Ok(output) => {
            if output.status.success() {
                Status::Online
            } else {
                Status::Offline
            }
        }
        Err(e) => {
            warn!("Failed to ping {}: {}", domain, e);
            Status::Offline
        }
    }
}
```

**Note on ping6:**
- macOS: `ping6` is available by default
- Linux: `ping6` is available in `iputils-ping` package
- Alternative: use `ping -6` on some systems
- For production, may want to check which command is available at startup

### 5. Templates (`src/templates.rs`)

Using `askama` for compile-time templates:

```rust
use askama::Template;
use crate::models::{Peer, Status};

#[derive(Template)]
#[template(source = r#"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Iron Discovery</title>
    <meta http-equiv="refresh" content="30">
    <style>
        :root {
            --nord0: #2e3440;
            --nord1: #3b4252;
            --nord2: #434c5e;
            --nord3: #4c566a;
            --nord4: #d8dee9;
            --nord6: #eceff4;
            --nord7: #8fbcbb;
            --nord11: #bf616a;
            --nord14: #a3be8c;
            --nord13: #ebcb8b;
        }
        
        * { margin: 0; padding: 0; box-sizing: border-box; }
        
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: var(--nord0);
            color: var(--nord4);
            padding: 2rem;
            line-height: 1.6;
        }
        
        .container { max-width: 900px; margin: 0 auto; }
        
        h1 {
            color: var(--nord6);
            margin-bottom: 0.5rem;
            font-size: 2rem;
        }
        
        .subtitle {
            color: var(--nord3);
            margin-bottom: 2rem;
            font-size: 0.9rem;
        }
        
        .add-form {
            background: var(--nord1);
            padding: 1.5rem;
            border-radius: 6px;
            margin-bottom: 2rem;
        }
        
        .form-group { margin-bottom: 1rem; }
        
        label {
            display: block;
            margin-bottom: 0.5rem;
            color: var(--nord6);
            font-weight: 500;
        }
        
        input[type="text"],
        input[type="textarea"] {
            width: 100%;
            padding: 0.75rem;
            background: var(--nord0);
            border: 1px solid var(--nord3);
            border-radius: 4px;
            color: var(--nord4);
            font-size: 1rem;
        }
        
        input:focus {
            outline: none;
            border-color: var(--nord7);
        }
        
        button {
            background: var(--nord7);
            color: var(--nord0);
            padding: 0.75rem 1.5rem;
            border: none;
            border-radius: 4px;
            font-size: 1rem;
            font-weight: 600;
            cursor: pointer;
            transition: background 0.2s;
        }
        
        button:hover { background: var(--nord8); }
        
        .peer-list { display: flex; flex-direction: column; gap: 1rem; }
        
        .peer {
            background: var(--nord1);
            padding: 1.5rem;
            border-radius: 6px;
            border-left: 4px solid var(--nord3);
        }
        
        .peer.online { border-left-color: var(--nord14); }
        .peer.offline { border-left-color: var(--nord11); }
        .peer.unknown { border-left-color: var(--nord13); }
        
        .peer-header {
            display: flex;
            justify-content: space-between;
            align-items: center;
            margin-bottom: 0.5rem;
        }
        
        .peer-domain {
            font-family: 'Courier New', monospace;
            color: var(--nord6);
            font-size: 1.1rem;
            font-weight: 600;
        }
        
        .status {
            padding: 0.25rem 0.75rem;
            border-radius: 4px;
            font-size: 0.85rem;
            font-weight: 600;
            text-transform: uppercase;
        }
        
        .status.online {
            background: var(--nord14);
            color: var(--nord0);
        }
        
        .status.offline {
            background: var(--nord11);
            color: var(--nord0);
        }
        
        .status.unknown {
            background: var(--nord13);
            color: var(--nord0);
        }
        
        .peer-description {
            color: var(--nord4);
            margin-bottom: 0.5rem;
        }
        
        .peer-meta {
            color: var(--nord3);
            font-size: 0.85rem;
        }
        
        .empty-state {
            text-align: center;
            padding: 3rem;
            color: var(--nord3);
        }
        
        .section-title {
            color: var(--nord6);
            font-size: 1.2rem;
            margin-bottom: 1rem;
            margin-top: 2rem;
        }
    </style>
</head>
<body>
    <div class="container">
        <h1>🔗 Iron Discovery</h1>
        <p class="subtitle">Peer-to-peer network discovery for .iron domains</p>
        
        <form class="add-form" method="POST" action="/peers">
            <h2 class="section-title">Add Your Node</h2>
            <div class="form-group">
                <label for="domain">Domain (.iron)</label>
                <input type="text" id="domain" name="domain" 
                       placeholder="abc123...xyz.iron" required 
                       pattern="[a-z0-9]+\.iron">
            </div>
            <div class="form-group">
                <label for="description">Description</label>
                <input type="text" id="description" name="description" 
                       placeholder="My awesome node" required maxlength="200">
            </div>
            <button type="submit">Add Peer</button>
        </form>
        
        {% if online_peers.len() > 0 %}
        <h2 class="section-title">🟢 Online Peers ({{ online_peers.len() }})</h2>
        <div class="peer-list">
            {% for peer in online_peers %}
            <div class="peer online">
                <div class="peer-header">
                    <div class="peer-domain">{{ peer.domain }}</div>
                    <div class="status online">Online</div>
                </div>
                <div class="peer-description">{{ peer.description }}</div>
                {% if peer.last_checked.is_some() %}
                <div class="peer-meta">Last checked: {{ peer.last_checked.unwrap().format("%Y-%m-%d %H:%M:%S UTC") }}</div>
                {% endif %}
            </div>
            {% endfor %}
        </div>
        {% endif %}
        
        {% if offline_peers.len() > 0 %}
        <h2 class="section-title">🔴 Offline Peers ({{ offline_peers.len() }})</h2>
        <div class="peer-list">
            {% for peer in offline_peers %}
            <div class="peer offline">
                <div class="peer-header">
                    <div class="peer-domain">{{ peer.domain }}</div>
                    <div class="status offline">Offline</div>
                </div>
                <div class="peer-description">{{ peer.description }}</div>
                {% if peer.last_checked.is_some() %}
                <div class="peer-meta">Last checked: {{ peer.last_checked.unwrap().format("%Y-%m-%d %H:%M:%S UTC") }}</div>
                {% endif %}
            </div>
            {% endfor %}
        </div>
        {% endif %}
        
        {% if unknown_peers.len() > 0 %}
        <h2 class="section-title">⚪ Unknown Status ({{ unknown_peers.len() }})</h2>
        <div class="peer-list">
            {% for peer in unknown_peers %}
            <div class="peer unknown">
                <div class="peer-header">
                    <div class="peer-domain">{{ peer.domain }}</div>
                    <div class="status unknown">Unknown</div>
                </div>
                <div class="peer-description">{{ peer.description }}</div>
            </div>
            {% endfor %}
        </div>
        {% endif %}
        
        {% if online_peers.len() == 0 && offline_peers.len() == 0 && unknown_peers.len() == 0 %}
        <div class="empty-state">
            <p>No peers yet. Add your first node above!</p>
        </div>
        {% endif %}
    </div>
</body>
</html>
"#, ext = "html")]
pub struct IndexTemplate {
    pub online_peers: Vec<Peer>,
    pub offline_peers: Vec<Peer>,
    pub unknown_peers: Vec<Peer>,
}
```

### 6. HTTP Handlers (`src/handlers.rs`)

```rust
use crate::models::{Peer, Status};
use crate::storage::Storage;
use crate::templates::IndexTemplate;
use axum::{
    extract::State,
    response::{Html, IntoResponse, Redirect},
    Form,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct AddPeerForm {
    domain: String,
    description: String,
}

pub async fn index(State(storage): State<Arc<Storage>>) -> impl IntoResponse {
    let peers = storage.get_peers().await;
    
    // Group by status
    let mut online_peers = Vec::new();
    let mut offline_peers = Vec::new();
    let mut unknown_peers = Vec::new();
    
    for peer in peers {
        match peer.status {
            Status::Online => online_peers.push(peer),
            Status::Offline => offline_peers.push(peer),
            Status::Unknown => unknown_peers.push(peer),
        }
    }
    
    let template = IndexTemplate {
        online_peers,
        offline_peers,
        unknown_peers,
    };
    
    Html(template.render().unwrap())
}

pub async fn add_peer(
    State(storage): State<Arc<Storage>>,
    Form(form): Form<AddPeerForm>,
) -> impl IntoResponse {
    // Validate domain format
    if !form.domain.ends_with(".iron") {
        // In production, return error page
        return Redirect::to("/");
    }
    
    let peer = Peer::new(form.domain, form.description);
    
    if let Err(e) = storage.add_peer(peer).await {
        tracing::error!("Failed to add peer: {}", e);
    }
    
    Redirect::to("/")
}

pub async fn health() -> &'static str {
    "ok"
}
```

### 7. Main Entry Point (`src/main.rs`)

```rust
mod handlers;
mod models;
mod pinger;
mod storage;
mod templates;

use anyhow::Result;
use axum::{routing::get, routing::post, Router};
use std::sync::Arc;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "iron_discovery=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Initialize storage
    let storage = Arc::new(storage::Storage::new("peers.json").await?);

    // Start background pinger (ping every 30 seconds)
    pinger::start_pinger(Arc::clone(&storage), 30).await;

    // Build router
    let app = Router::new()
        .route("/", get(handlers::index))
        .route("/peers", post(handlers::add_peer))
        .route("/health", get(handlers::health))
        .layer(TraceLayer::new_for_http())
        .with_state(storage);

    // Start server
    let bind_addr = "0.0.0.0:8080";
    tracing::info!("Starting iron-discovery on {}", bind_addr);
    
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
```

### 8. Nix Flake (`flake.nix`)

Based on the working iron flake, builds a native binary:

```nix
{
  description = "iron-discovery - Web UI for discovering .iron peers";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, crane, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        craneLib = crane.mkLib pkgs;

        commonArgs = {
          src = craneLib.cleanCargoSource ./.;
          strictDeps = true;

          buildInputs = [ ]
            ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
              pkgs.libiconv
            ];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        iron-discovery = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;

          meta = with pkgs.lib; {
            description = "Web UI for discovering .iron peers";
            homepage = "https://github.com/yourusername/iron-discovery";
            license = with licenses; [ mit asl20 ];
            mainProgram = "iron-discovery";
          };
        });
      in
      {
        packages = {
          default = iron-discovery;
          inherit iron-discovery;
        };

        apps.default = flake-utils.lib.mkApp {
          drv = iron-discovery;
        };

        checks = {
          inherit iron-discovery;
          
          iron-discovery-clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          });

          iron-discovery-fmt = craneLib.cargoFmt {
            src = ./.;
          };
        };

        devShells.default = craneLib.devShell {
          checks = self.checks.${system};

          packages = [
            pkgs.rust-analyzer
          ];

          RUST_LOG = "iron_discovery=debug";
        };
      }
    );
}
```

**Note:** The flake builds a native binary. No container image is needed since
iron-discovery must run natively to access the TUN device.

### 9. Systemd Service (Linux)

Create `iron-discovery.service`:

```ini
[Unit]
Description=Iron Discovery Web UI
Documentation=https://github.com/yourusername/iron-discovery
After=network.target iron.service
Requires=iron.service

[Service]
Type=simple
ExecStart=/usr/local/bin/iron-discovery
Restart=on-failure
RestartSec=5s

# Security hardening
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/iron-discovery

WorkingDirectory=/var/lib/iron-discovery

# Environment
Environment="RUST_LOG=iron_discovery=info"

[Install]
WantedBy=multi-user.target
```

### 10. README.md

```markdown
# iron-discovery

Web UI for discovering and monitoring .iron peers.

## Prerequisites

- `iron` must be running on the same machine
- Rust 1.70+ (for development)
- Nix with flakes (for reproducible builds)

## Building

### With Nix (Recommended)

\`\`\`bash
nix build
./result/bin/iron-discovery
\`\`\`

### With Cargo

\`\`\`bash
cargo build --release
./target/release/iron-discovery
\`\`\`

## Running

\`\`\`bash
# Ensure iron is running
sudo iron serve

# In another terminal
./result/bin/iron-discovery
# or
cargo run --release
\`\`\`

Visit http://localhost:8080

## Configuration

- **Storage:** Peers stored in `peers.json` in working directory
- **Port:** 8080 (hardcoded, can be made configurable)
- **Ping interval:** 30 seconds
- **Refresh interval:** 30 seconds (auto-refresh in browser)

## Deployment

Since iron-discovery needs to resolve .iron domains, it must run natively on the
same machine as iron with access to the TUN device. No container is used.

### Production

1. Build with Nix: `nix build`
2. Copy binary: `sudo cp result/bin/iron-discovery /usr/local/bin/`
3. Install systemd service (see iron-discovery.service)
4. Start: `sudo systemctl enable --now iron-discovery`

### Development

\`\`\`bash
nix develop
cargo run
\`\`\`

## Features

- ✅ Add .iron domains with descriptions
- ✅ Automatic ping checks (every 30s)
- ✅ Status display (online/offline/unknown)
- ✅ Grouped by status (online first)
- ✅ Nord color theme
- ✅ Auto-refresh (every 30s)
- ✅ Zero JavaScript required
- ✅ Persistent storage (JSON file)

## Architecture

- **Framework:** axum (async Rust web framework)
- **Templates:** askama (compile-time HTML templates)
- **Storage:** JSON file (simple and portable)
- **Pinging:** System ping6 command
- **Styling:** Inline CSS with Nord theme

## License

MIT OR Apache-2.0
\`\`\`

## Testing Strategy

### Unit Tests

1. **Storage tests:** Add/retrieve/update peers
2. **Model tests:** Peer creation, status changes
3. **Pinger tests:** Mock ping results, status updates

### Integration Tests

1. Start server
2. POST new peer
3. Verify peer appears in GET response
4. Wait for ping cycle
5. Verify status updated

### Manual Testing with Iron

**Setup:**
```bash
# Terminal 1: Start iron
cd ../iron
sudo nix run

# Terminal 2: Start discovery UI
cd ../iron-discovery
nix run

# Terminal 3: Access from another iron node
# Add your first node's .iron domain via the web UI
# Watch status update in real-time
```

## Progressive Enhancement

The app works without JavaScript but can be enhanced:

**Optional improvements (with minimal JS):**
- Live status updates (SSE or WebSocket)
- Delete peer button (send DELETE request)
- Client-side domain validation
- Sort/filter controls

Keep JavaScript minimal - the core experience should work without it.

## Context for Implementation

**Key information you'll need:**

1. **Iron source location:** `../iron` (relative to iron-discovery)
   - Reference for .iron domain format
   - DNS resolution behavior
   - IPv6 address scheme

2. **.iron domain format:**
   - Base32-encoded EndpointId (52 chars)
   - Example: `ot36ptgm67yp5vjt6b6dtz2l4ppejtggt5w3y64lqqrvztpl2wnq.iron`
   - Case-insensitive
   - No padding

3. **Testing connectivity:**
   - Iron must be running: `sudo iron serve`
   - Test DNS: `iron resolve <domain>.iron`
   - Test ping: `ping6 <domain>.iron`
   - Only works between different machines (no self-ping)

4. **Deployment considerations:**
   - Run on same machine as iron
   - No container needed (native is simpler)
   - Nix flake provides reproducible builds
   - Storage in working directory (make configurable for production)

## Implementation Order

1. ✅ Setup project structure (Cargo.toml, flake.nix)
2. ✅ Implement data models (Peer, Status)
3. ✅ Implement storage layer (JSON file)
4. ✅ Implement pinger (background task with ping6)
5. ✅ Create HTML template with Nord theme
6. ✅ Implement HTTP handlers (GET /, POST /peers)
7. ✅ Wire up main.rs with axum router
8. ✅ Test locally with iron running
9. ✅ Add systemd service file
10. ✅ Write README

## Known Limitations

- **No authentication:** Anyone can add peers (intended for friends)
- **No rate limiting:** Add if abuse becomes a problem
- **No peer deletion UI:** Can manually edit peers.json or add DELETE endpoint
- **No pagination:** Fine for <100 peers, add if needed
- **Ping6 dependency:** Requires iputils on Linux, built-in on macOS

## Future Enhancements (Optional)

- Peer deletion via UI
- Edit peer descriptions
- Ping history/uptime tracking
- RSS feed of online peers
- Basic authentication (password in env var)
- IPv6 address display alongside domain
- Last seen timestamp
- Configurable ping interval and port

---

**This implementation prioritizes simplicity and correctness over features.**
The goal is a working, maintainable tool that helps friends discover each
other on the iron network.
