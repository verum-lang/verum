#!/usr/bin/env python3
"""T0230 — stdlib proof-verification ratchet gate.

core/ carries ~692 theorem declarations that no build step verifies
(the bake runs no SMT; user builds consume the baked archive). This
gate makes the corpus's proof state a MONOTONE contract without
turning the 113 historically-unproved theorems into a merge blocker:

  * a file recorded `clean` must stay clean;
  * a runnable file's `proved` count must never decrease;
  * a runnable file must not become unverifiable-standalone;
  * improvements (more proved, failures cured, unverifiable now
    runnable) PASS with a note asking for a manifest reseed, so the
    ratchet only ever tightens.

Per-file counts include theorems pulled in through mounted modules —
that is fine for a per-file ratchet (like compares with like); the
corpus-total shown is therefore an upper bound, not a dedup count.

Usage:
  proof_gate.py [--bin PATH] [--seed] [--timeout SECS] [--manifest PATH]

Exit codes: 0 pass, 1 regression(s), 2 harness error.
"""
import argparse
import glob
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
DEFAULT_MANIFEST = os.path.join(ROOT, "scripts", "ci", "proof_gate_expectations.tsv")
SUMMARY_RE = re.compile(r"Summary: (\d+) proved, (\d+) failed, (\d+) timeout, (\d+) skipped")


def theorem_files():
    pat = re.compile(r"^\s*(public\s+)?(theorem|lemma|corollary)\s", re.M)
    out = []
    for f in sorted(glob.glob(os.path.join(ROOT, "core", "**", "*.vr"), recursive=True)):
        try:
            if pat.search(open(f, errors="replace").read()):
                out.append(os.path.relpath(f, ROOT))
        except OSError:
            pass
    return out


def run_one(binary, rel, timeout):
    try:
        proc = subprocess.run(
            [binary, "verify", rel],
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return ("unverifiable", 0, 0, "file-timeout")
    text = proc.stdout + proc.stderr
    sums = SUMMARY_RE.findall(text)
    if not sums:
        first_err = next(
            (l.strip() for l in text.splitlines() if "error" in l.lower()), "no summary"
        )
        return ("unverifiable", 0, 0, first_err[:160])
    proved, failed, timeo, _skipped = map(int, sums[-1])
    status = "clean" if failed == 0 and timeo == 0 else "failures"
    return (status, proved, failed, "")


def load_manifest(path):
    rows = {}
    for line in open(path):
        if not line.strip() or line.startswith("#"):
            continue
        f, status, proved, failed = line.rstrip("\n").split("\t")
        rows[f] = (status, int(proved), int(failed))
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=os.environ.get("VERUM_BIN", os.path.join(ROOT, "target", "release", "verum")))
    ap.add_argument("--manifest", default=DEFAULT_MANIFEST)
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--seed", action="store_true", help="write the manifest from the current run")
    args = ap.parse_args()

    if not os.path.exists(args.bin):
        print(f"proof-gate: verum binary not found at {args.bin} (set VERUM_BIN)", file=sys.stderr)
        return 2

    files = theorem_files()
    results = {}
    for rel in files:
        results[rel] = run_one(args.bin, rel, args.timeout)

    if args.seed:
        with open(args.manifest, "w") as f:
            f.write("# T0230 stdlib proof-gate ratchet manifest — regenerate with proof_gate.py --seed\n")
            f.write("# file\tstatus\tproved\tfailed\n")
            for rel in sorted(results):
                status, proved, failed, _ = results[rel]
                f.write(f"{rel}\t{status}\t{proved}\t{failed}\n")
        print(f"proof-gate: seeded {len(results)} rows into {args.manifest}")
        return 0

    expected = load_manifest(args.manifest)
    regressions, improvements, new_files = [], [], []
    for rel, (status, proved, failed, note) in sorted(results.items()):
        exp = expected.get(rel)
        if exp is None:
            new_files.append(rel)
            continue
        e_status, e_proved, _e_failed = exp
        if e_status in ("clean", "failures") and status == "unverifiable":
            regressions.append(f"{rel}: was {e_status}, now UNVERIFIABLE ({note})")
        elif e_status == "clean" and status == "failures":
            regressions.append(f"{rel}: was clean, now {failed} failed")
        elif status in ("clean", "failures") and proved < e_proved:
            regressions.append(f"{rel}: proved count fell {e_proved} → {proved}")
        elif (e_status == "failures" and status == "clean") or (
            status in ("clean", "failures") and proved > e_proved
        ) or (e_status == "unverifiable" and status != "unverifiable"):
            improvements.append(rel)
    for rel in expected:
        if rel not in results:
            regressions.append(f"{rel}: in manifest but no longer theorem-bearing — reseed consciously")

    total_proved = sum(r[1] for r in results.values())
    total_failed = sum(r[2] for r in results.values())
    print(
        f"proof-gate: {len(results)} files; per-file proved sum {total_proved}, failed sum {total_failed}"
    )
    if new_files:
        print(f"proof-gate: {len(new_files)} NEW theorem-bearing file(s) — add via --seed: " + ", ".join(new_files))
    if improvements:
        print(f"proof-gate: {len(improvements)} improvement(s) — tighten the ratchet with --seed: " + ", ".join(improvements))
    if regressions:
        print("proof-gate: REGRESSIONS:", file=sys.stderr)
        for r in regressions:
            print(f"  ✗ {r}", file=sys.stderr)
        return 1
    print("proof-gate: OK (ratchet holds)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
