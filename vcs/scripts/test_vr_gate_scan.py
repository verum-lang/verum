#!/usr/bin/env python3
"""
test_vr_gate_scan.py — pin tests for the shared `.vr`/`.md` syntax-gate
scanner (T0652). Plain assertions, no pytest dependency — consistent
with the "pure Python, no build required" ethos of the scripts it
covers. Run directly: `python3 vcs/scripts/test_vr_gate_scan.py`.

These are the checks that made the `check_no_double_colon.py` char-
literal fix trustworthy (every context kind, plus the exact
char-literal-then-real-code shape that exposed the bug on
`core/text/text.vr`), plus coverage for `extract_markdown_fences` —
the feasibility probe for gating fenced ```verum blocks in the docs.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from vr_gate_scan import extract_markdown_fences, scan_matches  # noqa: E402

STR_PATTERN = re.compile(r"&str\b")
IMPL_PATTERN = re.compile(r"\bimpl\b")

failures = []


def check(label, got, want):
    ok = got == want
    print(f"{'OK  ' if ok else 'FAIL'} {label}  got={got!r} want={want!r}")
    if not ok:
        failures.append(label)


def counts(text, pattern):
    code = other = 0
    for _pos, ctx, _line, _m in scan_matches(text, pattern):
        if ctx == "code":
            code += 1
        else:
            other += 1
    return code, other


# --- scan_matches: every context kind, plus the char-literal fix -----------

SCAN_CASES = [
    ("code position", "fn f(s: &str) -> Int { 0 }", (1, 0)),
    ("line comment", "// takes &str for compatibility\nfn f() {}", (0, 1)),
    ("block comment", "/* &str here */\nfn f() {}", (0, 1)),
    ("plain string literal", 'let x = "&str is not Text";', (0, 1)),
    ("triple string literal", 'let x = """\n&str inside\n""";', (0, 1)),
    ("raw string literal", 'let x = r"&str raw";', (0, 1)),
    ("no match at all", 'let x: Text = "&stream should not match";', (0, 0)),
    ("real cron.vr-style case", "type AliasEntry is { name: &str, value: Int };", (1, 0)),
    ("code AND comment mixed", "fn f(s: &str) {} // and &str again in a comment", (1, 1)),
    (
        "char literal w/ quote, THEN real code &str "
        "(the exact core/text/text.vr:3735->3767 shape)",
        "fn f() { g('\"'); }\nimplement From<&str> for Text {}",
        (1, 0),
    ),
    (
        "char literal w/ quote inside a string later doesn't break",
        "fn f() { g('\"'); }\nlet x = \"a &str b\";",
        (0, 1),
    ),
]

for label, text, want in SCAN_CASES:
    check(label, counts(text, STR_PATTERN), want)

# Regression pin: the real file and line that exposed the bug. Reads the
# live source, so it also catches anyone re-introducing the desync by
# editing that function.
_text_vr = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..", "..", "core", "text", "text.vr"
)
if os.path.isfile(_text_vr):
    text = open(_text_vr, encoding="utf-8").read()
    lines = text.split("\n")
    # 0-indexed: file line 3767 is lines[3766].
    target_line = next(
        (i for i, l in enumerate(lines) if "implement From<&str> for Text" in l), None
    )
    if target_line is not None:
        slice_text = "\n".join(lines[max(0, target_line - 40) : target_line + 5])
        code, _other = counts(slice_text, STR_PATTERN)
        check("core/text/text.vr real-file regression (the &str after '\"' must be ctx=code)", code >= 1, True)
    else:
        print("SKIP  core/text/text.vr regression — anchor line moved, not failing on that alone")
else:
    print("SKIP  core/text/text.vr regression — file not found from this checkout layout")


# --- extract_markdown_fences -----------------------------------------------

MD = """# Title

Some prose with `impl` as inline code (not a fence).

```verum
fn f(s: &str) -> Int { 0 }
```

```rust
impl Foo for Bar {}
```

Prose in between.

```verum
implement Foo {
    fn g() -> Int { 1 }
}
```
"""

fences = list(extract_markdown_fences(MD))
check("extract_markdown_fences: fence count (rust fence excluded)", len(fences), 2)
check("extract_markdown_fences: first fence start line", fences[0][1] if fences else None, 6)
check("extract_markdown_fences: second fence start line", fences[1][1] if len(fences) > 1 else None, 16)

# Composes with scan_matches: real &str hit maps back to the real .md line.
hits = []
for content, first_line in fences:
    for pos, ctx, local_line, m in scan_matches(content, STR_PATTERN):
        if ctx == "code":
            hits.append(first_line + local_line - 1)
check("extracted-fence &str hit maps to real .md line", hits, [6])

# The ```rust fence's `impl` must never leak through — language filter works.
impl_hits = 0
for content, first_line in fences:
    for pos, ctx, local_line, m in scan_matches(content, IMPL_PATTERN):
        impl_hits += 1
check("impl hits inside VERUM-tagged fences only (rust fence excluded)", impl_hits, 0)

# A lang= other than the default is honoured too.
rust_fences = list(extract_markdown_fences(MD, lang="rust"))
check("extract_markdown_fences(lang='rust') finds the rust fence", len(rust_fences), 1)


print()
if failures:
    print(f"{len(failures)} FAILED: {failures}")
    sys.exit(1)
print("ALL PASS")
