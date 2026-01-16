# iron architecture planning

## Introduction

`iron` is a project that aims to provide a "flat" network based on iroh's
dial-by-public-key approach.
We do this because it has become burdensome for end users to communicate in a
peer-to-peer fashion, as workarounds like NAT in combination with public
infrastructure like DNS and CAs place hard restrictions on users.

[iroh](https://www.iroh.computer/) already solves a lot of these problems by
addressing the routing issue, but it is so far only accessible as a library for
application developers. 

`iron` extends iroh's sphere of influence to the operating system level, like
`i2p` does, by providing a network interface and a resolver to route addresses
natively under the `.iron` TLD. 

The name `iron` is a shorthand for "iroh-onion" as this project is intended to
also support onion routing, which has been implemented in a separate crate.
We leave it out during the initial prototype, but plan to add support later. It 
also makes for a good TLD.

# Components

We require two components for this to work:
1. A `.iron` resolver which can map `<pubkey>.iron` to a temporary IPv6 address
  in the Unique Local Address (ULA) space. The mapping can be random, as long as
  it stays consistent. For close integration with existing software, it needs
  to be able to resolve `.iron` DNS queries.
2. A tun interface that facilitates communication to the outside network,
  advertising to route addresses within the ULA address spaces. It uses the
  resolver in reverse, taking the IPv6 address and getting its associated iroh
  public key. And sending the data off.

# Third Party Software
- iroh (https://docs.rs/iroh/latest/iroh/)
- tun (https://docs.rs/tun/0.8.5/tun)
- for shared memory or general communication between resolver and tun device,
  rapace seems like a good fit

# Considerations During Development

1. Do we need multiple processes (resolver, tun device), or is threading
  enough?
  1.1 if we need processes, how do we expose the reverse resolving to the tun
    device in a platform independent way? Sockets on Unix? (can we use https://rapace.bearcove.eu/ ?)
2. Do we have to be no-std for implementing the tun driver?
3. Platform support is unfortunately very important, since this software needs 
  to run on each device which wants to join a network. Linux, Mac, Windows is
  the minimum, Android, and iOS are also nice to have and should not be
  categorically ruled out by any architectural decisions should we choose to
  implement them later unless absolutely necessary.
