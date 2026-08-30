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

# The overlap as measured on 2026-08-30. A ratchet, not a target: it may
# only go DOWN. Renumbering is tracked separately.
BASELINE_COLLISIONS = 41


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
    args = ap.parse_args()

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

    print(f"error-code namespaces: {len(three)} three-digit, {len(four)} four-digit")
    print(
        f"four-digit codes whose three-digit twin is a DIFFERENT registered "
        f"code: {len(collisions)} (baseline {BASELINE_COLLISIONS})"
    )
    if args.list:
        for c in collisions:
            print(f"    {c} ~ E{c[2:]}")

    if len(collisions) > BASELINE_COLLISIONS:
        added = len(collisions) - BASELINE_COLLISIONS
        print(
            f"\n{added} NEW collision(s). A code printed as `E0xxx` whose "
            f"`Exxx` twin means something else sends a reader who dropped the "
            f"zero to the wrong explanation.\n"
            f"Pick a number no other namespace uses, or renumber deliberately "
            f"under the renumbering task.",
            file=sys.stderr,
        )
        return 1 if args.check else 0

    if len(collisions) < BASELINE_COLLISIONS:
        print(
            f"\nOverlap SHRANK to {len(collisions)}. Lower BASELINE_COLLISIONS "
            f"to {len(collisions)} so the ratchet holds the ground gained.",
            file=sys.stderr,
        )
        return 1 if args.check else 0

    print("no new overlap")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
