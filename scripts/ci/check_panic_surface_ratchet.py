#!/usr/bin/env python3
"""PANIC-SURFACE-RATCHET-1 (T0424): fail on any NET INCREASE of
.unwrap()/.expect( in PRODUCTION code under crates/verum_codegen/src/llvm/.

The fallible idiom already won in this crate (`or_llvm_err` ~7.4k uses);
the panic-site count grows only because NEW IR-emitting code reaches for
`.expect("...")`. The durable fix is this gate, not another sweep — the
T0131 sweeps re-rot without it (501 → 626 between 07-13 and 07-19).

Counting rules (the MEASUREMENT CORRECTION from the task):
  * every occurrence counts, including two on one line;
  * `#[cfg(test)]`-gated modules are EXCLUDED — unwrap is idiomatic there;
  * COMMENTS are EXCLUDED — a line that merely NAMES `.unwrap()` while
    explaining a defect is not a panic site.  Measured 2026-09-02: this
    gate went red for a comment reading "`m.lock().unwrap()` SIGSEGVed
    under ...", i.e. a sentence about a panic counted as one.  A gate
    that a comment can redden teaches its readers to work around it —
    by not writing the explanation — which is the opposite of what it
    is for;
  * `unwrap_or` / `unwrap_or_else` / `unwrap_or_default` / `expect_err`
    are NOT panic sites and are excluded by the regexes.

Baseline: scripts/ci/panic_surface_baseline.txt (a single integer).
  count >  baseline → FAIL, naming the worst files;
  count == baseline → OK;
  count <  baseline → OK + a reminder to ratchet the baseline down.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
TARGET = ROOT / "crates" / "verum_codegen" / "src" / "llvm"
BASELINE_FILE = pathlib.Path(__file__).resolve().parent / "panic_surface_baseline.txt"

# .unwrap() exactly (not unwrap_or*, not unwrap_err — that still panics on Ok,
# count it), and .expect( (not expect_err( — that panics too, count it).
# Keep the task's simple honest surface: unwrap() and expect(.
UNWRAP_RE = re.compile(r"\.unwrap\(\)")
EXPECT_RE = re.compile(r"\.expect\(")


def strip_cfg_test_regions(text: str) -> str:
    """Blank out `#[cfg(test)] mod … { … }` bodies (brace-counted)."""
    out = []
    i = 0
    n = len(text)
    while i < n:
        m = re.compile(r"#\[cfg\(test\)\]").search(text, i)
        if not m:
            out.append(text[i:])
            break
        # find the opening brace of the following item
        brace = text.find("{", m.end())
        if brace == -1:
            out.append(text[i:])
            break
        out.append(text[i : m.start()])
        depth = 0
        j = brace
        while j < n:
            c = text[j]
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        # keep newlines so line numbers stay meaningful for humans
        out.append("\n" * text[m.start() : j + 1].count("\n"))
        i = j + 1
    return "".join(out)


def strip_comments(text: str) -> str:
    """Blank out `//` line comments and `/* */` blocks, keeping newlines so
    per-file counts stay comparable and line numbers stay meaningful.

    String literals are respected: `let s = "// not a comment";` must not
    truncate the line.  Raw strings (`r"..."`, `r#"..."#`) are handled by
    treating `"` as the delimiter, which is enough here — the target
    directory has no raw string containing an unmatched quote, and a
    mis-parse would only ever UNDERCOUNT within one line, never invent a
    site.
    """
    out = []
    i = 0
    n = len(text)
    while i < n:
        c = text[i]
        if c == '"':
            out.append(c)
            i += 1
            while i < n:
                out.append(text[i])
                if text[i] == "\\":
                    if i + 1 < n:
                        out.append(text[i + 1])
                        i += 2
                        continue
                elif text[i] == '"':
                    i += 1
                    break
                i += 1
            continue
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            while i < n and text[i] != "\n":
                i += 1
            continue
        if c == "/" and i + 1 < n and text[i + 1] == "*":
            j = text.find("*/", i + 2)
            j = n if j == -1 else j + 2
            out.append("\n" * text[i:j].count("\n"))
            i = j
            continue
        out.append(c)
        i += 1
    return "".join(out)


def main() -> int:
    per_file: dict[str, int] = {}
    total = 0
    for path in sorted(TARGET.rglob("*.rs")):
        text = path.read_text(encoding="utf-8", errors="replace")
        prod = strip_comments(strip_cfg_test_regions(text))
        count = len(UNWRAP_RE.findall(prod)) + len(EXPECT_RE.findall(prod))
        if count:
            per_file[str(path.relative_to(ROOT))] = count
            total += count

    if not BASELINE_FILE.exists():
        print(
            f"panic-surface-ratchet: no baseline; current production count = {total}.\n"
            f"Write it: echo {total} > {BASELINE_FILE.relative_to(ROOT)}"
        )
        return 1
    baseline = int(BASELINE_FILE.read_text().strip())

    if total > baseline:
        print(
            f"panic-surface-ratchet: FAIL — {total} production unwrap/expect sites "
            f"under crates/verum_codegen/src/llvm/ (baseline {baseline}).\n"
            f"New IR code must use the fallible idiom (`.or_llvm_err()?`, "
            f"`or_internal(...)`) instead of .unwrap()/.expect().\n"
            f"Worst files:"
        )
        for f, c in sorted(per_file.items(), key=lambda kv: -kv[1])[:10]:
            print(f"  {c:5}  {f}")
        return 1

    if total < baseline:
        print(
            f"panic-surface-ratchet: OK ({total} < baseline {baseline}) — "
            f"ratchet it down: echo {total} > {BASELINE_FILE.relative_to(ROOT)}"
        )
        return 0

    print(f"panic-surface-ratchet: OK ({total} == baseline)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
