//! `.iron` DNS resolution.
//!
//! [`DnsResolver`] works on raw DNS messages (bytes in, bytes out), so the
//! same resolution logic serves every way a query can reach iron:
//!
//! - [`serve_udp`]: a UDP socket. On desktop the OS's split-DNS config
//!   (`iron_desktop::dns_config`) sends `.iron` queries to `127.0.0.1:5333`.
//! - TUN interception: queries sent to [`MAGIC_DNS_ADDR`] arrive as IPv6/UDP
//!   packets on the TUN device and are answered by
//!   [`crate::router::PacketRouter`]. This is how Android will deliver DNS,
//!   since `VpnService` can only point the OS resolver at an address inside
//!   the tunnel.
//!
//! Because a tunnel DNS server receives *all* of the device's queries,
//! non-`.iron` names can be forwarded to upstream servers ([`UdpUpstream`]).
//! Without upstreams they are refused, which is correct on desktop where
//! split DNS only ever sends `.iron` names here.

use crate::mapping::Registry;
use anyhow::{Result, anyhow};
use hickory_proto::op::{Header, Message, ResponseCode};
use hickory_proto::rr::{Name, RData, Record, RecordType};
use iroh::EndpointId;
use std::net::{Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tracing::{debug, info, trace, warn};

/// In-tunnel DNS server address (UDP port 53).
///
/// Collision-free by construction: [`Registry::derive_ip`] always produces
/// `fd69:726f:0:0:…` (groups 3–4 are zero), so nothing in the
/// `fd69:726f:0:1::/64` subnet can ever be a peer address, not even for a
/// deliberately ground vanity key. It still lies inside the
/// `fd69:726f::/32` route, so no extra route is needed on any platform.
pub const MAGIC_DNS_ADDR: Ipv6Addr = Ipv6Addr::new(0xfd69, 0x726f, 0, 1, 0, 0, 0, 0x53);

/// TTL of synthesized `.iron` AAAA records.
const IRON_RECORD_TTL: u32 = 300;

/// How long to wait for one upstream server before trying the next.
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(3);

/// Largest UDP datagram we accept.
const MAX_UDP_MESSAGE: usize = 65535;

/// Forwards raw DNS messages to plain-UDP upstream servers, in order.
///
/// On Android the upstream will be a loopback proxy that relays through the
/// platform resolver (honoring Private DNS), which is still just a UDP
/// address from iron's point of view.
#[derive(Clone, Debug)]
pub struct UdpUpstream {
    servers: Vec<SocketAddr>,
    timeout: Duration,
}

impl UdpUpstream {
    pub fn new(servers: Vec<SocketAddr>) -> Self {
        Self {
            servers,
            timeout: UPSTREAM_TIMEOUT,
        }
    }

    /// Sends `query` to each server in turn and returns the first reply
    /// whose DNS ID matches the query.
    async fn query(&self, query: &[u8]) -> Result<Vec<u8>> {
        let id = query.get(..2).ok_or_else(|| anyhow!("short DNS query"))?;
        for server in &self.servers {
            match self.query_one(*server, query, id).await {
                Ok(reply) => return Ok(reply),
                Err(e) => debug!("DNS upstream {} failed: {}", server, e),
            }
        }
        Err(anyhow!("all DNS upstreams failed"))
    }

    async fn query_one(&self, server: SocketAddr, query: &[u8], id: &[u8]) -> Result<Vec<u8>> {
        let bind = if server.is_ipv4() {
            SocketAddr::from(([0, 0, 0, 0], 0))
        } else {
            SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0))
        };
        let socket = UdpSocket::bind(bind).await?;
        socket.connect(server).await?;
        socket.send(query).await?;

        let mut reply = vec![0; MAX_UDP_MESSAGE];
        let len = tokio::time::timeout(self.timeout, socket.recv(&mut reply))
            .await
            .map_err(|_| anyhow!("timed out"))??;
        reply.truncate(len);
        if reply.get(..2) != Some(id) {
            return Err(anyhow!("reply ID does not match query"));
        }
        Ok(reply)
    }
}

/// Message-level `.iron` resolver, shared by all DNS frontends.
pub struct DnsResolver {
    registry: Arc<Registry>,
    upstream: Option<UdpUpstream>,
}

impl DnsResolver {
    /// `upstream` servers receive non-`.iron` queries; if empty, those are
    /// answered with REFUSED.
    pub fn new(registry: Arc<Registry>, upstream: Vec<SocketAddr>) -> Self {
        Self {
            registry,
            upstream: (!upstream.is_empty()).then(|| UdpUpstream::new(upstream)),
        }
    }

    /// Resolves a raw DNS query message into a raw response message.
    ///
    /// - `<base32>.iron` AAAA: the peer's derived IPv6 (authoritative)
    /// - other types for `.iron`: authoritative empty NOERROR
    /// - undecodable `.iron` label: NXDOMAIN
    /// - non-`.iron`: forwarded upstream (SERVFAIL if all fail), or REFUSED
    ///   without upstreams
    /// - malformed message: FORMERR if an ID can be read, otherwise `None`
    ///   (nothing to reply to)
    pub async fn resolve(&self, query: &[u8]) -> Option<Vec<u8>> {
        let Ok(message) = Message::from_vec(query) else {
            return form_error(query);
        };
        let Some(question) = message.queries().first() else {
            return Some(response(&message, ResponseCode::FormErr, false, vec![]));
        };
        let name = question.name();

        if !is_iron_name(name) {
            return Some(match &self.upstream {
                Some(upstream) => match upstream.query(query).await {
                    Ok(reply) => reply,
                    Err(e) => {
                        warn!("Forwarding {} upstream failed: {}", name, e);
                        response(&message, ResponseCode::ServFail, false, vec![])
                    }
                },
                None => response(&message, ResponseCode::Refused, false, vec![]),
            });
        }

        trace!("DNS query: {} {:?}", name, question.query_type());
        if question.query_type() != RecordType::AAAA {
            return Some(response(&message, ResponseCode::NoError, true, vec![]));
        }

        match parse_endpoint_from_domain(name) {
            Some(endpoint_id) => {
                let ip = self.registry.get_or_assign_ip(endpoint_id);
                debug!("Resolved {} -> {}", name, ip);
                let record =
                    Record::from_rdata(name.clone(), IRON_RECORD_TTL, RData::AAAA(ip.into()));
                Some(response(
                    &message,
                    ResponseCode::NoError,
                    true,
                    vec![record],
                ))
            }
            None => {
                warn!("Failed to parse EndpointId from domain: {}", name);
                Some(response(&message, ResponseCode::NXDomain, true, vec![]))
            }
        }
    }
}

/// Serves DNS over UDP on `listen` until the socket fails.
pub async fn serve_udp(resolver: Arc<DnsResolver>, listen: SocketAddr) -> Result<()> {
    let socket = Arc::new(UdpSocket::bind(listen).await?);
    info!("DNS server listening on {}", listen);
    let mut buf = vec![0; MAX_UDP_MESSAGE];
    loop {
        let (len, peer) = socket.recv_from(&mut buf).await?;
        let query = buf[..len].to_vec();
        let socket = Arc::clone(&socket);
        let resolver = Arc::clone(&resolver);
        tokio::spawn(async move {
            if let Some(reply) = resolver.resolve(&query).await
                && let Err(e) = socket.send_to(&reply, peer).await
            {
                warn!("Failed to send DNS response to {}: {}", peer, e);
            }
        });
    }
}

/// True for names in the `.iron` zone, case-insensitively. Anything that
/// isn't `.iron` may be forwarded upstream, so a case mismatch here would
/// leak peer lookups to third-party resolvers.
fn is_iron_name(name: &Name) -> bool {
    name.to_lowercase().to_string().ends_with(".iron.")
}

/// Parses `<base32 EndpointId>.iron.` (base32 without padding,
/// case-insensitive; 52 chars, fits a single DNS label).
fn parse_endpoint_from_domain(name: &Name) -> Option<EndpointId> {
    let text = name.to_lowercase().to_string();
    let label = match text.split('.').collect::<Vec<_>>().as_slice() {
        [label, "iron", ""] => *label,
        _ => return None,
    };
    let bytes = data_encoding::BASE32_NOPAD
        .decode(label.to_uppercase().as_bytes())
        .ok()?;
    EndpointId::from_bytes(&bytes.try_into().ok()?).ok()
}

fn response(
    request: &Message,
    code: ResponseCode,
    authoritative: bool,
    answers: Vec<Record>,
) -> Vec<u8> {
    let mut header = Header::response_from_request(request.header());
    header.set_response_code(code);
    header.set_authoritative(authoritative);
    let mut response = Message::new();
    response.set_header(header);
    for query in request.queries() {
        response.add_query(query.clone());
    }
    response.answers_mut().extend(answers);
    response.to_vec().unwrap_or_default()
}

/// FORMERR for a message we couldn't parse, if it's long enough to carry an
/// ID the client can match.
fn form_error(query: &[u8]) -> Option<Vec<u8>> {
    let id = query.get(..2)?;
    let mut header = Header::new();
    header.set_id(u16::from_be_bytes([id[0], id[1]]));
    let mut header = Header::response_from_request(&header);
    header.set_response_code(ResponseCode::FormErr);
    let mut response = Message::new();
    response.set_header(header);
    response.to_vec().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_endpoint_id;
    use hickory_proto::op::Query;
    use std::str::FromStr;

    fn iron_domain(endpoint_id: EndpointId) -> String {
        format!(
            "{}.iron.",
            data_encoding::BASE32_NOPAD
                .encode(endpoint_id.as_bytes())
                .to_lowercase()
        )
    }

    fn query_bytes(id: u16, name: &str, record_type: RecordType) -> Vec<u8> {
        let mut message = Message::new();
        message.set_id(id);
        message.set_recursion_desired(true);
        message.add_query(Query::query(Name::from_str(name).unwrap(), record_type));
        message.to_vec().unwrap()
    }

    fn resolver() -> (Arc<Registry>, DnsResolver) {
        let registry = Arc::new(Registry::new());
        let resolver = DnsResolver::new(Arc::clone(&registry), vec![]);
        (registry, resolver)
    }

    #[test]
    fn test_magic_dns_addr_never_collides_with_peers() {
        let prefix = |ip: Ipv6Addr| ip.segments()[..2] == [0xfd69, 0x726f];
        assert!(
            prefix(MAGIC_DNS_ADDR),
            "magic addr must be inside the routed /32"
        );
        for i in 0..=255u8 {
            let peer = Registry::derive_ip(test_endpoint_id(i));
            assert_ne!(peer.segments()[..4], MAGIC_DNS_ADDR.segments()[..4]);
        }
    }

    #[test]
    fn test_parse_endpoint_from_domain() {
        let endpoint_id = test_endpoint_id(42);
        let lower = iron_domain(endpoint_id);
        let cases = [
            (lower.clone(), Some(endpoint_id)),
            (lower.to_uppercase(), Some(endpoint_id)),
            ("example.com.".to_string(), None),
            ("invalidbase32withlowercase.iron.".to_string(), None),
            (format!("sub.{}", lower), None),
        ];
        for (domain, expected) in cases {
            let name = Name::from_str(&domain).unwrap();
            assert_eq!(
                parse_endpoint_from_domain(&name),
                expected,
                "parsing {domain}"
            );
        }
    }

    #[test]
    fn test_is_iron_name_is_case_insensitive() {
        let cases = [
            ("a.iron.", true),
            ("A.IRON.", true),
            ("a.Iron.", true),
            ("iron.com.", false),
            ("a.ironx.", false),
        ];
        for (domain, expected) in cases {
            assert_eq!(
                is_iron_name(&Name::from_str(domain).unwrap()),
                expected,
                "{domain}"
            );
        }
    }

    /// (query name, type) -> (response code, authoritative, answer count)
    #[tokio::test]
    async fn test_resolve_iron_queries() {
        let (registry, resolver) = resolver();
        let endpoint_id = test_endpoint_id(7);
        let domain = iron_domain(endpoint_id);
        let cases = [
            (
                domain.clone(),
                RecordType::AAAA,
                ResponseCode::NoError,
                true,
                1,
            ),
            (
                domain.to_uppercase(),
                RecordType::AAAA,
                ResponseCode::NoError,
                true,
                1,
            ),
            (
                domain.clone(),
                RecordType::A,
                ResponseCode::NoError,
                true,
                0,
            ),
            (
                "notbase32.iron.".to_string(),
                RecordType::AAAA,
                ResponseCode::NXDomain,
                true,
                0,
            ),
            (
                "example.com.".to_string(),
                RecordType::AAAA,
                ResponseCode::Refused,
                false,
                0,
            ),
        ];
        for (name, record_type, code, authoritative, answers) in cases {
            let reply = resolver
                .resolve(&query_bytes(0x1234, &name, record_type))
                .await
                .unwrap();
            let reply = Message::from_vec(&reply).unwrap();
            assert_eq!(reply.id(), 0x1234, "{name}: ID echoed");
            assert_eq!(reply.response_code(), code, "{name} {record_type}: code");
            assert_eq!(reply.authoritative(), authoritative, "{name}: AA flag");
            assert_eq!(reply.answers().len(), answers, "{name}: answer count");
        }

        // The AAAA answer is the peer's derived address, and resolving
        // registers the peer so packets to that address can be routed.
        let reply = resolver
            .resolve(&query_bytes(1, &domain, RecordType::AAAA))
            .await
            .unwrap();
        let reply = Message::from_vec(&reply).unwrap();
        let expected = Registry::derive_ip(endpoint_id);
        assert_eq!(reply.answers()[0].data(), &RData::AAAA(expected.into()));
        assert_eq!(registry.get_endpoint_id(&expected), Some(endpoint_id));
    }

    #[tokio::test]
    async fn test_resolve_malformed() {
        let (_, resolver) = resolver();
        assert_eq!(resolver.resolve(&[]).await, None, "nothing to reply to");
        assert_eq!(resolver.resolve(&[0xab]).await, None, "no complete ID");

        let reply = resolver.resolve(&[0xab, 0xcd, 0xff]).await.unwrap();
        let reply = Message::from_vec(&reply).unwrap();
        assert_eq!(reply.id(), 0xabcd);
        assert_eq!(reply.response_code(), ResponseCode::FormErr);
    }

    /// Fake upstream answering every query with a NOERROR response that
    /// echoes the query's ID and question.
    async fn fake_upstream() -> SocketAddr {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = socket.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = vec![0; MAX_UDP_MESSAGE];
            while let Ok((len, peer)) = socket.recv_from(&mut buf).await {
                let query = Message::from_vec(&buf[..len]).unwrap();
                let reply = response(&query, ResponseCode::NoError, false, vec![]);
                socket.send_to(&reply, peer).await.unwrap();
            }
        });
        addr
    }

    #[tokio::test]
    async fn test_resolve_forwards_non_iron_upstream() {
        let upstream = fake_upstream().await;
        let resolver = DnsResolver::new(Arc::new(Registry::new()), vec![upstream]);
        let reply = resolver
            .resolve(&query_bytes(0x4242, "example.com.", RecordType::A))
            .await
            .unwrap();
        let reply = Message::from_vec(&reply).unwrap();
        assert_eq!(reply.id(), 0x4242);
        assert_eq!(reply.response_code(), ResponseCode::NoError);
        assert!(
            !reply.authoritative(),
            "came from the fake upstream, not from us"
        );
    }

    #[tokio::test]
    async fn test_resolve_iron_is_never_forwarded() {
        // Upstream that would answer; `.iron` must still be answered locally.
        let upstream = fake_upstream().await;
        let resolver = DnsResolver::new(Arc::new(Registry::new()), vec![upstream]);
        let domain = iron_domain(test_endpoint_id(3)).to_uppercase();
        let reply = resolver
            .resolve(&query_bytes(1, &domain, RecordType::AAAA))
            .await
            .unwrap();
        let reply = Message::from_vec(&reply).unwrap();
        assert!(reply.authoritative());
        assert_eq!(reply.answers().len(), 1);
    }

    #[tokio::test]
    async fn test_upstream_falls_back_and_servfails() {
        // A bound socket that never answers.
        let silent = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let silent_addr = silent.local_addr().unwrap();
        let working = fake_upstream().await;

        let mut resolver = DnsResolver::new(Arc::new(Registry::new()), vec![silent_addr, working]);
        resolver.upstream.as_mut().unwrap().timeout = Duration::from_millis(100);
        let query = query_bytes(9, "example.com.", RecordType::A);

        // Silent first server times out, second answers.
        let reply = Message::from_vec(&resolver.resolve(&query).await.unwrap()).unwrap();
        assert_eq!(reply.response_code(), ResponseCode::NoError);

        // Only the silent server: the client gets SERVFAIL instead of a timeout.
        resolver.upstream.as_mut().unwrap().servers = vec![silent_addr];
        let reply = Message::from_vec(&resolver.resolve(&query).await.unwrap()).unwrap();
        assert_eq!(reply.response_code(), ResponseCode::ServFail);
        drop(silent);
    }

    #[tokio::test]
    async fn test_serve_udp_answers_queries() {
        let (_, resolver) = resolver();
        let probe = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let listen = probe.local_addr().unwrap();
        drop(probe);
        tokio::spawn(serve_udp(Arc::new(resolver), listen));
        tokio::time::sleep(Duration::from_millis(50)).await;

        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let domain = iron_domain(test_endpoint_id(5));
        client
            .send_to(&query_bytes(77, &domain, RecordType::AAAA), listen)
            .await
            .unwrap();
        let mut buf = vec![0; 1500];
        let len = tokio::time::timeout(Duration::from_secs(2), client.recv(&mut buf))
            .await
            .unwrap()
            .unwrap();
        let reply = Message::from_vec(&buf[..len]).unwrap();
        assert_eq!(reply.id(), 77);
        assert_eq!(reply.answers().len(), 1);
    }
}
