#!/usr/bin/env python3
"""Fail on aggregate growth of production panic sites in native codegen.

Count unwrap/expect and their error variants, plus panic, todo, unimplemented
and unreachable macros. Comments, literals and cfg(test) items are excluded.
Every occurrence counts; a PANIC-OK comment does not grant an exemption.

The baseline is one integer. A decrease passes and asks for a lower baseline;
an increase fails with source locations and the existing fallible idiom.
"""

import argparse
from bisect import bisect_right
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
TARGET = ROOT / "crates" / "verum_codegen" / "src" / "llvm"
BASELINE_FILE = pathlib.Path(__file__).resolve().parent / "panic_surface_baseline.txt"

SITE_RE = re.compile(
    r"\.\s*(?:unwrap|unwrap_err)\s*\(\s*\)"
    r"|\.\s*(?:expect|expect_err)\s*\("
    r"|\b(?:panic|todo|unimplemented|unreachable)\s*!"
)
RAW_STRING_RE = re.compile(r'(?:b|c)?r(#*)"')
CHAR_RE = re.compile(r"'(?:[^\\'\n]|\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|[^\n]))'")
CFG_TEST_RE = re.compile(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]")
FN_RE = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)")


def blank_range(out: list[str], start: int, end: int) -> None:
    for index in range(start, end):
        if out[index] != "\n":
            out[index] = " "


def blank_non_code(source: str) -> str:
    """Keep source offsets while masking Rust comments and literal contents.

    Scan before interpreting attributes or braces: either can occur inside a
    literal. Block comments nest; raw-string delimiters retain their hash count.
    A character literal is distinguished from a lifetime such as 'ctx.
    """
    out = list(source)
    index, length = 0, len(source)
    while index < length:
        end = None
        if source.startswith("//", index):
            newline = source.find("\n", index + 2)
            end = length if newline < 0 else newline
        elif source.startswith("/*", index):
            depth, end = 1, index + 2
            while end < length and depth:
                if source.startswith("/*", end):
                    depth += 1
                    end += 2
                elif source.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
        else:
            raw = RAW_STRING_RE.match(source, index)
            if raw and (index == 0 or not (source[index - 1].isalnum() or source[index - 1] == "_")):
                delimiter = '"' + raw.group(1)
                closing = source.find(delimiter, raw.end())
                end = length if closing < 0 else closing + len(delimiter)
            elif source[index] == '"':
                end = index + 1
                while end < length:
                    if source[end] == "\\":
                        end = min(length, end + 2)
                    elif source[end] == '"':
                        end += 1
                        break
                    else:
                        end += 1
            elif source[index] == "'":
                character = CHAR_RE.match(source, index)
                if character:
                    end = character.end()
        if end is None:
            index += 1
        else:
            blank_range(out, index, end)
            index = end
    return "".join(out)


def item_body_end(source: str, start: int) -> int | None:
    """Bound one attributed item, field, variant or parameter in masked source.

    Delimiters opened here must close here. A parent's closing delimiter and
    a sibling separator are boundaries, never a route to the next item's body.
    Generic commas remain nested; return arrows are not generic closers.
    After a top-level initializer/discriminant, comparisons are expressions,
    while turbofish arguments still nest. An unmatched group is not suppressed.
    """
    closing = {"(": ")", "[": "]", "{": "}"}
    stack: list[str] = []
    braces = 0
    expression = False
    body = False
    index = start
    while index < len(source):
        character = source[index]
        if not stack and character in ",;":
            return index + (character == ";")
        if character in closing:
            if character == "{":
                if not stack:
                    body = not expression
                braces += 1
            stack.append(closing[character])
        elif character in ")]}":
            if not stack:
                return index
            if stack[-1] != character:
                return None
            stack.pop()
            if character == "}":
                braces -= 1
                if not stack and body:
                    return index + 1
        elif character == "<" and not braces:
            previous = index - 1
            while previous >= start and source[previous].isspace():
                previous -= 1
            turbofish = source[max(start, previous - 1):previous + 1] == "::"
            if not expression or turbofish:
                stack.append(">")
        elif character == ">" and not braces:
            if index > start and source[index - 1] == "-":
                pass  # The second token of a return arrow.
            elif stack and stack[-1] == ">":
                stack.pop()
            elif not stack and not expression:
                return index
        elif character == "=" and not stack:
            expression = True
        index += 1
    return None


def strip_cfg_test_regions(masked: str) -> str:
    """Mask complete cfg(test) items without treating literals as syntax."""
    out = list(masked)
    position = 0
    while match := CFG_TEST_RE.search(masked, position):
        end = item_body_end(masked, match.end())
        if end is None:
            # An incomplete item must not hide all remaining production sites.
            position = match.end()
            continue
        blank_range(out, match.start(), end)
        position = end
    return "".join(out)


def panic_sites(source: str) -> list[tuple[int, str, str]]:
    """Return (line, enclosing function, kind); owners are diagnostic only."""
    production = strip_cfg_test_regions(blank_non_code(source))
    functions = []
    for match in FN_RE.finditer(production):
        end = item_body_end(production, match.end())
        if end is not None:
            functions.append((match.start(), end, match.group(1)))
    lines = [0] + [match.end() for match in re.finditer("\n", production)]
    sites = []
    for match in SITE_RE.finditer(production):
        owner = "<item>"
        for start, end, name in functions:
            if start <= match.start() < end:
                owner = name
        sites.append((bisect_right(lines, match.start()), owner, match.group()))
    return sites


def display_path(path: pathlib.Path) -> str:
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return str(path)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=pathlib.Path, default=TARGET,
                        help="Rust source directory to scan")
    parser.add_argument("--baseline", type=pathlib.Path, default=BASELINE_FILE,
                        help="File containing the permitted aggregate count")
    args = parser.parse_args(argv)
    if not args.root.is_dir():
        print(f"panic-surface-ratchet: source root not found: {args.root}", file=sys.stderr)
        return 2
    per_file = {}
    total = 0
    for path in sorted(args.root.rglob("*.rs")):
        sites = panic_sites(path.read_text(encoding="utf-8"))
        if sites:
            per_file[display_path(path)] = sites
            total += len(sites)

    if not args.baseline.exists():
        print(
            f"panic-surface-ratchet: no baseline; current production count = {total}.\n"
            f"Write it: echo {total} > {display_path(args.baseline)}"
        )
        return 1
    try:
        baseline = int(args.baseline.read_text().strip())
        if baseline < 0:
            raise ValueError("negative baseline")
    except ValueError:
        print(f"panic-surface-ratchet: invalid nonnegative baseline: {args.baseline}", file=sys.stderr)
        return 2

    print(f"panic-surface-ratchet: {total} panic sites (baseline {baseline})")
    if total > baseline:
        print(
            f"panic-surface-ratchet: FAIL — {baseline} -> {total} production sites.\n"
            f"New IR code must use the fallible idiom (`.or_llvm_err()?`, "
            f"`or_internal(...)`) instead of panicking.\nWorst files:"
        )
        for filename, sites in sorted(per_file.items(), key=lambda item: -len(item[1]))[:10]:
            print(f"  {len(sites):5}  {filename}")
            for line, owner, kind in sites[:12]:
                print(f"    {filename}:{line}: fn {owner}: {kind.strip()}")
        return 1
    if total < baseline:
        print(
            f"panic-surface-ratchet: OK — improved {baseline} -> {total}; "
            f"ratchet it down: echo {total} > {display_path(args.baseline)}"
        )
        return 0
    print(f"panic-surface-ratchet: OK ({total} == baseline)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
