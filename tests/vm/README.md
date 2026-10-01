# Sandboxed VM integration tests

These Linux-only NixOS checks boot three disposable VMs on a virtual LAN.
The infrastructure VM runs local iroh relay and pkarr/DNS servers; the tests
do not depend on public n0 services.

| Check | What it verifies |
|---|---|
| `iron-vm-ping-pong` | Workspace-pinned iroh endpoints discover a peer by EndpointId using local infrastructure, then exchange ping/pong without iron or a TUN. |
| `iron-vm-two-node` | Iron carries HTTP traffic in both directions, addressed by `<key>.iron` names. |
| `iron-vm-lossy-network` | A 10 MiB TCP transfer addressed by `.iron` name retains its checksum with 5% outgoing loss on both peers. |
| `iron-vm-outage` | The same TCP connection carries new data after a 45-second, 100%-loss blackout. It does not independently assert QUIC expiry or reconnection. |

The shared iron harness verifies DNS first, then connectivity by `.iron` name,
then connectivity by iron IPv6 address. Application clients use `.iron` names
and servers listen on IPv6 any (`::`). Expected `fd69:726f::/32` addresses in
DNS assertions are tunnel addresses, not sandbox LAN addresses. Link faults
are applied to `eth1`, below the tunnel; `eth0` is QEMU user networking.

Run a check with a Linux builder supporting `kvm` and `nixos-test`:

```sh
nix build .#checks.x86_64-linux.iron-vm-two-node --no-link -L
```

Use `--rebuild` to rerun an already built check rather than reuse its result.
Keep new files Git-tracked so the default flake source includes them.
The driver logs every command under named behavior-level subtests, with bounded
readiness waits and service journals on setup failure.

Interactive debugging is available through a check's `.driverInteractive`
attribute, but internet access there is not equivalent to the automated
sandboxed build. Use the automated checks as validation evidence.
