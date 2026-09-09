#!/usr/bin/env python3
"""Fail when an `implement P for T` block omits a method P requires.

WHY THIS EXISTS
---------------
The compiler checks that an implementation EXISTS, not that it is COMPLETE.
Measured 2026-08-18 (T0812), each line with a control in the same batch:

    implement P for R { fn m(&self) -> Int { 7 } }    full   -> runs, 7
    no implement at all, used at a bound <T: P>              -> E405

    implement P for R { }        with P requiring fn m
        r.m()                       checks CLEAN -> panics at run time
        through the bound <T: P>    checks CLEAN -> panics at run time

So a bound `T: P` promises a method the type does not have, and the E405
that catches the same mistake when the implementation is ABSENT is silenced
by writing an empty one.

The four `Debug` implementations that provided `fmt` where
core/base/protocols.vr:303 requires `fmt_debug` are FIXED — Data, MetaSpan,
SourceLocation and TokenStream. `Display` requires `fmt` and `Debug`
requires `fmt_debug`, and the four had simply taken the other protocol's
method name; the same files carry correct examples of both.

The eight that remain are empty marker implementations in the
category-theory modules, whose comments assert the laws hold by
construction — which the protocol does not say.

WHAT COUNTS AS REQUIRED
-----------------------
A protocol method with no body. A method WITH a body is a default and stays
optional — 124 of them in core/, and ignoring that distinction lights up
every Iterator implementation in the library.

INSTRUMENT NOTES — this count moved twice before it settled, both times up
-------------------------------------------------------------------------
A one-line default-body test reported 147, all Iterator adaptors:

    fn cloned<T: Clone>(self) -> Cloned<Self>
    where Self.Item = &T {
        Cloned { inner: self }
    }

the `where` clause pushes the `{` two lines down. Scan forward to the first
`;` or `{` at paren depth zero instead, which is what has_body does.

Scanning an impl body from the line AFTER `implement` reported 20: eight
capability implementations are one-liners —

    implement CanRead for DbUnsafe { fn _cap_can_read() -> Bool { true } }

so the `implement` line itself is scanned too.

Both errors inflate rather than deflate, so a jump in this number is a
reason to check the extractor before believing the library got worse.
"""

from __future__ import annotations

import re
import sys
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CORE = REPO / "core"

PROTO = re.compile(
    r"^[ \t]*(?:(?:public|pub)\s+)?type\s+([A-Z]\w*)(?:<[^>]*>)?\s+is\s+protocol\s*\{"
)
IMPL = re.compile(
    r"^[ \t]*implement\s*(?:<[^>]*>)?\s*([A-Z][\w.]*)(?:<[^>]*>)?\s+for\s+([A-Z][\w.]*)"
)
SIG_LINE = re.compile(
    r"^[ \t]*(?:(?:public|pub)\s+)?(?:async\s+|unsafe\s+|pure\s+)*fn\s+(\w+)\s*[<(]"
)
SIG_ANY = re.compile(
    r"(?:(?:public|pub)\s+)?(?:async\s+|unsafe\s+|pure\s+)*\bfn\s+(\w+)\s*[<(]"
)

# THE COUNT BECAME A ROSTER, 2026-09-09 (T1330).  A bare count says how
# many incomplete implementations are tolerated and never WHICH, so the
# shape it cannot report is the SWAP: complete one implementation, break
# another, and the total is unchanged and the gate prints `none new`.
# Measured the same day on a sibling gate of identical construction
# (`check_platform_call_parity.py`): with the population held at one and
# its membership replaced, the count ratchet printed `[ok] … none new`
# over a tree that had just acquired the defect.
#
# The key is `(protocol, target, file)` and carries NO line number: a
# roster keyed on positions goes red when an unrelated edit above shifts
# a line, which teaches the reader to re-baseline without looking.
#
# All six are in `core/math/`, and none is reachable from the language
# runtime — they are category-theory example structures whose protocols
# demand law methods (`associativity`, `left_identity`, …) that the
# example does not carry.  See T0812.
KNOWN: set[tuple[str, str, str]] = {
    ("Category", "SetCategory", "core/math/concrete_accessible.vr"),
    ("Category", "SmallCatCategory", "core/math/concrete_accessible.vr"),
    ("Category", "TopCategory", "core/math/concrete_accessible.vr"),
    ("Sieve", "ToposExampleMaximalSieve", "core/math/examples.vr"),
    ("InfPresheaf", "ConstantInfPresheaf", "core/math/examples.vr"),
    ("InfSubobjectClassifier", "TrivialInfSubobjectClassifier", "core/math/examples.vr"),
}


def compare(
    found: set[tuple[str, str, str]],
    roster: set[tuple[str, str, str]],
) -> tuple[list, list]:
    """Split what the tree has against what the roster claims.

    Separated from the scan so a control can drive it without a tree —
    the half a count ratchet has and never tests."""
    return sorted(found - roster), sorted(roster - found)


def self_test() -> int:
    """THE SWAP, which is the shape the count ratchet could not report and
    the reason this gate carries a roster.  Population size is 1 in both
    polarities; the membership differs, and a count is satisfied by both."""
    before = {("Category", "SetCategory", "core/math/concrete_accessible.vr")}
    after = {("Category", "TopCategory", "core/math/concrete_accessible.vr")}
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


def shown_path(path: Path) -> str:
    """Repo-relative when possible. The self-test runs the gate against a
    scratch tree outside the repo, and an unguarded relative_to turns a
    correct finding into a traceback."""
    try:
        return str(path.relative_to(REPO))
    except ValueError:
        return str(path)


def block_end(lines: list[str], start: int) -> int | None:
    depth = 0
    opened = False
    for k in range(start, len(lines)):
        if "{" in lines[k]:
            opened = True
        depth += lines[k].count("{") - lines[k].count("}")
        if opened and depth <= 0:
            return k
    return None


def has_body(lines: list[str], start: int, limit: int) -> bool:
    """True when this signature is followed by a body rather than a `;`."""
    depth = 0
    for k in range(start, min(start + 12, limit)):
        for ch in lines[k]:
            if ch in "(<":
                depth += 1
            elif ch in ")>":
                depth -= 1
            elif ch == "{" and depth <= 0:
                return True
            elif ch == ";" and depth <= 0:
                return False
    return False


def collect_protocols(sources):
    required: dict[str, set[str]] = defaultdict(set)
    defaulted: dict[str, set[str]] = defaultdict(set)
    for path, lines in sources:
        for i, line in enumerate(lines):
            m = PROTO.match(line)
            if not m:
                continue
            end = block_end(lines, i)
            if end is None:
                continue
            for j in range(i + 1, end):
                sm = SIG_LINE.match(lines[j])
                if not sm:
                    continue
                bucket = defaulted if has_body(lines, j, end) else required
                bucket[m.group(1)].add(sm.group(1))
    return required, defaulted


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()

    sources = [(p, p.read_text(errors="ignore").splitlines()) for p in sorted(CORE.rglob("*.vr"))]
    if not sources:
        print(f"check-protocol-conformance: no .vr files under {CORE}", file=sys.stderr)
        return 2

    required, defaulted = collect_protocols(sources)

    findings: dict[tuple[str, str, str], tuple[str, list]] = {}
    for path, lines in sources:
        for i, line in enumerate(lines):
            m = IMPL.match(line)
            if not m:
                continue
            protocol = m.group(1).split(".")[-1]
            if not required.get(protocol):
                continue
            end = block_end(lines, i)
            if end is None:
                continue
            # The `implement` line itself may carry the whole body.
            provided = set(SIG_ANY.findall(line[line.find("{"):] if "{" in line else ""))
            for j in range(i + 1, end):
                provided.update(SIG_LINE.findall(lines[j]))
            missing = required[protocol] - provided - defaulted.get(protocol, set())
            if missing:
                rel = shown_path(path)
                findings[(protocol, m.group(2), rel)] = (f"{rel}:{i + 1}", sorted(missing))

    total = len(findings)
    appeared, disappeared = compare(set(findings), KNOWN)
    bad = bool(appeared) or bool(disappeared)

    if "--list" in sys.argv or bad:
        stream = sys.stderr if bad else sys.stdout
        print(
            f"protocol conformance: {total} incomplete implementations "
            f"({len(KNOWN)} on the roster)",
            file=stream,
        )
        for key in sorted(findings):
            protocol, target, _ = key
            site, missing = findings[key]
            mark = "NEW " if key in set(appeared) else "    "
            shown = ", ".join(missing[:4]) + ("…" if len(missing) > 4 else "")
            print(f"  {mark}{protocol} for {target} is missing {shown}\n          {site}",
                  file=stream)

    if appeared:
        print(
            "\nThe implementation(s) marked NEW are not on the roster in this file.\n"
            "Each type-checks and panics at the call instead. A bound on the\n"
            "protocol promises a method the type does not have. If one has to\n"
            "stay, add it to KNOWN with the reason.",
            file=sys.stderr,
        )
        return 1
    if disappeared:
        print(
            "protocol conformance: the roster claims implementation(s) the tree no "
            "longer has —\n"
            + "".join(f"  {proto} for {target}  {rel}\n" for proto, target, rel in disappeared)
            + "Remove them from KNOWN in this file; the population shrank and the\n"
            "roster has to say so by name, not by a smaller number.",
            file=sys.stderr,
        )
        return 1
    print(f"[ok] protocol conformance: {total} known-incomplete implementations, roster exact")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
