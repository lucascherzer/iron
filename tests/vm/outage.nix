# An idle TCP connection through iron survives a network outage.
#
# Models a laptop that sleeps or loses Wi-Fi with an SSH session open: the
# link goes fully dark for 45 seconds, then the same TCP connection must
# carry new data. This does not independently assert QUIC timeout/reconnection.
{ lib }:
lib.mkTest {
  name = "outage";
  testScript = ''
    outage_secs = 45

    with subtest("A TCP connection echoes before the outage"):
        b.succeed("systemd-run --unit=echo-server --collect --no-block python3 ${./echo.py} server :: 7000")
        b.wait_for_open_port(7000, addr="::1", timeout=timedelta(seconds=30))
        # One connection: echo, wait for /tmp/resume, then echo again.
        a.succeed(f"systemd-run --unit=echo-client --collect --no-block python3 ${./echo.py} client {b_domain} 7000")
        a.wait_until_succeeds("journalctl -u echo-client --no-pager | grep -q 'echo 1 ok'", timeout=timedelta(seconds=30))

    with subtest(f"{outage_secs}s outage, then the same TCP connection works"):
        a.succeed(f"tc qdisc add dev {LAN_IF} root netem loss 100%")
        a.sleep(timedelta(seconds=outage_secs))
        a.succeed(f"tc qdisc del dev {LAN_IF} root")
        a.succeed("touch /tmp/resume")
        a.wait_until_succeeds("journalctl -u echo-client --no-pager | grep -q 'echo 2 ok'", timeout=timedelta(seconds=90))
  '';
}
