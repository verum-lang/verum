#!/usr/bin/env python3
"""Fail when a constant-time comparator is hand-rolled outside core/subtle/.

WHY THIS EXISTS
---------------
core/subtle/constant_time.vr declares the canonical primitives:

    public fn constant_time_eq(a: &[Byte], b: &[Byte]) -> Bool
    public fn constant_time_compare(a: &[Byte], b: &[Byte]) -> Int

Exactly ONE call site used them when this gate landed — a two-line wrapper
in core/net/tls13/handshake/psk.vr. Seven others hand-rolled the same
OR-accumulator privately, under six different names: ct_eq (twice,
in different subsystems), ct_eq_bytes, ct_eq_hex, constant_time_eq and
constant_time_bytes_eq (twice, in the SAME subsystem).

Every one of them is genuinely constant-time today — I read all seven
bodies. The defect is not that one is wrong; it is that nothing holds them
to it. A name beginning ct_ or constant_time_ is a promise about a security
property, made seven times by seven private functions that no gate compares
against the primitive they duplicate. Beside them sit nineteen
VARIABLE-time byte comparators under five more names, so an author writing
new security code faces two undifferentiated families with overlapping
names and no canonical entry point — and choosing wrong is a silent defect,
not a compile error.

Not every variable-time comparison is a mistake:
core/security/password_hash.vr compares an ALGORITHM IDENTIFIER, where
timing carries no secret. That is exactly the point — the distinction is
real and load-bearing, and it currently lives only in each author's head.

WHAT THIS GATE CHECKS
---------------------
Declarations whose name announces constant time — ct_* or constant_time_* —
that live outside core/subtle/. Eight today, and one of the eight is NOT a
hand-rolled accumulator: core/net/tls13/handshake/psk.vr:234 is a two-line
wrapper that delegates to the canonical primitive. It is counted anyway,
because the gate deliberately does not read bodies, and because a local
re-declaration of the name is still a place where the promise can drift
from the implementation. Seven is the number of independent accumulators;
eight is the number of declarations, and eight is what this ratchets. It does NOT inspect bodies: a body that
looks like an accumulator can still be compiled to an early exit, and a
gate that judged implementations would give exactly the false assurance
this family cannot afford. The remedy is to have one implementation, and
this counts the ones that are not it.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CORE = REPO / "core"
CANONICAL = CORE / "subtle"

DECL = re.compile(
    r"^[ \t]*(?:(?:public|pub)\s+)?(?:async\s+|unsafe\s+|pure\s+)*fn\s+"
    r"((?:ct_|constant_time_)\w*)\s*[<(]"
)

BASELINE = 8


def shown_path(path: Path) -> str:
    try:
        return str(path.relative_to(REPO))
    except ValueError:
        return str(path)


def main() -> int:
    sources = sorted(CORE.rglob("*.vr"))
    if not sources:
        print(f"check-constant-time-duplication: no .vr files under {CORE}", file=sys.stderr)
        return 2

    findings = []
    for path in sources:
        if CANONICAL in path.parents:
            continue
        for lineno, line in enumerate(path.read_text(errors="ignore").splitlines(), 1):
            m = DECL.match(line)
            if m:
                findings.append((f"{shown_path(path)}:{lineno}", m.group(1)))

    total = len(findings)
    if "--list" in sys.argv or total > BASELINE:
        stream = sys.stderr if total > BASELINE else sys.stdout
        print(
            f"constant-time duplication: {total} declaration(s) outside core/subtle/ "
            f"(baseline {BASELINE})",
            file=stream,
        )
        for site, name in findings:
            print(f"  {name}\n      {site}", file=stream)

    if total > BASELINE:
        print(
            "\nRoute the call through core.subtle.constant_time instead. A name that\n"
            "promises constant time should have one implementation to promise it of.",
            file=sys.stderr,
        )
        return 1
    if total < BASELINE:
        print(f"constant-time duplication: {total} found, below baseline {BASELINE} — lower it.")
        return 1
    print(f"[ok] constant-time duplication: {total} known declaration(s), none new")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
