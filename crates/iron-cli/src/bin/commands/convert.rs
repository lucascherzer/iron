//! `iron convert`: show a node ID in its different forms.

use anyhow::{Result, bail};
use iroh::EndpointId;
use iron_core::id;
use std::net::Ipv6Addr;

/// The forms a node ID can be rendered in. The ID itself is the only stored
/// value; every form is derived from it when printed (see `iron_core::id`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdFormat {
    Hex,
    Base32,
    Domain,
    Ipv6,
}

impl IdFormat {
    const ALL: [IdFormat; 4] = [Self::Hex, Self::Base32, Self::Domain, Self::Ipv6];

    fn parse(name: &str) -> Result<Self> {
        Ok(match name.to_lowercase().as_str() {
            "hex" => Self::Hex,
            "base32" => Self::Base32,
            "iron" | "domain" => Self::Domain,
            "ipv6" => Self::Ipv6,
            _ => bail!("Invalid format '{name}'. Valid formats: hex, base32, iron, ipv6"),
        })
    }

    fn label(self) -> &'static str {
        match self {
            Self::Hex => "Hex",
            Self::Base32 => "Base32",
            Self::Domain => "Domain",
            Self::Ipv6 => "IPv6",
        }
    }

    fn render(self, endpoint_id: &EndpointId) -> String {
        match self {
            Self::Hex => endpoint_id.to_string(),
            Self::Base32 => id::to_base32(endpoint_id),
            Self::Domain => id::to_domain(endpoint_id),
            Self::Ipv6 => id::derive_ip(endpoint_id).to_string(),
        }
    }
}

pub fn run(value: String, to: Option<String>) -> Result<()> {
    let endpoint_id = parse_id(&value)?;
    match to {
        Some(format) => println!("{}", IdFormat::parse(&format)?.render(&endpoint_id)),
        None => {
            println!("\nNode ID formats:");
            for format in IdFormat::ALL {
                let label = format!("{}:", format.label());
                println!("  {label:<9}{}", format.render(&endpoint_id));
            }
            println!();
        }
    }
    Ok(())
}

/// Parses a node ID given as `.iron` domain, base32 or hex.
///
/// IPv6 addresses are rejected with an explanation: they contain only the
/// last 64 bits of the key, so the ID can't be recovered from them.
fn parse_id(value: &str) -> Result<EndpointId> {
    let value = value.trim();
    let parsed = id::parse_domain(value)
        .or_else(|| id::parse_base32(value))
        .or_else(|| id::parse_hex(value));
    if let Some(endpoint_id) = parsed {
        return Ok(endpoint_id);
    }
    if value.parse::<Ipv6Addr>().is_ok() {
        bail!(
            "Cannot convert IPv6 address to Node ID\n\n\
            IPv6 addresses are derived from Node IDs (one-way function).\n\
            There is no reverse mapping from IPv6 to Node ID.\n\n\
            To find the Node ID for an IPv6, check the peer's logs or ask\n\
            for its .iron name."
        );
    }
    bail!(
        "Unable to detect format of '{value}'\n\n\
        Supported formats:\n\
        - Base32 Node ID (52 chars): df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq\n\
        - Hex Node ID (64 chars): 74df87cccf7e0fead1370fc39f65be3de44f5069f5db87f3b08435ccdaf3b5b9\n\
        - .iron domain: df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE32: &str = "df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq";

    #[test]
    fn test_parse_id() {
        let expected = id::parse_base32(BASE32).unwrap();
        let hex = expected.to_string();
        let cases = [
            (BASE32.to_string(), true),
            (BASE32.to_uppercase(), true),
            (format!("{BASE32}.iron"), true),
            (format!("  {BASE32}.iron\n"), true),
            (hex.clone(), true),
            (hex.to_uppercase(), true),
            ("fd69:726f::842:35cc:daf3:b5b9".to_string(), false),
            ("invalid".to_string(), false),
        ];
        for (input, ok) in cases {
            let parsed = parse_id(&input);
            assert_eq!(parsed.is_ok(), ok, "parsing {input:?}: {parsed:?}");
            if ok {
                assert_eq!(parsed.unwrap(), expected, "parsing {input:?}");
            }
        }
    }

    #[test]
    fn test_ipv6_explains_why_it_cannot_convert() {
        let err = parse_id("fd69:726f::842:35cc:daf3:b5b9").unwrap_err();
        assert!(err.to_string().contains("Cannot convert IPv6"));
    }

    #[test]
    fn test_render_is_consistent() {
        let endpoint_id = id::parse_base32(BASE32).unwrap();
        let cases = [
            ("hex", endpoint_id.to_string()),
            ("base32", BASE32.to_string()),
            ("iron", format!("{BASE32}.iron")),
            ("DOMAIN", format!("{BASE32}.iron")),
            ("ipv6", id::derive_ip(&endpoint_id).to_string()),
        ];
        for (name, expected) in cases {
            let format = IdFormat::parse(name).unwrap();
            assert_eq!(format.render(&endpoint_id), expected, "format {name}");
            // Every rendering except IPv6 parses back to the same ID.
            if format != IdFormat::Ipv6 {
                assert_eq!(
                    parse_id(&expected).unwrap(),
                    endpoint_id,
                    "roundtrip {name}"
                );
            }
        }
        assert!(IdFormat::parse("bogus").is_err());
    }
}
