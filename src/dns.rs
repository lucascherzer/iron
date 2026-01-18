use crate::mapping::Registry;
use anyhow::Result;
use async_trait::async_trait;
use hickory_proto::op::{Header, ResponseCode};
use hickory_proto::rr::{LowerName, RData, Record, RecordType};
use hickory_server::ServerFuture;
use hickory_server::authority::MessageResponseBuilder;
use hickory_server::server::{Request, RequestHandler, ResponseHandler, ResponseInfo};
use iroh::EndpointId;
use std::net::Ipv6Addr;
use std::sync::Arc;
use tokio::net::UdpSocket;

/// DNS resolver for .iron domains
///
/// Handles AAAA queries for domains in the format `<endpoint_id>.iron`,
/// where endpoint_id is base32 or hex encoded.
pub struct DnsResolver {
    registry: Arc<Registry>,
}

impl DnsResolver {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }

    /// Starts the DNS server listening on the specified address.
    ///
    /// # Arguments
    ///
    /// * `listen_addr` - Address to bind to (e.g., "127.0.0.1:5333")
    ///
    /// # Returns
    ///
    /// Returns Ok(()) on successful shutdown, or an error if the server fails to start
    pub async fn run(&self, listen_addr: &str) -> Result<()> {
        let handler = IronDnsHandler::new(Arc::clone(&self.registry));
        let mut server = ServerFuture::new(handler);

        tracing::info!("Starting DNS server on {}", listen_addr);
        let socket = UdpSocket::bind(listen_addr).await?;
        server.register_socket(socket);

        server.block_until_done().await?;
        Ok(())
    }
}

/// Internal request handler for DNS queries
struct IronDnsHandler {
    registry: Arc<Registry>,
}

impl IronDnsHandler {
    fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }

    /// Parse EndpointId from a .iron domain name
    ///
    /// Supports both base32 (no padding) and hex encoding.
    /// For hex encoding, supports multi-label domains due to DNS 63-char label limit
    /// (e.g., "abc...def.xyz...123.iron" -> concatenate "abc...def" + "xyz...123")
    fn parse_endpoint_from_domain(&self, name: &LowerName) -> Option<EndpointId> {
        let name_str = name.to_string();

        // Check if ends with .iron (DNS names have trailing dot)
        if !name_str.ends_with(".iron.") {
            return None;
        }

        // Extract all labels before .iron.
        // Split by '.' and take everything except the last two elements ("iron" and "")
        let parts: Vec<&str> = name_str.split('.').collect();
        if parts.len() < 3 {
            // Need at least: label + "iron" + ""
            return None;
        }

        // Concatenate all labels before ".iron." to support multi-label hex encoding
        let encoded_id: String = parts[..parts.len() - 2].join("");

        // Try hex decode first (most common, lowercase)
        if let Ok(bytes) = hex::decode(&encoded_id) {
            if bytes.len() == 32 {
                if let Ok(endpoint_id) = EndpointId::from_bytes(&bytes.try_into().unwrap()) {
                    return Some(endpoint_id);
                }
            }
        }

        // Try base32 decode (uppercase, no padding)
        if let Ok(bytes) = data_encoding::BASE32_NOPAD.decode(encoded_id.to_uppercase().as_bytes())
        {
            if bytes.len() == 32 {
                if let Ok(endpoint_id) = EndpointId::from_bytes(&bytes.try_into().unwrap()) {
                    return Some(endpoint_id);
                }
            }
        }

        None
    }

    /// Handle an AAAA query for a .iron domain
    fn handle_aaaa_query(&self, name: &LowerName) -> Option<Ipv6Addr> {
        // 1. Parse EndpointId from domain name
        let endpoint_id = self.parse_endpoint_from_domain(name)?;

        // 2. Lookup IPv6 from registry
        let ipv6 = self.registry.get_or_assign_ip(endpoint_id);

        tracing::debug!(
            "Resolved {}.iron -> {}",
            hex::encode(endpoint_id.as_bytes()),
            ipv6
        );

        Some(ipv6)
    }
}

#[async_trait]
impl RequestHandler for IronDnsHandler {
    async fn handle_request<R: ResponseHandler>(
        &self,
        request: &Request,
        mut response_handle: R,
    ) -> ResponseInfo {
        let request_info = request.request_info().expect("failed to parse request");
        let query = request_info.query;

        tracing::debug!("DNS query: {} {:?}", query.name(), query.query_type());

        // Only handle .iron domains
        if !query.name().to_string().ends_with(".iron.") {
            tracing::debug!("Not a .iron domain, returning NXDOMAIN");
            let response = MessageResponseBuilder::from_message_request(request)
                .error_msg(request.header(), ResponseCode::NXDomain);
            return response_handle.send_response(response).await.unwrap();
        }

        // Only handle AAAA queries
        if query.query_type() != RecordType::AAAA {
            tracing::debug!(
                "Not an AAAA query (got {:?}), returning empty response",
                query.query_type()
            );
            let response = MessageResponseBuilder::from_message_request(request)
                .build_no_records(*request.header());
            return response_handle.send_response(response).await.unwrap();
        }

        // Handle AAAA query
        if let Some(ipv6) = self.handle_aaaa_query(query.name()) {
            let record = Record::from_rdata(
                query.name().into(),
                300, // TTL in seconds
                RData::AAAA(ipv6.into()),
            );

            let mut header = Header::response_from_request(request.header());
            header.set_authoritative(true);
            header.set_response_code(ResponseCode::NoError);

            let records = vec![record];
            let response = MessageResponseBuilder::from_message_request(request).build(
                header,
                &records,
                &[],
                &[],
                &[],
            );

            response_handle.send_response(response).await.unwrap()
        } else {
            // Invalid domain format (couldn't parse EndpointId)
            tracing::warn!("Failed to parse EndpointId from {}", query.name());
            let response = MessageResponseBuilder::from_message_request(request)
                .error_msg(request.header(), ResponseCode::NXDomain);
            response_handle.send_response(response).await.unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;
    use std::str::FromStr;

    fn test_endpoint_id(seed: u8) -> EndpointId {
        let secret = SecretKey::from_bytes(&[seed; 32]);
        secret.public()
    }

    #[test]
    fn test_dns_resolver_new() {
        let registry = Arc::new(Registry::new());
        let _resolver = DnsResolver::new(registry);
        // Just verify it constructs
    }

    #[test]
    fn test_parse_endpoint_hex() {
        let registry = Arc::new(Registry::new());
        let handler = IronDnsHandler::new(registry);
        let endpoint_id = test_endpoint_id(42);

        // Create domain name: <hex>.iron.
        // Hex encoding of 32 bytes = 64 chars, but DNS labels are limited to 63 chars
        // So we split it: first 63 chars . last 1 char . iron .
        let hex_encoded = hex::encode(endpoint_id.as_bytes());
        let domain = format!("{}.{}.iron.", &hex_encoded[..63], &hex_encoded[63..]);
        let name = LowerName::from_str(&domain).unwrap();

        let parsed = handler.parse_endpoint_from_domain(&name);
        assert_eq!(parsed, Some(endpoint_id));
    }

    #[test]
    fn test_parse_endpoint_base32() {
        let registry = Arc::new(Registry::new());
        let handler = IronDnsHandler::new(registry);
        let endpoint_id = test_endpoint_id(42);

        // Create domain name: <base32>.iron.
        // Base32 encoding of 32 bytes = 52 chars (no padding), which fits in one label
        let base32_encoded = data_encoding::BASE32_NOPAD.encode(endpoint_id.as_bytes());
        let domain = format!("{}.iron.", base32_encoded.to_lowercase());
        let name = LowerName::from_str(&domain).unwrap();

        let parsed = handler.parse_endpoint_from_domain(&name);
        assert_eq!(parsed, Some(endpoint_id));
    }

    #[test]
    fn test_parse_endpoint_invalid_domain() {
        let registry = Arc::new(Registry::new());
        let handler = IronDnsHandler::new(registry);

        // Not a .iron domain
        let name = LowerName::from_str("example.com.").unwrap();
        assert!(handler.parse_endpoint_from_domain(&name).is_none());

        // Invalid encoding - parse will succeed but decode will fail
        let name = LowerName::from_str("invalid.iron.").unwrap();
        assert!(handler.parse_endpoint_from_domain(&name).is_none());
    }

    #[test]
    fn test_handle_aaaa_query() {
        let registry = Arc::new(Registry::new());
        let handler = IronDnsHandler::new(registry.clone());
        let endpoint_id = test_endpoint_id(42);

        // Create domain name (split hex to handle 63-char label limit)
        let hex_encoded = hex::encode(endpoint_id.as_bytes());
        let domain = format!("{}.{}.iron.", &hex_encoded[..63], &hex_encoded[63..]);
        let name = LowerName::from_str(&domain).unwrap();

        // Handle query
        let ipv6 = handler.handle_aaaa_query(&name);
        assert!(ipv6.is_some());

        // Verify it matches registry
        let expected = registry.get_or_assign_ip(endpoint_id);
        assert_eq!(ipv6.unwrap(), expected);
    }
}
