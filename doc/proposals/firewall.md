# Proposal: Firewall

Since the iron network is completely flat, each peer can send requests to all
others.
To increase security, we could implement a "firewall"-like component that
blocks requests based on certain criteria.

My initial considerations:
- Block- vs. whitelist: whitelist seems better for small networks, blocklists do
  not really make sense, since identities can be generated relatively cheaply
- How to configure rules?
  - declarative?
  - User-defined closures? (WASM?)
