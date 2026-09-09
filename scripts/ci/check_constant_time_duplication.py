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

# THE COUNT BECAME A ROSTER, 2026-09-09 (T1330).  A bare count says how
# many duplicate constant-time declarations are tolerated and never WHICH,
# so the shape it cannot report is the SWAP: route one through
# `core.subtle`, add another somewhere else, and the total is unchanged and
# the gate prints `none new`.  Measured the same day on a sibling gate of
# identical construction (`check_platform_call_parity.py`): with the
# population held at one and its membership replaced, the count ratchet
# printed `[ok] … none new` over a tree that had just acquired the defect.
#
# The key is `(name, file)` and carries NO line number: a roster keyed on
# positions goes red when an unrelated edit above shifts a line, which
# teaches the reader to re-baseline without looking.
KNOWN: set[tuple[str, str]] = {
    ("ct_eq", "core/database/postgres/auth/scram.vr"),
    ("constant_time_bytes_eq", "core/net/tls13/handshake/client_sm.vr"),
    ("constant_time_eq", "core/net/tls13/handshake/psk.vr"),
    ("constant_time_eq", "core/net/tls13/handshake/resume_verify.vr"),
    ("constant_time_bytes_eq", "core/net/tls13/handshake/server_sm.vr"),
    ("ct_eq_bytes", "core/security/kdf/argon2.vr"),
    ("ct_eq", "core/security/sigstore/rekor.vr"),
    ("ct_eq_hex", "core/security/tuf/client.vr"),
}


def compare(
    found: set[tuple[str, str]],
    roster: set[tuple[str, str]],
) -> tuple[list, list]:
    """Split what the tree has against what the roster claims.

    Separated from the scan so a control can drive it without a tree —
    the half a count ratchet has and never tests."""
    return sorted(found - roster), sorted(roster - found)


def shown_path(path: Path) -> str:
    try:
        return str(path.relative_to(REPO))
    except ValueError:
        return str(path)


def self_test() -> int:
    """THE SWAP, which is the shape the count ratchet could not report and
    the reason this gate carries a roster.  Population size is 1 in both
    polarities; the membership differs, and a count is satisfied by both."""
    before = {("ct_eq", "core/security/sigstore/rekor.vr")}
    after = {("ct_eq", "core/security/tuf/client.vr")}
    appeared, disappeared = compare(after, before)
    if not appeared or not disappeared:
        print(
            "self-test: a swap of equal size reported nothing — the roster "
            "comparison has degenerated back into a count",
            file=sys.stderr,
        )
        return 1
    if compare(before, before) != ([], []):
        print("self-test: an unchanged population reported a difference", file=sys.stderr)
        return 1
    print("[ok] self-test: a same-size swap is reported")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()

    sources = sorted(CORE.rglob("*.vr"))
    if not sources:
        print(f"check-constant-time-duplication: no .vr files under {CORE}", file=sys.stderr)
        return 2

    findings: dict[tuple[str, str], list[str]] = {}
    for path in sources:
        if CANONICAL in path.parents:
            continue
        rel = shown_path(path)
        for lineno, line in enumerate(path.read_text(errors="ignore").splitlines(), 1):
            m = DECL.match(line)
            if m:
                findings.setdefault((m.group(1), rel), []).append(f"{rel}:{lineno}")

    total = sum(len(v) for v in findings.values())
    appeared, disappeared = compare(set(findings), KNOWN)
    bad = bool(appeared) or bool(disappeared)

    if "--list" in sys.argv or bad:
        stream = sys.stderr if bad else sys.stdout
        print(
            f"constant-time duplication: {total} declaration(s) outside core/subtle/ "
            f"({len(KNOWN)} on the roster)",
            file=stream,
        )
        for key in sorted(findings):
            mark = "NEW " if key in set(appeared) else "    "
            print(f"  {mark}{key[0]}", file=stream)
            for site in findings[key]:
                print(f"          {site}", file=stream)

    if appeared:
        print(
            "\nThe declaration(s) marked NEW are not on the roster in this file.\n"
            "Route the call through core.subtle.constant_time instead. A name that\n"
            "promises constant time should have one implementation to promise it of.\n"
            "If it has to stay, add it to KNOWN with the reason.",
            file=sys.stderr,
        )
        return 1
    if disappeared:
        print(
            "constant-time duplication: the roster claims declaration(s) the tree no "
            "longer has —\n"
            + "".join(f"  {name}  {rel}\n" for name, rel in disappeared)
            + "Remove them from KNOWN in this file; the population shrank and the\n"
            "roster has to say so by name, not by a smaller number.",
            file=sys.stderr,
        )
        return 1
    print(f"[ok] constant-time duplication: {total} known declaration(s), roster exact")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
