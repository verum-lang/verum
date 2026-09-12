#!/usr/bin/env python3
"""Ratchet: source comments that hand known-broken behaviour to a DEAD task.

WHY THIS EXISTS
---------------
A comment that says "tracked in T0455", "guard moves to T0408", "pooled
separately as T0376" tells the reader a known defect has an owner.  When
that id is in the pool's `dead/`, nobody owns it: the workaround stays,
the reader chases the id into a graveyard, and open debt reads as
tracked debt.  Measured 2026-09-12: 21 killed ids cited from 61 places
in source files (the register's own rows excluded — see below).

MOST OF THESE ARE NOT JUDGEMENT CALLS.  The 2026-07-31 consolidation
FOLDED tasks into successors and each dead task's journal names its
heir: 18 of the 21 carry a `DEAD: folded into TXXXX` line.  So the
repair is usually "re-point at the heir", and this gate exists to stop
the population growing back while that is done.

THE RULE, AND WHY IT IS NOT "NO DEAD IDS"
-----------------------------------------
A dead id is legitimate as HISTORY — "ex-T0408", "absorbed T0408", "the
T0131 sweeps re-rot without this gate".  Banning the id outright would
report every one of those and teach everyone to ignore the gate.

So: a dead id is a violation only when NO resolvable id (open, claimed
or done) appears within the same neighbourhood of lines.  A citation
that names its live successor beside the dead one is exactly what the
repair produces, and it passes.

`done` counts as resolvable: the work landed and the task file still
says how.  Only `dead/` is a dead end.

WHAT IS NOT COUNTED
-------------------
`docs/architecture/tech-debt-register.md` and other prose: a register
row saying "T0323 and T0392 dead" is a CORRECT statement, and the whole
point of a row is to carry history.  This gate reads source files only
(.rs/.vr/.py/.yml/.sh).

THE POOL IS MACHINE-LOCAL
-------------------------
`.taskpool/` is gitignored — a clean CI checkout has none.  With no
pool this gate cannot know which ids are dead, and a gate that fails
for lack of its own input is worse than no gate: it SKIPS, says so, and
exits 0.

Usage:
  scripts/ci/check_a_comment_names_a_live_task.py
  scripts/ci/check_a_comment_names_a_live_task.py --selftest
  scripts/ci/check_a_comment_names_a_live_task.py --write-baseline
"""

import os
import re
import subprocess
import sys

SRC_EXT = (".rs", ".vr", ".py", ".yml", ".sh")
BASELINE = "scripts/ci/comments_naming_a_dead_task.txt"
# Repo-relative, spelled out rather than derived from `__file__`:
# `main` chdirs to the repo root first, so a relative `__file__`
# would be resolved against the WRONG directory and the exemption
# would silently stop matching.
SELF = "scripts/ci/check_a_comment_names_a_live_task.py"
# How far from the dead id to accept a live id as "the owner is named
# here too".  Measured on the repaired sites: the successor lands on the
# same line or the next one, and a comment paragraph in this tree runs
# to about five lines.
NEIGHBOURHOOD = 4
TASK = re.compile(r"\bT(\d{4})\b")


def pool_root():
    try:
        common = subprocess.run(
            ["git", "rev-parse", "--git-common-dir"],
            capture_output=True, text=True, check=True,
        ).stdout.strip()
    except Exception:
        return None
    root = os.path.join(common, "..", ".taskpool")
    return root if os.path.isdir(root) else None


def task_states(pool):
    """id -> state, for every task file in the pool."""
    states = {}
    for state in ("open", "claimed", "done", "dead"):
        d = os.path.join(pool, state)
        if not os.path.isdir(d):
            continue
        for f in os.listdir(d):
            if f.endswith(".md"):
                states[f[:-3]] = state
    return states


def violations(lines, states):
    """Dead ids on a line with no resolvable id in the neighbourhood.

    `lines` is a list of text lines; returns (lineno, dead_id) pairs,
    1-indexed.
    """
    out = []
    for i, line in enumerate(lines):
        dead_here = [
            "T" + m.group(1)
            for m in TASK.finditer(line)
            if states.get("T" + m.group(1)) == "dead"
        ]
        if not dead_here:
            continue
        lo = max(0, i - NEIGHBOURHOOD)
        hi = min(len(lines), i + NEIGHBOURHOOD + 1)
        near = " ".join(lines[lo:hi])
        resolvable = any(
            states.get("T" + m.group(1)) in ("open", "claimed", "done")
            for m in TASK.finditer(near)
        )
        if not resolvable:
            for d in dead_here:
                out.append((i + 1, d))
    return out


SELFTEST = [
    # (lines, states, expected count) — BOTH POLES, because a finder
    # that only ever reports zero is an assertion of absence.
    (["// guard moves to T0408."], {"T0408": "dead"}, 1),
    # the repair: the heir is named on the same line
    (["// tracked as T0458, which absorbed T0408."],
     {"T0408": "dead", "T0458": "open"}, 0),
    # the heir two lines away still counts as named
    (["// deeper root T0458, which absorbed", "//   T0408 in the",
      "//   2026-07-31 consolidation."],
     {"T0408": "dead", "T0458": "open"}, 0),
    # a DONE heir resolves too — the reader reaches a file that says how
    (["// closed under T0277 (was T0378)."],
     {"T0378": "dead", "T0277": "done"}, 0),
    # a live id alone is not a finding
    (["// tracked in T0424."], {"T0424": "open"}, 0),
    # an id the pool has never heard of is not a finding either: it may
    # predate the pool, and inventing a verdict for it is not measuring
    (["// see T9999."], {}, 0),
    # two dead ids on one line report twice
    (["// see T0408 and T0442."],
     {"T0408": "dead", "T0442": "dead"}, 2),
    # far away does NOT count — five lines is outside the neighbourhood
    (["// T0458 opens", "//", "//", "//", "//", "// guard moves to T0408."],
     {"T0408": "dead", "T0458": "open"}, 1),
]


def selftest():
    bad = 0
    for idx, (lines, states, expected) in enumerate(SELFTEST):
        got = len(violations(lines, states))
        if got != expected:
            print(f"[fail] self-test case {idx}: expected {expected}, got {got}")
            bad += 1
    if bad:
        return 1
    print(f"[ok] self-test: {len(SELFTEST)} case(s) hold")
    return 0


def scan(pool):
    states = task_states(pool)
    files = subprocess.run(
        ["git", "ls-files"], capture_output=True, text=True, check=True
    ).stdout.split()
    found = []
    for f in files:
        if not f.endswith(SRC_EXT):
            continue
        # THIS GATE AND ITS BASELINE ARE EXEMPT, and the exemption is
        # narrow on purpose. A gate that describes a pattern has to
        # QUOTE it: the docstring above names T0408, T0376, T0455,
        # T0131, T0323 and T0392 to show what a handover and a correct
        # history look like. Scanning itself would report every example
        # it teaches from, which is a gate arguing with its own manual.
        # Only these two paths are skipped — never a directory, never a
        # pattern.
        if f in (SELF, BASELINE):
            continue
        try:
            with open(f, encoding="utf-8", errors="replace") as fh:
                lines = fh.read().splitlines()
        except OSError:
            continue
        for lineno, dead in violations(lines, states):
            found.append(f"{f}\t{dead}")
    return sorted(set(found))


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "check"
    os.chdir(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
    if mode == "--selftest":
        return selftest()

    pool = pool_root()
    if pool is None:
        print("[skip] no .taskpool on this machine — the pool is "
              "machine-local and gitignored, so nothing can be said about "
              "which ids are dead")
        return 0

    found = scan(pool)
    if mode == "--write-baseline":
        with open(BASELINE, "w", encoding="utf-8") as fh:
            fh.write("# Source sites that hand debt to a DEAD task (T1460).\n")
            fh.write("# A RATCHET, not a zero: removing a line is a repair,\n")
            fh.write("# ADDING one is a decision to point a reader at a\n")
            fh.write("# graveyard. Regenerate with --write-baseline.\n")
            for line in found:
                fh.write(line + "\n")
        print(f"[ok] baseline written: {len(found)} site(s)")
        return 0

    baseline = set()
    if os.path.exists(BASELINE):
        with open(BASELINE, encoding="utf-8") as fh:
            baseline = {
                ln.rstrip("\n") for ln in fh
                if ln.strip() and not ln.startswith("#")
            }
    new = [f for f in found if f not in baseline]
    if new:
        print(f"[fail] {len(new)} NEW site(s) hand debt to a dead task:")
        for f in new:
            path, dead = f.split("\t")
            print(f"    {path}  ->  {dead}")
        print("\nThe dead task's own journal usually names its heir "
              "(`DEAD: folded into TXXXX`). Name the live one beside it, "
              "or state the fact without promising an owner.")
        return 1
    gone = len(baseline) - len([f for f in found if f in baseline])
    print(f"[ok] dead-task citations: {len(found)} site(s), baseline "
          f"{len(baseline)}" + (f", {gone} repaired" if gone else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
