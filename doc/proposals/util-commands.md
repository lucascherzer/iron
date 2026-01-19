# Proposal: Utility Commands

The CLI, at this point, is just a fancy way to start the application (resolver
+ tun device).
It could be useful to have some utilities like:
1. Converting the different node formats (hex, base32.iron, IPv6)
2. (Not really a utility)
3. iron self to view info about self (hex, base32.iron, ...)
4. A subcommand for generating keys that result in a base32 representation
   starting with a few desired characters, like onion services that have the
   start of their domain in the onion url.
