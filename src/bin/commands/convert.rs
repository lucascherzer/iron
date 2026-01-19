use anyhow::{Context, Result, anyhow};
use iroh::EndpointId;
use std::net::Ipv6Addr;

pub fn run(value: String, to: Option<String>) -> Result<()> {
    let formats = detect_and_convert(&value)?;

    // If specific format requested, show only that
    if let Some(format) = to {
        match format.to_lowercase().as_str() {
            "hex" => println!("{}", formats.hex),
            "base32" => println!("{}", formats.base32),
            "iron" | "domain" => println!("{}", formats.domain),
            "ipv6" => println!("{}", formats.ipv6),
            _ => {
                return Err(anyhow!(
                    "Invalid format '{}'. Valid formats: hex, base32, iron, ipv6",
                    format
                ));
            }
        }
    } else {
        // Show all formats
        println!("\nNode ID formats:");
        println!("  Hex:     {}", formats.hex);
        println!("  Base32:  {}", formats.base32);
        println!("  Domain:  {}", formats.domain);
        println!("  IPv6:    {}", formats.ipv6);
        println!();
    }

    Ok(())
}

#[derive(Debug)]
struct Formats {
    hex: String,
    base32: String,
    domain: String,
    ipv6: String,
}

fn detect_and_convert(value: &str) -> Result<Formats> {
    let trimmed = value.trim();

    // 1. Check if it's a .iron domain
    if let Some(base32_part) = trimmed.strip_suffix(".iron") {
        return convert_from_base32(base32_part);
    }

    // 2. Check if it's 52 chars (base32 Node ID)
    if trimmed.len() == 52 && is_valid_base32(trimmed) {
        return convert_from_base32(trimmed);
    }

    // 3. Check if it's 64 chars hex (hex Node ID)
    if trimmed.len() == 64 && is_valid_hex(trimmed) {
        return convert_from_hex(trimmed);
    }

    // 4. Check if it contains ':' (IPv6)
    if trimmed.contains(':') {
        return convert_from_ipv6(trimmed);
    }

    Err(anyhow!(
        "Unable to detect format of '{}'\n\n\
        Supported formats:\n\
        - Base32 Node ID (52 chars): df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq\n\
        - Hex Node ID (64 chars): 74df87cccf7e0fead1370fc39f65be3de44f5069f5db87f3b08435ccdaf3b5b9\n\
        - .iron domain: df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron\n\
        - IPv6 address: fd69:726f::0842:35cc:daf3:b5b9",
        trimmed
    ))
}

fn convert_from_base32(base32: &str) -> Result<Formats> {
    // Decode base32 to bytes
    let bytes = data_encoding::BASE32_NOPAD
        .decode(base32.to_uppercase().as_bytes())
        .context("Invalid base32 encoding")?;

    if bytes.len() != 32 {
        return Err(anyhow!(
            "Base32 decoded to {} bytes, expected 32",
            bytes.len()
        ));
    }

    // Convert to EndpointId
    let endpoint_id =
        EndpointId::from_bytes(&bytes.try_into().unwrap()).context("Invalid EndpointId bytes")?;

    // Derive IPv6
    let ipv6 = iron::mapping::Registry::derive_ip(endpoint_id);

    Ok(Formats {
        hex: hex::encode(endpoint_id.as_bytes()),
        base32: base32.to_lowercase(),
        domain: format!("{}.iron", base32.to_lowercase()),
        ipv6: ipv6.to_string(),
    })
}

fn convert_from_hex(hex_str: &str) -> Result<Formats> {
    // Decode hex to bytes
    let bytes = hex::decode(hex_str).context("Invalid hex encoding")?;

    if bytes.len() != 32 {
        return Err(anyhow!("Hex decoded to {} bytes, expected 32", bytes.len()));
    }

    // Convert to EndpointId
    let endpoint_id =
        EndpointId::from_bytes(&bytes.try_into().unwrap()).context("Invalid EndpointId bytes")?;

    // Encode as base32
    let base32 = data_encoding::BASE32_NOPAD
        .encode(endpoint_id.as_bytes())
        .to_lowercase();

    // Derive IPv6
    let ipv6 = iron::mapping::Registry::derive_ip(endpoint_id);

    Ok(Formats {
        hex: hex_str.to_lowercase(),
        base32: base32.clone(),
        domain: format!("{}.iron", base32),
        ipv6: ipv6.to_string(),
    })
}

fn convert_from_ipv6(ipv6_str: &str) -> Result<Formats> {
    let _ipv6: Ipv6Addr = ipv6_str.parse().context("Invalid IPv6 address format")?;

    Err(anyhow!(
        "Cannot convert IPv6 address to Node ID\n\n\
        IPv6 addresses are derived from Node IDs (one-way function).\n\
        There is no reverse mapping from IPv6 to Node ID.\n\n\
        To find the Node ID for an IPv6, you need to:\n\
        - Check iron's DNS resolver: dig @127.0.0.1 -p 5333 -x {}\n\
        - Or check the peer's registry/logs",
        ipv6_str
    ))
}

fn is_valid_base32(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_ascii_lowercase() || "234567".contains(c))
}

fn is_valid_hex(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_base32() {
        let result = detect_and_convert("df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq");
        assert!(result.is_ok());
    }

    #[test]
    fn test_detect_iron_domain() {
        let result =
            detect_and_convert("df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron");
        assert!(result.is_ok());
    }

    #[test]
    fn test_ipv6_no_reverse() {
        let result = detect_and_convert("fd69:726f::0842:35cc:daf3:b5b9");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Cannot convert IPv6")
        );
    }

    #[test]
    fn test_invalid_format() {
        let result = detect_and_convert("invalid");
        assert!(result.is_err());
    }
}
