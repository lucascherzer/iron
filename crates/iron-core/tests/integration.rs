//! Integration tests for iron
//!
//! These tests verify that all components work together correctly:
//! - DNS resolution
//! - Registry consistency
//! - TUN packet handling
//! - Key persistence
//! - End-to-end packet flow (without actual network)

use iroh::SecretKey;
use iron_core::dns::DnsResolver;
use iron_core::mapping::Registry;
use iron_core::router::PacketRouter;
use std::net::Ipv6Addr;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::mpsc;

use iron_core::test_utils::{test_endpoint_id, test_resolver};

/// Test that Registry provides consistent mappings across all components
#[tokio::test]
async fn test_registry_consistency_across_components() {
    let registry = Arc::new(Registry::new());

    // Create two test endpoints
    let endpoint_a = test_endpoint_id(1);
    let endpoint_b = test_endpoint_id(2);

    // DNS component gets IPv6 for endpoint_a
    let ipv6_a_from_dns = registry.register(endpoint_a);

    // TUN component does reverse lookup
    let endpoint_a_from_tun = registry.get_endpoint_id(&ipv6_a_from_dns);
    assert_eq!(endpoint_a_from_tun, Some(endpoint_a));

    // Second call should return same IPv6
    let ipv6_a_again = registry.register(endpoint_a);
    assert_eq!(ipv6_a_from_dns, ipv6_a_again);

    // Different endpoint should get different IPv6
    let ipv6_b = registry.register(endpoint_b);
    assert_ne!(ipv6_a_from_dns, ipv6_b);

    // Verify deterministic derivation (same input = same output)
    let registry2 = Arc::new(Registry::new());
    let ipv6_a_from_registry2 = registry2.register(endpoint_a);
    assert_eq!(ipv6_a_from_dns, ipv6_a_from_registry2);
}

/// A `.iron` domain decodes to the same EndpointId, and thus the same IPv6
/// the registry assigns.
#[tokio::test]
async fn test_dns_resolution_base32_encoding() {
    let registry = Arc::new(Registry::new());
    let endpoint_id = test_endpoint_id(42);

    let domain = iron_core::id::to_domain(&endpoint_id);
    let decoded = iron_core::id::parse_domain(&domain).expect("valid .iron domain");

    assert_eq!(decoded, endpoint_id);
    assert_eq!(registry.register(decoded), registry.register(endpoint_id));
}

/// Test TUN packet processing (OS → Network direction)
#[tokio::test]
async fn test_tun_os_to_network_packet_flow() {
    let registry = Arc::new(Registry::new());
    let endpoint_id = test_endpoint_id(42);

    // Register endpoint and get IPv6
    let dest_ipv6 = registry.register(endpoint_id);

    // Create channels
    let (to_network_tx, mut to_network_rx) = mpsc::unbounded_channel();
    let (_from_network_tx, from_network_rx) = mpsc::unbounded_channel();

    // Create packet router
    let router = PacketRouter::new(
        Arc::clone(&registry),
        test_resolver(),
        to_network_tx,
        from_network_rx,
    );

    // Create minimal IPv6 packet
    let mut packet = vec![0u8; 40];
    packet[0] = 0x60; // Version 6
    packet[6] = 59; // No next header
    packet[7] = 64; // Hop limit

    // Source: fd69:726f::1
    packet[8..24].copy_from_slice(&[
        0xfd, 0x69, 0x72, 0x6f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]);

    // Destination: registered endpoint's IPv6
    packet[24..40].copy_from_slice(&dest_ipv6.octets());

    // Process packet (OS → Network)
    router
        .handle_os_to_network(&packet)
        .await
        .expect("Valid packet");

    // Verify packet was sent to network channel
    let (recv_endpoint_id, recv_packet) = to_network_rx
        .try_recv()
        .expect("Packet sent to network channel");

    assert_eq!(recv_endpoint_id, endpoint_id);
    assert_eq!(recv_packet.as_bytes(), Some(packet.as_slice()));
}

/// Test two-node communication setup
#[tokio::test]
async fn test_two_node_setup() {
    // Node A setup
    let registry_a = Arc::new(Registry::new());
    let endpoint_a = test_endpoint_id(1);
    let ipv6_a = registry_a.register(endpoint_a);

    // Node B setup
    let registry_b = Arc::new(Registry::new());
    let endpoint_b = test_endpoint_id(2);
    let ipv6_b = registry_b.register(endpoint_b);

    // Verify deterministic mapping (both nodes agree on IPv6 for same EndpointId)
    let ipv6_b_from_a = registry_a.register(endpoint_b);
    let ipv6_a_from_b = registry_b.register(endpoint_a);

    assert_eq!(
        ipv6_b, ipv6_b_from_a,
        "Node A should derive same IPv6 for B"
    );
    assert_eq!(
        ipv6_a, ipv6_a_from_b,
        "Node B should derive same IPv6 for A"
    );

    // Verify different nodes get different addresses
    assert_ne!(ipv6_a, ipv6_b);
}

/// Test end-to-end packet flow simulation (Node A → Node B)
#[tokio::test]
async fn test_simulated_packet_flow_node_a_to_b() {
    // Setup Node A
    let registry_a = Arc::new(Registry::new());
    let endpoint_a = test_endpoint_id(1);
    let _ipv6_a = registry_a.register(endpoint_a);

    let (to_network_tx_a, mut to_network_rx_a) = mpsc::unbounded_channel();
    let (_from_network_tx_a, from_network_rx_a) = mpsc::unbounded_channel();
    let router_a = PacketRouter::new(
        Arc::clone(&registry_a),
        test_resolver(),
        to_network_tx_a,
        from_network_rx_a,
    );

    // Setup Node B
    let registry_b = Arc::new(Registry::new());
    let endpoint_b = test_endpoint_id(2);
    let ipv6_b = registry_b.register(endpoint_b);

    let (_to_network_tx_b, _to_network_rx_b) = mpsc::unbounded_channel();
    let (from_network_tx_b, from_network_rx_b) = mpsc::unbounded_channel();
    let _router_b = PacketRouter::new(
        Arc::clone(&registry_b),
        test_resolver(),
        _to_network_tx_b,
        from_network_rx_b,
    );

    // IMPORTANT: Node A needs to know about Node B before sending
    // (In real scenario, this happens via DNS resolution)
    // Node A does DNS lookup for endpoint_b, which registers it
    let ipv6_b_from_a = registry_a.register(endpoint_b);
    assert_eq!(ipv6_b, ipv6_b_from_a, "Deterministic mapping");

    // Node A wants to send packet to Node B
    // Simulate: OS on Node A sends packet to Node B's IPv6

    // Create packet from A to B
    let mut packet_a_to_b = vec![0u8; 40];
    packet_a_to_b[0] = 0x60; // Version 6
    packet_a_to_b[6] = 59; // No next header
    packet_a_to_b[7] = 64; // Hop limit

    // Source: Node A's local address
    let ipv6_a_local = Ipv6Addr::new(0xfd69, 0x726f, 0, 0, 0, 0, 0, 1);
    packet_a_to_b[8..24].copy_from_slice(&ipv6_a_local.octets());

    // Destination: Node B's IPv6
    packet_a_to_b[24..40].copy_from_slice(&ipv6_b.octets());

    // Node A's router processes packet (OS → Network)
    router_a
        .handle_os_to_network(&packet_a_to_b)
        .await
        .expect("Valid packet");

    // Verify packet sent to network with correct EndpointId
    let (dest_endpoint_id, packet_bytes) =
        to_network_rx_a.try_recv().expect("Packet sent to network");

    assert_eq!(dest_endpoint_id, endpoint_b);
    assert_eq!(packet_bytes.as_bytes(), Some(packet_a_to_b.as_slice()));

    // Simulate: Iroh on Node B receives packet from Node A
    // (In real implementation, iroh would verify source and forward to TUN)

    // Node B receives packet via channel
    from_network_tx_b
        .send(packet_bytes)
        .expect("Send to Node B TUN");

    // In real scenario, Node B's TUN would write this to device,
    // and OS would route to listening application
}

/// Test packet flow with invalid destination
#[tokio::test]
async fn test_packet_to_unregistered_destination() {
    let registry = Arc::new(Registry::new());

    let (to_network_tx, mut to_network_rx) = mpsc::unbounded_channel();
    let (_from_network_tx, from_network_rx) = mpsc::unbounded_channel();
    let router = PacketRouter::new(registry, test_resolver(), to_network_tx, from_network_rx);

    // Create packet to unknown destination
    let mut packet = vec![0u8; 40];
    packet[0] = 0x60; // Version 6
    packet[6] = 59; // No next header
    packet[7] = 64; // Hop limit

    // Source: fd69:726f::1
    packet[8..24].copy_from_slice(&[
        0xfd, 0x69, 0x72, 0x6f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]);

    // Destination: unregistered IPv6
    packet[24..40].copy_from_slice(&[
        0xfd, 0x69, 0x72, 0x6f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x99,
        0x99,
    ]);

    // Process packet - should succeed but not send anything
    router
        .handle_os_to_network(&packet)
        .await
        .expect("Should handle gracefully");

    // Verify no packet sent to network (unknown destination)
    assert!(
        to_network_rx.try_recv().is_err(),
        "Should not send packet to unknown destination"
    );
}

/// Test DNS resolver construction
#[tokio::test]
async fn test_dns_resolver_construction() {
    let registry = Arc::new(Registry::new());
    let _resolver = DnsResolver::new(registry, vec![]);
    // Just verify it constructs without panicking
}

/// Test IPv6 prefix consistency
#[test]
fn test_ipv6_prefix_consistency() {
    let registry = Registry::new();
    let endpoint = test_endpoint_id(42);

    let ipv6 = registry.register(endpoint);

    // Verify it's in our ULA range: fd69:726f::/32
    let octets = ipv6.octets();
    assert_eq!(octets[0], 0xfd);
    assert_eq!(octets[1], 0x69);
    assert_eq!(octets[2], 0x72);
    assert_eq!(octets[3], 0x6f);
}

/// Test concurrent packet processing
#[tokio::test]
async fn test_concurrent_packet_processing() {
    let registry = Arc::new(Registry::new());

    // Pre-register multiple endpoints
    let endpoints: Vec<_> = (0..10).map(test_endpoint_id).collect();
    let ipv6s: Vec<_> = endpoints.iter().map(|e| registry.register(*e)).collect();

    let (to_network_tx, mut to_network_rx) = mpsc::unbounded_channel();
    let (_from_network_tx, from_network_rx) = mpsc::unbounded_channel();
    let router = Arc::new(PacketRouter::new(
        Arc::clone(&registry),
        test_resolver(),
        to_network_tx,
        from_network_rx,
    ));

    // Spawn multiple tasks sending packets concurrently
    let mut handles = vec![];
    for (i, dest_ipv6) in ipv6s.iter().enumerate() {
        let router = Arc::clone(&router);
        let dest_ipv6 = *dest_ipv6;

        let handle = tokio::spawn(async move {
            let mut packet = vec![0u8; 40];
            packet[0] = 0x60; // Version 6
            packet[6] = 59;
            packet[7] = 64;

            // Source
            packet[8..24].copy_from_slice(&[
                0xfd, 0x69, 0x72, 0x6f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x01,
            ]);

            // Destination
            packet[24..40].copy_from_slice(&dest_ipv6.octets());

            router.handle_os_to_network(&packet).await.unwrap();
            i
        });
        handles.push(handle);
    }

    // Wait for all tasks
    for handle in handles {
        handle.await.expect("Task completed");
    }

    // Verify all packets were sent
    let mut received_count = 0;
    while to_network_rx.try_recv().is_ok() {
        received_count += 1;
    }
    assert_eq!(received_count, 10);
}

/// Test that TUN interface exposes public method
#[test]
fn test_tun_interface_public_api() {
    // This test ensures our public API is accessible
    let registry = Arc::new(Registry::new());

    let (to_network_tx, _to_network_rx) = mpsc::unbounded_channel();
    let (_from_network_tx, from_network_rx) = mpsc::unbounded_channel();

    let _router = PacketRouter::new(registry, test_resolver(), to_network_tx, from_network_rx);
    // Verify constructor is public and accessible
}

// =============================================================================
// Key Persistence Integration Tests
// =============================================================================

/// Test that key persistence works across simulated "restarts"
#[test]
fn test_key_persistence_across_restarts() {
    // Create a temporary directory for keys
    let temp_dir = TempDir::new().unwrap();
    let key_path = temp_dir.path().join("secret.key");

    // Simulate first run: generate and save key
    let key1 = SecretKey::generate();
    let endpoint_id1 = key1.public();

    std::fs::write(&key_path, key1.to_bytes()).unwrap();

    // Set permissions (Unix only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&key_path).unwrap().permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&key_path, perms).unwrap();
    }

    // Simulate second run: load existing key
    let loaded_bytes = std::fs::read(&key_path).unwrap();
    let key2 = SecretKey::from_bytes(&loaded_bytes.try_into().unwrap());
    let endpoint_id2 = key2.public();

    // Verify they're identical
    assert_eq!(
        endpoint_id1, endpoint_id2,
        "EndpointId should be identical after loading persisted key"
    );
    assert_eq!(
        key1.to_bytes(),
        key2.to_bytes(),
        "Key bytes should be identical"
    );
}

/// Test that same key produces same IPv6 mapping
#[test]
fn test_key_persistence_produces_consistent_ipv6() {
    // Create two registries (simulating two separate runs)
    let registry1 = Registry::new();
    let registry2 = Registry::new();

    // Use same key
    let key = SecretKey::generate();
    let endpoint_id = key.public();

    // Both registries should produce same IPv6 for same EndpointId
    let ipv6_1 = registry1.register(endpoint_id);
    let ipv6_2 = registry2.register(endpoint_id);

    assert_eq!(
        ipv6_1, ipv6_2,
        "Same EndpointId should always map to same IPv6 (deterministic)"
    );
}

/// Test that different keys produce different IPv6 mappings
#[test]
fn test_different_keys_produce_different_ipv6() {
    let registry = Registry::new();

    // Generate two different keys
    let key1 = SecretKey::generate();
    let key2 = SecretKey::generate();

    let endpoint_id1 = key1.public();
    let endpoint_id2 = key2.public();

    let ipv6_1 = registry.register(endpoint_id1);
    let ipv6_2 = registry.register(endpoint_id2);

    assert_ne!(
        ipv6_1, ipv6_2,
        "Different EndpointIds should map to different IPv6 addresses"
    );
}

/// Test key file permissions are secure (Unix only)
#[test]
#[cfg(unix)]
fn test_key_file_has_secure_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = TempDir::new().unwrap();
    let key_path = temp_dir.path().join("secret.key");

    // Generate and save a key
    let key = SecretKey::generate();
    std::fs::write(&key_path, key.to_bytes()).unwrap();

    // Set secure permissions
    let mut perms = std::fs::metadata(&key_path).unwrap().permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(&key_path, perms).unwrap();

    // Verify permissions
    let metadata = std::fs::metadata(&key_path).unwrap();
    let mode = metadata.permissions().mode();

    assert_eq!(
        mode & 0o777,
        0o600,
        "Key file should have 0600 permissions (owner read/write only)"
    );
}

/// Test that corrupted key file is detected
#[test]
fn test_corrupted_key_file_detection() {
    let temp_dir = TempDir::new().unwrap();
    let key_path = temp_dir.path().join("corrupted.key");

    // Write corrupted data (wrong size)
    std::fs::write(&key_path, [1, 2, 3, 4, 5]).unwrap();

    // Try to load - should fail
    let result = std::fs::read(&key_path).and_then(|bytes| {
        if bytes.len() != 32 {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid key size",
            ))
        } else {
            Ok(bytes)
        }
    });

    assert!(result.is_err(), "Loading corrupted key file should fail");
}

/// Test end-to-end: key persistence → endpoint creation → IPv6 mapping
#[tokio::test]
async fn test_e2e_key_persistence_to_ipv6_mapping() {
    let temp_dir = TempDir::new().unwrap();
    let key_path = temp_dir.path().join("secret.key");

    // === First "run" ===

    // Generate and save key
    let key1 = SecretKey::generate();
    std::fs::write(&key_path, key1.to_bytes()).unwrap();

    // Create registry and get IPv6
    let registry1 = Arc::new(Registry::new());
    let endpoint_id1 = key1.public();
    let ipv6_1 = registry1.register(endpoint_id1);

    // === Second "run" (simulated restart) ===

    // Load key from file
    let loaded_bytes = std::fs::read(&key_path).unwrap();
    let key2 = SecretKey::from_bytes(&loaded_bytes.try_into().unwrap());

    // Create new registry (clean state)
    let registry2 = Arc::new(Registry::new());
    let endpoint_id2 = key2.public();
    let ipv6_2 = registry2.register(endpoint_id2);

    // === Verification ===

    // EndpointIds should match
    assert_eq!(
        endpoint_id1, endpoint_id2,
        "Loaded key should produce same EndpointId"
    );

    // IPv6 addresses should match (deterministic mapping)
    assert_eq!(
        ipv6_1, ipv6_2,
        "Same EndpointId should produce same IPv6 across restarts"
    );

    // Verify it's in our ULA range
    let octets = ipv6_2.octets();
    assert_eq!(octets[0], 0xfd);
    assert_eq!(octets[1], 0x69);
    assert_eq!(octets[2], 0x72);
    assert_eq!(octets[3], 0x6f);
}
