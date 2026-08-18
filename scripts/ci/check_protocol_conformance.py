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

BASELINE = 7


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
    sources = [(p, p.read_text(errors="ignore").splitlines()) for p in sorted(CORE.rglob("*.vr"))]
    if not sources:
        print(f"check-protocol-conformance: no .vr files under {CORE}", file=sys.stderr)
        return 2

    required, defaulted = collect_protocols(sources)

    findings = []
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
                findings.append((f"{shown_path(path)}:{i + 1}", protocol, m.group(2), sorted(missing)))

    total = len(findings)
    if "--list" in sys.argv or total > BASELINE:
        stream = sys.stderr if total > BASELINE else sys.stdout
        print(f"protocol conformance: {total} incomplete implementations (baseline {BASELINE})",
              file=stream)
        for site, protocol, target, missing in findings:
            shown = ", ".join(missing[:4]) + ("…" if len(missing) > 4 else "")
            print(f"  {protocol} for {target} is missing {shown}\n      {site}", file=stream)

    if total > BASELINE:
        print(
            "\nEach of these type-checks and panics at the call instead. A bound on\n"
            "the protocol promises a method the type does not have.",
            file=sys.stderr,
        )
        return 1
    if total < BASELINE:
        print(f"protocol conformance: {total} found, below baseline {BASELINE} — lower BASELINE.")
        return 1
    print(f"[ok] protocol conformance: {total} known-incomplete implementations, none new")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
