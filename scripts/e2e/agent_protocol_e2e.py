#!/usr/bin/env python3
"""E2E client for the Verum Agent Protocol skeleton (T0853 v1).

Drives the accepted-design agent cycle over Content-Length frames:
session.open → parse.check(content) → arch.query(content) →
session.journal → shutdown — and VERIFIES the laws, not just liveness:
content addressing (the answer cites the sha256 of what we sent),
the envelope shape, journal completeness, and stream ownership
(stdout parses as frames start to finish).
"""
import hashlib
import json
import subprocess
import sys

VERUM = sys.argv[1]

ESCALATING = """
@arch_module(
    foundation: Foundation.ZfcTwoInacc,
    lifecycle: Lifecycle.Definition,
    requires: [Capability.Read(ResourceTag.Logger)],
)
module fixtures.escalating;

fn leaf() { core.net.tcp.connect("evil.example", 443); }
public fn entry() { leaf(); }
"""

proc = subprocess.Popen(
    [VERUM, "serve", "--agent"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
)

def send(obj):
    body = json.dumps(obj).encode()
    proc.stdin.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    proc.stdin.flush()

def recv():
    header = b""
    while not header.endswith(b"\r\n\r\n"):
        b1 = proc.stdout.read(1)
        assert b1, "server closed mid-frame"
        header += b1
    length = int(header.decode().split("Content-Length:")[1].split("\r\n")[0])
    return json.loads(proc.stdout.read(length))

failures = []

def check(name, cond, detail=""):
    if cond:
        print(f"  ok: {name}")
    else:
        failures.append(name)
        print(f"  FAIL: {name} {detail}")

# 1. session.open
send({"jsonrpc": "2.0", "id": 1, "method": "session.open", "params": {}})
r = recv()
env = r["result"]
check("open: envelope has protocol_version", env.get("protocol_version") == "1")
check("open: session_id present", bool(env["data"].get("session_id")))
check("open: arch.query advertised", "arch.query" in env["data"]["methods"])

# 2. parse.check with a content buffer
send({"jsonrpc": "2.0", "id": 2, "method": "parse.check",
      "params": {"content": ESCALATING}})
r = recv()
env = r["result"]
expected_hash = hashlib.sha256(ESCALATING.encode()).hexdigest()
check("parse: ok", env["data"]["ok"] is True, str(env["data"])[:120])
check("parse: content-address law", env.get("content_hash") == expected_hash,
      f"got {env.get('content_hash')}")

# 3. arch.query — the verdict answer with provenance
send({"jsonrpc": "2.0", "id": 3, "method": "arch.query",
      "params": {"content": ESCALATING}})
r = recv()
env = r["result"]
data = env["data"]
check("query: content-address law", env.get("content_hash") == expected_hash)
check("query: provenance computed_from matches",
      env["provenance"]["computed_from"]["content_sha256"] == expected_hash)
check("query: escalation caught",
      any("Network" in a["atom"] for a in (data.get("escalations") or [])),
      str(data.get("escalations"))[:160])
check("query: dead right caught",
      any("Logger" in d for d in (data.get("dead_rights") or [])))

# 4. unknown method → error, session survives
send({"jsonrpc": "2.0", "id": 4, "method": "no.such.method", "params": {}})
r = recv()
check("unknown method errors without killing session", "error" in r)

# 5. journal — the ledger holds every request with hashes
send({"jsonrpc": "2.0", "id": 5, "method": "session.journal", "params": {}})
r = recv()
entries = r["result"]["data"]["entries"]
methods = [e["method"] for e in entries]
check("journal: open recorded", "session.open" in methods)
check("journal: query recorded with hash",
      any(e["method"] == "arch.query" and e.get("content_hash") == expected_hash
          for e in entries))

# 5b. test.run oracle — green on a passing program, red on a failing
# one, and the journal carries FRAME hashes (K-4 seam law).
import tempfile, os
green_src = 'fn main() -> Int { assert(1 + 1 == 2); 0 }'
red_src = 'fn main() -> Int { assert(1 + 1 == 3); 0 }'
gpath = tempfile.mktemp(suffix=".vr"); open(gpath, "w").write(green_src)
rpath = tempfile.mktemp(suffix=".vr"); open(rpath, "w").write(red_src)

send({"jsonrpc": "2.0", "id": 50, "method": "test.run",
      "params": {"path": gpath, "budget_s": 120}})
r = recv()
check("test.run: green verdict", r["result"]["data"]["verdict"] == "green",
      str(r["result"]["data"]))
send({"jsonrpc": "2.0", "id": 51, "method": "test.run",
      "params": {"path": rpath, "budget_s": 120}})
r = recv()
check("test.run: red verdict", r["result"]["data"]["verdict"] == "red",
      str(r["result"]["data"]))
os.unlink(gpath); os.unlink(rpath)

send({"jsonrpc": "2.0", "id": 52, "method": "session.journal", "params": {}})
r = recv()
entries = r["result"]["data"]["entries"]
check("journal: frame hashes stamped (K-4)",
      all(e.get("request_frame_sha256") for e in entries
          if e["method"] != "session.journal" or True),
      str([{k: bool(v) for k, v in e.items()} for e in entries[-3:]]))
check("journal: responses hashed on respondables",
      any(e["method"] == "test.run" and e.get("response_frame_sha256")
          for e in entries))

# 6. shutdown
send({"jsonrpc": "2.0", "id": 6, "method": "shutdown", "params": {}})
r = recv()
check("shutdown acknowledged", r["result"]["data"]["ok"] is True)

proc.wait(timeout=10)
leftover = proc.stdout.read()
check("stream ownership: no bytes after final frame", leftover == b"",
      repr(leftover[:60]))

print()
if failures:
    print(f"E2E: {len(failures)} FAILURES: {failures}")
    sys.exit(1)
print("E2E: all protocol laws hold")
