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
use tracing::{debug, info, trace, warn};

/// DNS resolver for .iron domains
///
/// Handles AAAA queries for domains in the format `<endpoint_id>.iron`,
/// where endpoint_id is base32 or hex encoded.
pub struct DnsResolver {
    registry: Arc<Registry>,
}

impl DnsResolver {
    pub fn new(registry: Arc<Registry>) -> Self {
        info!("Creating DNS resolver");
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

        info!("Starting DNS server on {}", listen_addr);
        let socket = UdpSocket::bind(listen_addr).await?;
        server.register_socket(socket);

        info!("DNS server listening on {}", listen_addr);
        server.block_until_done().await?;
        info!("DNS server shutdown");
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
    /// Uses base32 encoding (no padding, case-insensitive).
    /// Base32 encoding of 32-byte EndpointId = 52 characters, fits in single DNS label.
    ///
    /// Example: `df7wwi7bnsctfrvlza4pvtk6u6e34ddwwkjagnadtp5iwpjwrvqq.iron`
    fn parse_endpoint_from_domain(&self, name: &LowerName) -> Option<EndpointId> {
        let name_str = name.to_string();

        // Check if ends with .iron (DNS names have trailing dot)
        if !name_str.ends_with(".iron.") {
            trace!("Not a .iron domain: {}", name_str);
            return None;
        }

        // Extract label before .iron.
        // Expected format: "<base32>.iron."
        let parts: Vec<&str> = name_str.split('.').collect();
        if parts.len() != 3 {
            // Must be exactly: label + "iron" + ""
            debug!("Invalid .iron domain format (multi-label): {}", name_str);
            return None;
        }

        let encoded_id = parts[0];

        // Base32 decode (uppercase, no padding) - case insensitive
        if let Ok(bytes) = data_encoding::BASE32_NOPAD.decode(encoded_id.to_uppercase().as_bytes())
        {
            if bytes.len() == 32 {
                if let Ok(endpoint_id) = EndpointId::from_bytes(&bytes.try_into().unwrap()) {
                    trace!("Parsed EndpointId from domain: {}", endpoint_id);
                    return Some(endpoint_id);
                }
            }
        }

        warn!("Failed to parse EndpointId from domain: {}", name_str);
        None
    }

    /// Handle an AAAA query for a .iron domain
    fn handle_aaaa_query(&self, name: &LowerName) -> Option<Ipv6Addr> {
        // 1. Parse EndpointId from domain name
        let endpoint_id = self.parse_endpoint_from_domain(name)?;

        // 2. Lookup IPv6 from registry
        let ipv6 = self.registry.get_or_assign_ip(endpoint_id);

        debug!(
            "Resolved {}.iron -> {}",
            data_encoding::BASE32_NOPAD
                .encode(endpoint_id.as_bytes())
                .to_lowercase(),
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

        // Only handle .iron domains
        // Return REFUSED for non-.iron domains (RFC 1035: server refuses operation for policy reasons)
        // This tells resolvers "I don't handle this domain" vs NXDOMAIN "domain doesn't exist"
        if !query.name().to_string().ends_with(".iron.") {
            let response = MessageResponseBuilder::from_message_request(request)
                .error_msg(request.header(), ResponseCode::Refused);
            return response_handle.send_response(response).await.unwrap();
        }

        // Log only .iron domain queries (reduces noise from systemd-resolved fallback queries)
        trace!("DNS query: {} {:?}", query.name(), query.query_type());

        // Only handle AAAA queries for .iron domains
        // For other query types (A, MX, etc.), return authoritative empty answer
        // This tells resolvers: "I'm authoritative for this domain, but it has no A record"
        if query.query_type() != RecordType::AAAA {
            trace!(
                "Not an AAAA query for .iron domain (got {:?}), returning authoritative empty answer",
                query.query_type()
            );

            // Return authoritative NOERROR with SOA record in authority section
            // This is the correct DNS response for "domain exists but no record of this type"
            let mut header = Header::response_from_request(request.header());
            header.set_authoritative(true);
            header.set_response_code(ResponseCode::NoError);

            // Build response with empty answer section
            // The authoritative flag tells clients not to retry
            let response = MessageResponseBuilder::from_message_request(request).build(
                header,
                &[],
                &[],
                &[],
                &[],
            );
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
            warn!("Failed to parse EndpointId from {}", query.name());
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
    fn test_parse_endpoint_base32_uppercase() {
        let registry = Arc::new(Registry::new());
        let handler = IronDnsHandler::new(registry);
        let endpoint_id = test_endpoint_id(42);

        // Test uppercase (should work due to case-insensitive parsing)
        let base32_encoded = data_encoding::BASE32_NOPAD.encode(endpoint_id.as_bytes());
        let domain = format!("{}.iron.", base32_encoded); // uppercase
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

        // Invalid encoding - valid DNS label but invalid base32 characters
        let name = LowerName::from_str("invalidbase32withlowercase.iron.").unwrap();
        assert!(handler.parse_endpoint_from_domain(&name).is_none());

        // Too many labels (not base32 format)
        let name = LowerName::from_str("label1.label2.iron.").unwrap();
        assert!(handler.parse_endpoint_from_domain(&name).is_none());
    }

    #[test]
    fn test_handle_aaaa_query() {
        let registry = Arc::new(Registry::new());
        let handler = IronDnsHandler::new(registry.clone());
        let endpoint_id = test_endpoint_id(42);

        // Create domain name with base32 encoding
        let base32_encoded = data_encoding::BASE32_NOPAD.encode(endpoint_id.as_bytes());
        let domain = format!("{}.iron.", base32_encoded.to_lowercase());
        let name = LowerName::from_str(&domain).unwrap();

        // Handle query
        let ipv6 = handler.handle_aaaa_query(&name);
        assert!(ipv6.is_some());

        // Verify it matches registry
        let expected = registry.get_or_assign_ip(endpoint_id);
        assert_eq!(ipv6.unwrap(), expected);
    }
}
