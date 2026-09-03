#!/usr/bin/env python3
"""MOUNT-GROUP-INTEGRITY-1 (T1073): a `mount` group must be closed before
the next item begins.

2026-09-03 a mechanical sweep inserted a mount line after "the last line
matching `^mount `".  In a file whose last such line is the HEADER of a
multi-line group, the new line landed INSIDE the group:

    mount core.intrinsics.runtime.text.{
    mount core.intrinsics.memory.{ptr_offset};      <- here
        text_from_static, ...
    };

`core/text/text.vr` and `core/mem/heap.vr` stopped parsing.  An
unparseable module ships as a silent zero in the stdlib bake, so the
sidecar carried no `Text.<static>` at all and every one of the 201
`Text.new()` call sites in `core/` failed with "no method named `new`
found for type `Text`".  Cost: a full corpus regression (1808 -> 1975
errors) plus an hour of attributing it to an unrelated commit window.

The sweep HAD an auto-revert keyed on "did the error count fall".  It
did not fire, because after a parse break the count FALLS: a parse error
truncates the file and every later diagnostic disappears.  An error
count is not a monotone quality measure — it drops both when a defect is
fixed and when there is nothing left to check.

This gate is the missing second measurement, in its cheapest form: a
purely syntactic scan, no compiler, no bake, whole tree in well under a
second.

The rule generalises past mounts, and the table below is the point:
a construct is at risk exactly when it has a MULTI-LINE form whose FIRST
line matches the anchor an insertion sweep would use.  For each such
construct, two rules:

  1. no top-level statement may begin while the construct is open;
  2. the construct must be closed by the end of the file.

    construct        opens with             closed by
    mount group      `mount ...{` (EOL)     brace depth back to 0
    attribute        `@name(` unbalanced    paren depth back to 0

The attribute row is verum-2b's, contributed with its anchor spelling
after they measured zero instances of it in `core/` — a rule added for
the shape rather than for a live defect, which is the cheap moment to
add one.  A third construct joins as a table row, not as new code.

Strings and comments are skipped so a delimiter inside either cannot
open a phantom construct.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCAN_DIRS = ["core", "core-tests", "vcs/specs"]

# Anchor spelling for a multi-line attribute (verum-2b, T1061).
ATTR_OPEN_RE = re.compile(r"^@[a-zA-Z_][a-zA-Z0-9_]*\(")


def strip_line(line: str) -> str:
    """Drop a trailing line comment and any string bodies."""
    out = []
    i = 0
    n = len(line)
    while i < n:
        if line.startswith("//", i):
            break
        ch = line[i]
        if ch == '"':
            i += 1
            while i < n:
                if line[i] == "\\":
                    i += 2
                    continue
                if line[i] == '"':
                    i += 1
                    break
                i += 1
            continue
        out.append(ch)
        i += 1
    return "".join(out)


def check_file(path: pathlib.Path) -> list[str]:
    """Track every at-risk construct in one pass.

    `depth`/`open_line` per construct; a top-level statement seen while
    any of them is open is the defect.
    """
    problems: list[str] = []
    brace_depth = 0          # mount group
    brace_line = 0
    paren_depth = 0          # multi-line attribute
    paren_line = 0

    for lineno, raw in enumerate(path.read_text(errors="replace").split("\n"), 1):
        line = strip_line(raw).strip()
        if not line:
            continue

        starts_item = line.startswith("mount ") or line.startswith("module ")

        if brace_depth > 0 and starts_item:
            problems.append(
                f"{path.relative_to(ROOT)}:{lineno}: a top-level statement inside "
                f"the mount group opened at line {brace_line}"
            )
        if paren_depth > 0 and starts_item:
            problems.append(
                f"{path.relative_to(ROOT)}:{lineno}: a top-level statement inside "
                f"the attribute opened at line {paren_line}"
            )

        # mount group: `mount ...{` at end of line opens it.
        if line.startswith("mount ") and line.endswith("{"):
            brace_depth += 1
            brace_line = lineno
        elif brace_depth > 0 and (line.startswith("}") or line.endswith("};")):
            brace_depth -= 1

        # multi-line attribute: `@name(` with more `(` than `)` opens it,
        # and the running paren balance closes it.
        if paren_depth == 0:
            if ATTR_OPEN_RE.match(line) and line.count("(") > line.count(")"):
                paren_depth = line.count("(") - line.count(")")
                paren_line = lineno
        else:
            paren_depth += line.count("(") - line.count(")")
            if paren_depth < 0:
                paren_depth = 0

    if brace_depth > 0:
        problems.append(
            f"{path.relative_to(ROOT)}: mount group opened at line {brace_line} is "
            f"never closed"
        )
    if paren_depth > 0:
        problems.append(
            f"{path.relative_to(ROOT)}: attribute opened at line {paren_line} is "
            f"never closed"
        )
    return problems


def main() -> int:
    problems: list[str] = []
    scanned = 0
    for d in SCAN_DIRS:
        base = ROOT / d
        if not base.exists():
            continue
        for path in base.rglob("*.vr"):
            # `vcs/specs/parser/fail/` is a corpus of DELIBERATELY malformed
            # files — the parser's negative tests. Two of them are unclosed
            # attributes, and they served as this gate's positive control for
            # the attribute rule before being excluded here.
            if "/fail/" in str(path):
                continue
            scanned += 1
            problems.extend(check_file(path))

    if problems:
        print(f"GATE FAIL: mount-group-integrity: {len(problems)} problem(s) "
              f"in {scanned} file(s)")
        for p in problems[:20]:
            print(f"  {p}")
        print("  A construct with a multi-line form must be closed before the next")
        print("  item begins. A statement spliced inside one makes the whole file")
        print("  unparseable, and an unparseable module ships as a SILENT ZERO in")
        print("  the bake — where an error COUNT reads the break as an improvement.")
        return 1

    print(f"[ok] mount-group integrity: {scanned} file(s), no group left open "
          f"and no statement spliced inside one")
    return 0


if __name__ == "__main__":
    sys.exit(main())
