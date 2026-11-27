"""Generate tests/vectors/rcon.json from the ORIGINAL rcon.py (no network: a fake socket).

Run:  python3 -I rust/ark-ctl/scripts/gen-vectors.py
The vectors are what rcon.py returns for synthetic inputs; the Rust tests must reproduce them.
"""
import importlib.util
import json
import os
import struct

root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
spec = importlib.util.spec_from_file_location("rcon", os.path.join(root, "rcon.py"))
rcon = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rcon)


class FakeSock:
    """Answers each command with the queued payloads, using the request id it was sent."""

    def __init__(self, replies, auth_ok=True):
        self.buf = b""
        self.replies = replies
        self.auth_ok = auth_ok
        self.timeout = None

    def settimeout(self, t):
        self.timeout = t

    def sendall(self, data):
        _, rid, ptype = struct.unpack("<iii", data[:12])
        if ptype == rcon.SERVERDATA_AUTH:
            self.buf += rcon.build_packet(rid if self.auth_ok else -1, rcon.SERVERDATA_AUTH_RESPONSE, "")
        else:
            for kind, payload in self.replies:
                pid = rid if kind != "other" else rid + 1
                self.buf += rcon.build_packet(pid, rcon.SERVERDATA_RESPONSE_VALUE, payload)

    def recv(self, n):
        out, self.buf = self.buf[:n], self.buf[n:]
        return out


vectors = {"packets": [], "decode": [], "commands": [], "players": []}

for rid, ptype, payload in [
    (1, 3, "pw"), (0, 2, ""), (7, 2, "ListPlayers"), (2147483647, 0, "x"), (-1, 2, "héllo \U0001f996"),
    (5, 2, "a" * 300), (9, 3, "pass word"),
]:
    pkt = rcon.build_packet(rid, ptype, payload)
    vectors["packets"].append({"id": rid, "type": ptype, "payload": payload, "hex": pkt.hex()})

import socket
for rid, ptype, payload in [(3, 0, "ok"), (4, 2, ""), (-1, 2, ""), (11, 0, "hé \U0001f996"), (12, 0, "bad \xff".encode("latin-1").decode("utf-8", "replace"))]:
    pkt = rcon.build_packet(rid, ptype, payload)
    a, b = socket.socketpair()
    a.sendall(pkt)
    a.close()
    r_id, r_type, r_payload = rcon.read_packet(b)
    b.close()
    vectors["decode"].append({"hex": pkt.hex(), "id": r_id, "type": r_type, "payload": r_payload})

for name, replies in {
    "single": [("match", "hello")],
    "two_packets": [("match", "part one "), ("match", "part two")],
    "keep_alive_first": [("match", "Keep Alive"), ("match", "real")],
    "other_id_second": [("match", "first"), ("other", "ignored")],
    "empty": [("match", "")],
}.items():
    sock = FakeSock(replies)
    out = rcon.rcon_command(sock, "Cmd")
    vectors["commands"].append({"name": name, "replies": replies, "output": out})

for name, text in {
    "empty": "",
    "no_players": "No Players Connected",
    "no_players_upper": "NO PLAYERS here",
    "two": "0. Alice, 0002abc\n1. Bob, 0003def\n",
    "unnumbered": "header\nplayer a\nplayer b",
    "mixed": "Header\n1. A, x\nfooter\n2. B, y",
    "blank_lines": "\n\n0. A, 1\n\n",
    "indented": "  3. A, 1\n  4. B, 2",
}.items():
    sock = FakeSock([("match", text)])
    vectors["players"].append({"name": name, "text": text, "count": rcon.get_player_count(sock)})

out = os.path.join(root, "rust", "ark-ctl", "tests", "vectors", "rcon.json")
json.dump(vectors, open(out, "w"), indent=1, ensure_ascii=False)
print("wrote", out, {k: len(v) for k, v in vectors.items()})
