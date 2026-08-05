#!/usr/bin/env python3
"""LANGUAGE-LAWS census (spec §4 Stage M): run the checker over the
tree under VERUM_LANGUAGE_LAWS=warn and count E430/E431/E432 warning
lines. Stage M is done when this prints zero under strict candidates;
Stage E wires it as a CI gate (fail on any).

Usage: census_language_laws.py <verum-binary> [paths...]
Default paths: vcs/specs core-tests. Prints per-code counts and the
top offender files.
"""
import subprocess, sys, pathlib, collections, re

ROOT = pathlib.Path(__file__).resolve().parents[2]
BIN = sys.argv[1] if len(sys.argv) > 1 else "verum"
PATHS = sys.argv[2:] or ["vcs/specs", "core-tests"]

pat = re.compile(r"warning<(E43[012])>")
counts = collections.Counter()
files = collections.Counter()
n = 0
for base in PATHS:
    for f in sorted((ROOT / base).rglob("*.vr")):
        n += 1
        try:
            r = subprocess.run(
                [BIN, "check", str(f)],
                capture_output=True, text=True, timeout=120,
                env={**__import__("os").environ, "VERUM_LANGUAGE_LAWS": "warn"},
            )
        except subprocess.TimeoutExpired:
            continue
        hits = pat.findall(r.stderr or "")
        for h in hits:
            counts[h] += 1
        if hits:
            files[str(f.relative_to(ROOT))] += len(hits)

print(f"census over {n} files: " + (", ".join(f"{k}={v}" for k, v in sorted(counts.items())) or "0 violations"))
for f, c in files.most_common(15):
    print(f"  {c:4}  {f}")
sys.exit(0)
