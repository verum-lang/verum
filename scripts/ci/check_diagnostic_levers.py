#!/usr/bin/env python3
"""A lever whose VALUE is a name pattern must be on the levers page.

WHY THIS EXISTS
---------------
Most `VERUM_*` levers are presence flags — any value turns them on.
Eighteen take a SUBSTRING matched against a name, so `LEVER=1` selects
names containing the digit one, which is usually none, and prints
nothing. An empty trace is indistinguishable from a code path that
never ran.

Measured 2026-09-10: that reading was reached twice in one session by
two people, and the second time it was filed as a missing-instrument
defect before the lever was read. The knowledge existed — a doc comment
in `verum_types/src/infer/path_resolution.rs` says `VERUM_DUMP_VBC`
filters by function name — and was in a file nobody consults about
`VERUM_DUMP_VBC`.

BOTH DIRECTIONS FAIL. A new filter-shaped lever missing from the page
is the obvious one. A row on the page whose lever is no longer
filter-shaped is the dangerous one: a reader who checks the reference,
finds nothing, and concludes "it must be a flag" is exactly the person
this page is for.

FINDING THE SHAPE — three binding forms, and the first version of this
detector knew only one:

    let x    = env::var("L")            let-binding
    if let Ok(x) = env::var("L")        THE CONFIRMED CASE
    let Ok(x) = env::var("L") else      let-else

`VERUM_DUMP_VBC` uses the second, so a detector that knew only the
first reported ONE filter-shaped lever in the whole tree and missed the
one that had just cost a session. A detector that cannot find the case
you already know about has not measured anything yet.
"""

from __future__ import annotations

import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
CRATES = REPO / "crates"
PAGE = REPO / "docs" / "architecture" / "diagnostic-levers.md"

BIND = re.compile(
    r"(?:if\s+let\s+(?:Ok|Some)\(([a-z_][a-z0-9_]*)\)|"
    r"let\s+(?:Ok|Some)\(([a-z_][a-z0-9_]*)\)|"
    r"let\s+(?:mut\s+)?([a-z_][a-z0-9_]*))\s*=\s*[^;]*"
    r'env::var(?:_os)?\("(VERUM_[A-Z0-9_]+)"\)'
)
ROW = re.compile(r"^\|\s*`(VERUM_[A-Z0-9_]+)`\s*\|", re.M)
WINDOW = 60


def filter_shaped() -> dict[str, str]:
    """lever -> the first site where its value is used as a pattern."""
    out: dict[str, str] = {}
    for f in sorted(CRATES.rglob("*.rs")):
        try:
            lines = f.read_text(errors="replace").split("\n")
        except OSError:
            continue
        for i, line in enumerate(lines):
            m = BIND.search(line)
            if not m:
                continue
            var = m.group(1) or m.group(2) or m.group(3)
            lever = m.group(4)
            if lever in out:
                continue
            window = "\n".join(lines[i + 1: i + 1 + WINDOW])
            v = re.escape(var)
            used = re.compile(
                rf"\.contains\(&?{v}\)|starts_with\(&?{v}\)|"
                rf"ends_with\(&?{v}\)|split\(&?{v}\)|{v}\s*!=\s*\"\*\""
            )
            if used.search(window):
                out[lever] = f"{f.relative_to(REPO)}:{i + 1}"
    return out


def documented() -> set[str]:
    if not PAGE.is_file():
        return set()
    return set(ROW.findall(PAGE.read_text(errors="replace")))


def self_test() -> int:
    bad = 0
    cases = {
        "if-let (the confirmed form)":
            'if let Ok(filter) = std::env::var("VERUM_A") {\n'
            '    if filter != "*" && !name.contains(&filter) { continue; }\n',
        "plain let":
            'let want = std::env::var("VERUM_B").unwrap_or_default();\n'
            '    if shape.contains(&want) { eprintln!("x"); }\n',
        "let-else":
            'let Ok(pat) = std::env::var("VERUM_C") else { return; };\n'
            '    if n.starts_with(&pat) { eprintln!("y"); }\n',
    }
    for label, src in cases.items():
        lines = src.split("\n")
        found = False
        for i, line in enumerate(lines):
            m = BIND.search(line)
            if not m:
                continue
            var = m.group(1) or m.group(2) or m.group(3)
            v = re.escape(var)
            window = "\n".join(lines[i + 1:])
            if re.search(rf"\.contains\(&?{v}\)|starts_with\(&?{v}\)|{v}\s*!=\s*\"\*\"",
                         window):
                found = True
        if not found:
            print(f"self-test: {label} not recognised", file=sys.stderr)
            bad += 1

    flag = 'if std::env::var_os("VERUM_D").is_some() {\n    eprintln!("on");\n}\n'
    lines = flag.split("\n")
    for i, line in enumerate(lines):
        m = BIND.search(line)
        if m:
            var = m.group(1) or m.group(2) or m.group(3)
            v = re.escape(var)
            if re.search(rf"\.contains\(&?{v}\)", "\n".join(lines[i + 1:])):
                print("self-test: a presence flag was read as a filter", file=sys.stderr)
                bad += 1

    if not documented():
        print(f"self-test: the page parsed to zero rows ({PAGE.name})", file=sys.stderr)
        bad += 1

    if bad:
        print(f"self-test: {bad} FAILED", file=sys.stderr)
        return 1
    print(f"[ok] self-test: {len(cases)} binding form(s) recognised, "
          f"1 presence flag rejected, {len(documented())} row(s) parsed")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()

    found = filter_shaped()
    doc = documented()
    if not doc:
        print(f"check-diagnostic-levers: {PAGE} has no lever table — the page "
              f"is the roster and this gate cannot run without it",
              file=sys.stderr)
        return 2

    missing = sorted(set(found) - doc)
    stale = sorted(doc - set(found))
    print(f"check-diagnostic-levers: {len(found)} filter-shaped lever(s) in "
          f"crates/, {len(doc)} on the page, {len(missing)} undocumented, "
          f"{len(stale)} documented but no longer filter-shaped")

    if missing:
        print("  UNDOCUMENTED — its value is a pattern and the page does not")
        print("  say so, so `LEVER=1` will read as a silent mechanism:")
        for lever in missing:
            print(f"    + {lever}  ({found[lever]})")
    if stale:
        print("  STALE — the page claims a pattern the code no longer takes.")
        print("  A reader who checks a reference and finds nothing concludes")
        print("  the lever is a flag, which is the reading this page exists")
        print("  to prevent:")
        for lever in stale:
            print(f"    - {lever}")
    return 1 if (missing or stale) else 0


if __name__ == "__main__":
    sys.exit(main())
