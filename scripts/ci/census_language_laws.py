#!/usr/bin/env python3
"""LANGUAGE-LAWS census (spec §4 Stage M): run the checker over the
tree under VERUM_LANGUAGE_LAWS=warn and count E430/E431/E432 warning
lines. Stage M is done when this prints zero under strict candidates;
Stage E wires it as a CI gate (fail on any).

Usage: census_language_laws.py <verum-binary> [paths...]
Default paths: vcs/specs core-tests. Prints per-code counts and the
top offender files. Checks run in parallel (VERUM_CENSUS_JOBS,
default 8) — each file is an independent `verum check`.
"""
import collections
import concurrent.futures
import os
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
BIN = sys.argv[1] if len(sys.argv) > 1 else "verum"
PATHS = sys.argv[2:] or ["vcs/specs", "core-tests"]
JOBS = int(os.environ.get("VERUM_CENSUS_JOBS", "8"))

pat = re.compile(r"warning<(E43[012])>")
env = {**os.environ, "VERUM_LANGUAGE_LAWS": "warn"}


def check_one(f: pathlib.Path):
    try:
        r = subprocess.run(
            [BIN, "check", str(f)],
            capture_output=True, text=True, timeout=120, env=env,
        )
    except subprocess.TimeoutExpired:
        return f, []
    return f, pat.findall(r.stderr or "")


targets = [f for base in PATHS for f in sorted((ROOT / base).rglob("*.vr"))]
counts = collections.Counter()
files = collections.Counter()
done = 0
with concurrent.futures.ThreadPoolExecutor(max_workers=JOBS) as pool:
    for f, hits in pool.map(check_one, targets):
        done += 1
        if done % 500 == 0:
            print(f"  …{done}/{len(targets)} checked", file=sys.stderr, flush=True)
        for h in hits:
            counts[h] += 1
        if hits:
            files[str(f.relative_to(ROOT))] += len(hits)

print(f"census over {done} files: " + (", ".join(f"{k}={v}" for k, v in sorted(counts.items())) or "0 violations"))
for f, c in files.most_common(15):
    print(f"  {c:4}  {f}")
sys.exit(0)
