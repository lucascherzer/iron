use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use iroh::EndpointId;
use std::net::Ipv6Addr;
use std::path::Path;
use tracing::{debug, trace, warn};

/// The peers this node knows, indexed by their iron address.
///
/// Each peer's address is derived from its key ([`crate::id::derive_ip`]), a
/// pure function, so the EndpointId → IPv6 direction needs no state at all.
/// The registry only stores the reverse direction, which is what routing an
/// outbound packet needs, and which is only known for peers seen before (via
/// DNS, an incoming connection, or the known-peers cache). With a single map
/// the two directions can never disagree.
///
/// Addresses use only the last 64 bits of a key, so two keys *can* map to
/// the same address (a 64-bit collision; impractical to force). The first
/// registered peer keeps the address; later ones are logged and not routable.
///
/// Uses `DashMap` for concurrent access from the DNS resolver, router and
/// protocol tasks.
pub struct Registry {
    peers: DashMap<Ipv6Addr, EndpointId>,
}

impl Registry {
    /// Creates a new empty Registry.
    pub fn new() -> Self {
        Self {
            peers: DashMap::new(),
        }
    }

    /// Records `endpoint_id` as a known peer and returns its address, so
    /// packets to that address can be routed to it.
    pub fn register(&self, endpoint_id: EndpointId) -> Ipv6Addr {
        let ip = crate::id::derive_ip(&endpoint_id);
        match self.peers.entry(ip) {
            Entry::Vacant(entry) => {
                debug!("New peer: {} -> {}", endpoint_id, ip);
                entry.insert(endpoint_id);
            }
            Entry::Occupied(entry) if *entry.get() != endpoint_id => {
                warn!(
                    "Address collision: {} and {} both map to {}; keeping the first",
                    entry.get(),
                    endpoint_id,
                    ip
                );
            }
            Entry::Occupied(_) => trace!("Known peer: {} -> {}", endpoint_id, ip),
        }
        ip
    }

    /// The peer registered for `ip`, if any.
    pub fn get_endpoint_id(&self, ip: &Ipv6Addr) -> Option<EndpointId> {
        self.peers.get(ip).map(|entry| *entry)
    }

    /// Saves known peer EndpointIds to disk for persistence across restarts
    ///
    /// This prevents the issue where applications cache IPv6 addresses but iron
    /// loses the corresponding EndpointId mappings on restart.
    ///
    /// # Format
    ///
    /// Stores only EndpointIds in base32 encoding (same format as .iron domains).
    /// IPv6 addresses are derived deterministically on load.
    ///
    /// Example:
    /// ```json
    /// [
    ///   "rex7gp6zhc4g57hgjaq2hn5ch6xxixhxhqb74d6llmxmnrl2qeau",
    ///   "sgclirglbav3rnznuqbemvyc2eaxxsxcxwge5jvedmdzyvuytsd5"
    /// ]
    /// ```
    ///
    /// # Security
    ///
    /// - File is written with 0600 permissions
    /// - Only saves EndpointIds that were legitimately discovered (via DNS or incoming connections)
    /// - Never "guesses" EndpointIds - only remembers verified peers
    ///
    /// The location comes from the caller (see
    /// [`crate::paths::StatePaths::known_peers_file`]) so that platforms
    /// without a `$HOME` can store state where the OS wants it.
    pub fn save_peers(&self, peers_path: &Path) -> Result<(), std::io::Error> {
        use std::fs;
        use std::io::Write;

        // Ensure directory exists
        if let Some(parent) = peers_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Collect all EndpointIds and encode as base32
        let peers: Vec<String> = self
            .peers
            .iter()
            .map(|entry| crate::id::to_base32(entry.value()))
            .collect();

        // Serialize to pretty JSON for human readability
        let json = serde_json::to_string_pretty(&peers)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        // Write atomically using temp file + rename
        let temp_path = peers_path.with_extension("json.tmp");
        let mut file = fs::File::create(&temp_path)?;

        // Set restrictive permissions (0600 - owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = file.metadata()?.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&temp_path, perms)?;
        }

        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        fs::rename(temp_path, peers_path)?;

        debug!("Saved {} known peers to {:?}", peers.len(), peers_path);
        Ok(())
    }

    /// Loads known peer EndpointIds from disk
    ///
    /// This is called at startup to restore previously discovered peers,
    /// preventing issues with cached IPv6 addresses in applications.
    ///
    /// Each EndpointId is [`register`](Self::register)ed, so its derived
    /// address routes to it again.
    pub fn load_peers(&self, peers_path: &Path) -> Result<usize, std::io::Error> {
        use std::fs;

        // If file doesn't exist, that's okay - just starting fresh
        if !peers_path.exists() {
            debug!("No known peers file found at {:?}", peers_path);
            return Ok(0);
        }

        let contents = fs::read_to_string(peers_path)?;

        // Deserialize from JSON (array of base32-encoded EndpointIds)
        let peer_ids: Vec<String> = serde_json::from_str(&contents)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        let mut loaded = 0;
        for peer_base32 in peer_ids {
            let Some(endpoint_id) = crate::id::parse_base32(&peer_base32) else {
                warn!("Invalid EndpointId in known peers: {}", peer_base32);
                continue;
            };

            self.register(endpoint_id);
            loaded += 1;
        }

        debug!("Loaded {} known peers from {:?}", loaded, peers_path);
        Ok(loaded)
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_endpoint_id;
    use std::sync::Arc;

    #[test]
    fn test_register_and_lookup() {
        let registry = Registry::new();
        for seed in [1u8, 2, 42, 255] {
            let endpoint_id = test_endpoint_id(seed);
            let ip = registry.register(endpoint_id);
            assert_eq!(ip, crate::id::derive_ip(&endpoint_id), "seed {seed}");
            assert_eq!(
                registry.register(endpoint_id),
                ip,
                "idempotent, seed {seed}"
            );
            assert_eq!(
                registry.get_endpoint_id(&ip),
                Some(endpoint_id),
                "seed {seed}"
            );
        }
    }

    #[test]
    fn test_unknown_address_is_none() {
        let registry = Registry::new();
        registry.register(test_endpoint_id(1));
        let unknown = crate::id::derive_ip(&test_endpoint_id(2));
        assert_eq!(registry.get_endpoint_id(&unknown), None);
    }

    /// Two keys with the same last 8 bytes: the first keeps the address.
    #[test]
    fn test_collision_keeps_first_peer() {
        let registry = Registry::new();
        let first = test_endpoint_id(1);
        // Find a second valid key sharing `first`'s last 8 bytes.
        let second = (0u8..=255)
            .find_map(|b| {
                let mut bytes = *first.as_bytes();
                bytes[0] ^= b.max(1);
                EndpointId::from_bytes(&bytes).ok()
            })
            .expect("some prefix variation is a valid key");
        let ip = registry.register(first);
        assert_eq!(registry.register(second), ip, "same derived address");
        assert_eq!(registry.get_endpoint_id(&ip), Some(first));
    }

    #[test]
    fn test_concurrent_registration() {
        let registry = Arc::new(Registry::new());
        let handles: Vec<_> = (0..16)
            .map(|seed| {
                let registry = Arc::clone(&registry);
                std::thread::spawn(move || {
                    let endpoint_id = test_endpoint_id(seed);
                    let ip = registry.register(endpoint_id);
                    assert_eq!(registry.get_endpoint_id(&ip), Some(endpoint_id));
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn test_save_and_load_peers_roundtrip() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let peers_path = temp_dir.path().join("known_peers.json");

        let registry = Registry::new();
        let endpoint_a = test_endpoint_id(1);
        let endpoint_b = test_endpoint_id(2);
        let ip_a = registry.register(endpoint_a);
        let ip_b = registry.register(endpoint_b);
        registry.save_peers(&peers_path).unwrap();

        // A fresh registry restores the same mappings from the file
        let restored = Registry::new();
        assert_eq!(restored.load_peers(&peers_path).unwrap(), 2);
        assert_eq!(restored.get_endpoint_id(&ip_a), Some(endpoint_a));
        assert_eq!(restored.get_endpoint_id(&ip_b), Some(endpoint_b));
    }

    #[test]
    fn test_load_peers_skips_invalid_entries() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let peers_path = temp_dir.path().join("known_peers.json");
        let valid = crate::id::to_base32(&test_endpoint_id(3));
        std::fs::write(&peers_path, format!(r#"["{valid}", "not-a-key"]"#)).unwrap();

        let registry = Registry::new();
        assert_eq!(registry.load_peers(&peers_path).unwrap(), 1);
    }

    #[test]
    fn test_load_peers_missing_file_is_empty() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let registry = Registry::new();
        let loaded = registry
            .load_peers(&temp_dir.path().join("nonexistent.json"))
            .unwrap();
        assert_eq!(loaded, 0);
    }
}
