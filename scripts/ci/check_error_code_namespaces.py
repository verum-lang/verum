#!/usr/bin/env python3
"""Gate: the tree has ONE error-code namespace, and `explain` answers about
the code the user actually saw.

Measured 2026-08-30, and the shape is worse than "a code the registry does
not know":

    error<E0203>: Result type mismatch in '?' operator   <- what is printed
    $ verum explain E203                                 <- the zero dropped
      module not found                                   <- a different defect

Two spellings are in use — `Exxx` in `verum_error`'s registry and `E0xxx`
in `verum_diagnostics`' explanations plus `verum_compiler`'s lints — and
where their digits coincide their MEANINGS do not:

    E0101 use-after-free        E101 undefined type
    E0313 integer overflow      E313 dangling reference
    E0203 `?` type mismatch     E203 module not found

So a user who types the code they saw, minus a leading zero that no other
code has, is confidently told about something else. That is worse than
"code not found", which at least fails honestly.

WHAT THIS GATE DOES. It counts, and it ratchets. Renumbering one of the
two namespaces is a large change with its own task; until then this
refuses to let the overlap GROW, which is the part that costs nothing to
enforce and everything to discover late.

    scripts/ci/check_error_code_namespaces.py [--check] [--list]
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
REGISTRY = REPO / "crates" / "verum_error" / "src" / "registry.rs"

# How a code reaches a user: a DiagnosticBuilder code, a registry entry,
# or a lint's `error_code` field.
EMITTED = re.compile(r'(?:\.code\(|code:\s*|error_code:\s*)"(E\d{3,4})"')

# THE COUNT BECAME A ROSTER, 2026-09-09 (T1330), and doing so revealed
# that the ratchet had never fired.  Two separate defects, kept separate:
#
# 1. THE VERDICT LIVED IN A FLAG NOBODY PASSED.  Both failure branches read
#    `return 1 if args.check else 0`, and the Makefile invokes this script
#    bare.  So the gate printed `21 NEW collision(s)` and exited ZERO, and
#    `gates-source` reported "all source-only gates green" over it.  A gate
#    that says FAIL and exits 0 is worse than no gate: it teaches the
#    reader that this script's output is noise.  Fixed here — a finding is
#    a failure whether or not `--check` was asked for.
#
# 2. TWENTY-ONE COLLISIONS ARRIVED UNOBSERVED, over ten days.  A/B that
#    separates the instrument from the tree, because "the gate got
#    stricter" is the cheaper explanation and it is wrong here — this file
#    has exactly ONE commit (fa8c67c9b) and had never been edited:
#
#        gate fa8c67c9b vs tree fa8c67c9b   51 four-digit, 41 collisions
#        gate fa8c67c9b vs tree today       71 four-digit, 62 collisions
#
#    Same script, both runs.  Twenty four-digit codes added, twenty-one
#    collisions with them, ZERO gone.  They are spread over four crates
#    and four mechanisms (LSP diagnostics, a lint's `error_code`, a
#    registry entry, a meta builtin), which is why no reviewer saw them as
#    a group.  Renumbering is T1332 and is NOT blessed by this roster: the
#    roster records that they are KNOWN, T1332 records that they are WRONG.
#
# The key is the four-digit code itself.  A count could not have told the
# difference between "one renumbered, one added" and "nothing happened".
ARRIVED_UNBLESSED = {
    "E0000", "E0002", "E0003", "E0004", "E0010", "E0011", "E0012",
    "E0013", "E0020", "E0099", "E0204", "E0319", "E0410", "E0412",
    "E0601", "E0604", "E0701", "E0702", "E0803", "E0804", "E1000",
}
KNOWN = {
    "E0000", "E0001", "E0002", "E0003", "E0004", "E0010",
    "E0011", "E0012", "E0013", "E0020", "E0099", "E0101",
    "E0102", "E0103", "E0201", "E0202", "E0203", "E0204",
    "E0302", "E0303", "E0304", "E0305", "E0306", "E0310",
    "E0311", "E0312", "E0313", "E0314", "E0315", "E0316",
    "E0317", "E0318", "E0319", "E0400", "E0401", "E0402",
    "E0403", "E0404", "E0405", "E0406", "E0410", "E0412",
    "E0500", "E0501", "E0601", "E0604", "E0700", "E0701",
    "E0702", "E0800", "E0801", "E0803", "E0804", "E0900",
    "E0901", "E0902", "E1000", "E1001", "E1002", "E1003",
    "E1004", "E1005",
}
assert ARRIVED_UNBLESSED <= KNOWN, "the unblessed set must be part of the roster"
BASELINE_COLLISIONS = len(KNOWN)


def compare(found: set[str], roster: set[str]) -> tuple[list, list]:
    """Split what the tree has against what the roster claims.

    Separated from the scan so a control can drive it without a tree —
    the half a count ratchet has and never tests."""
    return sorted(found - roster), sorted(roster - found)


def self_test() -> int:
    """THE SWAP, which is the shape the count ratchet could not report and
    the reason this gate carries a roster.  Population size is 1 in both
    polarities; the membership differs, and a count is satisfied by both."""
    appeared, disappeared = compare({"E0404"}, {"E0405"})
    if not appeared or not disappeared:
        print("self-test: a swap of equal size reported nothing — the roster "
              "comparison has degenerated back into a count", file=sys.stderr)
        return 1
    if compare({"E0404"}, {"E0404"}) != ([], []):
        print("self-test: an unchanged population reported a difference",
              file=sys.stderr)
        return 1
    # The verdict must not be conditional on a flag: that is defect 1 above,
    # and a control that only checks `--check` would not have caught it.
    #
    # THE GUARD READS CODE, NOT PROSE.  Written first as a substring search
    # over the whole file, it matched the COMMENT above that quotes the old
    # branch verbatim — a guard failing on the sentence explaining why it
    # exists.  Comment and blank lines are stripped before the search.
    offenders = [
        n for n, line in enumerate(Path(__file__).read_text().splitlines(), 1)
        if not line.lstrip().startswith("#")
        and line.lstrip().startswith("return ")
        and "args.check" in line
    ]
    if offenders:
        print(f"self-test: line(s) {offenders} return a verdict that depends on "
              "--check — the verdict is back in a flag", file=sys.stderr)
        return 1
    print(f"[ok] self-test: roster holds {len(KNOWN)} code(s), "
          f"{len(ARRIVED_UNBLESSED)} of them unblessed debt under T1332; "
          f"a same-size swap is reported")
    return 0


def scan() -> tuple[set[str], set[str], set[str]]:
    three: set[str] = set()
    four: set[str] = set()
    for path in (REPO / "crates").rglob("*.rs"):
        if "target" in path.parts:
            continue
        try:
            txt = path.read_text(errors="replace")
        except OSError:
            continue
        for code in EMITTED.findall(txt):
            (four if len(code) == 5 else three).add(code)
    known: set[str] = set()
    if REGISTRY.is_file():
        known = set(re.findall(r'code:\s*"(E\d{3,4})"', REGISTRY.read_text()))
    return three, four, known


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    three, four, known = scan()

    # Positive control: an empty scan would report a clean zero overlap.
    if len(three) < 50:
        print(
            f"REFUSING TO PASS: found only {len(three)} three-digit codes — "
            "the scan is broken, not the tree.",
            file=sys.stderr,
        )
        return 2

    collisions = sorted(c for c in four if ("E" + c[2:]) in known)
    appeared, disappeared = compare(set(collisions), KNOWN)

    print(f"error-code namespaces: {len(three)} three-digit, {len(four)} four-digit")
    print(
        f"four-digit codes whose three-digit twin is a DIFFERENT registered "
        f"code: {len(collisions)} ({len(KNOWN)} on the roster, "
        f"{len(ARRIVED_UNBLESSED)} of them unblessed debt under T1332)"
    )
    if args.list or appeared or disappeared:
        for c in collisions:
            mark = "NEW " if c in set(appeared) else (
                "debt" if c in ARRIVED_UNBLESSED else "    ")
            print(f"    {mark} {c} ~ E{c[2:]}")

    # THE VERDICT IS NOT CONDITIONAL ON A FLAG.  It used to be
    # `return 1 if args.check else 0`, and the Makefile calls this script
    # bare — so every failure exited 0 and `gates-source` printed "all
    # source-only gates green" over `21 NEW collision(s)`.  `--check` is
    # still accepted so existing invocations keep working; it no longer
    # decides whether a failure is one.
    if appeared:
        print(
            f"\n{len(appeared)} NEW collision(s), not on the roster in this "
            f"file: {' '.join(appeared)}\n"
            f"A code printed as `E0xxx` whose `Exxx` twin means something else "
            f"sends a reader who dropped the zero to the wrong explanation.\n"
            f"Pick a number no other namespace uses, or renumber deliberately "
            f"under T1332.",
            file=sys.stderr,
        )
        return 1

    if disappeared:
        print(
            f"\nThe roster claims {len(disappeared)} collision(s) the tree no "
            f"longer has: {' '.join(disappeared)}\n"
            f"Remove them from KNOWN in this file (and from ARRIVED_UNBLESSED "
            f"if listed there) so the ratchet holds the ground gained — by "
            f"name, not by a smaller number.",
            file=sys.stderr,
        )
        return 1

    print("no new overlap: roster exact")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
