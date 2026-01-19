# Proposal: Key Persistence

**Status:** ✅ Implemented

## Goal

Persist the iroh secret key across sessions to maintain a stable domain name (`.iron` address).

## Implementation

### Key Storage Location

- **Path:** `~/.config/iron/secret.key`
- **Format:** Raw 32-byte Ed25519 private key
- **Permissions:** 
  - File: `0600` (owner read/write only)
  - Directory: `0700` (owner access only)

### Behavior

1. **First run:** Generates new key and saves to `~/.config/iron/secret.key`
2. **Subsequent runs:** Loads existing key from file
3. **Result:** Same Node ID and `.iron` domain name across restarts

### Security

- Key file is only readable by the owner (0600 permissions)
- Directory is only accessible by the owner (0700 permissions)
- On non-Unix systems, relies on filesystem default protections

### Code

- **Module:** `src/keys.rs`
- **Functions:**
  - `load_or_generate_key()` - Main entry point
  - `load_key()` - Load from file
  - `save_key()` - Save with secure permissions

### Testing

Run iron twice and verify the Node ID stays the same:

```bash
# First run
sudo ./target/release/iron
# Note the Node ID

# Stop and restart
sudo ./target/release/iron
# Node ID should be identical

# Verify key file
ls -la ~/.config/iron/secret.key
# Should show: -rw------- (0600 permissions)
```

## Benefits

- ✅ Stable `.iron` domain names
- ✅ Better UX (can share your domain name once)
- ✅ Follows XDG Base Directory specification
- ✅ Secure key storage with proper permissions
