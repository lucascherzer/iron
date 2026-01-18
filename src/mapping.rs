use dashmap::DashMap;
use iroh::EndpointId;
use std::net::Ipv6Addr;
use tracing::{debug, trace};

/// Manages the bi-directional mapping between Iroh EndpointIds and IPv6 addresses.
///
/// The Registry provides deterministic mapping from EndpointIds to IPv6 addresses in the
/// ULA space (fd69:726f::/32). The mapping is cached in DashMaps for O(1) bidirectional lookup.
///
/// # IPv6 Address Space
///
/// - **Prefix**: `fd69:726f::/32` (iron-branded ULA)
/// - **Derivation**: Last 8 bytes of EndpointId used as IPv6 suffix
/// - **Format**: `fd69:726f:0000:0000:xxxx:xxxx:xxxx:xxxx`
///
/// # Thread Safety
///
/// Registry uses DashMap for concurrent access from multiple tokio tasks (DNS resolver and TUN interface).
pub struct Registry {
    // We could have invalid states here. To be safe, we need to guarantee that
    // none of our functions can put the registry into an invalid state.
    // This means extensive unit tests.
    endpoint_to_ip: DashMap<EndpointId, Ipv6Addr>,
    ip_to_endpoint: DashMap<Ipv6Addr, EndpointId>,
}

impl Registry {
    /// Creates a new empty Registry.
    pub fn new() -> Self {
        debug!("Creating new Registry");
        Self {
            endpoint_to_ip: DashMap::new(),
            ip_to_endpoint: DashMap::new(),
        }
    }

    /// Gets or creates an IPv6 address for a given EndpointId.
    ///
    /// If the EndpointId is not in the cache, derives a deterministic IPv6 address
    /// and adds it to both lookup maps.
    ///
    /// # Arguments
    ///
    /// * `endpoint_id` - The iroh EndpointId to map
    ///
    /// # Returns
    ///
    /// The IPv6 address in the fd69:726f::/32 range
    pub fn get_or_assign_ip(&self, endpoint_id: EndpointId) -> Ipv6Addr {
        // Check if we already have this mapping
        if let Some(ip) = self.endpoint_to_ip.get(&endpoint_id) {
            trace!("Cache hit: {} -> {}", endpoint_id, *ip);
            return *ip;
        }

        // Derive a new IPv6 address
        let ip = self.derive_ip(&endpoint_id);
        debug!("New mapping: {} -> {}", endpoint_id, ip);

        // Insert into both maps for bi-directional lookup
        self.endpoint_to_ip.insert(endpoint_id, ip);
        self.ip_to_endpoint.insert(ip, endpoint_id);

        ip
    }

    /// Resolves an IPv6 address back to an EndpointId.
    ///
    /// # Arguments
    ///
    /// * `ip` - The IPv6 address to lookup
    ///
    /// # Returns
    ///
    /// The corresponding EndpointId if found, None otherwise
    pub fn get_endpoint_id(&self, ip: &Ipv6Addr) -> Option<EndpointId> {
        let result = self.ip_to_endpoint.get(ip).map(|entry| *entry);
        if result.is_some() {
            trace!("Reverse lookup: {} -> {:?}", ip, result);
        } else {
            trace!("Reverse lookup miss: {}", ip);
        }
        result
    }

    /// Derives a stable IPv6 address from an EndpointId.
    ///
    /// Uses the last 8 bytes (64 bits) of the 32-byte EndpointId as the IPv6 suffix,
    /// combined with the iron ULA prefix fd69:726f::/32.
    ///
    /// # Implementation
    ///
    /// ```ignore
    /// let bytes = endpoint_id.as_bytes(); // 32 bytes
    /// let suffix = &bytes[24..32];        // Last 8 bytes
    /// // Construct: fd69:726f:0000:0000:[suffix as 4x u16]
    /// ```
    fn derive_ip(&self, endpoint_id: &EndpointId) -> Ipv6Addr {
        let bytes = endpoint_id.as_bytes();

        // Take last 8 bytes (64 bits) for the IPv6 suffix
        let suffix = &bytes[24..32];

        // Construct IPv6 address with iron ULA prefix
        Ipv6Addr::new(
            0xfd69, // ULA + 'i'
            0x726f, // 'r' + 'o'
            0x0000, // Reserved
            0x0000, // Reserved
            u16::from_be_bytes([suffix[0], suffix[1]]),
            u16::from_be_bytes([suffix[2], suffix[3]]),
            u16::from_be_bytes([suffix[4], suffix[5]]),
            u16::from_be_bytes([suffix[6], suffix[7]]),
        )
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
    use iroh::SecretKey;

    /// Helper to create a test EndpointId from a seed value
    fn test_endpoint_id(seed: u8) -> EndpointId {
        let secret = SecretKey::from_bytes(&[seed; 32]);
        secret.public()
    }

    #[test]
    fn test_registry_new() {
        let registry = Registry::new();

        // Registry should be empty initially
        let test_ip = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 0);
        assert!(registry.get_endpoint_id(&test_ip).is_none());
    }

    #[test]
    fn test_derive_ip_deterministic() {
        let registry = Registry::new();
        let endpoint_id = test_endpoint_id(42);

        // Derive IP multiple times - should always be the same
        let ip1 = registry.derive_ip(&endpoint_id);
        let ip2 = registry.derive_ip(&endpoint_id);
        let ip3 = registry.derive_ip(&endpoint_id);

        assert_eq!(ip1, ip2, "Derivation should be deterministic");
        assert_eq!(ip2, ip3, "Derivation should be deterministic");
    }

    #[test]
    fn test_derive_ip_prefix() {
        let registry = Registry::new();
        let endpoint_id = test_endpoint_id(1);

        let ip = registry.derive_ip(&endpoint_id);
        let segments = ip.segments();

        // Check that prefix is correct: fd69:726f:0000:0000
        assert_eq!(segments[0], 0xfd69, "First segment should be 0xfd69");
        assert_eq!(segments[1], 0x726f, "Second segment should be 0x726f");
        assert_eq!(segments[2], 0x0000, "Third segment should be 0x0000");
        assert_eq!(segments[3], 0x0000, "Fourth segment should be 0x0000");
    }

    #[test]
    fn test_get_or_assign_ip_caching() {
        let registry = Registry::new();
        let endpoint_id = test_endpoint_id(1);

        // First call should derive and cache
        let ip1 = registry.get_or_assign_ip(endpoint_id);

        // Second call should return cached value
        let ip2 = registry.get_or_assign_ip(endpoint_id);

        assert_eq!(ip1, ip2, "Cached IP should match");
    }

    #[test]
    fn test_bidirectional_lookup() {
        let registry = Registry::new();
        let endpoint_id = test_endpoint_id(5);

        // Get IP for endpoint
        let ip = registry.get_or_assign_ip(endpoint_id);

        // Reverse lookup should return the same endpoint
        let found_endpoint = registry.get_endpoint_id(&ip);
        assert_eq!(
            found_endpoint,
            Some(endpoint_id),
            "Reverse lookup should return original endpoint"
        );
    }

    #[test]
    fn test_get_endpoint_id_not_found() {
        let registry = Registry::new();
        let random_ip = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 1, 2, 3, 4);

        // Should return None for unmapped IP
        assert!(
            registry.get_endpoint_id(&random_ip).is_none(),
            "Should return None for unmapped IP"
        );
    }

    #[test]
    fn test_different_endpoints_different_ips() {
        let registry = Registry::new();
        let endpoint1 = test_endpoint_id(1);
        let endpoint2 = test_endpoint_id(2);

        let ip1 = registry.get_or_assign_ip(endpoint1);
        let ip2 = registry.get_or_assign_ip(endpoint2);

        assert_ne!(ip1, ip2, "Different endpoints should have different IPs");
    }

    #[test]
    fn test_multiple_endpoints() {
        // Test with array pattern as per AGENTS.md
        let test_cases = [
            (1u8, "endpoint 1"),
            (2u8, "endpoint 2"),
            (10u8, "endpoint 10"),
            (255u8, "endpoint 255"),
        ];

        fn run_test_case(seed: u8, description: &str) {
            let registry = Registry::new();
            let endpoint_id = test_endpoint_id(seed);

            // Get IP
            let ip = registry.get_or_assign_ip(endpoint_id);

            // Verify prefix
            let segments = ip.segments();
            assert_eq!(
                segments[0], 0xfd69,
                "{}: prefix should be correct",
                description
            );

            // Verify bidirectional lookup
            let found = registry.get_endpoint_id(&ip);
            assert_eq!(
                found,
                Some(endpoint_id),
                "{}: bidirectional lookup should work",
                description
            );

            // Verify deterministic (get again)
            let ip2 = registry.get_or_assign_ip(endpoint_id);
            assert_eq!(ip, ip2, "{}: should be deterministic", description);
        }

        for (seed, description) in test_cases {
            run_test_case(seed, description);
        }
    }

    #[test]
    fn test_concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let registry = Arc::new(Registry::new());
        let endpoint_id = test_endpoint_id(42);

        // Spawn multiple threads accessing the same endpoint
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let registry = Arc::clone(&registry);
                thread::spawn(move || registry.get_or_assign_ip(endpoint_id))
            })
            .collect();

        // Collect all IPs
        let ips: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        // All threads should get the same IP
        let first_ip = ips[0];
        for ip in &ips {
            assert_eq!(*ip, first_ip, "All threads should get the same IP");
        }
    }

    #[test]
    fn test_large_number_of_endpoints() {
        let registry = Registry::new();
        let count = 1000;

        // Create and map many endpoints
        let endpoints: Vec<_> = (0..count).map(|i| test_endpoint_id(i as u8)).collect();

        for endpoint in &endpoints {
            registry.get_or_assign_ip(*endpoint);
        }

        // Verify all are still accessible
        for endpoint in &endpoints {
            let ip = registry.get_or_assign_ip(*endpoint);
            let found = registry.get_endpoint_id(&ip);
            assert_eq!(
                found,
                Some(*endpoint),
                "Endpoint should be found after many insertions"
            );
        }
    }

    #[test]
    fn test_registry_default() {
        let registry = Registry::default();
        let endpoint_id = test_endpoint_id(1);

        // Should work the same as new()
        let ip = registry.get_or_assign_ip(endpoint_id);
        let found = registry.get_endpoint_id(&ip);
        assert_eq!(found, Some(endpoint_id));
    }
}
