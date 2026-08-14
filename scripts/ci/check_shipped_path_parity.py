#!/usr/bin/env python3
"""Ratchet: specs the conformance runner accepts but `verum run` does not.

The runner and the shipped binary execute different pipelines. `verum run`
goes through `run_interpreter` — safety gate, type check, resolved-call-target
application, dependency analysis, verify, CBGR analysis — while a
`@test: run-interpreter` spec goes through `run_for_test`, which has none of
those six phases (see the R1b row in
docs/architecture/tech-debt-register.md). A spec can therefore be green in the
suite and broken for every user of the language.

This gate measures that gap on the only population where it CAN be measured
without changing the runner: specs that declare `@expected-stdout` and own a
`main`, so the same yardstick applies to both paths. Measured 2026-08-14: 826
`run-interpreter` specs exist, 26 declare an expected stdout, 25 of those have
a `main`, and 3 of the 25 diverged — three DIFFERENT defects, all green in CI:

    atomic_bool_rmw.vr      Assertion failed at pc 52: left != right —
                            FIXED. Not an atomics defect at all: the script
                            cache returned a module whose 62 global ctors
                            had been dropped by serialisation, so the second
                            and every later run skipped static init (T0737).
    block_on_end_to_end.vr  still divergent, and now for a third reason:
                            `async fn` compiles to a plain function at
                            Tier 0, so calling one yields the VALUE where
                            the typechecker promised a Future, and
                            `block_on` polls a 42 (T0734).
    tcp_listen_v2.vr        error<E402>: module `core.sys.raw` not found —
                            FIXED. core/sys/raw never existed; four net
                            specs mounted it and all four passed anyway.
                            The three intrinsics they want are declared
                            `public fn` in core/intrinsics/runtime/os.vr.

The remaining 800 execution specs assert nothing about their output at all,
so this gate cannot speak for them; closing that hole needs the runner fix.

Usage:
    check_shipped_path_parity.py <verum-binary> [--check] [--specs DIR]
"""

from __future__ import annotations

import pathlib
import re
import subprocess
import sys

# Frozen at the measured divergence. Lower it in the same commit that earns
# it; a silently improving number is how a gate stops measuring.
BASELINE_DIVERGENT = 1

# Per-spec wall-clock ceiling. Generous on purpose: this gate judges OUTPUT,
# never speed, and a timeout must not be mistaken for a divergence — it is
# reported separately.
TIMEOUT_S = 180


def declared_stdout(text: str) -> str | None:
    m = re.search(r"@expected-stdout:\s*(.+)", text)
    if not m:
        return None
    return m.group(1).strip().replace("\\n", "\n")


def eligible(path: pathlib.Path) -> bool:
    text = path.read_text(encoding="utf-8", errors="ignore")
    return (
        "@test: run-interpreter" in text
        and declared_stdout(text) is not None
        and re.search(r"^fn main", text, re.M) is not None
    )


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    check = "--check" in sys.argv[1:]
    specs_dir = pathlib.Path("vcs/specs")
    for a in sys.argv[1:]:
        if a.startswith("--specs="):
            specs_dir = pathlib.Path(a.split("=", 1)[1])
    if len(args) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    verum = pathlib.Path(args[0])
    if not verum.is_file():
        print(f"verum binary not found: {verum}", file=sys.stderr)
        return 2

    candidates = sorted(p for p in specs_dir.rglob("*.vr") if eligible(p))

    # A run that finds nothing is a broken invocation, not a clean sheet.
    if not candidates:
        print(
            f"no eligible specs under {specs_dir} (need `@test: run-interpreter`, "
            f"`@expected-stdout:` and a `main`). Refusing to report 0/0 as success.",
            file=sys.stderr,
        )
        return 2

    matched, divergent, timed_out = [], [], []
    for spec in candidates:
        want = declared_stdout(spec.read_text(encoding="utf-8", errors="ignore"))
        try:
            got = subprocess.run(
                [str(verum), "run", str(spec)],
                capture_output=True,
                text=True,
                timeout=TIMEOUT_S,
            ).stdout
        except subprocess.TimeoutExpired:
            timed_out.append(spec)
            continue
        if got.strip() == (want or "").strip():
            matched.append(spec)
        else:
            divergent.append((spec, want, got))

    print(f"eligible specs   : {len(candidates)}")
    print(f"same under both  : {len(matched)}")
    print(f"DIVERGENT        : {len(divergent)} (baseline {BASELINE_DIVERGENT})")
    if timed_out:
        print(f"timed out        : {len(timed_out)} (not counted as divergence)")
        for spec in timed_out:
            print(f"    {spec}")
    for spec, want, got in divergent:
        print(f"    {spec}")
        print(f"        declared: {(want or '')[:70]!r}")
        print(f"        shipped : {got.strip()[:70]!r}")

    if not check:
        return 0

    if len(divergent) > BASELINE_DIVERGENT:
        print(
            f"RATCHET: {len(divergent)} specs pass the conformance runner and fail "
            f"under `verum run` (baseline {BASELINE_DIVERGENT}). Each one is green "
            f"in CI and broken for users.",
            file=sys.stderr,
        )
        return 1
    if len(divergent) < BASELINE_DIVERGENT:
        print(
            f"RATCHET: divergence dropped to {len(divergent)} (baseline "
            f"{BASELINE_DIVERGENT}). Lower the baseline in the same commit that "
            f"earns it.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
