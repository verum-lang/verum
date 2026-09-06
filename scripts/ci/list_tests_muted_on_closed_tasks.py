#!/usr/bin/env python3
"""A LOCAL LIST, and deliberately NOT a CI gate: tests muted on a task
that has since closed.

TWO CORPORA, TWO DIRECTIVES, counted separately because they are not the
same thing. `vcs/specs` mutes a whole FILE with a `@skip:` comment in
its header; `core-tests` mutes ONE LAW with an `@ignore("reason")`
attribute, and a file can carry twenty. Reporting a single number over
both would hide which kind of coverage went quiet.

WHY IT CANNOT BE A GATE, and this is the whole reason for the `list_`
prefix. The task pool lives at `<main-checkout>/.taskpool/`, which is
MACHINE-LOCAL and gitignored. A CI clone has no pool, so a gate written
against it would find zero skips-on-closed-tasks in every run and print
a clean sheet forever — the exact false-green shape that
`check_no_internal_refs`'s history is about, where six gates measured an
absent directory and one of them printed "0 violations".

So this refuses to run at all when the pool is missing, and it is
invoked by hand.

WHAT IT FOUND ON ITS FIRST RUN (2026-09-07), in `vcs/specs`: FIVE
specs muted on a task that was already closed —

    format_slot_calls_the_named_method            T0814
    implement_must_be_complete                    T0812
    member_named_size_is_a_member                 T0815
    generic_arity_is_counted_in_both_directions   T0922
    a_units_own_function_owns_its_name            T1120

Every one passed its own declared criterion the moment the directive
was removed — the exact `@expected-stdout` or the exact
`@expected-error`, not a judgement call. The last had asked for it in
writing: "Remove this line when T1120 closes; the file needs no other
change."

WHY THIS CLASS IS WORSE THAN A FAILING TEST. A muted guard is not
reported as missing and not reported as failing. It is reported as
SKIPPED, which reads as a decision somebody made on purpose, and the
suite's counts stay stable while the protection is gone. Nothing about
closing a task reaches the files that were waiting on it.

TWO KINDS OF SKIP IT DELIBERATELY DOES NOT ACCUSE, both measured:

  * a skip naming NO task. It cannot be checked mechanically and is a
    separate hygiene question — reported in its own section rather than
    mixed in.
  * a skip whose reason is a HARNESS limitation rather than a compiler
    defect. `two_file_project_rejects_unknown_module` names T0732, a
    closed task, and stays muted correctly: T0732 was about the
    `compile-fail` path being a third pipeline entry, so closing it
    changed nothing about whether this file can run.

That second case is why the output says READ, not FIX: a closed task is
evidence the reason MIGHT have expired, never proof that it has.

AND THE SECOND CORPUS PAID THAT RULE BACK IMMEDIATELY. Of 152
`@ignore("...")` pins in `core-tests`, only TEN name a task — and all
ten name a CLOSED one. They are not stale bookkeeping: the pins' own
text says "T0108, stride leg — STILL OPEN; the read/write leg of that
task is fixed and its laws are live". A task was closed on half its
acceptance, and the surviving half is recorded only in a test comment.
Verified: `ptr_is_aligned(p)` still panics with "@intrinsic
(\"ptr_is_aligned\") is not implemented in this build ... it has no
registry entry".

So the ten to read are not "unpin these", they are "here is what a
closed task did not do". The 142 pins naming NO task are a different
and larger question, tracked as T1145.
"""
import os
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SPECS = REPO / "vcs" / "specs"
CORE_TESTS = REPO / "core-tests"
SKIP = re.compile(r"^//\s*@skip:\s*(.+)$", re.M)
# `core-tests` mutes with an ATTRIBUTE, not a directive comment, and the
# distinction matters for the reader as much as for the regex: a `@skip`
# comment sits in the file header and describes the FILE, while
# `@ignore("...")` sits on one test and describes that ONE law. A file
# can carry twenty of them.
#
# Counting the two together would also be wrong in a way that flatters:
# `@ignore'd` appears 85 times in PROSE across these files, and a
# line-counting sweep reports 613 where there are 153 pins.
IGNORE = re.compile(r'@ignore\("([^"]*)"\)')
TASK = re.compile(r"\bT\d{4}\b")


def pool_root() -> Path | None:
    env = os.environ.get("VERUM_TASKPOOL_ROOT")
    if env:
        p = Path(env)
        return p if p.is_dir() else None
    try:
        common = subprocess.run(
            ["git", "rev-parse", "--git-common-dir"],
            cwd=REPO, capture_output=True, text=True, check=True).stdout.strip()
    except Exception:
        return None
    p = (REPO / common / ".." / ".taskpool").resolve()
    return p if p.is_dir() else None


def main() -> int:
    pool = pool_root()
    if pool is None:
        print(
            "list-specs-muted-on-closed-tasks: the task pool is not present.\n"
            "It is machine-local and gitignored, so this instrument cannot run\n"
            "in a CI clone — which is why it is a `list_` and not a `check_`.\n"
            "Refusing to print a clean sheet over an input it never found.",
            file=sys.stderr)
        return 2

    states = {}
    for state in ("open", "claimed", "done", "dead"):
        d = pool / state
        if d.is_dir():
            for f in d.glob("T*.md"):
                states[f.stem] = state
    if len(states) < 50:
        print(f"list-specs-muted-on-closed-tasks: only {len(states)} tasks "
              "indexed — the pool layout changed. Refusing to report.",
              file=sys.stderr)
        return 2
    print(f"tasks indexed: {len(states)} "
          f"({sum(1 for v in states.values() if v in ('done', 'dead'))} closed)\n")

    expired, unnamed, live = [], [], []
    total = 0
    for f in sorted(SPECS.rglob("*.vr")):
        m = SKIP.search(f.read_text(encoding="utf-8", errors="replace")[:4000])
        if not m:
            continue
        total += 1
        rel = str(f.relative_to(SPECS))
        ids = sorted(set(TASK.findall(m.group(1))))
        if not ids:
            unnamed.append((rel, m.group(1).strip()[:64]))
        elif any(states.get(i) in ("open", "claimed") for i in ids):
            live.append(rel)
        elif all(states.get(i) in ("done", "dead") for i in ids):
            expired.append((rel, ",".join(ids)))
        else:
            unnamed.append((rel, "names a task the pool does not know: "
                                 + ",".join(ids)))

    print(f"specs carrying a @skip : {total}")
    print(f"  reason still open    : {len(live)}")
    print(f"  reason CLOSED        : {len(expired)}")
    print(f"  no task named        : {len(unnamed)}\n")

    if expired:
        print("TO READ — the stated reason has closed. Remove the directive and "
              "re-run the file against its OWN @expected-* value; a closed task "
              "is evidence the reason MIGHT have expired, not proof:")
        for rel, ids in expired:
            print(f"   {rel:<62} {ids}")
    else:
        print("No spec is muted on a closed task.")

    if unnamed:
        print("\nSeparately — a @skip that names no task cannot be checked "
              "mechanically and will never appear above:")
        for rel, why in unnamed:
            print(f"   {rel:<62} {why}")

    # ---- second corpus: core-tests' per-law @ignore attribute --------
    ig_tot = ig_named = 0
    ig_expired = []
    for f in sorted(CORE_TESTS.rglob("*.vr")):
        for m in IGNORE.finditer(f.read_text(encoding="utf-8", errors="replace")):
            ig_tot += 1
            ids = sorted(set(TASK.findall(m.group(1))))
            if not ids:
                continue
            ig_named += 1
            if all(states.get(i) in ("done", "dead") for i in ids):
                ig_expired.append((str(f.relative_to(CORE_TESTS)),
                                   ",".join(ids), m.group(1)[:56]))
    print(f"\ncore-tests @ignore(\"...\") pins : {ig_tot}")
    print(f"  naming a task id             : {ig_named}")
    print(f"  whose task(s) are CLOSED     : {len(ig_expired)}")
    if ig_expired:
        print("\nTO READ — same rule, and these have already repaid it once: "
              "every pin here named a CLOSED task, and the pins' own text says "
              "the task was closed on HALF its acceptance. A pin can be the "
              "only surviving record of what a closed task did not fix.")
        seen = set()
        for rel, ids, why in ig_expired:
            if (rel, ids) in seen:
                continue
            seen.add((rel, ids))
            print(f"   {rel:<52} {ids:<12} {why}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
