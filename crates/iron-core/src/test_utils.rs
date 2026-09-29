use iroh::{EndpointId, SecretKey};

/// Create a test `EndpointId` from a seed byte.
/// The same seed always produces the same `EndpointId`,
/// making tests deterministic and reproducible.
pub fn test_endpoint_id(seed: u8) -> EndpointId {
    let secret = SecretKey::from_bytes(&[seed; 32]);
    secret.public()
}

/// A DNS resolver with its own empty registry and no upstreams, for tests
/// that need a [`crate::router::PacketRouter`] but don't exercise DNS.
pub fn test_resolver() -> std::sync::Arc<crate::dns::DnsResolver> {
    std::sync::Arc::new(crate::dns::DnsResolver::new(
        std::sync::Arc::new(crate::mapping::Registry::new()),
        vec![],
    ))
}
