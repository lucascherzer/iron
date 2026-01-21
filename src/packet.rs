use serde::{Deserialize, Serialize};

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
}

impl Packet {
    /// Creates a new Raw packet
    pub fn raw(data: Vec<u8>) -> Self {
        Packet::Raw(data)
    }

    /// Returns the raw bytes of the packet
    ///
    /// For Raw packets, this returns the inner Vec<u8> directly.
    /// For future packet types, this may perform serialization.
    pub fn into_bytes(self) -> Vec<u8> {
        match self {
            Packet::Raw(data) => data,
        }
    }

    /// Returns a reference to the raw bytes
    ///
    /// For Raw packets, this returns a reference to the inner data.
    /// For future packet types, this may not be available.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Packet::Raw(data) => Some(data),
        }
    }

    /// Returns the size of the packet in bytes
    pub fn len(&self) -> usize {
        match self {
            Packet::Raw(data) => data.len(),
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
}
