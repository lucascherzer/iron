"""TCP echo pair for tests/vm/outage.nix.

server HOST PORT: echo every line back, one connection at a time.
client HOST PORT: open one connection, echo once, wait until /tmp/resume
exists, echo again on the same connection. Logs "echo N ok" per round trip.
"""

import os
import socket
import sys
import time


def server(host, port):
    srv = socket.create_server((host, port), family=socket.AF_INET6)
    while True:
        conn, _ = srv.accept()
        with conn, conn.makefile("rwb", buffering=0) as f:
            for line in f:
                f.write(line)


def client(host, port):
    conn = socket.create_connection((host, port))
    f = conn.makefile("rwb", buffering=0)
    for n in (1, 2):
        if n == 2:
            while not os.path.exists("/tmp/resume"):
                time.sleep(0.5)
        msg = f"ping {n}\n".encode()
        f.write(msg)
        assert f.readline() == msg
        print(f"echo {n} ok", flush=True)


if __name__ == "__main__":
    mode, host, port = sys.argv[1], sys.argv[2], int(sys.argv[3])
    {"server": server, "client": client}[mode](host, port)
