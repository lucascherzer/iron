use anyhow::{Context, Result};
use serde_json::json;
use std::net::UdpSocket;
use std::time::Instant;

pub async fn run(
    domain: String,
    server: String,
    timeout_secs: u64,
    format: Option<String>,
) -> Result<()> {
    // Validate it's a .iron domain
    if !domain.ends_with(".iron") {
        anyhow::bail!("Error: Not a .iron domain");
    }

    let json_output = format.as_ref().map(|f| f.to_lowercase()) == Some("json".to_string());

    if !json_output {
        println!("\nQuerying {} for {}...", server, domain);
        println!();
    }

    let start = Instant::now();

    // Simple DNS query using hickory-proto directly
    use hickory_proto::op::{Message, Query};
    use hickory_proto::rr::{Name, RecordType};

    // Build query
    let name = Name::from_utf8(&domain).context("Invalid domain name")?;
    let mut query_msg = Message::new();
    query_msg.add_query(Query::query(name.clone(), RecordType::AAAA));
    query_msg.set_recursion_desired(false);

    // Encode query
    let buf = query_msg.to_vec()?;

    // Send UDP query
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_read_timeout(Some(std::time::Duration::from_secs(timeout_secs)))?;
    socket.send_to(&buf, &server)?;

    // Receive response
    let mut response_buf = vec![0u8; 512];
    let (len, _) = socket
        .recv_from(&mut response_buf)
        .context("DNS query failed: connection refused (is iron running?)")?;

    // Parse response
    let response = Message::from_vec(&response_buf[..len])?;

    let elapsed = start.elapsed();

    // Extract AAAA records
    let answers = response.answers();
    if answers.is_empty() {
        anyhow::bail!("No AAAA records found for {}", domain);
    }

    // Get first IPv6 address
    let ipv6 = answers
        .iter()
        .find_map(|record| {
            if let hickory_proto::rr::RData::AAAA(addr) = record.data() {
                Some(addr.to_string())
            } else {
                None
            }
        })
        .context("No IPv6 address in response")?;

    let ttl = answers[0].ttl();

    // Parse Node ID from domain (already validated to end with .iron)
    let base32_part = domain
        .strip_suffix(".iron")
        .context("Domain should end with .iron (already validated)")?;
    let node_id_hex = if let Ok(bytes) =
        data_encoding::BASE32_NOPAD.decode(base32_part.to_uppercase().as_bytes())
    {
        if bytes.len() == 32 {
            hex::encode(bytes)
        } else {
            "unknown".to_string()
        }
    } else {
        "unknown".to_string()
    };

    // Output
    if json_output {
        let output = json!({
            "domain": domain,
            "ipv6": ipv6,
            "ttl": ttl,
            "query_time_ms": elapsed.as_millis(),
            "node_id": {
                "hex": node_id_hex,
                "base32": base32_part
            }
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("✓ Resolved:");
        println!("  Domain:   {}", domain);
        println!("  IPv6:     {}", ipv6);
        println!("  TTL:      {} seconds", ttl);
        println!("  Time:     {}ms", elapsed.as_millis());
        println!();
        println!("Node ID:");
        println!("  Base32:   {}", base32_part);
        println!("  Hex:      {}", node_id_hex);
        println!();
    }

    Ok(())
}
