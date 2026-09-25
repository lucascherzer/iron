use iroh::{EndpointId, SecretKey};

/// Create a test `EndpointId` from a seed byte.
/// The same seed always produces the same `EndpointId`,
/// making tests deterministic and reproducible.
pub fn test_endpoint_id(seed: u8) -> EndpointId {
    let secret = SecretKey::from_bytes(&[seed; 32]);
    secret.public()
}
