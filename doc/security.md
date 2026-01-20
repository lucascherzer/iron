# iron Security Model

This document describes the security architecture, threat model, and security considerations for iron.

---

## Table of Contents

- [Overview](#overview)
- [Trust Model](#trust-model)
- [Cryptographic Authentication](#cryptographic-authentication)
- [Source Address Rewriting](#source-address-rewriting)
- [Key Management](#key-management)
- [DNS Security](#dns-security)
- [Attack Vectors](#attack-vectors)
- [Security Best Practices](#security-best-practices)

---

## Overview

Iron's security is based on **iroh's cryptographic authentication** rather than traditional network security models. Each node has a cryptographic identity (EndpointId derived from a public key), and all connections are authenticated using QUIC with TLS 1.3.

**Key Principle**: Trust the crypto, not the network.

---

## Trust Model

### What We Trust

1. **Iroh's cryptographic implementation**
   - Ed25519 public key cryptography
   - QUIC with TLS 1.3 for transport
   - Authenticated connection establishment
   - Encrypted packet transport

2. **Local operating system**
   - TUN device isolation
   - File system permissions
   - Process isolation

3. **Physical access control**
   - Key file stored on local disk
   - Root access required for TUN device

### What We Don't Trust

1. **DNS responses** - DNS is only a convenience mapping
   - Actual security comes from iroh's authentication
   - DNS spoofing cannot compromise security
   - Worst case: wrong IP, connection fails or goes to wrong peer (who can't authenticate)

2. **Network infrastructure**
   - No trust in routers, relays, or ISPs
   - All traffic encrypted via QUIC
   - NAT traversal via iroh's relay system

3. **Packet source addresses** - Rewritten before trusting
   - Don't trust source IPv6 in incoming packets
   - Rewrite to authenticated sender's derived address
   - See [Source Address Rewriting](#source-address-rewriting)

---

## Cryptographic Authentication

### Node Identity

Every iron node has a persistent identity based on an Ed25519 keypair:

```
Secret Key (32 bytes) → Public Key (32 bytes) = EndpointId
```

**Properties**:
- **Unique**: Each secret key generates a unique EndpointId
- **Persistent**: Same key = same EndpointId across restarts
- **Unforgeable**: Cannot derive EndpointId without secret key
- **Verifiable**: Anyone can verify EndpointId matches public key

### Connection Authentication

When two iron nodes connect:

1. **QUIC Handshake** (based on TLS 1.3)
   - Both peers prove possession of their secret keys
   - Establishes encrypted channel
   - Prevents man-in-the-middle attacks

2. **EndpointId Verification**
   - Iroh verifies each peer's EndpointId
   - Connection only succeeds if both peers authenticate
   - No possibility of impersonation

3. **Channel Encryption**
   - All packets encrypted with QUIC's encryption
   - Forward secrecy (ephemeral keys per connection)
   - Replay attack protection

**Result**: When you receive a packet from a connection, you **know** which EndpointId sent it (cryptographically verified).

---

## Source Address Rewriting

### The Problem

IPv6 packets contain a source address field. A malicious peer could send packets with:
- Fake source address (claiming to be another peer)
- Spoofed source address (for routing attacks)
- Arbitrary source address (to confuse the OS)

If we trust the packet's source address, this breaks security.

### The Solution

**Source Address Rewriting** - Overwrite the packet's source IPv6 with the authenticated sender's address.

### How It Works

```rust
// src/protocol.rs:292-328
fn rewrite_source_address(
    packet: Vec<u8>,
    sender_id: &EndpointId,  // Cryptographically verified by iroh
    registry: &Arc<Registry>,
) -> Result<Vec<u8>> {
    // Parse IPv6 header
    let (mut header, payload) = Ipv6Header::from_slice(&packet)?;
    
    // Get sender's derived IPv6 (deterministic)
    let sender_ipv6 = registry.get_or_assign_ip(*sender_id);
    
    // Rewrite source address (IGNORE what packet claims)
    header.source = sender_ipv6.octets();
    
    // Rebuild packet with correct source
    rebuild_packet(header, payload)
}
```

### Security Guarantee

1. **Iroh authenticates** the connection (QUIC + TLS)
2. **We know** the sender's EndpointId (cryptographically verified)
3. **We derive** the IPv6 from EndpointId (deterministic)
4. **We rewrite** the source address in the packet
5. **OS receives** packet with verified source address

**Result**: 
- Peer cannot spoof source address
- OS always sees correct source for return packets
- Routing works correctly
- No trust in packet contents needed

### Why This Is Secure

**Attack scenario**: Malicious peer tries to impersonate another peer

1. **Attacker connects** to you (authenticated as attacker's EndpointId)
2. **Attacker sends packet** with fake source address (claiming to be victim)
3. **Iron rewrites** source to attacker's address (ignoring fake source)
4. **OS receives** packet from attacker's address (not victim's)
5. **Attack fails** - packet is correctly attributed to attacker

**Cannot forge**: Even with packet injection, we trust iroh's authentication, not packet headers.

---

## Key Management

### Key Storage

**Location**: `~/.config/iron/secret.key`

**Permissions**: `0600` (owner read/write only)
```bash
-rw------- 1 user user 32 secret.key
```

**Protection**:
- Only file owner can read/write
- Other users cannot access (even with root, need explicit permission override)
- Directory permissions: `0700` (owner access only)

### Key Generation

**First run**:
```rust
// src/keys.rs:49-62
pub fn load_or_generate_key() -> Result<SecretKey> {
    if key_exists() {
        load_key()
    } else {
        let key = SecretKey::generate(&mut rand::rng());  // Cryptographically secure RNG
        save_key(&key)?;  // Save with 0600 permissions
        Ok(key)
    }
}
```

**Security properties**:
- Uses `rand::rng()` (cryptographically secure random number generator)
- 32 bytes = 256 bits of entropy
- Comparable to Ed25519 key strength

### Key Persistence

**Why persist keys?**
1. Consistent identity across restarts
2. Peers can save your EndpointId
3. DNS mappings remain valid
4. Connection caching works

**Risks**:
- Key file compromise = identity compromise
- Attacker with key can impersonate node
- No revocation mechanism (yet)

**Mitigation**:
- File permissions (0600)
- Encrypt disk (full disk encryption recommended)
- Backup key securely
- Don't share key file (only share EndpointId/base32)

### Sudo Ownership Fix

**Problem**: When run with `sudo`, key files may be created as `root:root`

**Solution**: Automatic ownership fix (src/bin/iron.rs:459-523)
```rust
fn fix_key_directory_ownership() -> Result<()> {
    // Get original user from SUDO_USER env var
    if let Some(user) = get_original_user() {
        // Change ownership to original user
        chown_recursive(key_dir, user.uid, user.gid)?;
    }
}
```

**Security benefit**: Keys remain owned by user, not root

---

## DNS Security

### DNS Is NOT Authenticated

**Important**: DNS responses are **not** cryptographically verified.

**Why this is okay**:
1. DNS only maps `.iron` domains to IPv6 addresses
2. Actual security is in the QUIC connection authentication
3. DNS spoofing worst case: connection to wrong peer fails authentication

### Attack Scenario: DNS Spoofing

**Attacker's goal**: Redirect you to malicious peer

1. **Attacker spoofs DNS** response for `alice.iron`
   - Returns attacker's IPv6 instead of Alice's IPv6
2. **You lookup** EndpointId for attacker's IPv6
   - Registry lookup fails (attacker's IP not in registry)
   - OR: Returns wrong EndpointId
3. **You attempt connection** to attacker's EndpointId
   - QUIC handshake succeeds (you connect to attacker)
   - But you expected Alice's EndpointId
   - Application-level check needed: "Is this the peer I wanted?"

**Current limitation**: No verification that DNS result matches expected EndpointId

**Mitigation** (current):
- DNS server runs on localhost (127.0.0.1:5333)
- No network exposure = no remote DNS spoofing
- Only local processes can query

**Future enhancement**: DNSSEC or signed DNS records

### DNS Auto-Configuration Security

When iron auto-configures DNS:

**macOS** (`/etc/resolver/iron`):
- Requires root to write
- Only affects `.iron` domains
- Coexists with normal DNS

**Linux** (`/etc/systemd/resolved.conf.d/iron.conf`):
- Requires root to write
- Only affects `.iron` domains
- Isolated from other DNS config

**Security properties**:
- Root required to modify
- Automatically cleaned up on shutdown
- No persistent backdoor if iron compromised
- Only routes `.iron` to localhost

---

## Attack Vectors

### 1. Key File Theft

**Attack**: Steal `~/.config/iron/secret.key`

**Impact**: 
- Attacker can impersonate the node
- Accept connections as the node
- Decrypt past traffic (if QUIC doesn't use forward secrecy properly)

**Mitigation**:
- File permissions (0600)
- Disk encryption
- Secure backups
- Physical security

**Detection**: None (attacker has full credentials)

### 2. IPv6 Collision

**Attack**: Generate EndpointId with same IPv6 suffix (last 64 bits)

**Probability**: ~2^-64 = 1 in 18 quintillion

**Impact**:
- Two peers map to same IPv6
- Registry confusion
- Connection to wrong peer

**Mitigation**:
- Extremely low probability
- Source address rewriting prevents spoofing
- Each connection still authenticated

**Practical**: Not feasible

### 3. DNS Spoofing (Local)

**Attack**: Local process spoofs DNS responses

**Impact**:
- Victim connects to wrong EndpointId
- Attacker can receive connection
- But cannot impersonate other peers

**Mitigation**:
- DNS server on localhost only
- Requires local access
- Application should verify expected EndpointId

**Severity**: Low (requires local access)

### 4. TUN Device Hijacking

**Attack**: Create conflicting TUN device or intercept packets

**Impact**:
- Packet sniffing (but packets are encrypted in QUIC)
- Packet injection (but source rewriting prevents spoofing)
- Denial of service (drop packets)

**Mitigation**:
- TUN device requires root
- OS-level process isolation
- Encrypted transport (QUIC)

**Severity**: Medium (requires root, limited impact)

### 5. QUIC Implementation Vulnerabilities

**Attack**: Exploit bugs in QUIC or TLS implementation

**Impact**: Depends on vulnerability

**Mitigation**:
- Use well-tested iroh library
- Regular dependency updates
- Follow iroh security advisories

**Responsibility**: Upstream (iroh project)

### 6. Traffic Analysis

**Attack**: Observe packet sizes, timing, destinations

**Impact**:
- Infer who you're communicating with
- Traffic patterns
- Metadata leakage

**Mitigation** (current): None (out of scope for MVP)

**Future**: Onion routing (original vision)

---

## Security Best Practices

### For Users

1. **Protect your key file**
   ```bash
   # Verify permissions
   ls -la ~/.config/iron/secret.key
   # Should show: -rw------- (0600)
   ```

2. **Backup securely**
   ```bash
   # Export key
   iron key export --output backup.key
   
   # Store in encrypted backup
   # NEVER commit to git!
   # NEVER share publicly!
   ```

3. **Use full disk encryption**
   - FileVault (macOS)
   - LUKS (Linux)
   - BitLocker (Windows)

4. **Verify peer identities**
   - Exchange EndpointIds securely (not via .iron DNS)
   - Compare base32 Node IDs verbally or via secure channel
   - Consider out-of-band verification

5. **Run iron as root only when needed**
   - Daemon mode requires root (TUN device)
   - Utility commands don't need root
   - Don't run as root unnecessarily

6. **Keep iron updated**
   - Security fixes in dependencies
   - Protocol improvements
   - Bug fixes

### For Developers

1. **Never log secret keys**
   - Log EndpointIds (public keys) only
   - Redact sensitive data in error messages

2. **Validate all inputs**
   - Check packet sizes (MAX_PACKET_SIZE)
   - Verify IPv6 addresses are in ULA range
   - Sanitize DNS queries

3. **Use constant-time comparisons**
   - For key comparisons
   - For crypto operations

4. **Follow Rust safety**
   - No unsafe code without careful review
   - Use type system for invariants
   - Audit dependencies

5. **Keep dependencies updated**
   ```bash
   cargo update
   cargo audit
   ```

---

## Security Limitations (Current)

1. **No forward secrecy for long-lived keys**
   - EndpointId keys are persistent
   - Compromise reveals all past connections to that peer
   - QUIC provides forward secrecy per-connection

2. **No peer revocation**
   - Cannot revoke compromised keys
   - No blocklist mechanism
   - No key rotation protocol

3. **No traffic padding**
   - Packet sizes leak information
   - Timing attacks possible
   - Metadata not protected

4. **No rate limiting**
   - DoS possible by flooding packets
   - No bandwidth limits
   - No connection limits

5. **No audit logging**
   - Cannot track past connections
   - Limited forensics capability
   - Minimal security events logged

6. **DNS not authenticated**
   - Must trust localhost DNS
   - No cryptographic binding of EndpointId to .iron domain
   - Depends on DNS security

---

## Future Security Enhancements

### Short-term

1. **Connection rate limiting**
   - Limit connections per peer
   - Throttle packet rates
   - Prevent resource exhaustion

2. **Peer blocklist**
   - Block specific EndpointIds
   - Temporary and permanent blocks
   - Configuration file support

3. **Audit logging**
   - Log connection events
   - Track peer connections
   - Security event logging

### Medium-term

4. **DNSSEC integration**
   - Signed DNS records
   - Cryptographic binding of EndpointId to domain
   - Prevent DNS spoofing

5. **Key rotation**
   - Periodic key rotation protocol
   - Maintain identity across rotations
   - Backward compatibility

6. **Metrics and monitoring**
   - Connection statistics
   - Bandwidth monitoring
   - Anomaly detection

### Long-term

7. **Onion routing** (original vision)
   - Multi-hop packet routing
   - Traffic analysis resistance
   - Anonymous communication

8. **Perfect forward secrecy**
   - Ephemeral keys for peer relationships
   - Limit damage of key compromise
   - Balance with performance

9. **Zero-knowledge authentication**
   - Prove identity without revealing EndpointId
   - Privacy-preserving connections
   - Research needed

---

## Responsible Disclosure

If you discover a security vulnerability in iron:

1. **DO NOT** publish it publicly
2. **DO** report it privately to the maintainers
3. **DO** provide details:
   - Description of vulnerability
   - Steps to reproduce
   - Potential impact
   - Suggested fix (if known)

**Contact**: (Add contact information here)

---

## Security Assumptions

Iron's security model assumes:

1. **Cryptography is hard to break**
   - Ed25519 is secure
   - QUIC/TLS 1.3 is secure
   - RNG is cryptographically secure

2. **Operating system is trusted**
   - File permissions work
   - Process isolation works
   - TUN device isolation works

3. **Physical security exists**
   - Attacker doesn't have physical access to machine
   - Disk is encrypted
   - Key file is protected

4. **Implementation is correct**
   - No critical bugs in iron
   - No critical bugs in dependencies
   - Code follows security best practices

**If any assumption breaks, security may be compromised.**

---

## Conclusion

Iron's security is based on:
1. **Cryptographic authentication** via iroh (strongest layer)
2. **Source address rewriting** (prevents spoofing)
3. **Key file protection** (filesystem security)
4. **Encrypted transport** (QUIC with TLS 1.3)

**Key insight**: DNS is convenience, crypto is security.

Trust the authentication, not the network. Trust the code, not the packets. Trust the keys, not the addresses.

---

## See Also

- `doc/networking.md` - Networking details including security sections
- `doc/arch.md` - Architecture overview
- `doc/cli.md` - Key management commands
- [Iroh Security](https://iroh.computer/docs) - Upstream security model
