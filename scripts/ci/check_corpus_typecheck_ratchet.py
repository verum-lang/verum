#!/usr/bin/env python3
"""CORPUS-TYPECHECK-RATCHET-1 (T1073): the number of type errors in
`core/` may not grow.

`core/` is 2560 files and, measured 2026-09-03, had NOT ONE gate that
typechecks them — every existing core/ gate is a grep. The cost showed
up the same day: nine crypto call sites, covering every signature
verification in the tree, called a static method on a type that is an
array alias and cannot carry one. Nothing was measuring, so nothing
said so.

Three quantities, from `corpus_typecheck_census.sh`, and the second and
third are not decoration:

    errors   the sum of `compilation failed with N error(s)` — taken
             from the summary line, because 318 diagnostics in this tree
             print with no code and a `^error<` grep misses all of them.
    parse    errors reading `Parse error`. AN ERROR COUNT IS NOT A
             MONOTONE QUALITY MEASURE: a parse failure truncates the
             file and every later diagnostic disappears, so BREAKING a
             file makes its count FALL. This is not hypothetical — a
             sweep with auto-revert keyed on "did the count drop" scored
             a broken `core/text/text.vr` as its best result, and the
             unparseable module then shipped as a silent zero in the
             stdlib bake.
    mute     files whose output shows no sign of work at all. Without
             it, "0 errors" and "the instrument did not answer" read the
             same.

The gate fails when errors rise, when parse errors rise, or when any
file goes mute. Parse errors and mutes are ratcheted at ZERO
independently of the error total, precisely so that the total cannot be
lowered by breaking something.

Baseline: scripts/ci/corpus_typecheck_baseline.txt, three integers.

This is NOT a PR gate as it stands: a full run is 2560 compiler
invocations. Run it in the nightly lane, or on a deterministic sample
(`--sample N`, the same N files every time) for a cheaper signal.
"""

import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
BASELINE = pathlib.Path(__file__).resolve().parent / "corpus_typecheck_baseline.txt"


def read_baseline() -> dict[str, int]:
    if not BASELINE.exists():
        return {}
    out = {}
    for line in BASELINE.read_text().split("\n"):
        line = line.split("#", 1)[0].strip()
        if not line or "=" not in line:
            continue
        k, v = line.split("=", 1)
        out[k.strip()] = int(v.strip())
    return out


def read_census(path: pathlib.Path) -> dict[str, int]:
    total = None
    for line in path.read_text().split("\n"):
        if line.startswith("TOTAL\t"):
            total = {}
            for field in line.split("\t")[1:]:
                if "=" in field:
                    k, v = field.split("=", 1)
                    total[k] = int(v)
    if total is None:
        raise SystemExit(f"{path}: no TOTAL line — the census did not finish")
    return total


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: check_corpus_typecheck_ratchet.py <census.tsv>")
        print("  produce the census with corpus_typecheck_census.sh")
        return 2
    census = read_census(pathlib.Path(sys.argv[1]))
    base = read_baseline()

    if not base:
        print("corpus-typecheck-ratchet: no baseline yet")
        print(f"  measured: {census}")
        print(f"  write these into {BASELINE.name} to arm the gate")
        return 1

    failures = []
    # Parse errors and mutes ratchet at zero, INDEPENDENTLY of the error
    # total — the total can be lowered by breaking a file, and these two
    # are what makes that visible.
    if census.get("parse", 0) > base.get("parse", 0):
        failures.append(
            f"PARSE ERRORS rose: {census['parse']} > {base['parse']}. "
            "A file that stopped parsing LOWERS the error total, so this "
            "column is what stops a break reading as an improvement."
        )
    if census.get("mute", 0) > base.get("mute", 0):
        failures.append(
            f"MUTE files rose: {census['mute']} > {base['mute']}. "
            "A file whose output shows no sign of work makes its `0` "
            "meaningless."
        )
    if census.get("errors", 0) > base.get("errors", 0):
        failures.append(
            f"TYPE ERRORS rose: {census['errors']} > {base['errors']}."
        )

    if failures:
        print("GATE FAIL: corpus-typecheck-ratchet")
        for f in failures:
            print(f"  {f}")
        print(f"  measured: {census}")
        return 1

    lowered = [
        k for k in ("errors", "parse", "mute")
        if census.get(k, 0) < base.get(k, 0)
    ]
    msg = (
        f"[ok] corpus-typecheck-ratchet: {census.get('files', 0)} file(s), "
        f"{census.get('errors', 0)} error(s), {census.get('parse', 0)} parse, "
        f"{census.get('mute', 0)} mute, {census.get('failing', 0)} failing"
    )
    print(msg)
    if lowered:
        print(f"  ratchet down: {', '.join(lowered)} improved — update {BASELINE.name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
