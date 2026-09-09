#!/usr/bin/env python3
"""Ratchet: how many stage-5 stub ids carry MORE THAN ONE name.

WHY THIS EXISTS
---------------
A mount miss inside one function mints a stage-5 stub and installs it in
the shared slot under the SIMPLE name, authoritatively
(`verum_vbc/src/codegen/expressions.rs:6695`,
`register_function_authoritative`).  Every later call of that name in the
compilation unit then resolves to the stub.  Because the id is minted per
MINT SITE and the name is captured per NAME, one id ends up standing for
many different qualified targets.

Measured 2026-09-09 18:09 on a complete bake of this tree (rc=0, archive
written, 5718 log lines):

     11 "stage-5 mount-miss stub" lines over 6 ids
     TWO of those 6 carry more than one name
     worst: 5 names on 4269801471 (STAGE5_BASE - 0) —
            InvalidInput, NotFound, access_name, flags_default,
            open_readonly

That last line is why the runtime diagnostic cannot help on its own:
`stub never resolved id=4269801471` is true of five unrelated names, and
two sessions spent an hour each chasing it from the id alone.

A LARGER CENSUS WAS QUOTED HERE FIRST — 78 stub lines over 16 ids, 11
shared, worst 23 — and it is NOT this tree's. It came from another
session's bake at a different moment (before two dead-name repairs
landed, and while `open` was declared variadic), and I adopted it
without taking my own. The baseline below is the number this tree
produces; the other one is left in the record because a reader who finds
it elsewhere should know which tree it describes rather than assume the
gate drifted.

THE NUMBER IS A COUNT OF NAMES, NOT OF ROOTS, and the census itself
says so: 23 of the 89 names share ONE id.  So `N stub lines` and `N
defects` are different quantities, and the ratio is neither known nor
constant.

MEASURED DELTA, so nobody has to guess the ratio: repairing one dead
name (`sys.darwin.thread.thread_yield`) moved that session's stub count
by exactly one and left its id count unchanged.  One measurement is one
measurement: it says the ratio CAN be 1:1, not that it always is.

A larger claim (one root moving the count by fifteen) was briefly
recorded here on 2026-09-09 and is RETRACTED: its inputs were read from
a bake log that was still being written, by a wait loop that grepped
after its timeout instead of stopping.  Corrected against the finished
files.  Read the direction; treat the magnitude as unexplained until
some measurement explains it.

INPUT
-----
A bake log produced with the trace enabled, which is how the census is
taken at all:

    VERUM_TRACE_QCALL=1 verum stdlib precompile --stdlib-path core \\
        -o /tmp/x.vbca > bake.log 2>&1
    check_stage5_stub_sharing.py bake.log --check

The gate takes a LOG rather than baking, for the same reason
`check_bake_diagnostics.py` does: a bake is ~12 minutes and CI already
has one.  If nothing produces a log, this gate protects nothing — that
was true of its neighbour for long enough to let nine panic stubs in
(T1325), so the wiring is part of the fix, not an afterthought.
"""

from __future__ import annotations

import collections
import pathlib
import re
import sys

# `[qcall] stage-5 mount-miss stub 'NAME' → 'QUALIFIED' (arity=N) id=ID in FN`
STUB = re.compile(
    r"\[qcall\] stage-5 mount-miss stub '([^']*)' \S+ '([^']*)' "
    r"\(arity=(\d+)\) id=(\d+) in (\S+)"
)

# Set from the measurement above.  Lowered by FIXING — a root removed —
# never by argument, and never raised to match a drift.
BASELINE_SHARED_IDS = 2


def census(text: str):
    """(names per id, every parsed record)."""
    by_id: dict[str, set[str]] = collections.defaultdict(set)
    records = []
    for m in STUB.finditer(text):
        name, qualified, arity, sid, fn = m.groups()
        by_id[sid].add(name)
        records.append((name, qualified, arity, sid, fn))
    return by_id, records


def self_test() -> int:
    bad = 0
    sample = (
        "[qcall] stage-5 mount-miss stub 'open' → '…libsystem.open' "
        "(arity=3) id=4269801458 in KqueueBackend.add\n"
        "[qcall] stage-5 mount-miss stub 'from' → '…oid.Oid.from' "
        "(arity=1) id=4269801471 in parse_signature_alg\n"
        "[qcall] stage-5 mount-miss stub 'read_to_text' → '…file.read_to_text' "
        "(arity=1) id=4269801471 in read_file_to_text\n"
    )
    by_id, records = census(sample)
    if len(records) != 3:
        print(f"self-test: parsed {len(records)} records, wanted 3"); bad += 1
    if sorted(by_id) != ["4269801458", "4269801471"]:
        print(f"self-test: ids {sorted(by_id)}"); bad += 1
    shared = [i for i, n in by_id.items() if len(n) > 1]
    if shared != ["4269801471"]:
        print(f"self-test: shared ids {shared}, wanted the two-name one"); bad += 1
    # A log with no trace in it must be UNMEASURED, never a clean zero:
    # a gate that reports "0 shared" for a log taken without the flag is
    # the exact failure its neighbours carry a floor against.
    if census("nothing here\n")[1]:
        print("self-test: parsed records out of an empty log"); bad += 1
    print("self-test: OK" if not bad else f"self-test: {bad} FAILED")
    return bad


def main() -> int:
    argv = sys.argv[1:]
    if "--self-test" in argv:
        return self_test()
    check = "--check" in argv
    paths = [a for a in argv if not a.startswith("--")]
    if len(paths) != 1:
        print("usage: check_stage5_stub_sharing.py <bake-log> [--check]",
              file=sys.stderr)
        return 2
    log = pathlib.Path(paths[0])
    if not log.is_file():
        print(f"check-stage5-stub-sharing: {log} is not a file — UNMEASURED "
              "rather than passing on an absent input.", file=sys.stderr)
        return 2

    by_id, records = census(log.read_text(errors="ignore"))
    if not records:
        # NOT a clean zero.  Either the bake minted no stubs (possible, and
        # then this line is the good news) or the log was taken without
        # `VERUM_TRACE_QCALL=1` (likely, and then the zero is about the
        # flag).  The two are indistinguishable from the count alone, so
        # the gate says which condition it could not verify.
        print("check-stage5-stub-sharing: no `[qcall] stage-5 mount-miss stub` "
              "lines in this log. Either the bake minted none, or it ran "
              "without VERUM_TRACE_QCALL=1 — UNMEASURED, not zero.")
        return 0

    shared = {i: n for i, n in by_id.items() if len(n) > 1}
    worst = max(by_id.items(), key=lambda kv: len(kv[1]))
    print(f"check-stage5-stub-sharing: {len(shared)} of {len(by_id)} stage-5 "
          f"stub id(s) carry more than one name (baseline "
          f"{BASELINE_SHARED_IDS}); {len(records)} stub line(s)")
    print(f"  worst: {len(worst[1])} names on id {worst[0]}")
    for sid, names in sorted(shared.items(), key=lambda kv: -len(kv[1]))[:5]:
        sample = ", ".join(sorted(names)[:6])
        more = "" if len(names) <= 6 else f", … ({len(names)} total)"
        print(f"  id {sid}: {sample}{more}")

    if len(shared) > BASELINE_SHARED_IDS:
        print(f"  ABOVE BASELINE by {len(shared) - BASELINE_SHARED_IDS}. A new "
              "id started standing for several names, which means a new bare "
              "name was captured by a stub — read this as ONE new root, not "
              "as N new defects.")
        return 1 if check else 0
    if len(shared) < BASELINE_SHARED_IDS:
        print(f"  BELOW baseline by {BASELINE_SHARED_IDS - len(shared)} — lower "
              "BASELINE_SHARED_IDS so the ground stays held.")
        return 1 if check else 0
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
