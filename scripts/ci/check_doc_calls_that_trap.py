#!/usr/bin/env python3
"""A page must not teach a call that panics on an unimplemented intrinsic
without saying so.

`check-doc-methods-declared` asks whether a documented name EXISTS.  This
asks the next question: does calling it produce a value.  A body
`@intrinsic("verum.…")` whose key has no implementation compiles, warns
where nobody reads it (the bake's `tracing` output is swallowed by
cargo), and traps at runtime naming a key the reader has never seen:

    Panic: @intrinsic("verum.compress.gzip_encode") is not implemented in
           this build (called from Gzip.encode); it has no registry
           entry, so there is no value to return

Measured by copying that page's own example and running it.

WHAT COUNTS AS "TEACHING IT".  The page must MOUNT a module and NAME a
function of that module whose body reaches an unimplemented key.  Three
narrowings got the population from a plausible-looking eighty-seven to
five, and each removed a class of false positive that a coarser gate
would have shipped:

  * a bare name match anywhere on the site — `verify`, `write`, `close`,
    `connect` belong to dozens of types, so the match says nothing;
  * a page that mounts a module holding a key ANYWHERE — five pages were
    caught by a key that appears only in a COMMENT about history
    (`core.random.secure`, `blake3`, `xxhash` and two more).  "The
    secure RNG traps" was one banner away from being written;
  * MODULE granularity — three more pages mount
    `core.intrinsics.runtime.os`, which has one unimplemented function
    out of fifty (`write_stderr`) that none of them documents.

So the test is: mounted module, named function, key in code.

WHAT SATISFIES IT.  A `:::caution` (or `:::warning` / `:::danger`) block
anywhere on the page.  The gate does not read the wording — it cannot
tell a good banner from a bad one, and pretending otherwise would make
it a style checker.  What it can do is refuse silence.

KNOWN GAP, stated because a roster that hides its blind spot is worse
than no roster: this matches a page's `mount` line against modules
holding a key DIRECTLY.  A page one hop away is not seen — the QUIC
cookbooks mount `core.net.quic.api`, whose functions call the module
holding the seventeen `verum.quic.*` keys.  Those pages were found by
hand and carry blocks; a page that reaches a trapping module through two
hops would not be caught here.  Closing that needs a census built from
call edges, and built from a different source than this one, because a
roster assembled by one search cannot enumerate what that search misses.
"""

from __future__ import annotations

import argparse
import os
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
CORE = REPO / "core"

# The site is a SEPARATE checkout beside this one.  Resolved the way its
# sibling gates resolve it (`VERUM_DOCS_DIR`, else the neighbouring
# `website/`) and never by naming the working copy's private path — a
# tracked file may not reference that directory at all.  Gate:
# `make check-internal-refs`.
_DOCS_ROOT = os.environ.get("VERUM_DOCS_DIR")
DOCS = pathlib.Path(_DOCS_ROOT) if _DOCS_ROOT else REPO.parent / "website" / "docs"
ROSTER = pathlib.Path(__file__).resolve().parent / "intrinsic_keys_unimplemented.txt"

KEY = re.compile(r'@intrinsic\(\s*"(verum\.[A-Za-z0-9_.]+)"')
FN = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)")
MOUNT = re.compile(r"mount\s+((?:core|cog)[A-Za-z0-9_.]*)")
ADMONITION = re.compile(r"^:::(caution|warning|danger)", re.M)


def code_only(text: str) -> str:
    """Strip `//` comments.  Not cosmetic: without this the census names
    modules whose only `verum.*` mention is prose about history."""
    return "\n".join(l.split("//", 1)[0] for l in text.splitlines())


def module_path(p: pathlib.Path) -> str:
    parts = list(p.relative_to(REPO).with_suffix("").parts)
    if parts[-1] == "mod":
        parts = parts[:-1]
    return ".".join(parts)


def trapping_functions(keys: set[str]) -> dict[str, set[str]]:
    """module path -> function names whose body reaches an unimplemented key."""
    out: dict[str, set[str]] = {}
    for p in sorted(CORE.rglob("*.vr")):
        try:
            text = p.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        if "verum." not in text:
            continue
        lines = code_only(text).splitlines()
        fns: set[str] = set()
        for i, line in enumerate(lines):
            for k in KEY.findall(line):
                if k not in keys:
                    continue
                for j in range(i, max(-1, i - 60), -1):
                    m = FN.search(lines[j])
                    if m:
                        fns.add(m.group(1))
                        break
        if fns:
            out[module_path(p)] = fns
    return out


def offending_pages(mod_fns: dict[str, set[str]]) -> list[tuple[str, list[str], bool]]:
    rows = []
    for p in sorted(DOCS.rglob("*.md*")):
        try:
            text = p.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        mods = set(MOUNT.findall(text))
        if not mods:
            continue
        named: set[str] = set()
        for m in mods:
            for tm, fns in mod_fns.items():
                if not (m == tm or m.startswith(tm + ".") or tm.startswith(m + ".")):
                    continue
                for fn in fns:
                    if re.search(r"(?<![A-Za-z0-9_.])" + re.escape(fn) + r"\s*\(", text):
                        named.add(fn)
        if named:
            rel = str(p.relative_to(DOCS))
            rows.append((rel, sorted(named), bool(ADMONITION.search(text))))
    return rows


def self_test() -> int:
    if code_only("a // b\nc") != "a \nc":
        print("[FAIL] self-test: comment stripping")
        return 2
    if not ADMONITION.search("intro\n:::caution X\nbody\n:::\n"):
        print("[FAIL] self-test: admonition not detected")
        return 2
    if ADMONITION.search("a :::caution inline\n"):
        print("[FAIL] self-test: an inline ::: must not count as a block")
        return 2
    print("  [ok] self-test: 3 cases")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return self_test()
    rc = self_test()
    if rc:
        return rc

    if not DOCS.is_dir():
        print(f"[skip] {DOCS} not present (the site is a separate checkout).")
        return 0
    if not ROSTER.is_file():
        print(f"[FAIL] roster missing: {ROSTER} — refusing to judge.")
        return 2
    keys = {
        l.strip()
        for l in ROSTER.read_text(encoding="utf-8").splitlines()
        if l.strip() and not l.startswith("#")
    }

    mod_fns = trapping_functions(keys)
    if not mod_fns:
        print("[FAIL] no trapping functions found in core/ at all — the scan "
              "did not run, and an empty scan would pass this gate vacuously.")
        return 2

    rows = offending_pages(mod_fns)
    silent = [(pg, fns) for pg, fns, has in rows if not has]

    print(f"doc calls that trap: {len(mod_fns)} core module(s) hold an "
          f"unimplemented key; {len(rows)} page(s) teach one")
    if not silent:
        print(f"[ok] all {len(rows)} carry a status block")
        return 0

    print(f"\n[FAIL] {len(silent)} page(s) teach a trapping call and say nothing:")
    for pg, fns in silent:
        print(f"    {pg}")
        print(f"      names: {', '.join(fns[:6])}{' …' if len(fns) > 6 else ''}")
    print(
        "\n  Add a `:::caution` block saying what happens and WHICH HALF of\n"
        "  the page still works — these pages are typically half real, and a\n"
        "  blanket 'not shipped' is false on the working half and teaches\n"
        "  readers to scroll past the banner."
    )
    return 1 if args.check else 0


if __name__ == "__main__":
    sys.exit(main())
