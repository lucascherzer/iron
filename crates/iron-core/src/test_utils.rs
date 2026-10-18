use crate::platform::PrivateFileWriter;
use iroh::{EndpointId, SecretKey};
use std::{io::Write, path::Path};

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

/// Create-only [`PrivateFileWriter`] without permission handling; the real
/// 0600/0700 behavior is the platform crate's job and tested there.
pub struct TestPrivateFileWriter;

impl PrivateFileWriter for TestPrivateFileWriter {
    fn create_new(&self, path: &Path, contents: &[u8]) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(contents)?;
        Ok(())
    }
}
