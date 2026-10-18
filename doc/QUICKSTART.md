# iron Quick Start

# Installation
1. download executable from https://drive.proton.me/urls/3D5HZV96P0#2N1VjBcwMQOj
2. chmod +x and all that

# Running

The `serve` subcommand starts a local DNS resolver for the `.iron` TLD as well
as a network interface.
```sh
sudo ./iron serve
```
To see your own id:
```sh
./iron self
```

# Connecting to others

I set up a sample HTTP server at http://...

# Good to know

## IPv6
We use IPv6 under the hood, so if some tool does not work, try its ipv6 version.
Example: `ping` does not work, `ping6` does not

## `.iron` TLD not recognized by some browsers
Firefox for example, does not recognize `.iron` as a TLD, so when opening links,
make sure to include `http://` in front

## Encryption
Though for example in browsers, we have to use http instead of https, since we
can not get CA signed TLS certificates for the .iron domain (as it does not
really exist).
This does not mean our traffic is unencrypted or insecure. All p2p traffic is
encrpted using automatically generated keys

## Pretty keys
The keys are actually our domain names, at least the public keys. You can get a
"pretty" key, one starting with a few letters of your choosing.

This is computationally expensive, as we are just brute-forcing key generation
until we find one whose public key has the correct starting sequence.

Anything more than five letters will take long though, depending on your
hardware.

To generate a pretty key:
```
./iron vanity "server" --save
# this will ask you to overwrite your current key, which is fine if you did not
# do anything yet. More on that in the next section
```

## Key lifecycle
Your private key is tied to your iron domain name. The defaults are
`/var/lib/iron/secret.key` on Linux and
`/Library/Application Support/iron/secret.key` on macOS. Configuration files
and known-peer state are stored alongside platform state; see the
[configuration reference](cli.md#configuration) to override paths.
You can manage keys with the `iron key` subcommand.

Legacy state can be moved explicitly with `sudo iron migrate --from OLD_DIR`.
Migration never runs automatically, refuses to overwrite destination state,
and leaves the original directory intact. This compatibility command is
deprecated and planned for removal in the next release. Run one managed iron
node per machine; do not share state files between instances.

For Android, pass explicit app-private key and known-peer paths; the portable
core does not select Android filesystem locations.
