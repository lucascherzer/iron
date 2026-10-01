# An idle TCP connection through iron survives a network outage.
#
# Models a laptop that sleeps or loses Wi-Fi with an SSH session open: the
# link goes fully dark for longer than QUIC's 30s idle timeout, so iron's
# QUIC connections die and must be re-established. The TCP connection inside
# the tunnel was idle throughout, so it must just carry on afterwards.
{ lib }:
lib.mkTest {
  name = "outage";
  testScript = ''
    outage_secs = 45

    b.succeed(f"systemd-run --unit=echo-server --collect --no-block python3 ${./echo.py} server {b_ip} 7000")
    b.wait_for_open_port(7000, addr=b_ip, timeout=30)
    # Opens one TCP connection, echoes once, waits for /tmp/resume, echoes again.
    a.succeed(f"systemd-run --unit=echo-client --collect --no-block python3 ${./echo.py} client {b_ip} 7000")
    a.wait_until_succeeds("journalctl -u echo-client --no-pager | grep -q 'echo 1 ok'", timeout=30)

    with subtest(f"{outage_secs}s outage, then the same TCP connection works"):
        a.succeed(f"tc qdisc add dev {LAN_IF} root netem loss 100%")
        a.sleep(outage_secs)
        a.succeed(f"tc qdisc del dev {LAN_IF} root")
        a.succeed("touch /tmp/resume")
        a.wait_until_succeeds("journalctl -u echo-client --no-pager | grep -q 'echo 2 ok'", timeout=90)
  '';
}
