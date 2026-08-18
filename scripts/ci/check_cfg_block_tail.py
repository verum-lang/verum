#!/usr/bin/env python3
"""Fail when a function's value is meant to come from an @cfg block.

WHY THIS EXISTS
---------------
A gated block is a STATEMENT. A chain of them therefore produces no value,
so a function whose body ends in one falls off the end and yields `Unit`:

    fn separator() -> Text {
        @cfg(target_os = "windows")      { ";" }
        @cfg(not(target_os = "windows")) { ":" }
    }

    error<E400>: Type mismatch: expected 'Text', found 'Unit'

The diagnostic lands on the SIGNATURE, so it reads as a wrong return type
rather than a missing value, and the two arms are exhaustive and mutually
exclusive — nothing about the author's intent is unclear. Only the form
fails to express it.

Two spellings do work, and both appear in core/ already:

    @cfg(P) { return v; }         keeps one block per platform
    if @cfg(P) { a } else { b }   one exit; right when the arms are values

Better still, give the function a plain tail expression as its general case
and let the gated arms be exceptions — core/sys/mod.vr:765 `page_size` is
the model. Written that way a function is total by construction: adding a
fourth platform cannot silently turn it into a Unit-returning stub.

WHAT COUNTS
-----------
A function that returns a value (not Unit, not `!`), whose body's last
substantive element is an @cfg block with NO tail expression after it, and
at least one of whose arms relies on its own tail rather than saying
`return`.

The fallback test is not optional. 14 functions in core/ have arms for only
some platforms AND a tail expression as the general case; they are correct,
and a check that ignores the tail reports every one of them.

THE COUNT MOVED, AND THE FIRST NUMBER WAS FOUR TIMES TOO SMALL
--------------------------------------------------------------
A first census asked whether the body STARTED with a gated block and found
12. The right question is whether it ENDS with one, which finds 43. Both
numbers were measured on the same tree. Starting-with is neither necessary
nor sufficient: `rdtsc_serialized` opens with two @asm blocks — statements,
correctly — and has a real tail below them.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CORE = REPO / "core"

FN = re.compile(
    r"^([ \t]*)(?:(?:public|pub)\s+)?(?:async\s+|unsafe\s+|pure\s+)*fn\s+(\w+)"
    r"[^;{]*->\s*([^{]+?)\s*\{\s*$"
)

BASELINE = 0


def shown_path(path: Path) -> str:
    """Repo-relative when possible. The self-check runs this gate against a
    scratch tree outside the repo, and an unguarded relative_to turns a
    correct finding into a traceback."""
    try:
        return str(path.relative_to(REPO))
    except ValueError:
        return str(path)


def block_end(lines: list[str], start: int) -> int | None:
    """Line closing the block opened at `start`.

    The naive `depth <= 0 and k > start` test breaks on a one-line
    `{ expr }`, where the braces balance on `start` itself: it returns the
    NEXT line, the arm then looks multi-line, and the site is skipped in
    silence. That is how core/shell/escape.vr survived a first sweep.
    """
    depth = 0
    opened = False
    for k in range(start, len(lines)):
        if "{" in lines[k]:
            opened = True
        depth += lines[k].count("{") - lines[k].count("}")
        if opened and depth <= 0:
            return k
    return None


def _opens_block(lines: list[str], k: int) -> bool:
    """Does the `@cfg(...)` at line `k` introduce a BLOCK?

    Two spellings in core/: `@cfg(x) {` on one line, and `@cfg(x)`
    followed by a line that is just `{`.  Anything else — `const`,
    `let`, `fn`, `type`, `mount` — is the attribute form.
    """
    after = lines[k].split(")", 1)[-1].strip()
    if after.startswith("{"):
        return True
    for nxt in lines[k + 1:]:
        t = nxt.strip()
        if not t or t.startswith("//"):
            continue
        return t.startswith("{")
    return False


def offenders(lines: list[str]) -> list[tuple[int, str, str]]:
    found = []
    for i, line in enumerate(lines):
        m = FN.match(line)
        if not m or m.group(3).strip() in ("()", "Unit", "!"):
            continue
        end = block_end(lines, i)
        if end is None:
            continue
        # A gated BLOCK, not a gated declaration.  `@cfg(...)` also
        # attributes a `const` / `let` / `fn`, and those produce a value
        # the normal way — the function below them can end in an
        # ordinary tail expression and be perfectly total.  Counting
        # them made `UdpSocket.try_clone` a finding: its last `@cfg`
        # attributes a `let`, `block_end` then walked on to the `{` of
        # the `Result.Ok(UdpSocket { … })` tail, and the tail-after-block
        # test found nothing after it because that WAS the tail.
        gated = [
            k for k in range(i + 1, end)
            if lines[k].lstrip().startswith("@cfg(") and _opens_block(lines, k)
        ]
        if not gated:
            continue
        last_close = block_end(lines, gated[-1])
        if last_close is None or last_close >= end:
            continue
        tail = [
            l.strip()
            for l in lines[last_close + 1:end]
            if l.strip() and not l.strip().startswith("//")
        ]
        if tail:
            continue  # a general case exists — the function is total
        for g in gated:
            s = g
            while s < end and "{" not in lines[s]:
                s += 1
            e = block_end(lines, s)
            if e is None:
                continue
            if not re.search(r"\breturn\b", "\n".join(lines[s:e + 1])):
                found.append((i + 1, m.group(2), m.group(3).strip()))
                break
    return found


def main() -> int:
    sources = sorted(CORE.rglob("*.vr"))
    if not sources:
        print(f"check-cfg-block-tail: no .vr files under {CORE}", file=sys.stderr)
        return 2

    findings = []
    for path in sources:
        for lineno, name, ret in offenders(path.read_text(errors="ignore").splitlines()):
            findings.append((f"{shown_path(path)}:{lineno}", name, ret))

    total = len(findings)
    if "--list" in sys.argv or total > BASELINE:
        stream = sys.stderr if total > BASELINE else sys.stdout
        print(f"cfg-block tail: {total} function(s) yield Unit (baseline {BASELINE})", file=stream)
        for site, name, ret in findings:
            print(f"  {name} -> {ret}\n      {site}", file=stream)

    if total > BASELINE:
        print(
            "\nSay it with `return` inside each arm, or lift the predicate into an\n"
            "if-expression, or give the function a plain tail as its general case.",
            file=sys.stderr,
        )
        return 1
    if total < BASELINE:
        print(f"cfg-block tail: {total} found, below baseline {BASELINE} — lower BASELINE.")
        return 1
    print(f"[ok] cfg-block tail: {total} known site(s), none new")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
