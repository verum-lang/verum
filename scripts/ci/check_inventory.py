#!/usr/bin/env python3
"""INVENTORY-LIVENESS-GATE-1 (T0220): the per-module conformance truth
table (`core-tests/INVENTORY.md`) is hand-maintained and rots against
the drifting bake — twice proven (runtime/recovery 'all GREEN' rows red
on the then-current binary; the platform suite authored 2026-06-27 was
never inventoried at all). This gate makes the table self-verifying.

Two layers, so the cheap half always runs:

STRUCTURAL (no test run needed — `--structural-only`):
  S1  every module row is unique (twin of the frozen unit test
      `inventory_module_rows_unique.rs` — duplicated here so the gate
      is self-contained in CI lanes that don't run cargo tests);
  S2  every module row's directory exists under core-tests/;
  S3  every core-tests/ directory carrying *_test.vr files has a row
      (the 'authored but never inventoried' class);
  S4  every row carries a recognized status token — the table's own
      vocabulary, formalized:
          **stable** | **complete**       -> claims interp-green
          **regression-only**             -> claims interp-green
                                             (regression scope)
          **partial**                     -> claims a mixed state
      A row with no token is UNCLAIMED and fails S4 (silent rows are
      exactly how false-green enters the table).

LIVENESS (needs a results file from a real suite run):
  L1  a module whose row claims interp-green (stable/complete/
      regression-only) must measure ZERO failed and ZERO
      compile-error tests at interp (ignored tests are fine — they
      are the pinned-defect channel);
  L2  when the row's prose pins an explicit green count
      (`N GREEN`, `N/M GREEN`, `N passed / 0 failed`), the measured
      pass count must be >= N (passes may grow; a shrink is drift);
  L3  a module with measured results but NO row is reported (subset
      of S3 that also fires when the row exists but the module path
      spelling drifted).

Results file: JSON-lines as emitted by
    verum test --interp --format json > results.json
(each line: {"event":"test","name":"<module>/<file>::<test>",
             "outcome":"ok|failed|ignored|compile-error", ...}).

Usage:
    python3 scripts/ci/check_inventory.py --structural-only
    python3 scripts/ci/check_inventory.py --results results.json
    python3 scripts/ci/check_inventory.py --results results.json \
        --modules net/http,signal        # verify a subset of rows

Exit 0 = table verified at the requested depth; exit 1 = drift or
structural rot, each finding printed with its row line number.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
INVENTORY = REPO / "core-tests" / "INVENTORY.md"
CORE_TESTS = REPO / "core-tests"

ROW_RE = re.compile(r"^\|\s*`([^`]+)`\s*\|")
# The table's own status vocabulary (census 2026-08-01: 343/378 rows
# already carry one of these; S4 ratchets the remaining rows in).
# **unverified** is the honest fifth state: the row exists (so S3 can
# close) but no measured run has backed its claims yet — the liveness
# layer treats it like **partial** (no green claim to verify).
TOKEN_RE = re.compile(
    r"\*\*(stable|complete|partial|regression-only|unverified)[^*]*\*\*",
    re.IGNORECASE,
)
GREEN_CLAIMS = {"stable", "complete", "regression-only"}
# Explicit numeric green claims, most-specific first.
COUNT_RES = [
    re.compile(r"\b(\d+)\s*/\s*\d+\s+GREEN\b"),
    re.compile(r"\b(\d+)\s+GREEN\b"),
    re.compile(r"\b(\d+)\s+passed\s*/\s*0\s+failed\b"),
]
# Directories under core-tests/ that are infrastructure, not modules.
NON_MODULE_DIRS = {"target"}


def parse_rows(path: Path):
    """-> list of (lineno, module, status_token|None, max_green_claim|None)."""
    rows = []
    for lineno, line in enumerate(path.read_text().splitlines(), 1):
        m = ROW_RE.match(line)
        if not m or m.group(1) == "module":
            continue
        module = m.group(1).strip()
        tok = TOKEN_RE.search(line)
        token = tok.group(1).lower() if tok else None
        counts = [int(c.group(1)) for r in COUNT_RES for c in r.finditer(line)]
        rows.append((lineno, module, token, max(counts) if counts else None))
    return rows


def discover_module_dirs():
    """core-tests dirs that carry at least one *_test.vr (recursive leaf
    dirs, expressed relative to core-tests/)."""
    found = set()
    for f in CORE_TESTS.rglob("*_test.vr"):
        rel = f.parent.relative_to(CORE_TESTS)
        parts = rel.parts
        if parts and parts[0] in NON_MODULE_DIRS:
            continue
        found.add("/".join(parts))
    return found


def load_results(path: Path):
    """-> module -> {"ok": n, "failed": n, "ignored": n, "compile-error": n}"""
    per = defaultdict(lambda: defaultdict(int))
    with path.open() as fh:
        for line in fh:
            line = line.strip()
            if not line or not line.startswith("{"):
                continue
            try:
                ev = json.loads(line)
            except json.JSONDecodeError:
                continue
            if ev.get("event") != "test":
                continue
            name = ev.get("name", "")
            # name = "<module-path>/<file>::<test>"
            head = name.split("::", 1)[0]
            module = head.rsplit("/", 1)[0] if "/" in head else head
            per[module][ev.get("outcome", "unknown")] += 1
    return per


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--results", type=Path, default=None)
    ap.add_argument("--structural-only", action="store_true")
    ap.add_argument(
        "--modules",
        default=None,
        help="comma-separated module subset to verify (liveness layer)",
    )
    args = ap.parse_args()

    rows = parse_rows(INVENTORY)
    findings: list[str] = []

    # S1 — unique module rows.
    seen: dict[str, int] = {}
    for lineno, module, _tok, _cnt in rows:
        if module in seen:
            findings.append(
                f"S1 duplicate row: `{module}` at lines {seen[module]} and {lineno}"
            )
        else:
            seen[module] = lineno

    # S2 — row -> directory (or a root-level single-file suite:
    # `tests/<module>.vr` — the runner names those file-modules by
    # their stem, discovered by the first full liveness sweep).
    for lineno, module, _tok, _cnt in rows:
        if (CORE_TESTS / module).is_dir():
            continue
        if (REPO / "tests" / f"{module}.vr").is_file():
            continue
        if (REPO / "tests" / module).is_dir():
            continue
        findings.append(
            f"S2 row without directory: `{module}` (line {lineno}) — "
            f"none of core-tests/{module}/, tests/{module}/ or "
            f"tests/{module}.vr exists"
        )

    # S3 — directory -> row (the 'never inventoried' class).
    dirs = discover_module_dirs()
    row_names = set(seen)
    for d in sorted(dirs - row_names):
        findings.append(
            f"S3 test directory never inventoried: core-tests/{d}/ carries "
            f"*_test.vr but INVENTORY.md has no `{d}` row"
        )

    # S4 — every row carries a recognized status token.
    for lineno, module, token, _cnt in rows:
        if token is None:
            findings.append(
                f"S4 unclaimed row: `{module}` (line {lineno}) carries none of "
                f"**stable**/**complete**/**partial**/**regression-only** — "
                f"an unclaimed row is unverifiable and reads as green"
            )

    if not args.structural_only:
        if args.results is None:
            print(
                "check-inventory: liveness layer requested but no --results "
                "file given (run `verum test --interp --format json > "
                "results.json` first, or pass --structural-only)",
                file=sys.stderr,
            )
            return 1
        per = load_results(args.results)
        subset = (
            {m.strip() for m in args.modules.split(",")} if args.modules else None
        )
        measured_names = set(per)
        for lineno, module, token, green_claim in rows:
            if subset is not None and module not in subset:
                continue
            got = per.get(module)
            if got is None:
                # A green-claiming row with NO measured tests is drift:
                # the suite no longer discovers this module at all.
                if token in GREEN_CLAIMS:
                    findings.append(
                        f"L1 `{module}` (line {lineno}) claims **{token}** but "
                        f"the run discovered ZERO tests for it"
                    )
                continue
            failed = got.get("failed", 0) + got.get("compile-error", 0)
            if token in GREEN_CLAIMS and failed:
                findings.append(
                    f"L1 `{module}` (line {lineno}) claims **{token}** but "
                    f"measures {failed} failing "
                    f"(failed={got.get('failed', 0)}, "
                    f"compile-error={got.get('compile-error', 0)}, "
                    f"ok={got.get('ok', 0)}, ignored={got.get('ignored', 0)})"
                )
            if green_claim is not None and got.get("ok", 0) < green_claim:
                findings.append(
                    f"L2 `{module}` (line {lineno}) pins {green_claim} GREEN "
                    f"but measures only {got.get('ok', 0)} passing"
                )
        if subset is None:
            for m in sorted(measured_names - row_names):
                findings.append(
                    f"L3 measured module with no row: `{m}` "
                    f"({dict(per[m])}) — inventory it"
                )

    if findings:
        print(f"check-inventory: {len(findings)} finding(s):\n")
        for f in findings:
            print(f"  {f}")
        return 1
    depth = "structural" if (args.structural_only or args.results is None) else "structural+liveness"
    print(f"check-inventory OK ({depth}; {len(rows)} rows)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
