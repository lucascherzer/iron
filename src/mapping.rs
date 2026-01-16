use dashmap::DashMap;
use iroh::NodeId;
use std::net::Ipv6Addr;

/// Manages the bi-directional mapping between Iroh NodeIds and IPv6 addresses.
pub struct Registry {
    // We could have invalid states here. To be safe, we need to guarantee that
    // none of our functions can put the registry into an invalid state.
    // This means extensive unit tests.
    pubkey_to_ip: DashMap<NodeId, Ipv6Addr>,
    ip_to_pubkey: DashMap<Ipv6Addr, NodeId>,
}

impl Registry {
    pub fn new() -> Self {
        todo!()
    }

    /// Gets or creates an IPv6 address for a given NodeId.
    pub fn get_or_assign_ip(&self, node_id: NodeId) -> Ipv6Addr {
        todo!()
    }

    /// Resolves an IPv6 address back to a NodeId.
    pub fn get_node_id(&self, ip: &Ipv6Addr) -> Option<NodeId> {
        todo!()
    }

    /// Derives a stable IPv6 address from a NodeId (deterministic hashing).
    fn derive_ip(&self, node_id: &NodeId) -> Ipv6Addr {
        todo!()
    }
}
