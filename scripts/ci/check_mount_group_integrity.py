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

Two rules, both violated by the defect above:
  1. no `mount` statement may begin while a mount group is open;
  2. every mount group must be closed by the end of the file.

Strings and comments are skipped so a `{` inside either cannot open a
phantom group.
"""

import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCAN_DIRS = ["core", "core-tests", "vcs/specs"]


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
    problems: list[str] = []
    depth = 0
    open_line = 0
    for lineno, raw in enumerate(path.read_text(errors="replace").split("\n"), 1):
        line = strip_line(raw).strip()
        if not line:
            continue
        if depth > 0 and line.startswith("mount "):
            problems.append(
                f"{path.relative_to(ROOT)}:{lineno}: a `mount` statement inside the "
                f"group opened at line {open_line}"
            )
        if line.startswith("mount ") and line.endswith("{"):
            depth += 1
            open_line = lineno
        elif depth > 0 and (line.startswith("}") or line.endswith("};")):
            depth -= 1
    if depth > 0:
        problems.append(
            f"{path.relative_to(ROOT)}: mount group opened at line {open_line} is "
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
            scanned += 1
            problems.extend(check_file(path))

    if problems:
        print(f"GATE FAIL: mount-group-integrity: {len(problems)} problem(s) "
              f"in {scanned} file(s)")
        for p in problems[:20]:
            print(f"  {p}")
        print("  A `mount` group must be closed before the next item begins;")
        print("  a statement spliced inside one makes the whole file unparseable,")
        print("  and an unparseable module ships as a SILENT ZERO in the bake.")
        return 1

    print(f"[ok] mount-group integrity: {scanned} file(s), no group left open "
          f"and no statement spliced inside one")
    return 0


if __name__ == "__main__":
    sys.exit(main())
