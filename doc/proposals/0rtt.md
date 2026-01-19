# Proposal: 0-RTT Connections

In order to minimize the impact of handshakes on the network performance,
we could add support for zero-round-trip-time connections.

# Limitations
The iroh blog mentions replay attacks being a known problem of 0-RTT
connections. We would therefore need to carefully examine where this would be
beneficial.

# Resources
https://www.iroh.computer/blog/0rtt-api
