# Implementation Plan - iron

## Phase 1: Foundation & Scaffolding
- [ ] Initialize `Cargo.toml` with dependencies (`iroh`, `tun`, `trust-dns-server`, `tokio`, `dashmap`).
- [ ] Define the core module structure (`mapping`, `dns`, `tun`, `node`).
- [ ] Implement skeleton structs and method signatures with `todo!`.

## Phase 2: Address Mapping (The Registry)
- [ ] Implement a bi-directional mapping store (`PubKey <-> Ipv6Addr`).
- [ ] Strategy: Use a deterministic derivation (hashing) for stable IPs, but cache them in a `BiMap` for O(1) lookups in both directions.

## Phase 3: DNS Resolver
- [ ] Implement a basic DNS server using `trust-dns-server`.
- [ ] Logic: If a query ends in `.iron`, lookup/generate an IPv6 mapping and return it.

## Phase 4: TUN Interface
- [ ] Setup a TUN device using the `tun` crate.
- [ ] Implement a packet loop:
    - **Inbound (from OS):** Read IPv6 packet -> Extract Dest IP -> Lookup PubKey -> Forward to Iroh.
    - **Outbound (from Iroh):** Receive data -> Wrap in IPv6 packet (if needed) -> Write to TUN.

## Phase 5: Iroh Integration
- [ ] Initialize Iroh `Endpoint`.
- [ ] Implement the transport protocol for raw packets over Iroh (likely using ALPN for `iron` traffic).

## Phase 6: CLI & Orchestration
- [ ] Create a main entry point to start all components and manage lifecycle.
