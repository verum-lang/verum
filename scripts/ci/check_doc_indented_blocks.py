#!/usr/bin/env python3
"""check_doc_indented_blocks.py — a doc comment must fence its code.

WHY THIS EXISTS.  An indented block inside `///` is a Markdown code
block, and rustdoc compiles every one of them as RUST.  Two such blocks
held Verum:

    error: expected one of `!` or `::`, found `.`
      crates/verum_types/src/infer/modules.rs - module_root_is_unknown

and the blocking CI job `Doc examples compile` (ci.yml, `cargo test
--doc -p verum_types …`) was RED on main for four days without anyone
looking.  Nine more of the same shape sat in `verum_codegen` and
`verum_vbc`, silent only because those crates are not in that job's
crate list.

SO WHY A SECOND CHECK, when the compiler already answers?  Because the
compiler answers only for crates the `--doc` job names, and only for
snippets that FAIL to parse.  A fragment that happens to be valid Rust
compiles and says nothing — `verum_vbc` had two of those.  This check
asks a different question, one the compiler cannot: is there an
indented block at all?  That has no false negatives for the class, and
what it costs is that a genuine Rust example must be fenced too — which
it should be regardless, since a fence is where `ignore` / `no_run` /
`text` are spelled.

The deleted predecessor of the `--doc` job tried to classify block
CONTENT by tokens and reported clean on the very block it was written
for.  This one never looks at content.

Usage:
    check_doc_indented_blocks.py [--selftest] [root ...]
"""

import re
import sys
from pathlib import Path

DOC = re.compile(r"^(\s*)(///|//!)(.*)$")
FENCE = re.compile(r"^\s*(///|//!)\s*```")


def blocks_in(text):
    """Yield (line_no, first_line) for each indented doc code block."""
    in_fence = False
    after_blank = False
    in_block = False
    for i, line in enumerate(text.splitlines(), 1):
        if FENCE.match(line):
            in_fence = not in_fence
            after_blank = False
            in_block = False
            continue
        if in_fence:
            continue
        m = DOC.match(line)
        if not m:
            after_blank = False
            in_block = False
            continue
        body = m.group(3)
        if not body.strip():
            after_blank = True
            in_block = False
            continue
        indented = len(body) - len(body.lstrip(" ")) >= 4
        if indented and after_blank and not in_block:
            yield i, body.strip()
            in_block = True
        if not indented:
            in_block = False
        after_blank = False


SELFTEST_OFFENDER = '''
/// Some prose introducing an example:
///
///     a.b.absent()   this is Verum and rustdoc will build it as Rust
///
/// and prose after it.
fn f() {}
'''

SELFTEST_CLEAN = '''
/// Some prose introducing an example:
///
/// ```text
/// a.b.absent()   fenced, so rustdoc leaves it alone
/// ```
///
/// A continuation line indented for readability is not a block:
/// this wraps
///     and this is its continuation, with no blank line before it.
fn f() {}
'''


def selftest():
    """The check must be able to come back POSITIVE, and to stay quiet.

    A detector that reports nothing looks exactly like a clean tree.
    """
    ok = True
    found = list(blocks_in(SELFTEST_OFFENDER))
    if len(found) != 1:
        print(f"SELFTEST FAILED: offender sample -> {len(found)} blocks, expected 1")
        ok = False
    quiet = list(blocks_in(SELFTEST_CLEAN))
    if quiet:
        print(f"SELFTEST FAILED: clean sample -> {quiet}, expected none")
        ok = False
    print("selftest: ok" if ok else "selftest: FAILED")
    return 0 if ok else 1


def main(argv):
    if "--selftest" in argv:
        return selftest()
    roots = [Path(a) for a in argv[1:]] or [Path("crates")]
    files = [
        p
        for root in roots
        for p in root.rglob("*.rs")
        if "/tests/" not in str(p) and "/target/" not in str(p)
    ]
    if not files:
        print(f"check_doc_indented_blocks: no .rs files under {roots} — "
              f"the scan is looking in the wrong place and would pass vacuously",
              file=sys.stderr)
        return 2
    hits = []
    for path in sorted(files):
        try:
            text = path.read_text(errors="replace")
        except OSError:
            continue
        for line_no, first in blocks_in(text):
            hits.append((path, line_no, first))
    if hits:
        print(
            "these doc comments contain an INDENTED code block.  rustdoc "
            "compiles each one as Rust:\n"
        )
        for path, line_no, first in hits:
            print(f"  {path}:{line_no}  {first[:60]}")
        print(
            "\nFence them.  ```text for prose, tables and non-Rust snippets "
            "(Verum included);\n```rust for a real example that should be "
            "compiled and run."
        )
        return 1
    print(f"check_doc_indented_blocks: {len(files)} files, no indented doc blocks")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
