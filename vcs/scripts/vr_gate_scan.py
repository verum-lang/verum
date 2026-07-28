#!/usr/bin/env python3
"""
vr_gate_scan.py — shared comment/string-aware scanner for `.vr`-source
syntax gates (T0652 follow-up).

WHY THIS EXISTS
---------------
`check_no_double_colon.py` walks a `.vr` file's characters tracking
code / `//` line-comment / `/* */` block-comment / string (plain, raw,
triple-quoted) context, so it never mistakes a Rust-porting artefact
sitting inside a comment or string literal for real syntax. That state
machine is general-purpose — useful for ANY "does this pattern appear
in real code" gate over `.vr` sources, not just `::`. This module lifts
it out so a new detector (starting with `&str`, T0652) reuses it
instead of re-implementing its own copy.

`check_no_double_colon.py` itself is NOT changed to consume this module
yet — it is a live CI gate (`.github/workflows/ci.yml`) and migrating it
needs its own before/after diff verification, deliberately left as a
separate, later step rather than folded into this one.

WHAT CHANGED VS. THE ORIGINAL INLINE `scan()`
----------------------------------------------
The original increments its comment/string sub-loop cursor by exactly 1
after every position (even a match), so a self-overlapping pattern like
`":::"` yields two overlapping `::` hits. This generalised version skips
to the END of a match once one is found — cleaner, and the right
behaviour for a non-self-overlapping pattern like `&str`. If
`check_no_double_colon.py` is ever migrated onto this module, that
quirk must be checked for equivalence first (byte-for-byte diff against
the original `--report` output over the same corpus), not assumed away.

MODES OF USE
------------
`scan_matches(text, pattern)` — `pattern` is a compiled `re.Pattern`.
Yields `(pos, ctx, line, match)` for every match found anywhere in the
text (code, comments, and string literals alike); the caller decides
which contexts count as a violation. Matched via
`pattern.match(text, pos)` (anchored exactly at `pos`, not searched
forward), so positions come out of the same char-by-char walk that
resolves context.

`iter_vr_files(root, roots)` — yields absolute paths to every `.vr`
file under `root/roots[i]`, skipping the same build/vcs housekeeping
directories `check_no_double_colon.py` skips.
"""
import os
import re

SKIP_DIRS = {"target", ".claude", ".git", "node_modules"}

# A Verum char literal: `'x'`, `'\n'`, `'\''`, `'"'`, `'\u{2603}'`. Matched
# so the main code-context walk can skip over one as a unit — see the
# comment at its use site for why that matters.
CHAR_LITERAL = re.compile(r"'(?:\\u\{[0-9a-fA-F]+\}|\\.|[^'\\\n])'")


def iter_vr_files(root, roots):
    """Yield absolute paths to every `.vr` file under root/roots[i]."""
    for r in roots:
        for dp, dns, fns in os.walk(os.path.join(root, r)):
            dns[:] = [d for d in dns if d not in SKIP_DIRS]
            for fn in fns:
                if fn.endswith(".vr"):
                    yield os.path.join(dp, fn)


def scan_matches(text, pattern):
    """
    Walk `text` tracking code|line|block|string context — the same
    state machine `check_no_double_colon.py` uses for `::` — and yield
    `(pos, ctx, line, match)` for every match of the compiled regex
    `pattern` found anywhere. The caller decides which contexts are
    violations (for `&str` that's `ctx == "code"` only — see
    `check_no_str_alias.py`).
    """
    i, n, line = 0, len(text), 1
    while i < n:
        c = text[i]
        if c == "\n":
            line += 1
            i += 1
            continue
        two = text[i:i + 2]
        if two == "//":
            j = i
            while j < n and text[j] != "\n":
                m = pattern.match(text, j)
                if m:
                    yield (j, "line", line, m)
                    j = m.end()
                    continue
                j += 1
            i = j
            continue
        if two == "/*":
            j = i + 2
            while j < n and text[j:j + 2] != "*/":
                if text[j] == "\n":
                    line += 1
                m = pattern.match(text, j)
                if m:
                    yield (j, "block", line, m)
                    j = m.end()
                    continue
                j += 1
            i = (j + 2) if j < n else n
            continue
        m_raw = re.match(r'r(#{0,4})"', text[i:])  # raw string r"..." / r#"..."#
        if m_raw:
            close = '"' + m_raw.group(1)
            start = i + m_raw.end()
            j = text.find(close, start)
            end = (j + len(close)) if j != -1 else n
            k = start
            while k < min(end, n):
                if text[k] == "\n":
                    line += 1
                m = pattern.match(text, k)
                if m:
                    yield (k, "string", line, m)
                    k = m.end()
                    continue
                k += 1
            i = end
            continue
        if text[i:i + 3] == '"""':  # multiline string
            start = i + 3
            j = text.find('"""', start)
            end = (j + 3) if j != -1 else n
            k = start
            while k < min(end, n):
                if text[k] == "\n":
                    line += 1
                m = pattern.match(text, k)
                if m:
                    yield (k, "string", line, m)
                    k = m.end()
                    continue
                k += 1
            i = end
            continue
        m_char = CHAR_LITERAL.match(text, i)  # 'x', '\n', '"', '\'', '\u{...}'
        if m_char:
            # A char literal's CONTENT can itself be a `"` (`'"'`), and the
            # plain-string handler below has no concept of char literals —
            # unconditionally treating the next `"` it sees as a string
            # open. Without this branch, `f.write_char('"')` desyncs the
            # quote parity for the rest of the file: the `"` inside `'"'`
            # opens a false string, the next REAL `"..."` looks like its
            # close, and every string boundary after that is off by one —
            # found by hand via a scan_matches/manual-grep diff over
            # `core/text/text.vr`, where it swallowed `implement
            # From<&str> for Text {` (a real code-context `&str`) fifteen
            # lines later into what the scanner thought was string
            # content. `check_no_double_colon.py`'s inline scan() has this
            # exact gap too (never checked here) — see the T0652 note in
            # check_no_str_alias.py.
            i = m_char.end()
            continue
        if c == '"':  # plain / f / b string
            j = i + 1
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == "\n":
                    line += 1
                    j += 1
                    continue
                if text[j] == '"':
                    break
                m = pattern.match(text, j)
                if m:
                    yield (j, "string", line, m)
                    j = m.end()
                    continue
                j += 1
            i = j + 1
            continue
        m = pattern.match(text, i)
        if m:
            yield (i, "code", line, m)
            i = m.end()
            continue
        i += 1
