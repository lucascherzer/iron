//! Textual and address forms of a node identity.
//!
//! The single source of truth is [`EndpointId`] (the node's Ed25519 public
//! key). Everything else is derived from it on demand and never stored:
//!
//! | Form   | Example                         | Reversible |
//! |--------|---------------------------------|------------|
//! | hex    | `EndpointId`'s `Display` / [`parse_hex`] | yes |
//! | base32 | [`to_base32`] / [`parse_base32`] | yes |
//! | domain | [`to_domain`] / [`parse_domain`] | yes |
//! | IPv6   | [`derive_ip`]                   | no: only the last 64 bits of the key |

use iroh::EndpointId;
use std::net::Ipv6Addr;

/// Top-level DNS label of iron names.
pub const IRON_TLD: &str = "iron";

/// Lowercase base32 without padding: 52 characters, fits a DNS label.
pub fn to_base32(id: &EndpointId) -> String {
    data_encoding::BASE32_NOPAD
        .encode(id.as_bytes())
        .to_lowercase()
}

/// Parses base32 without padding, case-insensitively.
pub fn parse_base32(s: &str) -> Option<EndpointId> {
    let bytes = data_encoding::BASE32_NOPAD
        .decode(s.to_uppercase().as_bytes())
        .ok()?;
    EndpointId::from_bytes(&bytes.try_into().ok()?).ok()
}

/// Parses 64 hex digits, case-insensitively. (`EndpointId`'s own `FromStr`
/// only accepts lowercase.)
pub fn parse_hex(s: &str) -> Option<EndpointId> {
    let bytes = hex::decode(s).ok()?;
    EndpointId::from_bytes(&bytes.try_into().ok()?).ok()
}

/// `<base32>.iron`
pub fn to_domain(id: &EndpointId) -> String {
    format!("{}.{IRON_TLD}", to_base32(id))
}

/// Parses `<base32>.iron`, with or without the trailing root dot, case-
/// insensitively. Subdomains (`a.<base32>.iron`) are rejected.
pub fn parse_domain(s: &str) -> Option<EndpointId> {
    let s = s.strip_suffix('.').unwrap_or(s);
    let (label, tld) = s.rsplit_once('.')?;
    if !tld.eq_ignore_ascii_case(IRON_TLD) || label.contains('.') {
        return None;
    }
    parse_base32(label)
}

/// The node's address in the iron ULA range `fd69:726f::/32`:
/// `fd69:726f:0:0:` followed by the last 8 bytes of the key.
///
/// Groups 3–4 are always zero; [`crate::dns::MAGIC_DNS_ADDR`] relies on that
/// to be collision-free.
pub fn derive_ip(id: &EndpointId) -> Ipv6Addr {
    let bytes = id.as_bytes();
    let suffix = |i: usize| u16::from_be_bytes([bytes[i], bytes[i + 1]]);
    Ipv6Addr::new(
        0xfd69, // ULA + 'i'
        0x726f, // 'r' + 'o'
        0,
        0,
        suffix(24),
        suffix(26),
        suffix(28),
        suffix(30),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_endpoint_id;

    #[test]
    fn test_base32_roundtrip() {
        let id = test_endpoint_id(42);
        let encoded = to_base32(&id);
        let cases = [
            (encoded.clone(), Some(id)),
            (encoded.to_uppercase(), Some(id)),
            (format!("{encoded}a"), None),
            ("not base32!".to_string(), None),
            (String::new(), None),
        ];
        assert_eq!(encoded.len(), 52);
        assert_eq!(encoded, encoded.to_lowercase());
        for (input, expected) in cases {
            assert_eq!(parse_base32(&input), expected, "parsing {input:?}");
        }
    }

    #[test]
    fn test_parse_hex() {
        let id = test_endpoint_id(9);
        let hex = id.to_string();
        let cases = [
            (hex.clone(), Some(id)),
            (hex.to_uppercase(), Some(id)),
            (hex[..62].to_string(), None),
            (format!("{hex}00"), None),
            ("zz".repeat(32), None),
        ];
        for (input, expected) in cases {
            assert_eq!(parse_hex(&input), expected, "parsing {input:?}");
        }
    }

    #[test]
    fn test_parse_domain() {
        let id = test_endpoint_id(7);
        let domain = to_domain(&id);
        let cases = [
            (domain.clone(), Some(id)),
            (format!("{domain}."), Some(id)),
            (domain.to_uppercase(), Some(id)),
            (format!("www.{domain}"), None),
            (to_base32(&id), None),
            (format!("{}.com", to_base32(&id)), None),
            ("iron".to_string(), None),
            (".iron".to_string(), None),
        ];
        for (input, expected) in cases {
            assert_eq!(parse_domain(&input), expected, "parsing {input:?}");
        }
    }

    #[test]
    fn test_derive_ip() {
        let id = test_endpoint_id(1);
        let ip = derive_ip(&id);
        let segments = ip.segments();
        assert_eq!(segments[..4], [0xfd69, 0x726f, 0, 0]);
        assert_eq!(ip.octets()[8..], id.as_bytes()[24..]);
        assert_eq!(derive_ip(&id), ip, "deterministic");
        assert_ne!(derive_ip(&test_endpoint_id(2)), ip);
    }
}
