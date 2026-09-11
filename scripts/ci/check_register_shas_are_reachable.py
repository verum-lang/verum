#!/usr/bin/env python3
"""A commit cited by the tech-debt register must be reachable from `main`.

Measured 2026-09-06: of 112 sha-like tokens in
`docs/architecture/tech-debt-register.md`, FORTY-SIX name real commits that
are on NO branch — dangling objects, several from June and July, reachable
only from the reflog. (A first count said fifteen; it had been taken over
the `A`-rows alone, which are a third of the file.) A row that says "fixed in 34129a636" and points at a
commit the project no longer has is worse than a row with no citation:
the reader believes there is something to look at.

Rebases are how they get there. A commit is cited while it is on a
branch; the branch is rebased onto main; the citation keeps the OLD sha,
which becomes unreachable the moment the reflog expires.

    check_register_shas_are_reachable.py            report
    check_register_shas_are_reachable.py --check    exit 1 above the baseline
    check_register_shas_are_reachable.py --self-test

BASELINE is a ratchet, not zero: the forty-six already there cannot be
repaired by this script — recovering what each one became is forty-six
separate investigations. The ratchet stops the population growing, which
is the part that is cheap and the part that matters.
"""
from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
REGISTER = REPO / "docs" / "architecture" / "tech-debt-register.md"

# THE COUNT BECAME A ROSTER, 2026-09-09 (T1330, T1333).  `BASELINE = 46`
# said how many dead citations were tolerated and never WHICH, so it could
# not report a SWAP — and a swap is the LIKELY event here, because the
# docstring above names the mechanism that produces these continuously:
# a rebase turns a live citation into a dead one at any time.  Repair one
# row, rebase another branch, and the count is still 46.
#
# THE GATE ALSO REPORTED A CLEAN SHEET WHERE IT CANNOT MEASURE (T1333).
# It asked `git cat-file -t` and then `merge-base --is-ancestor`, and a
# citation whose object is NOT PRESENT locally was dropped in silence.
# Dangling objects are not cloned: in a fresh checkout all forty-six would
# be absent rather than unreachable, `dangling` would be 0, and the gate
# printed
#
#     0 unreachable, below the baseline of 46 — lower BASELINE to hold
#     the gain.
#
# — an invitation to set the ratchet to zero on a measurement that
# measured nothing, after which the gate is red forever in every working
# copy that still HAS the objects.  Absence is now a REFUSAL, not a pass.
DANGLING = {
    "01dbb1024", "072fafab4", "11a6aafb6", "1aad1409a", "25ed364cd", "2d8dd42f0",
    "34129a636", "40a41bc13", "44343afc1", "44d8b8620", "4a667d087", "4bddeaa8d",
    "58f5519af", "58fd3947b", "5cfb3c0e3", "6e984cc01", "710c4c6b2", "7eca17515",
    "83be4c7aa", "85253727b", "88629f3d5", "89d95451d", "8bf486ddc", "8f82d9f16",
    "94e212dae", "968f09f80", "99989c7a5", "a068e0f89", "a12de4287", "a4346829f",
    "a74924270", "a90449a4d", "aab469ef0", "b0bb38dfc", "b494ddc169", "bfc328704",
    "c8f6205aa", "ca22a3891", "d054c2b13", "d135ad92d", "da30100b9", "dfb47e4b1",
    "eaa332813", "f15dae9e9", "f59611876", "fed8b940b",
}
BASELINE = len(DANGLING)

# NOT COMMITS AT ALL, and the reason the denominator was inflated: the
# sha pattern matches any 7-40 hex run that is not all digits, and seven
# such runs in the register are something else.  They were silently
# dropped before, which is why "absent locally" could not be used as the
# discriminator for "this checkout has no objects" until they were named.
#
#   ed25519            an algorithm name that happens to be all hex
#   187337bc           a SESSION id ("session 187337bc")
#   99f6b93e           the same ("2026-07-25 re-measured (99f6b93e)")
#   936a8375           a sha256 of CONTENT ("byte-identical (sha256 …)")
#   1452fdec87f1d17e   a sha256 of a snapshot
#   cce44fb820de7fe1   a blake3 cache key
#   1754943508222875e  CUT OUT OF THE NUMBER 1.1754943508222875e-38 — the
#                      pattern forbids an all-DIGIT token, and this one
#                      ends in `e`, with the decimal point counting as a
#                      word boundary on its left
NOT_COMMITS = {
    "ed25519", "187337bc", "99f6b93e", "936a8375",
    "1452fdec87f1d17e", "cce44fb820de7fe1", "1754943508222875e",
}


def compare(found: set, roster: set) -> tuple[list, list]:
    """Split what the register cites against what the roster claims.

    Separated from the scan so a control can drive it without a git
    history — the half a count ratchet has and never tests."""
    return sorted(found - roster), sorted(roster - found)

# A sha-like token: 7-40 hex chars, not all digits (a bare number is a
# count, a line, a size — never a commit). Word-bounded so `0x7ff9…` and
# the inside of a longer hash do not match.
SHA = re.compile(r"\b(?![0-9]+\b)([0-9a-f]{7,40})\b")


# A citation that names ANOTHER repository before the sha. The register
# cites the website repo this way — "…which is pure Verum and runs —
# website b742125" — and that commit is real, with a title matching the
# claim, in the sibling checkout. It is simply not an object HERE.
#
# Before this, such a citation reached the dangling branch and the gate
# refused to report at all, which is the right refusal for the wrong
# reason: a foreign sha is not evidence that this checkout is shallow.
# Measured 2026-09-11: one citation, `website b742125`, and the gate had
# never run in CI to say so (the target is in no aggregate — T1439).
#
# Deliberately NOT resolved against the sibling checkout: a fresh clone
# does not have one, and a gate whose answer depends on what else is on
# the disk is the kind this file already refuses to be.
FOREIGN = re.compile(r"\b(?:website|site|registry)\s+([0-9a-f]{7,40})\b")


def foreign_shas(text: str) -> set:
    """Shas the register explicitly attributes to another repository."""
    return {m.group(1) for m in FOREIGN.finditer(text)}


def cited_shas(text: str) -> list[str]:
    foreign = foreign_shas(text)
    return sorted({m.group(1) for m in SHA.finditer(text)} - foreign)


def is_commit(sha: str) -> bool:
    r = subprocess.run(["git", "cat-file", "-t", sha],
                       capture_output=True, text=True, cwd=REPO)
    return r.returncode == 0 and r.stdout.strip() == "commit"


def reachable_from_main(sha: str) -> bool:
    r = subprocess.run(["git", "merge-base", "--is-ancestor", sha, "main"],
                       capture_output=True, cwd=REPO)
    return r.returncode == 0


SELF_TEST = [
    # (text, how many sha-like tokens it yields) — the filter must not
    # take a decimal count or a hex constant for a commit.
    ("fixed in 34129a636 (T0123)", 1),
    ("measured 15 files, 4096 blocks", 0),
    ("the nil sentinel 0x7ff9000000000000", 0),
    ("commits 072fafab4 and 1aad1409a", 2),
    ("no shas here at all", 0),
    # A sha attributed to another repository is not this repository's to
    # resolve, and must not reach the dangling branch.
    ("pure Verum and runs — website b742125", 0),
    # …but only when the attribution is actually there. The same sha with
    # no repository named in front of it is this repository's problem.
    ("pure Verum and runs — b742125", 1),
]


def self_test() -> int:
    bad = 0
    for text, want in SELF_TEST:
        got = len(cited_shas(text))
        if got != want:
            bad += 1
            print(f"FAIL {text!r} -> {got}, expected {want}", file=sys.stderr)
    # THE SWAP, which is the shape the count could not report and the
    # reason this gate carries a roster.  Population size is 1 in both
    # polarities; the membership differs, and a count is satisfied by both.
    app, gone = compare({"34129a636"}, {"01dbb1024"})
    if not app or not gone:
        bad += 1
        print("FAIL: a swap of equal size reported nothing — the roster "
              "comparison has degenerated back into a count", file=sys.stderr)
    if compare({"01dbb1024"}, {"01dbb1024"}) != ([], []):
        bad += 1
        print("FAIL: an unchanged population reported a difference",
              file=sys.stderr)

    # THE SEVEN NON-COMMITS MUST STAY EXCLUDED.  `1754943508222875e` is cut
    # out of the float `1.1754943508222875e-38`, and until it was named the
    # gate could not tell "not a commit" from "this checkout has no
    # objects" — which is what let an unmeasurable run report zero.
    if cited_shas("the constant evaluates to 1.1754943508222875e-38 since T1314"):
        if not (set(cited_shas("1.1754943508222875e-38")) <= NOT_COMMITS):
            bad += 1
            print("FAIL: a float fragment is read as a commit and is not in "
                  "NOT_COMMITS", file=sys.stderr)

    if bad:
        print(f"self-test: {bad} case(s) FAILED", file=sys.stderr)
        return 1
    print(f"check-register-shas --self-test: {len(SELF_TEST)}/{len(SELF_TEST)} "
          f"pattern case(s) pass, roster holds {len(DANGLING)} sha(s), "
          f"{len(NOT_COMMITS)} hex tokens excluded")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not REGISTER.exists():
        print(f"check-register-shas: no register at {REGISTER}", file=sys.stderr)
        return 1
    shas = set(cited_shas(REGISTER.read_text())) - NOT_COMMITS
    present = {s for s in shas if is_commit(s)}
    absent = sorted(shas - present)

    # AN ABSENCE IS A REFUSAL, NOT A PASS.  Dangling objects are not
    # cloned, so in a fresh checkout every roster entry lands here — and
    # the old code dropped them in silence and reported zero unreachable.
    if absent:
        on_roster = [s for s in absent if s in DANGLING]
        print(
            f"check-register-shas: REFUSING to report — {len(absent)} cited "
            f"sha(s) are not objects in this repository "
            f"({len(on_roster)} of them on the roster).\n"
            f"    {' '.join(absent[:12])}{' …' if len(absent) > 12 else ''}\n"
            "A dangling commit is not fetched by `git clone`, so a fresh\n"
            "checkout cannot answer this question at all — reporting zero\n"
            "unreachable there would be an invitation to zero the ratchet.\n"
            "If a sha is not a commit citation, add it to NOT_COMMITS with\n"
            "the reason; if it is, this checkout is the wrong place to run\n"
            "this gate.",
            file=sys.stderr,
        )
        return 2

    dangling = {s for s in present if not reachable_from_main(s)}
    appeared, disappeared = compare(dangling, DANGLING)
    print(f"check-register-shas: {len(shas)} commit citation(s) "
          f"({len(NOT_COMMITS)} hex tokens excluded as non-commits), "
          f"{len(dangling)} unreachable from main, {len(DANGLING)} on the roster")
    for s in sorted(dangling):
        subj = subprocess.run(["git", "log", "-1", "--format=%s", s],
                              capture_output=True, text=True, cwd=REPO).stdout.strip()
        mark = "NEW " if s in set(appeared) else "    "
        print(f"    {mark}{s}  {subj[:60]}")

    if appeared:
        print(f"\n{len(appeared)} citation(s) marked NEW are not on the roster: "
              f"{' '.join(appeared)}\n"
              "A row that cites a commit the project no longer has is worse "
              "than a row with no citation — the reader believes there is "
              "something to look at. Repoint the row at the surviving commit, "
              "or add the sha to DANGLING with the reason it cannot be "
              "recovered.", file=sys.stderr)
        return 1
    if disappeared:
        print(f"\nThe roster claims {len(disappeared)} citation(s) that are now "
              f"reachable: {' '.join(disappeared)}\n"
              "Remove them from DANGLING in this file — the ground gained is "
              "recorded by NAME, not by a smaller number.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
