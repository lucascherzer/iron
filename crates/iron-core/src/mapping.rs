use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use iroh::EndpointId;
use std::net::Ipv6Addr;
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

    /// Snapshot of currently registered EndpointIds for persistence.
    pub fn endpoint_ids(&self) -> Vec<EndpointId> {
        self.peers.iter().map(|entry| *entry.value()).collect()
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
}
