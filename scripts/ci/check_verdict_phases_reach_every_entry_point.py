#!/usr/bin/env python3
"""Gate: every verdict-bearing phase must be reachable from every entry point.

`pipeline/dispatch.rs` states the invariant next to `phase_verify`:

    check ⊆ build must hold for verdicts

It was a comment, and it was violated three times in one day:

  T0105  phase_verify        never ran in check-only, so a false theorem
                             passed `verum check` and failed `verum build`
  T1095  phase_context_validation  ran under `build`, not under `check` —
                             twice over, since `check_project` and the
                             single-file `run_check_only` are separate
                             entry points and each needed its own fix
  T1101  phase_cbgr_analysis ran under `run`, not under `check`: a
                             reference outliving its referent passed
                             `verum check` clean and panicked at run time

WHY THIS IS NOT check_entry_points_agree_on_their_phases.py. That gate
records the mapping of DIRECT calls in three flat bodies and fails on any
change, which is what caught the T1095 edit. This one asks a different
question — is the phase REACHED — and so it can include `check_project`,
which reaches most of its phases through `on_module_parsed` and would be
permanent noise under a direct-call reading.

The two answer "who calls it here" and "who runs it". Both are worth
having; neither substitutes for the other.

VERDICT PHASES are listed by hand, because "does this phase decide
whether the user's program is accepted" is not derivable from the source.
The list is short and every entry cites what goes wrong when it is
skipped. A phase NOT on it is not thereby unimportant — it is merely not
one whose absence silently changes a verdict.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
PIPELINE = REPO / "crates" / "verum_compiler" / "src" / "pipeline"
PIPELINE_RS = REPO / "crates" / "verum_compiler" / "src" / "pipeline.rs"

# phase -> what a user gets when an entry point skips it.
VERDICT_PHASES = {
    "phase_type_check": "a type error goes unreported",
    "phase_verify": "a false theorem is accepted (T0105)",
    "phase_context_validation": "`using [!Ctx]` forbids nothing (T1095)",
    "phase_cbgr_analysis": "a reference outliving its referent is accepted (T1101)",
    "phase_dependency_analysis": "a target constraint is not enforced",
}

# Entry points that print a verdict to a user, and where they live.
#
# `check_project` is DELIBERATELY ABSENT, and the reason matters more
# than the omission. It does not call the `phase_*` wrappers: its body
# is 46k characters that register types and call `check_item(...)`
# directly, so name-based reachability cannot see work it genuinely
# does. Including it produced four "gaps" of which three were false —
# `verum check` in a project reports E400 on a type error, measured.
#
# A gate that reports a gap which is not there is worse than no gate:
# the next reader either chases a phantom or learns to skim the output.
# When `check_project` is folded into the orchestrated phase list — the
# open half of T0692 — it belongs here, and the fold is what makes the
# question askable, not this file.
ENTRY_POINTS = {
    "run_check_only": "dispatch.rs",
    "run_interpreter": "dispatch.rs",
    "run_for_test": "interpreter.rs",
}

CALL = re.compile(r"self\.([a-z_][a-z_0-9]*)\s*\(")


def body(text: str, sig: str) -> str | None:
    """The brace-balanced body of `fn <sig>(`, or None."""
    i = text.find(f"fn {sig}(")
    if i == -1:
        return None
    j = text.find("{", i)
    if j == -1:
        return None
    d = 0
    for k in range(j, len(text)):
        if text[k] == "{":
            d += 1
        elif text[k] == "}":
            d -= 1
            if d == 0:
                return text[j:k]
    return text[j:]


def sources(root: Path, extra: Path) -> dict[Path, str]:
    out = {p: p.read_text(errors="ignore") for p in sorted(root.glob("*.rs"))}
    if extra.is_file():
        out[extra] = extra.read_text(errors="ignore")
    return out


def reaches(fn: str, srcs: dict[Path, str], target: str,
            seen: set[str] | None = None, depth: int = 0) -> bool:
    """Does `fn` call `target`, directly or through another method?"""
    if depth > 6:
        return False
    seen = seen if seen is not None else set()
    if fn in seen:
        return False
    seen.add(fn)
    for text in srcs.values():
        b = body(text, fn)
        if b is None:
            continue
        callees = set(CALL.findall(b))
        if target in callees:
            return True
        for c in callees:
            if c == fn or c in seen:
                continue
            if reaches(c, srcs, target, seen, depth + 1):
                return True
    return False


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pipeline", type=Path, default=PIPELINE)
    ap.add_argument("--pipeline-rs", type=Path, default=PIPELINE_RS)
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()

    srcs = sources(args.pipeline, args.pipeline_rs)
    if len(srcs) < 3:
        print(f"check-verdict-phases: only {len(srcs)} pipeline sources found — "
              "refusing to pass vacuously", file=sys.stderr)
        return 1

    if args.self_test:
        ok = True
        # Every entry point must be findable, or its column is free.
        for fn in ENTRY_POINTS:
            if not any(body(t, fn) for t in srcs.values()):
                print(f"self-test FAIL: entry point `{fn}` not found")
                ok = False
        # A phase nothing calls must NOT read as reachable.
        if reaches("run_check_only", srcs, "phase_zzq_does_not_exist"):
            print("self-test FAIL: an absent phase read as reachable")
            ok = False
        # The known-indirect case must read as reachable — this is the
        # whole reason the gate walks instead of grepping one body.
        if not reaches("run_for_test", srcs, "phase_context_validation"):
            print("self-test FAIL: run_for_test -> validate_module -> "
                  "phase_context_validation was not followed")
            ok = False
        # And a direct one, so the walk is not the only thing tested.
        if not reaches("run_check_only", srcs, "phase_type_check"):
            print("self-test FAIL: a direct call was not seen")
            ok = False
        print("self-test: ok" if ok else "self-test: FAILED")
        return 0 if ok else 1

    gaps: list[str] = []
    for phase, consequence in sorted(VERDICT_PHASES.items()):
        for fn in sorted(ENTRY_POINTS):
            if not reaches(fn, srcs, phase):
                gaps.append(f"  {fn} does not reach {phase} — {consequence}")
    print(f"check-verdict-phases: {len(VERDICT_PHASES)} verdict phases × "
          f"{len(ENTRY_POINTS)} entry points, {len(gaps)} gaps")
    if gaps:
        print("\nA verdict phase an entry point never reaches means that "
              "command answers a different question than the others:",
              file=sys.stderr)
        for g in gaps:
            print(g, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
