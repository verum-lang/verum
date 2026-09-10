#!/usr/bin/env python3
"""A lever whose VALUE is a name pattern must be on the levers page.

WHY THIS EXISTS
---------------
Most `VERUM_*` levers are presence flags — any value turns them on.
Twenty-two match their value against a name, so `LEVER=1` selects names
containing the digit one, which is usually none, and prints nothing.
An empty trace is indistinguishable from a code path that never ran.

TWO MATCH KINDS, and they disagree about the empty string. A substring
lever shows everything for `LEVER=` (`name.contains("")` is true);
`VERUM_TRACE_TYPE_CLAIM` compares for EQUALITY (`w == "*" || w == name`)
and shows nothing for it. Seven levers additionally guard with
`!v.is_empty()`, so no value shows everything at all. The page carries
the per-lever answer; this gate only keeps the ROSTER exact.

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

FINDING THE SHAPE — four binding forms, added one census at a time,
each after a count that looked complete without it:

    let x    = env::var("L")            let-binding          ->  1 lever
    if let Ok(x) = env::var("L")        carried DUMP_VBC     -> 18 levers
    let Ok(x) = env::var("L") else      let-else
    match env::var("L") { Ok(x) => …    carried TYPE_CLAIM,
                                        BARE_VARIANT, CANON  -> 21 levers
    env::var("L").map(|x| …)            carried CONST_RESOLVE -> 22 levers

Every one of those four numbers looked like an answer, and none was
refuted by inspection. Each was refuted by a lever whose behaviour
somebody already knew being absent from the list — twice by a peer
applying the test to this detector after reading it here.

The fifth form is the sharpest: the name is a CLOSURE PARAMETER, bound
by nothing the other four look for, and the lever's FIRST appearance in
that same condition is a bare `is_ok()`.

    A DETECTOR THAT CANNOT FIND A CASE WHOSE ANSWER YOU ALREADY KNOW
    HAS NOT MEASURED ANYTHING YET.

The self-test carries that as an anchor: `VERUM_TRACE_TYPE_CLAIM` must
be on the page, and the match-arm form must be recognised. Without an
anchor a population count cannot be refuted — it always looks plausible.
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
# The FOURTH form, and the one that carried three levers past a census
# that looked complete: `match env::var("L") { Ok(v) => … }`.  The name
# is bound in an ARM, on a later line than the call.
CALL = re.compile(r'env::var(?:_os)?\("(VERUM_[A-Z0-9_]+)"\)')
ARM = re.compile(r"(?:Ok|Some)\(([a-z_][a-z0-9_]*)\)\s*=>")
# The FIFTH form: the name arrives as a CLOSURE PARAMETER —
# `env::var("L").map(|w| name.contains(&w))`.  Nothing is bound with
# `let`, nothing is matched with an arm, so the four earlier forms walk
# straight past it.
CLOSURE = re.compile(r"\.(?:map|map_or|map_or_else|is_ok_and|and_then)\s*\(\s*\|"
                     r"([a-z_][a-z0-9_]*)\|")
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
            c = CALL.search(line)
            if not c or line.lstrip().startswith("//"):
                continue
            lever = c.group(1)
            if lever in out:
                continue
            b = BIND.search(line)
            var = (b.group(1) or b.group(2) or b.group(3)) if b else None
            if var is None:
                for j in range(i, min(i + 4, len(lines))):
                    a = ARM.search(lines[j]) or CLOSURE.search(lines[j])
                    if a:
                        var = a.group(1)
                        break
            if var is None:
                continue
            window = "\n".join(lines[i: i + 1 + WINDOW])
            v = re.escape(var)
            substring = re.compile(
                rf"\.contains\(&?{v}\)|starts_with\(&?{v}\)|"
                rf"ends_with\(&?{v}\)|split\(&?{v}\)|{v}\s*!=\s*\"\*\""
            )
            # `w == "*" || w == name` — equality against a NAME, not a
            # literal. The `== "*"` half is what tells the two apart from
            # a plain `== "1"` presence comparison.
            exact = (re.search(rf"{v}\s*==\s*[a-z_][a-z0-9_.()]*\b(?!\")", window)
                     and f'{var} == "*"' in window)
            if substring.search(window) or exact:
                out[lever] = f"{f.relative_to(REPO)}:{i + 1}"
    return out


def documented() -> set[str]:
    if not PAGE.is_file():
        return set()
    return set(ROW.findall(PAGE.read_text(errors="replace")))


def self_test() -> int:
    bad = 0
    cases = {
        "closure parameter (carried CONST_RESOLVE)":
            'if std::env::var("VERUM_F").is_ok()\n'
            '    && std::env::var("VERUM_F")\n'
            '        .map(|w| func_name.contains(&w) || w == "*")\n'
            '        .unwrap_or(false)\n',
        "match arm (carried TYPE_CLAIM, BARE_VARIANT, CANON)":
            'match std::env::var("VERUM_E") {\n'
            '    Ok(w) => w == "*" || w == name,\n'
            '    Err(_) => false,\n'
            '}\n',
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
            if not CALL.search(line):
                continue
            b = BIND.search(line)
            var = (b.group(1) or b.group(2) or b.group(3)) if b else None
            if var is None:
                for j in range(i, min(i + 4, len(lines))):
                    a = ARM.search(lines[j]) or CLOSURE.search(lines[j])
                    if a:
                        var = a.group(1)
                        break
            if var is None:
                continue
            v = re.escape(var)
            window = "\n".join(lines[i:])
            if (re.search(rf"\.contains\(&?{v}\)|starts_with\(&?{v}\)|{v}\s*!=\s*\"\*\"",
                          window)
                    or (re.search(rf"{v}\s*==\s*[a-z_][a-z0-9_.()]*\b(?!\")", window)
                        and f'{var} == "*"' in window)):
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
    for anchor in ("VERUM_TRACE_TYPE_CLAIM", "VERUM_TRACE_CONST_RESOLVE"):
        if anchor not in documented():
            print(f"self-test: the anchor {anchor} is not on the page",
                  file=sys.stderr)
            bad += 1
    anchor = "2 anchors"
    if bad:
        print(f"self-test: {bad} FAILED", file=sys.stderr)
        return 1
    print(f"[ok] self-test: {len(cases)} binding form(s) recognised, "
          f"1 presence flag rejected, {len(documented())} row(s) parsed, "
          f"{anchor} present")
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
