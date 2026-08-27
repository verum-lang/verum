#!/usr/bin/env python3
"""Gate: the compiler must give the same answer twice.

`verum check core/logic/kripke.vr` returns 0 errors on some runs and 20
on others, from the SAME binary on the SAME file, with no edits between
them — and not merely a different count: the runs disagree about what is
wrong. One reports `E0203` on the `?` operator against a residual type
whose error parameter is still an open inference variable; another
reports `E0205`, "`Result` does not implement `Try`", at all; a third
reports nothing.

`verum_common::Map` and `Set` wrap `std::collections::HashMap` /
`HashSet`, whose iteration order is randomised per PROCESS, so any
compiler decision taken while iterating one is a coin flip across runs.
The hazard is known — `ProtocolRegistry::get_implementations` sorts its
exact matches by declaration index for exactly this reason (T0368) — and
was closed there case by case rather than as a class.

WHY A GATE AND NOT JUST A FIX: non-determinism is the one defect class
that hides from every other gate, because every other gate runs once. A
suite that is 95% deterministic looks green 95% of the time and its
failures read as unrelated flakes. This measures the property directly.

It also protects against the wrong kind of fix: pinning one decision's
order makes THAT file stable and says nothing about the rest, so the
number this reports is the only way to tell a class fix from a patch.

Usage:
    check_determinism.py --sample 40               # report (20 runs each)
    check_determinism.py --check                   # exit 1 on any variance
    check_determinism.py --files a.vr b.vr         # specific files
    check_determinism.py --self-test
"""

from __future__ import annotations

import argparse
import os
import random
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


def verum_binary() -> Path:
    # `VERUM_BIN` FIRST, and it is not a convenience: sessions in this
    # repo build into a private CARGO_TARGET_DIR, so a gate that only
    # looks under `target/` finds a STALE binary and reports clean. This
    # one did, on the very file it was written for, until the override
    # the docstring already promised was actually read.
    override = os.environ.get("VERUM_BIN")
    if override:
        p = Path(override)
        if not p.is_file():
            raise SystemExit(f"VERUM_BIN={override} is not a file")
        return p
    for candidate in (
        REPO / "target" / "release" / "verum",
        REPO / "target" / "debug" / "verum",
    ):
        if candidate.is_file():
            return candidate
    # Session-private target dirs are the norm in this repo; accept an
    # explicit override rather than guessing at one.
    raise SystemExit(
        "no verum binary at target/{release,debug}/verum — pass one with "
        "VERUM_BIN=/path/to/verum"
    )


def error_count(binary: Path, path: Path, timeout: int) -> int | None:
    """Errors reported for `path`, or None if the run did not finish.

    A timeout must NOT be read as zero errors: a file that never finishes
    is the loudest possible result, and counting it clean is how an
    interrupted sweep produces false greens.
    """
    try:
        out = subprocess.run(
            [str(binary), "check", str(path)],
            cwd=REPO,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return None
    blob = out.stdout + out.stderr
    return sum(1 for line in blob.splitlines() if line.startswith("error"))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--runs",
        type=int,
        default=20,
        help="runs per file (>=2). MEASURED: kripke.vr disagrees on 20%% of runs, "
        "so five runs miss it a third of the time and a determinism gate that "
        "usually passes is worse than none. 20 runs miss it under 1%% of the time.",
    )
    ap.add_argument("--sample", type=int, default=40, help="random core/ files")
    ap.add_argument("--files", nargs="*", help="check these instead of a sample")
    ap.add_argument("--seed", type=int, default=20260828, help="sample seed")
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--check", action="store_true", help="exit 1 on any variance")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        # The gate's own logic, with no compiler involved: a file whose
        # counts differ must be reported, one whose counts agree must not,
        # and a None (timeout) must never be read as agreement.
        ok = True
        for counts, expect_flag in (
            ([0, 20, 0], True),
            ([3, 3, 3], False),
            ([None, 0, 0], True),
        ):
            distinct = {c for c in counts}
            flagged = len(distinct) > 1
            if flagged != expect_flag:
                print(f"self-test FAIL: {counts} -> flagged={flagged}")
                ok = False
        print("self-test: ok" if ok else "self-test: FAILED")
        return 0 if ok else 1

    if args.runs < 2:
        raise SystemExit("--runs must be at least 2; one run measures nothing")

    binary = verum_binary()
    if args.files:
        targets = [Path(f) for f in args.files]
    else:
        everything = sorted((REPO / "core").rglob("*.vr"))
        rng = random.Random(args.seed)
        targets = rng.sample(everything, min(args.sample, len(everything)))
        targets = [t.relative_to(REPO) for t in targets]

    unstable: list[tuple[str, list[int | None]]] = []
    for path in targets:
        counts = [error_count(binary, path, args.timeout) for _ in range(args.runs)]
        if len({c for c in counts}) > 1:
            unstable.append((str(path), counts))

    for name, counts in unstable:
        shown = ", ".join("timeout" if c is None else str(c) for c in counts)
        print(f"{name}: {shown}")

    print(
        f"\ncheck-determinism: {len(unstable)} of {len(targets)} files gave "
        f"different answers across {args.runs} runs"
    )
    if unstable and args.check:
        print(
            "A compiler that answers differently on identical input makes every "
            "other gate advisory (T0927)."
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
