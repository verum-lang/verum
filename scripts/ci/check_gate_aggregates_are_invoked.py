#!/usr/bin/env python3
"""A gate nobody runs is a gate that is not there.

WHY, MEASURED 2026-09-11
------------------------
`gates-docs` listed 33 targets. THREE of them were invoked by no
workflow at all — not by name, not by their scripts, not through a
`make gates-docs` that no file contains:

    check-examples-run          runs every documented example
    check-doc-names-exist       the name floor this campaign quotes
    check-doc-meta-functions

They ran only when somebody typed `make`. The middle one is the sharp
edge: a documentation campaign had been reporting its roster ("16 pairs")
as a ratchet for weeks, and nothing on a pull request would have moved if
that number grew.

This is the same family as the earlier find that seven website-reading
gates sat in an aggregate whose CI job had no website checkout, and three
of those were listed in BOTH aggregates so their green came from the job
without the input. The lesson there was to ask not what a gate does but
WHERE IT RUNS. This asks it mechanically, for every target, every time.

WHAT COUNTS AS INVOKED, and the third clause is the one this gate got
wrong on its first run: the target's own name appears in a workflow, or
one of the scripts its Makefile recipe runs does, OR THE AGGREGATE IS
INVOKED WHOLE. All three forms are in use — `gates-source` is run as
`make gates-source` in one step, while every `gates-docs` target is
called script-by-script. A rule with only the first two clauses reported
28 correct targets and one real one, which is the false-positive
direction that makes a gate ignorable.

WHAT IT DELIBERATELY DOES NOT CHECK: whether the job that runs a gate
gives it the INPUT it needs. That is a different question, it burned this
repo once already, and a rule for it belongs beside the aggregate whose
job knows what it checks out — not here, where the answer would be a
guess about environment variables.
"""

from __future__ import annotations

import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
MAKEFILE = REPO / "Makefile"
WORKFLOWS = REPO / ".github" / "workflows"
AGGREGATES = ("gates-docs", "gates-source")


def targets_of(makefile: str, aggregate: str) -> list[str]:
    m = re.search(rf"^{aggregate}:(.*?)##", makefile, re.S | re.M)
    if not m:
        return []
    return [t for t in m.group(1).replace("\\", " ").split() if t.startswith("check-")]


def scripts_of(makefile: str, target: str) -> list[str]:
    m = re.search(rf"^{re.escape(target)}:.*?\n((?:\t[^\n]*\n)+)", makefile, re.M | re.S)
    if not m:
        return []
    return re.findall(r"scripts/ci/([a-z0-9_]+\.py)", m.group(1))


def self_test() -> int:
    bad = 0
    parsed = 0
    mk = ("gates-x: check-a check-b ## an aggregate\n"
          "\t@echo done\n\n"
          "check-a: ## a\n\tpython3 scripts/ci/check_a.py\n\n"
          "check-b: ## b\n\tpython3 scripts/ci/check_b.py\n")
    parsed += 1
    if targets_of(mk, "gates-x") != ["check-a", "check-b"]:
        print("self-test: aggregate parsing wrong", file=sys.stderr)
        bad += 1
    parsed += 1
    if scripts_of(mk, "check-a") != ["check_a.py"]:
        print("self-test: recipe parsing wrong", file=sys.stderr)
        bad += 1
    # A recipe with no script at all must not read as "invoked".
    parsed += 1
    if scripts_of(mk, "check-missing") != []:
        print("self-test: a target with no recipe should yield no scripts",
              file=sys.stderr)
        bad += 1
    # THE CLAUSE THIS GATE GOT WRONG FIRST TIME. An aggregate invoked as a
    # whole invokes everything in it, and without this the check reported
    # 28 correct targets.
    whole_re = lambda a, text: bool(
        re.search(rf"make\s+(?:[-\w]+\s+)*{re.escape(a)}(?=\s|$)", text, re.M))
    whole_cases = (
        ("a real invocation counts", "        run: make gates-x\n", True),
        ("with flags in between", "run: make -s gates-x\n", True),
        # THE TRAP THIS GATE FELL INTO WHILE BEING WRITTEN: a comment that
        # merely names the aggregate would have marked it run-whole and
        # blinded the check for good.
        ("a MENTION does not", "          # these live in gates-x\n", False),
        # `gates-x-report` is a DIFFERENT target. It happens to run the
        # same gates, but a rule that accepts it accepts any name with the
        # aggregate as a prefix, and the fixture said True only because
        # that is what the first regex did — an expectation written to the
        # code rather than to the intent.
        ("a similarly-named target does not",
         "run: make gates-x-report\n", False),
    )
    for label, text, want in whole_cases:
        if whole_re("gates-x", text) != want:
            print(f"self-test: {label}: expected {want}", file=sys.stderr)
            bad += 1
    if bad:
        print(f"self-test: {bad} FAILED", file=sys.stderr)
        return 1
    print(f"[ok] self-test: {parsed} parsing case(s), "
          f"{len(whole_cases)} whole-aggregate case(s)")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not MAKEFILE.is_file() or not WORKFLOWS.is_dir():
        print(f"check-gate-aggregates-invoked: no Makefile at {MAKEFILE} or no "
              f"workflows at {WORKFLOWS} — REFUSING to report OK; a missing "
              f"input is a broken checkout, not 'nothing to do'.",
              file=sys.stderr)
        return 2

    mk = MAKEFILE.read_text(errors="replace")
    wf = "\n".join(p.read_text(errors="replace")
                   for p in sorted(WORKFLOWS.glob("*.yml")))
    if not wf.strip():
        print("check-gate-aggregates-invoked: no workflow content read — "
              "REFUSING to report OK.", file=sys.stderr)
        return 2

    total = 0
    # `make <aggregate>`, NOT a mention. Measured while writing this: a
    # comment in ci.yml that merely said the words "gates-docs" made an
    # earlier version report that aggregate as run whole, which would have
    # blinded this gate permanently and silently. A gate fooled by prose
    # about itself is worse than no gate.
    whole = [a for a in AGGREGATES
             if re.search(rf"make\s+(?:[-\w]+\s+)*{re.escape(a)}(?=\s|$)", wf,
                          re.M)]
    orphans: list[tuple[str, str, list[str]]] = []
    for agg in AGGREGATES:
        if agg in whole:
            total += len(targets_of(mk, agg))
            continue
        for t in targets_of(mk, agg):
            total += 1
            ss = scripts_of(mk, t)
            if t in wf or any(s in wf for s in ss):
                continue
            orphans.append((agg, t, ss))

    print(f"check-gate-aggregates-invoked: {total} target(s) across "
          f"{len(AGGREGATES)} aggregate(s), {len(whole)} run whole "
          f"({', '.join(whole) or 'none'}), {len(orphans)} invoked by no "
          f"workflow")
    for agg, t, ss in orphans:
        where = ", ".join(ss) if ss else "(recipe runs no scripts/ci script)"
        print(f"    + {agg} lists {t}, and no workflow names it or {where}. "
              f"It runs only when somebody types `make`.")

    # THE SECOND KIND, which the loop above cannot see by construction.
    #
    # It asks whether every AGGREGATE MEMBER is reached, so a target in no
    # aggregate at all is outside its question — and that is where gates
    # go to be forgotten. Measured 2026-09-11: the Makefile declared 89
    # `check-*` targets, 66 were in an aggregate (all reached, this gate
    # was green), and of the remaining 23 SEVENTEEN were named by no
    # workflow. Two of them were RED and had been for long enough that
    # nobody knew — `check-register-rows` was reporting a false positive
    # that hid a real one, and `check-register-shas` was refusing to
    # report at all. A third, `check-newtype-transparency`, is labelled in
    # the Makefile as the gate for T1192, which was open with exactly that
    # symptom.
    #
    # Reported, not failed, and the reason is honest rather than timid: a
    # target can legitimately live outside every aggregate (it needs a
    # built artefact, a long-lived checkout, a binary path) and several
    # here do. What must not happen is that it becomes invisible. The
    # count is the ratchet a reader can watch; turning it into a failure
    # would force every such target into an aggregate that cannot run it.
    declared = sorted(set(re.findall(r"^(check-[a-z0-9-]+):", mk, re.M)))
    in_aggregate = set()
    for agg in AGGREGATES:
        in_aggregate |= set(targets_of(mk, agg))
    outside = []
    for tgt in declared:
        if tgt in in_aggregate:
            continue
        ss = scripts_of(mk, tgt)
        if tgt in wf or any(s in wf for s in ss):
            continue
        outside.append(tgt)
    print(f"check-gate-aggregates-invoked: {len(declared)} `check-*` "
          f"target(s) declared, {len(in_aggregate)} in an aggregate, "
          f"{len(outside)} in none and named by no workflow")
    for tgt in outside:
        print(f"    - {tgt}")

    return 1 if orphans else 0


if __name__ == "__main__":
    sys.exit(main())
