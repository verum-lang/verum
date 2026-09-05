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

# Measured 2026-09-06. Lower it when a citation is repaired; never raise it.
BASELINE = 46

# A sha-like token: 7-40 hex chars, not all digits (a bare number is a
# count, a line, a size — never a commit). Word-bounded so `0x7ff9…` and
# the inside of a longer hash do not match.
SHA = re.compile(r"\b(?![0-9]+\b)([0-9a-f]{7,40})\b")


def cited_shas(text: str) -> list[str]:
    return sorted({m.group(1) for m in SHA.finditer(text)})


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
]


def self_test() -> int:
    bad = 0
    for text, want in SELF_TEST:
        got = len(cited_shas(text))
        if got != want:
            bad += 1
            print(f"FAIL {text!r} -> {got}, expected {want}", file=sys.stderr)
    if bad:
        print(f"self-test: {bad} case(s) FAILED", file=sys.stderr)
        return 1
    print(f"check-register-shas --self-test: {len(SELF_TEST)}/{len(SELF_TEST)} pass")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not REGISTER.exists():
        print(f"check-register-shas: no register at {REGISTER}", file=sys.stderr)
        return 1
    shas = cited_shas(REGISTER.read_text())
    dangling = [s for s in shas if is_commit(s) and not reachable_from_main(s)]
    print(f"check-register-shas: {len(shas)} sha-like token(s), "
          f"{len(dangling)} naming a commit unreachable from main")
    for s in dangling:
        subj = subprocess.run(["git", "log", "-1", "--format=%s", s],
                              capture_output=True, text=True, cwd=REPO).stdout.strip()
        print(f"    {s}  {subj[:64]}")
    if "--check" in sys.argv and len(dangling) > BASELINE:
        print(f"{len(dangling)} unreachable citation(s), baseline {BASELINE}.",
              file=sys.stderr)
        return 1
    if "--check" in sys.argv and len(dangling) < BASELINE:
        print(f"{len(dangling)} unreachable, below the baseline of {BASELINE} — "
              f"lower BASELINE to hold the gain.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
