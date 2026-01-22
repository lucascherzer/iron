use crate::firewall::OwnershipClaim;
use serde::{Deserialize, Serialize};

// Re-export auth types for convenience
pub use AuthMessage::*;
pub use AuthResponse::*;

/// Packet types that can be transported over iron's protocol layer
///
/// This enum provides type-safe packet handling and enables future features
/// like onion routing and firewall functionality.
///
/// # Phase 1 (Current)
///
/// Only `Raw` variant is used. All packets are transported as raw IPv6 bytes
/// without serialization on the wire. This maintains backward compatibility
/// while enabling internal type safety.
///
/// # Future Phases
///
/// - Phase 2: Add wire format serialization using postcard
/// - Phase 3: Add `Onion` variant for multi-hop encrypted routing
/// - Phase 3: Add `Auth` variant for firewall authentication
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Packet {
    /// Raw IPv6 packet from TUN interface
    ///
    /// This is the standard packet type for direct peer-to-peer communication.
    /// Contains the complete IPv6 packet including header and payload.
    Raw(Vec<u8>),

    /// Authentication packet carrying device ownership proof
    ///
    /// Sent as the first message when establishing a connection to a
    /// firewall-enabled peer. Must be validated before any other packets
    /// are accepted.
    ///
    /// Authentication is cached persistently - once a device is verified,
    /// it remains in the verified_devices cache until the claim expires
    /// or the person key is removed from the whitelist.
    Auth(AuthMessage),
}

/// Authentication message for firewall
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthMessage {
    /// Device ownership claim
    ///
    /// When initiating communication to a peer, we identify ourselves as being
    /// owned by a person the receiver trusts.
    ///
    /// For MVP, each device has exactly one ownership claim (one person key).
    /// Future versions may support multiple ownership claims for shared devices.
    Claim(OwnershipClaim),

    /// Response to claim (accept/reject)
    Response(AuthResponse),
}

/// Response to an authentication claim
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthResponse {
    /// Authentication accepted
    Accepted,

    /// Authentication rejected with reason
    Rejected { reason: String },
}

impl Packet {
    /// Creates a new Raw packet
    pub fn raw(data: Vec<u8>) -> Self {
        Packet::Raw(data)
    }

    /// Creates a new Auth packet
    pub fn auth(message: AuthMessage) -> Self {
        Packet::Auth(message)
    }

    /// Returns the raw bytes of the packet
    ///
    /// For Raw packets, this returns the inner Vec<u8> directly.
    /// For Auth packets, this serializes using postcard.
    pub fn into_bytes(self) -> Vec<u8> {
        match self {
            Packet::Raw(data) => data,
            Packet::Auth(msg) => {
                // Serialize auth message using postcard
                postcard::to_allocvec(&msg).unwrap_or_default()
            }
        }
    }

    /// Returns a reference to the raw bytes
    ///
    /// For Raw packets, this returns a reference to the inner data.
    /// For Auth packets, this returns None (use into_bytes for serialization).
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Packet::Raw(data) => Some(data),
            Packet::Auth(_) => None,
        }
    }

    /// Returns the size of the packet in bytes
    pub fn len(&self) -> usize {
        match self {
            Packet::Raw(data) => data.len(),
            Packet::Auth(msg) => postcard::to_allocvec(msg).map(|v| v.len()).unwrap_or(0),
        }
    }

    /// Returns true if the packet is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl From<Vec<u8>> for Packet {
    fn from(data: Vec<u8>) -> Self {
        Packet::Raw(data)
    }
}

impl From<Packet> for Vec<u8> {
    fn from(packet: Packet) -> Self {
        packet.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::firewall::PersonSecretKey;

    #[test]
    fn test_packet_raw_creation() {
        let data = vec![1, 2, 3, 4];
        let packet = Packet::raw(data.clone());
        assert_eq!(packet, Packet::Raw(data));
    }

    #[test]
    fn test_packet_from_vec() {
        let data = vec![1, 2, 3, 4];
        let packet: Packet = data.clone().into();
        assert_eq!(packet, Packet::Raw(data));
    }

    #[test]
    fn test_packet_into_bytes() {
        let data = vec![1, 2, 3, 4];
        let packet = Packet::raw(data.clone());
        assert_eq!(packet.into_bytes(), data);
    }

    #[test]
    fn test_packet_as_bytes() {
        let data = vec![1, 2, 3, 4];
        let packet = Packet::raw(data.clone());
        assert_eq!(packet.as_bytes(), Some(data.as_slice()));
    }

    #[test]
    fn test_packet_len() {
        let packet = Packet::raw(vec![1, 2, 3, 4]);
        assert_eq!(packet.len(), 4);
    }

    #[test]
    fn test_packet_is_empty() {
        let empty = Packet::raw(vec![]);
        let not_empty = Packet::raw(vec![1]);
        assert!(empty.is_empty());
        assert!(!not_empty.is_empty());
    }

    #[test]
    fn test_packet_serialization() {
        let data = vec![1, 2, 3, 4];
        let packet = Packet::raw(data);

        // Test postcard serialization (for future wire format)
        let serialized = postcard::to_allocvec(&packet).unwrap();
        let deserialized: Packet = postcard::from_bytes(&serialized).unwrap();

        assert_eq!(packet, deserialized);
    }

    #[test]
    fn test_packet_clone() {
        let packet = Packet::raw(vec![1, 2, 3, 4]);
        let cloned = packet.clone();
        assert_eq!(packet, cloned);
    }

    #[test]
    fn test_packet_auth_creation() {
        let person_secret = PersonSecretKey::generate();
        let device_key = iroh::SecretKey::generate(&mut rand::rng()).public();
        let claim = crate::firewall::OwnershipClaim::new(&person_secret, device_key, 3600);

        let packet = Packet::auth(AuthMessage::Claim(claim.clone()));
        assert!(matches!(packet, Packet::Auth(AuthMessage::Claim(_))));
    }

    #[test]
    fn test_packet_auth_serialization() {
        let person_secret = PersonSecretKey::generate();
        let device_key = iroh::SecretKey::generate(&mut rand::rng()).public();
        let claim = crate::firewall::OwnershipClaim::new(&person_secret, device_key, 3600);

        let packet = Packet::auth(AuthMessage::Claim(claim));

        // Serialize and deserialize
        let serialized = postcard::to_allocvec(&packet).unwrap();
        let deserialized: Packet = postcard::from_bytes(&serialized).unwrap();

        assert_eq!(packet, deserialized);
    }

    #[test]
    fn test_packet_auth_response() {
        let packet = Packet::auth(AuthMessage::Response(AuthResponse::Accepted));
        assert!(matches!(
            packet,
            Packet::Auth(AuthMessage::Response(AuthResponse::Accepted))
        ));

        let packet = Packet::auth(AuthMessage::Response(AuthResponse::Rejected {
            reason: "Not trusted".to_string(),
        }));
        assert!(matches!(
            packet,
            Packet::Auth(AuthMessage::Response(AuthResponse::Rejected { .. }))
        ));
    }

    #[test]
    fn test_packet_auth_into_bytes() {
        let packet = Packet::auth(AuthMessage::Response(AuthResponse::Accepted));
        let bytes = packet.into_bytes();
        assert!(!bytes.is_empty());
    }
}
