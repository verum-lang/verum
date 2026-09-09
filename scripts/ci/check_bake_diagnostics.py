#!/usr/bin/env python3
"""Ratchet: FIELD-GUESS-HARD-1 diagnostics and panic stubs in a stdlib bake.

Both counts are the visible surface of defects that do NOT fail the build:

  * `error[FIELD-GUESS-HARD-1]` — a field's position was GUESSED by scanning
    every type in the program and taking the most-fields candidate. The
    message says "error", the bake proceeds, and the emitted index may be a
    foreign slot or out of bounds. 167 of these were real defects: a variant
    the type never declared, a field renamed at the declaration and not at the
    call, a type name resolving nowhere.

  * `[lenient] SKIP` — a function that FAILED to compile and was replaced by a
    stub that panics at runtime. 31 of these shipped in the archive.

Neither number can be read off the source tree; both come from running a bake.
The gate therefore takes a bake log rather than re-baking (a bake is ~12
minutes), and CI is expected to pass the log it already produced.

Usage:
    check_bake_diagnostics.py <bake-log>            # report
    check_bake_diagnostics.py <bake-log> --check    # ratchet, exit 1 on drift
"""

from __future__ import annotations

import pathlib
import re
import sys

# FIELD-GUESS reached ZERO on 2026-08-14. The last site was not an unwritten
# design after all: `theory_topos()` assembled a record literal for
# `SheafInfinityTopos`, which is a PROTOCOL, while the same module already
# built that object correctly through the protocol's own constructor
# (`theory_universe`). A duplicate that had drifted, not a gap.
#
# The two panic stubs remain and ARE that class: `compose_geometric` and
# `id_geometric` over `InfinityFunctor`. Writing them from a call site would
# be inventing a design nobody wrote.
#
# `QuicStream` used to be counted here and was NOT that class: api/stream.vr
# declares `QuicApiStream` with exactly the three fields the call sites build,
# and the bare name resolved to the unrelated transport-level record. Renaming
# the uses took FIELD-GUESS from 2 to 1 — hence this baseline moving in the
# same commit that earned it.
# Lower these in the same commit that earns it.
BASELINE_FIELD_GUESS = 0
BASELINE_PANIC_STUBS = 2


# THE COUNT GAINED A ROSTER, 2026-09-09 (T1330) — AND THE BASELINE STAYS
# AT TWO.  `nightly.yml` records the standing decision in as many words:
# "THE BASELINE MUST NOT BE RAISED TO MATCH: a ratchet lowered by argument
# is not a ratchet, and raising this one would bless nine regressions in
# the act of repairing the gate."  So this roster is NOT a baseline.  It
# describes the current RED by name, and the gate stays red at 11 > 2.
#
# What it buys, which the count could not: a twelfth stub, or a swap that
# keeps the total at eleven while replacing one name with another, is
# reported as a NAME rather than hidden inside an unchanged number.  The
# population is T1315's, whose taxonomy is six causes; this file only has
# to notice membership changing.
#
# 11 -> 9 on 2026-09-09: `QuicPath.on_bytes_sent` and `on_bytes_received`
# left, and the roster is how that was checked. The gate reported
# `0 new, 2 gone` and named both, against a prediction of exactly those
# two written down before the bake started. A count would have said `9`
# and left "which two" to be taken on trust.
#
# 5 -> 2 on 2026-09-09, and TWO is the baseline: the ratchet is GREEN
# for the first time since it landed. `DbConnectionPool.try_acquire`
# and both `verify_cog` functions left when their shared-name variants
# were qualified (T1342). Again `0 new`, again the departures named.
#
# The two that remain are the ORIGINAL baseline and are not a
# regression: `compose_geometric` and `id_geometric` call
# `compose_functors` / `identity_functor`, which nothing in the tree
# declares, and the neighbouring `*_handle` functions return
# `FunctorHandle` where the fields are `InfinityFunctor<Y, X>`.
# Writing them from a call site would be inventing a design nobody
# wrote. See T1344.
#
# 9 -> 5 on 2026-09-09: the four `Notification` constructors left, again
# against a prediction written before the bake — `0 new, 4 gone`, and the
# gate named all four. Root cause T1337 (a method call on an associated
# const), worked around at the four call sites.
OBSERVED = {
    # name                                cause, as the bake itself prints it
    "id_geometric":                       "undefined function: identity_functor",
    "compose_geometric":                  "undefined function: compose_functors",
}

# TWO SPELLINGS, and reading only one made the pattern miss half a
# `--verbose` log (measured 2026-09-09):
#
#   WARN   [lenient] SKIP top-level fn verify_cog (bug-class): …
#   DEBUG  [lenient] SKIP top-level fn verify_cog: …
#
# The name is what sits between the marker and either the class or the
# colon, with `top-level fn ` dropped. Both forms name the same function,
# which is why the count is taken over the SET.
SKIP_NAME = re.compile(r"\[lenient\] SKIP (?:top-level fn )?([A-Za-z_][\w.]*)\s*[(:]")


def counts(log_text: str) -> tuple[int, int]:
    """FIELD-GUESS lines, and STUBBED FUNCTIONS — not stub LINES.

    Counting lines made the number depend on the log's verbosity: a
    `--verbose` bake prints each skip twice (a WARN and a DEBUG), so the
    same tree measured 11 without the flag and 18 with it.  A ratchet
    whose value moves with a logging flag is not measuring the tree.
    Measured 2026-09-09 on two logs of the same population."""
    guesses = sum(1 for line in log_text.splitlines() if "FIELD-GUESS-HARD-1" in line)
    return guesses, len(skipped_names(log_text))


def skipped_names(log_text: str) -> set[str]:
    """The stubbed functions BY NAME.

    Kept separate from `counts` so the two can disagree loudly: a SKIP
    line this pattern cannot parse would silently shrink the roster
    comparison while leaving the count right, so `main` checks that the
    two agree before reading either."""
    return {m.group(1) for m in SKIP_NAME.finditer(log_text)}


def compare(found: set, roster: set) -> tuple[list, list]:
    """Split what the bake produced against what the roster claims.

    Separated from the scan so a control can drive it without a bake —
    the half a count ratchet has and never tests."""
    return sorted(found - roster), sorted(roster - found)


def self_test() -> int:
    """Both halves, and both must be able to FAIL.

    The pattern half: a SKIP line the name pattern cannot read shrinks the
    roster comparison while leaving the count right — `main` refuses on
    that disagreement, and this checks the pattern reads both spellings.
    The roster half: THE SWAP, which is the shape a count cannot report."""
    bad = 0
    sample = (
        "[lenient] SKIP top-level fn verify_cog (bug-class): undefined variable\n"
        "[lenient] SKIP QuicPath.on_bytes_sent (bug-class): cannot assign\n"
    )
    got = skipped_names(sample)
    if got != {"verify_cog", "QuicPath.on_bytes_sent"}:
        print(f"self-test: SKIP_NAME read {sorted(got)} from two lines — the "
              "top-level and method spellings are not both handled",
              file=sys.stderr)
        bad += 1
    if len(got) != sum(1 for l in sample.splitlines() if "[lenient] SKIP" in l):
        print("self-test: the parsed names and the line count disagree on a "
              "sample where they must not", file=sys.stderr)
        bad += 1

    app, gone = compare({"a"}, {"b"})
    if not app or not gone:
        print("self-test: a swap of equal size reported nothing — the roster "
              "comparison has degenerated back into a count", file=sys.stderr)
        bad += 1
    if compare({"a"}, {"a"}) != ([], []):
        print("self-test: an unchanged population reported a difference",
              file=sys.stderr)
        bad += 1

    if bad:
        return 1
    print(f"[ok] self-test: SKIP_NAME reads {len(got)} name(s) from two "
          f"spellings; roster holds {len(OBSERVED)}, baseline stays at "
          f"{BASELINE_PANIC_STUBS}; a same-size swap is reported")
    return 0


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        return self_test()
    args = [a for a in sys.argv[1:] if a != "--check"]
    check = "--check" in sys.argv[1:]
    if len(args) != 1:
        print(__doc__, file=sys.stderr)
        return 2

    path = pathlib.Path(args[0])
    if not path.is_file():
        print(f"bake log not found: {path}", file=sys.stderr)
        return 2

    text = path.read_text(encoding="utf-8", errors="ignore")
    guesses, stubs = counts(text)

    # A log that contains NEITHER marker is far more likely to be the wrong
    # file — or a bake that died early — than a perfect bake. Refuse to report
    # a clean sheet we cannot distinguish from an empty one.
    # A completed bake announces itself one of two ways: the build-script
    # wrapper prints "Modules compiled"/"Archive size", and the precompiler
    # binary run directly ends with
    # "verum_stdlib_precompiler: 590 modules, 49956 functions in …s, … bytes".
    # The second form was missing here, so a genuine 3092-line bake log was
    # rejected as "not a bake" — the refusal was right in spirit and wrong in
    # its list.
    completed = (
        "Modules compiled" in text
        or "Archive size" in text
        or "verum_stdlib_precompiler:" in text
    )
    if not completed:
        print(
            f"{path} does not look like a completed bake log (no 'Modules compiled' "
            f"or 'Archive size' line). Refusing to read 0/0 as success.",
            file=sys.stderr,
        )
        return 2

    names = skipped_names(text)
    print(f"FIELD-GUESS-HARD-1 : {guesses} (baseline {BASELINE_FIELD_GUESS})")
    print(f"stubbed function(s): {stubs} (baseline {BASELINE_PANIC_STUBS}, "
          f"{len(OBSERVED)} on the roster) "
          f"— {sum(1 for l in text.splitlines() if '[lenient] SKIP' in l)} log line(s)")

    # EVERY SKIP LINE MUST YIELD A NAME.  This used to compare the line
    # count against the parsed-name count and refuse when they differed —
    # which fired on a `--verbose` log for the wrong reason: the pattern
    # had read every name, and the log simply printed each skip twice.
    # The guard was right to fire and wrong about why, so it now asks the
    # question it meant to ask: is there a SKIP line this pattern cannot
    # read? A pattern gap would shrink the roster comparison silently,
    # which is the shape this file exists to refuse.
    unparsed = [
        line.strip()
        for line in text.splitlines()
        if "[lenient] SKIP" in line and not SKIP_NAME.search(line)
    ]
    if unparsed:
        print(
            f"{len(unparsed)} SKIP line(s) that SKIP_NAME cannot read, so the "
            f"roster comparison below would be taken over a subset. First:\n"
            f"    {unparsed[0][:160]}\n"
            "Fix the pattern before reading any number.",
            file=sys.stderr,
        )
        return 2

    appeared, disappeared = compare(names, set(OBSERVED))
    for n in sorted(names):
        mark = "NEW " if n in set(appeared) else "    "
        print(f"    {mark}{n}  —  {OBSERVED.get(n, 'cause not on the roster')}")
    for n in disappeared:
        print(f"    GONE {n}  —  no longer stubbed; remove it from OBSERVED")

    if not check:
        return 0

    failed = False
    if appeared or disappeared:
        print(
            f"MEMBERSHIP CHANGED: {len(appeared)} new, {len(disappeared)} gone. "
            f"The roster in this file is a description of the current red, not "
            f"a baseline — update it in the commit that changes the population, "
            f"and do NOT raise BASELINE_PANIC_STUBS to match (nightly.yml "
            f"records why). The population is T1315's.",
            file=sys.stderr,
        )
        failed = True
    for name, got, want in (
        ("FIELD-GUESS-HARD-1", guesses, BASELINE_FIELD_GUESS),
        ("panic stubs", stubs, BASELINE_PANIC_STUBS),
    ):
        if got > want:
            print(
                f"RATCHET: {name} rose to {got} (baseline {want}). Each one is a "
                f"defect the build does NOT fail on — a guessed field index or a "
                f"function replaced by a runtime panic.",
                file=sys.stderr,
            )
            failed = True
        elif got < want:
            print(
                f"RATCHET: {name} dropped to {got} (baseline {want}). Lower the "
                f"baseline in the same commit that earns it — a silently improving "
                f"number is how a gate stops measuring.",
                file=sys.stderr,
            )
            failed = True

    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
