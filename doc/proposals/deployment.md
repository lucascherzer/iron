# Proposal: Deployment Strategy

**Status**: Draft  
**Created**: 2026-01-20  
**Goal**: Make iron deployment as simple as possible - ideally a single static binary

---

## Current State Analysis

### Binary Characteristics

**Good News**: We're already in excellent shape for easy deployment!

```
Size: 19 MB (release build, macOS ARM64)
Type: Single statically-linked executable
Dependencies: Only system libraries (no external runtime dependencies)
```

**macOS Dynamic Libraries** (all system-provided):
```
- SystemConfiguration.framework (network config detection)
- CoreFoundation.framework (system utilities)  
- libiconv.2.dylib (character encoding)
- libSystem.B.dylib (system calls, libc)
```

**Linux**: Would link against glibc and standard system libraries only.

### Runtime Behavior

**Files Created**:
1. **User key file**: `~/.config/iron/secret.key` (32 bytes, 0600 permissions)
   - Auto-generated on first run if missing
   - Can be pre-seeded or imported via CLI

2. **System DNS config** (requires root, optional):
   - macOS: `/etc/resolver/iron` (~100 bytes)
   - Linux systemd: `/etc/systemd/resolved.conf.d/iron.conf` (~200 bytes)
   - Auto-created on first `iron serve`
   - Auto-cleaned on graceful shutdown

**No Other Runtime Files**:
- ✅ No config files required
- ✅ No databases
- ✅ No templates or assets
- ✅ No plugins or modules
- ✅ Everything works from single binary

### Dependency Analysis

**Pure Rust Dependencies** (statically compiled in):
```
Core:
- iroh (P2P networking)
- tokio (async runtime)
- futures (async utilities)

Networking:
- tun (TUN device interface)
- hickory-{server,proto,client} (DNS)
- etherparse (packet parsing)

Utilities:
- clap (CLI parsing)
- anyhow/thiserror (error handling)
- tracing (logging)
- serde_json (JSON output)
- data-encoding, hex (encoding)
- dashmap (concurrent hashmap)
- rand (RNG)
```

**Zero C dependencies** (except standard libc/system frameworks)

---

## Deployment Complexity Score

**Current Rating: 🟢 EXCELLENT (1/10 difficulty)**

| Aspect | Status | Notes |
|--------|--------|-------|
| Binary dependencies | ✅ None | Only system libs |
| Runtime files | ✅ Minimal | Single config file auto-generated |
| Configuration | ✅ Optional | Works with defaults |
| Database/state | ✅ None | Stateless except for identity key |
| Installation | ✅ Copy binary | No install script needed |
| Uninstallation | ✅ Clean | Delete binary + `rm -rf ~/.config/iron` |

**Comparison to other tools**:
- ✅ Better than Docker (no daemon, no images)
- ✅ Better than Kubernetes (obviously)
- ✅ Same level as Tailscale binary (excellent)
- ✅ Better than most VPN clients (no kernel modules)

---

## Deployment Options

### Option 1: Single Static Binary (Current - Recommended)

**For friends without Nix**:

```bash
# Download and install
curl -L https://github.com/you/iron/releases/latest/download/iron-$(uname -s)-$(uname -m) -o iron
chmod +x iron
sudo mv iron /usr/local/bin/

# Run
sudo iron serve
```

**Pros**:
- Zero dependencies
- Works on any system with compatible libc
- 19 MB download (acceptable)
- No build tools needed

**Cons**:
- Need separate binaries per platform/arch
- macOS requires notarization for Gatekeeper

**Platforms to support**:
- ✅ macOS (x86_64, arm64)
- ✅ Linux x86_64 (glibc)
- ✅ Linux x86_64 (musl - fully static)
- ✅ Linux arm64 (Raspberry Pi, etc.)
- ⚠️ Windows (TUN support limited)

### Option 2: Nix Flake (For Power Users)

**Target UX**:
```bash
# Try without installing
nix run github:you/iron -- serve

# Install to profile
nix profile install github:you/iron

# Use in NixOS configuration
{
  services.iron = {
    enable = true;
    autoStart = true;
  };
}
```

**Flake should expose**:
1. `packages.default` - iron binary
2. `apps.default` - runnable iron
3. `checks.*` - all tests
4. `nixosModules.iron` - NixOS service module
5. `devShells.default` - development environment

**Benefits**:
- Declarative configuration
- Reproducible builds
- Automatic updates
- Integration with NixOS services
- Easy cross-compilation

### Option 3: Package Managers

**Homebrew** (macOS):
```bash
brew install iron
```

**apt/deb** (Debian/Ubuntu):
```bash
curl -fsSL https://your-domain.com/gpg | sudo apt-key add -
echo "deb https://your-domain.com/apt stable main" | sudo tee /etc/apt/sources.list.d/iron.list
sudo apt update && sudo apt install iron
```

**AUR** (Arch Linux):
```bash
yay -S iron-bin  # binary package
yay -S iron      # build from source
```

**Pros**: Familiar to users, automatic updates  
**Cons**: Maintenance burden, slow review processes

### Option 4: Container (Not Recommended)

**Why NOT to use containers for iron**:
- Requires privileged mode for TUN device
- Network namespace complications
- Larger download (base image + binary)
- Container runtime dependency
- Goes against iron's simplicity philosophy

**Only use case**: Testing in CI/CD

---

## Static Binary Strategy

### Building Fully Static Binaries

**Linux (musl libc)**:
```bash
# Add musl target
rustup target add x86_64-unknown-linux-musl

# Build fully static binary
cargo build --release --target x86_64-unknown-linux-musl

# Result: Zero dynamic dependencies
ldd target/x86_64-unknown-linux-musl/release/iron
# "not a dynamic executable"
```

**Benefits**:
- Works on ANY Linux distro (even Alpine, embedded systems)
- No glibc version issues
- Truly portable
- Can run in containers without base image

**Size impact**: +1-2 MB (acceptable tradeoff)

### Cross-Compilation

**Using cross** (recommended):
```bash
cargo install cross

# Build for all targets
cross build --release --target x86_64-unknown-linux-musl
cross build --release --target aarch64-unknown-linux-musl
cross build --release --target x86_64-apple-darwin
cross build --release --target aarch64-apple-darwin
```

**Using Nix** (reproducible):
```nix
{
  packages = {
    iron-linux-x64 = pkgs.pkgsCross.musl64.callPackage ./. {};
    iron-linux-arm64 = pkgs.pkgsCross.aarch64-multiplatform-musl.callPackage ./. {};
    iron-macos-x64 = pkgs.pkgsCross.x86_64-darwin.callPackage ./. {};
    iron-macos-arm64 = pkgs.pkgsCross.aarch64-darwin.callPackage ./. {};
  };
}
```

---

## Nix Flake Design

### Tooling Choice: Fenix or Naersk?

After researching the ecosystem, here are the main options for building Rust in Nix:

**1. nixpkgs `rustPlatform.buildRustPackage`** (Standard)
- Built into nixpkgs, no external dependencies
- Uses standard Cargo.lock parsing
- Simple, well-maintained
- ✅ **Recommended for most projects**

**2. crane** (Modern, Incremental)
- Library by ipetkov focused on incremental builds
- Caches dependency artifacts separately
- Best for large projects with many dependencies
- Active development, good documentation
- ✅ **Recommended for iron** (better CI/dev experience)

**3. naersk** (Legacy)
- Older approach, still maintained
- Parses Cargo.lock in pure Nix
- Less incremental caching than crane
- ⚠️ Being superseded by crane

**4. fenix** (Toolchain Management)
- NOT a build tool - provides Rust toolchains
- Nightly rust-analyzer, multiple channels
- Use WITH crane/buildRustPackage, not instead of
- ✅ **Use if you need specific Rust versions**

**Recommendation for iron:**

Use **crane** for building + **fenix** for toolchain management (optional).

Why crane?
- ✅ Incremental builds (dependencies cached separately)
- ✅ Better CI performance (don't rebuild deps every time)
- ✅ Rich ecosystem (clippy, coverage, cross-compilation helpers)
- ✅ Active development, modern design
- ✅ Works great with fenix for toolchain selection

### Minimal Viable Flake (Using Crane)

```nix
{
  description = "iron - P2P network interface based on iroh";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    
    # Optional: for custom Rust toolchains
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, crane, flake-utils, fenix }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        
        # Use default Rust from nixpkgs
        craneLib = crane.mkLib pkgs;
        
        # Or use fenix for custom toolchain:
        # craneLib = (crane.mkLib pkgs).overrideToolchain
        #   fenix.packages.${system}.stable.toolchain;
        
        # Common args for crane
        commonArgs = {
          src = craneLib.cleanCargoSource ./.;
          strictDeps = true;
          
          # macOS frameworks
          buildInputs = pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
            pkgs.darwin.apple_sdk.frameworks.CoreFoundation
            pkgs.libiconv
          ];
        };
        
        # Build dependencies only (for caching)
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        
        # Build the actual binary
        iron = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          
          meta = with pkgs.lib; {
            description = "P2P network interface based on iroh";
            homepage = "https://github.com/you/iron";
            license = with licenses; [ mit asl20 ];
            mainProgram = "iron";
          };
        });
      in
      {
        packages = {
          default = iron;
          inherit iron;
        };

        apps.default = {
          type = "app";
          program = "${iron}/bin/iron";
        };

        checks = {
          # Run tests
          iron-test = craneLib.cargoTest (commonArgs // {
            inherit cargoArtifacts;
          });
          
          # Run clippy
          iron-clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          });
          
          # Check formatting
          iron-fmt = craneLib.cargoFmt {
            src = ./.;
          };
          
          # Audit dependencies
          iron-audit = craneLib.cargoAudit {
            inherit (commonArgs) src;
          };
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ iron ];
          
          packages = [
            pkgs.rust-analyzer
            pkgs.cargo-watch
            pkgs.cargo-edit
          ];
        };
      }
    ) // {
      # NixOS module
      nixosModules.iron = { config, lib, pkgs, ... }:
        with lib;
        let
          cfg = config.services.iron;
        in {
          options.services.iron = {
            enable = mkEnableOption "iron P2P network";
            
            logLevel = mkOption {
              type = types.str;
              default = "info";
              description = "Log level (trace, debug, info, warn, error)";
            };
            
            dnsPort = mkOption {
              type = types.port;
              default = 5333;
              description = "DNS server port";
            };
          };

          config = mkIf cfg.enable {
            systemd.services.iron = {
              description = "iron P2P Network Interface";
              after = [ "network.target" ];
              wantedBy = [ "multi-user.target" ];
              
              serviceConfig = {
                ExecStart = "${self.packages.${pkgs.system}.iron}/bin/iron serve --log-level ${cfg.logLevel} --dns-port ${toString cfg.dnsPort}";
                Restart = "on-failure";
                RestartSec = 5;
                
                # Security hardening
                CapabilityBoundingSet = [ "CAP_NET_ADMIN" ];
                AmbientCapabilities = [ "CAP_NET_ADMIN" ];
                NoNewPrivileges = true;
                PrivateTmp = true;
                ProtectSystem = "strict";
                ProtectHome = true;
              };
            };
          };
        };
    };
}
```

### Alternative: Simple Flake (Using buildRustPackage)

If you want the absolute simplest approach without external dependencies:

```nix
{
  description = "iron - P2P network interface based on iroh";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        
        iron = pkgs.rustPlatform.buildRustPackage {
          pname = "iron";
          version = "0.1.0";
          
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          
          buildInputs = pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
            pkgs.darwin.apple_sdk.frameworks.CoreFoundation
            pkgs.libiconv
          ];
          
          meta = with pkgs.lib; {
            description = "P2P network interface based on iroh";
            homepage = "https://github.com/you/iron";
            license = with licenses; [ mit asl20 ];
            mainProgram = "iron";
          };
        };
      in
      {
        packages.default = iron;
        apps.default = {
          type = "app";
          program = "${iron}/bin/iron";
        };
        devShells.default = pkgs.mkShell {
          inputsFrom = [ iron ];
        };
      }
    );
}
```

This works fine, but crane gives you better CI/dev experience with incremental builds.

### Flake Usage Examples

```bash
# Build
nix build

# Run tests and checks
nix flake check

# Development shell
nix develop

# Run without installing
nix run . -- self

# Install to profile
nix profile install .

# Build for Linux musl (with cross-compilation)
nix build .#packages.x86_64-linux.iron-musl

# Use in NixOS config
{
  inputs.iron.url = "github:you/iron";
  
  outputs = { nixpkgs, iron }: {
    nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
      modules = [
        iron.nixosModules.iron
        {
          services.iron = {
            enable = true;
            logLevel = "debug";
          };
        }
      ];
    };
  };
}
```

---

## Release Distribution

### GitHub Releases Strategy

**Automated with GitHub Actions**:

```yaml
name: Release

on:
  push:
    tags:
      - 'v*'

jobs:
  build:
    strategy:
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-musl
            name: iron-linux-x86_64
          - os: ubuntu-latest
            target: aarch64-unknown-linux-musl
            name: iron-linux-aarch64
          - os: macos-latest
            target: x86_64-apple-darwin
            name: iron-macos-x86_64
          - os: macos-latest
            target: aarch64-apple-darwin
            name: iron-macos-aarch64
    
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v3
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      
      - name: Build
        run: cargo build --release --target ${{ matrix.target }}
      
      - name: Strip binary
        run: strip target/${{ matrix.target }}/release/iron
      
      - name: Create archive
        run: |
          cd target/${{ matrix.target }}/release
          tar czf ${{ matrix.name }}.tar.gz iron
      
      - name: Upload to release
        uses: softprops/action-gh-release@v1
        with:
          files: target/${{ matrix.target }}/release/${{ matrix.name }}.tar.gz
```

**Release artifacts**:
```
iron-linux-x86_64.tar.gz      (~5 MB compressed)
iron-linux-aarch64.tar.gz     (~5 MB compressed)
iron-macos-x86_64.tar.gz      (~6 MB compressed)
iron-macos-aarch64.tar.gz     (~6 MB compressed)
```

### Installation Script

**For non-Nix users**:

```bash
#!/bin/bash
# install.sh - One-line installer

set -e

# Detect OS and architecture
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$ARCH" in
    x86_64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *) echo "Unsupported architecture: $ARCH" && exit 1 ;;
esac

# Download URL
RELEASE="https://github.com/you/iron/releases/latest/download"
BINARY="iron-${OS}-${ARCH}.tar.gz"

echo "Downloading iron for ${OS}-${ARCH}..."
curl -L "${RELEASE}/${BINARY}" | tar xz

echo "Installing to /usr/local/bin (requires sudo)..."
sudo mv iron /usr/local/bin/iron
sudo chmod +x /usr/local/bin/iron

echo "✓ iron installed successfully!"
echo ""
echo "Run 'sudo iron serve' to start"
```

**One-line install**:
```bash
curl -fsSL https://iron.network/install.sh | bash
```

---

## Recommendations

### Immediate Actions (Phase 1)

1. **Create Nix flake** ✅ PRIORITY
   - Package binary
   - Expose tests
   - Create dev shell
   - Target: `nix run github:you/iron` works

2. **Setup release automation**
   - GitHub Actions for multi-platform builds
   - Automated releases on git tags
   - Build Linux musl static binaries

3. **Create install script**
   - Simple curl | bash installer
   - Detect platform automatically
   - Fallback to manual instructions

### Medium Term (Phase 2)

4. **NixOS module**
   - Systemd service definition
   - Configuration options
   - Security hardening
   - Auto-start capability

5. **Package managers**
   - Homebrew formula (easiest, most users)
   - AUR package (community maintained)
   - Maybe: apt repository (if demand exists)

### Future (Phase 3)

6. **Advanced distribution**
   - Docker image (for testing only)
   - Flatpak (if sandboxing makes sense)
   - Snap (if Ubuntu users request)

---

## Security Considerations

### Binary Signing

**macOS**:
- Code signing required for Gatekeeper
- Notarization for smoother UX
- Use Apple Developer account

**Linux**:
- GPG sign releases
- Publish checksums (SHA256)
- Consider reproducible builds (Nix helps here)

### Supply Chain

**Current state**: ✅ GOOD
- All deps from crates.io (auditable)
- No git dependencies
- No binary blobs
- Cargo.lock checked in

**Best practices**:
- Run `cargo audit` in CI
- Pin Rust version in flake
- Use `cargo vendor` for offline builds
- Consider `cargo-crev` for dep review

---

## Testing Distribution

### Pre-release Checklist

```bash
# Build all targets
cross build --release --target x86_64-unknown-linux-musl
cross build --release --target aarch64-unknown-linux-musl

# Test binary works standalone
./target/x86_64-unknown-linux-musl/release/iron self --exists
./target/x86_64-unknown-linux-musl/release/iron --help

# Check size
ls -lh target/*/release/iron

# Verify static linking
ldd target/x86_64-unknown-linux-musl/release/iron
# Should output: "not a dynamic executable"

# Test on fresh system (Docker)
docker run -it --rm alpine sh
# Copy binary and test

# Run full test suite
cargo test --all
nix flake check
```

---

## Conclusion

**Current Status**: ⭐ **Already excellent for deployment!**

Iron is already designed as a single-binary application with minimal runtime dependencies. We're in a great position:

1. ✅ **No external dependencies** beyond system libs
2. ✅ **Minimal runtime files** (just user key)
3. ✅ **Clean installation** (copy binary, done)
4. ✅ **Clean uninstallation** (delete binary and config)

**Priority work**:
1. Create Nix flake (enables `nix run github:you/iron`)
2. Setup automated releases (provides binaries for all platforms)
3. Write install script (one-line install for non-Nix users)

**Result**: 
- Nix users: `nix profile install github:you/iron`
- Everyone else: `curl -fsSL iron.network/install.sh | bash`
- Manual: Download binary from GitHub releases

This achieves the goal: **deployment as easy as it gets** 🎉
