# Project Structure

```
iron/
├── Cargo.toml              # Project dependencies and binary configuration
├── README.md               # Main documentation
├── AGENTS.md              # AI agent coding guidelines
│
├── src/                    # Source code
│   ├── lib.rs             # Library exports
│   ├── mapping.rs         # EndpointId ↔ IPv6 registry
│   ├── dns.rs             # DNS resolver for .iron domains
│   ├── tun.rs             # TUN interface and packet handling
│   ├── protocol.rs        # iroh QUIC packet transport
│   ├── node.rs            # Component orchestration
│   └── bin/
│       └── iron.rs        # CLI binary (main entry point)
│
├── tests/
│   └── integration.rs     # Integration tests (10 tests)
│
├── examples/
│   └── test_tun.rs        # TUN device creation example
│
├── scripts/               # Helper scripts
│   ├── README.md          # Scripts documentation
│   ├── node-id-to-dns.sh  # Convert hex to base32
│   ├── test-dns.sh        # Interactive DNS testing
│   ├── test-interactive.sh # Comprehensive tests
│   ├── test-iron.sh       # Automated startup test
│   └── test-tun-minimal.sh # Minimal TUN test
│
└── doc/                   # Documentation
    ├── arch.md            # Architecture decisions
    ├── plan.md            # Implementation plan and status
    ├── packet-flow.md     # Detailed packet flow diagrams
    ├── networking.md      # Network specifications
    ├── dns-setup.md       # DNS configuration guide ⭐
    ├── tun-fix.md         # TUN device fix details
    └── testing/           # Testing documentation
        ├── MANUAL_TESTS.md    # Manual testing procedures
        ├── SUCCESS.md         # Success verification guide
        └── TESTING.md         # General testing info
```

## Key Files

### Entry Points
- **`src/bin/iron.rs`** - CLI application (use this!)
- **`src/lib.rs`** - Library exports for reusable components

### Core Components
- **`src/mapping.rs`** - Registry for EndpointId ↔ IPv6 mapping (227 lines, 11 tests)
- **`src/dns.rs`** - DNS server for `.iron` domains (221 lines, 5 tests)
- **`src/tun.rs`** - TUN interface packet routing (273 lines, 4 tests)
- **`src/protocol.rs`** - iroh QUIC transport (236 lines)
- **`src/node.rs`** - Orchestrates all components (130 lines)

### Documentation
- **`README.md`** - Start here! User-facing documentation
- **`doc/dns-setup.md`** - ⭐ Essential for configuring DNS resolution
- **`doc/arch.md`** - Architecture and design decisions
- **`doc/plan.md`** - Implementation status (all phases complete!)

### Testing
- **Unit tests** - Embedded in each source file (20 tests total)
- **Integration tests** - `tests/integration.rs` (10 tests)
- **Helper scripts** - `scripts/` directory

## Statistics

- **Total Tests:** 30 (all passing ✅)
  - Unit: 20 tests
  - Integration: 10 tests
- **Source Lines:** ~1,800 lines of Rust
- **Documentation:** ~2,000 lines across 10+ docs
- **Dependencies:** 15 crates (see Cargo.toml)

## Documentation Categories

### User Documentation
- `README.md` - Getting started, usage, troubleshooting
- `doc/dns-setup.md` - DNS configuration methods
- `scripts/README.md` - Helper script usage

### Developer Documentation
- `AGENTS.md` - Coding guidelines for AI agents
- `doc/arch.md` - Architecture and design rationale
- `doc/plan.md` - Implementation phases and status
- `doc/packet-flow.md` - Detailed packet routing
- `doc/networking.md` - Network specifications

### Testing Documentation
- `doc/testing/MANUAL_TESTS.md` - Manual test procedures
- `doc/testing/SUCCESS.md` - Verification checklist
- `doc/testing/TESTING.md` - General testing guide

### Technical Documentation
- `doc/tun-fix.md` - TUN device configuration fix
- Inline comments - Comprehensive code documentation

## Development Workflow

1. **Read:** `README.md` for overview
2. **Configure:** DNS using `doc/dns-setup.md`
3. **Build:** `cargo build --release`
4. **Test:** `cargo test`
5. **Run:** `sudo ./target/release/iron`
6. **Verify:** `./scripts/test-interactive.sh`

## Future Additions

When adding new features:
- Add source files to `src/`
- Add tests (unit in source, integration in `tests/`)
- Update `doc/plan.md` with status
- Document in `README.md` and relevant `doc/*.md` files
- Add helper scripts to `scripts/` if needed
