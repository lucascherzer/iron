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
Your private key lives in `$XDG_CONFIG_DIR/iron/secret.key`
(usually `~/.config/iron/secret.key`). The key is tied to your iron domain name
and in order for others to not have to change your domain frequently, you should
not change it too often.
You can manage keys with the `iron key` subcommand.

## Pretty Key is not recognized
On linux systems, the key location may not be what `iron key info` reports.
Since it ran as root, the key will have been saved in `/root/.config/iron/secret.key`
instead of your home directory. Follow these steps to fix:
```sh
sudo rm /root/.config/iron/secret.key
sudo mv /home/<your user>/.config/iron/secret.key /root/.config/iron/secret.key
chown root:root /root/.config/iron/secret.key
```
Then restart iron.
