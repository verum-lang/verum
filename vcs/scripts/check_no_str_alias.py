#!/usr/bin/env python3
"""
check_no_str_alias.py — gate CANDIDATE: flag Rust `str` (as `&str`) in
Verum (.vr) type positions. (T0652 follow-up.)

WHY THIS EXISTS
---------------
`str` is not a Verum type. It works today only because
`verum_types/src/infer/env.rs` (`TypeChecker::register_primitives`)
registers it as a compiler-level compatibility alias for `Text`
("str is an alias for Text (for compatibility)"), reinforced by a
`From<&str> for Text` fallback coercion in `infer/types.rs`. But
CLAUDE.md's semantic-types rule is mandatory: `Text`, never `str` /
`String`. Every `&str` in `core/` is a Rust-porting artefact exactly
like the `::` this repo already gates — this script is the same idea,
aimed at a second construct.

UNLIKE `::`: no exclusion set is needed. `::` needs one because it has
genuine competing meanings in real data — IPv6 literals, SQL casts,
SQLite URIs. `&str` has no such domain: outside of comments and string
literals (which the shared scanner already separates out), `&str` in a
`.vr` file always means the type. See `vr_gate_scan.py` for the shared
comment/string-aware state machine this reuses.

STATUS: NOT WIRED INTO `make` OR CI. There are ~123 existing `&str`
sites in `core/` today (see the T0652 pool row) — enabling this as a
hard gate before that cleanup lands would turn the tree red on
landing. Run manually; `--check`'s exit code is for a human or a
future, deliberately-scheduled CI wiring decision, not automatic
enforcement yet.

MODES
  --check   (default) exit 1 if any real-code `&str` found, else 0
  --report  list every hit, grouped by context (code vs comment/string)
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from vr_gate_scan import iter_vr_files, scan_matches  # noqa: E402

# Repo root = two levels up from vcs/scripts/ (matches check_no_double_colon.py).
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ROOTS = ["core", "core-tests", "vcs"]

# Word-boundary-safe: excludes `&stream`, `&stripped`, `&stringify_x`,
# `&stream_id`, `&str_val`, etc. — the exact false-positive class found
# by hand while sizing T0652.
PATTERN = re.compile(r"&str\b")


def find_hits():
    """Return (code_hits, other_hits) — each a list of (rel, line, ctx, snippet)."""
    code_hits, other_hits = [], []
    for path in iter_vr_files(ROOT, ROOTS):
        try:
            text = open(path, encoding="utf-8").read()
        except Exception:
            continue
        if "&str" not in text:
            continue
        rel = os.path.relpath(path, ROOT)
        for pos, ctx, line, _m in scan_matches(text, PATTERN):
            line_start = text.rfind("\n", 0, pos) + 1
            line_end = text.find("\n", pos)
            if line_end == -1:
                line_end = len(text)
            snip = text[line_start:line_end].strip()
            if ctx == "code":
                code_hits.append((rel, line, ctx, snip))
            else:
                other_hits.append((rel, line, ctx, snip))
    return code_hits, other_hits


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "--check"
    code_hits, other_hits = find_hits()

    if mode == "--check":
        if code_hits:
            print(
                f"GATE CANDIDATE (not yet enforced): {len(code_hits)} `&str` in "
                f"real .vr code positions — semantic types require `Text`, never "
                f"`str` (T0652)."
            )
            print("This is informational only; not wired into make/CI.")
            for rel, line, _ctx, snip in code_hits[:100]:
                print(f"  {rel}:{line}  …{snip}…")
            if len(code_hits) > 100:
                print(f"  … and {len(code_hits) - 100} more")
            sys.exit(1)
        print("OK: no `&str` in real .vr code.")
        sys.exit(0)

    # --report
    print(f"=== SUMMARY ===  code={len(code_hits)}  comment/string={len(other_hits)}")
    print(f"\n=== CODE-context (real violations): {len(code_hits)} ===")
    for rel, line, _ctx, snip in code_hits:
        print(f"  {rel}:{line}  {snip}")
    print(f"\n=== comment/string context (informational, not counted): {len(other_hits)} ===")
    for rel, line, ctx, snip in other_hits:
        print(f"  {rel}:{line} [{ctx}]  {snip}")


if __name__ == "__main__":
    main()
