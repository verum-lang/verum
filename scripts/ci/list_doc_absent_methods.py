#!/usr/bin/env python3
"""A LIST TO READ, never a gate: methods the docs call on a VARIABLE
whose name appears nowhere in `core/`.

WHY IT IS NOT `check_`. Read the floor section at the bottom first. A
page may legitimately define its own type in prose, in an earlier fence,
or in the reader's imagination — `mock.expect_row(...)` in a testing
recipe is not a defect. No syntactic rule separates that from a page
calling an API the library does not have, so the count is a reading
list and a nonzero value is normal.

THE HOLE IT COVERS. `check_doc_method_names.py` checks `Name.method(…)`
where `Name` is a core TYPE — an uppercase receiver. That is a real
gate and it is green. But it says so in its own denominator: "1401
call(s) on core-declared types". The site makes ~3000 method calls on
LOWERCASE receivers — `resolver.`, `heap.`, `s.`, `m.` — and nothing
looked at any of them.

WHAT IT FOUND ON ITS FIRST RUN, all verified by hand against `core/`
before being believed:

    cookbook/dns.md          TWELVE async resolver methods that do not
                             exist. `core/net/dns.vr` has exactly three
                             async functions and no `Resolver` method is
                             async. Plus a whole caching section —
                             cache_clear / cache_invalidate /
                             cache_stats / cache_capacity — against a
                             file containing the string "cache" ZERO
                             times, complete with a "~50 ns" figure for
                             a cache hit.
    cookbook/collections.md  `heap.into_sorted_vec()` — Rust's
                             BinaryHeap method, and `vec` is not even
                             Verum's vocabulary.
    cookbook/tcp.md          `set_read_timeout_ms` where core declares
                             `set_read_timeout` — a near-miss, one token
                             away, which is the kind a reader trusts.

A PERFORMANCE NUMBER ON AN ABSENT MECHANISM is the strongest signal in
the whole list. Nobody measures 50ns for something that is not there, so
the figure proves the section was written from memory of another
library.

MY OWN HAND-CHECK WAS THE THING THAT WAS WRONG ONCE, and the correction
is why the extractor requires `(` or `<` after the name. Checking
`domain` with `grep -c "fn domain\b" core/` returned a hit and I nearly
declared a false positive. The hit was inside a STRING LITERAL —
`f"sqlite scalar fn domain error: {m}"`. The strict pattern below had
already rejected it correctly. A looser grep is not a control for a
stricter instrument; it is a different instrument with a worse answer.

THE FLOOR, measured. Of the ~139 spans this reports, exactly TWO have a
receiver whose type the block itself annotates with a core type
(`let heap: BinaryHeap = …` then `heap.into_sorted_vec()`). Those two
are provable. Everything else needs a reader, because the receiver's
type is not on the page in any form a script can follow. That ratio —
2 provable out of 139 — is the entire argument for `list_` over
`check_`, and it is printed on every run so nobody has to take it on
trust.
"""
import collections
import os
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CORE = REPO / "core"
DOCS = Path(os.environ.get("VERUM_DOCS_DIR") or (REPO.parent / "website" / "docs"))

# `fn name(` or `fn name<` — the paren/angle is what keeps prose inside a
# string literal from counting as a declaration. See the docstring.
DECL = re.compile(r"\bfn ([a-z_]\w*)\s*[(<]")
# Only fences that are Verum, or untagged. A ```text fence holding a
# gate's own output reads `sys.fs_watch(r1.0)` as a method call on
# `sys` — measured, on stdlib/overview.md, where those three lines are
# MODULE NAMES WITH VERSIONS pasted from a dependency-cycle report.
FENCE = re.compile(r"^```(verum|vr|)\n(.*?)^```", re.M | re.S)
CALL = re.compile(r"\b([a-z_]\w*)\.([a-z_]\w*)\(")
# A page that writes `foo()` in prose has introduced the name.
PROSE_NAME = re.compile(r"`([a-z_]\w*)\(")
TYPE_DECL = re.compile(r"^(?:public\s+|pub\s+)?type ([A-Z]\w*)", re.M)
BIND = re.compile(r"\blet\s+(?:mut\s+)?([a-z_]\w*)\s*:\s*([A-Z]\w*)")

# Verified by hand against `core/` on the day this was written. The
# first three MUST be absent, the last three MUST be present — a sweep
# that reports everything and a sweep that reports nothing look
# identical from the outside.
# `cache_stats` was the first choice here and the control REJECTED it on
# the first run: it is declared in `core/database/sqlite/native/l1_pager`
# and `core/meta/contexts`. The claim I could actually defend was
# narrower — absent from `core/net/dns.vr` and from any `Resolver` — and
# a control must assert what the instrument checks, which is core-wide.
# The trap has a name: a NAME FOUND IN THE WRONG TABLE.
CONTROLS_ABSENT = ["into_sorted_vec", "lookup_a_async", "cache_invalidate"]
CONTROLS_PRESENT = ["push", "len", "as_bytes"]


def core_names():
    out = set()
    for f in CORE.rglob("*.vr"):
        out.update(DECL.findall(f.read_text(encoding="utf-8", errors="replace")))
    return out


def sweep(names):
    rows, provable, scanned = [], [], 0
    types = set()
    for f in CORE.rglob("*.vr"):
        types.update(TYPE_DECL.findall(f.read_text(encoding="utf-8", errors="replace")))
    for p in sorted(DOCS.rglob("*.md")) + sorted(DOCS.rglob("*.mdx")):
        text = p.read_text(encoding="utf-8", errors="replace")
        blocks = [b for _, b in FENCE.findall(text)]
        local = set(PROSE_NAME.findall(text))
        for b in blocks:
            local.update(DECL.findall(b))
        for b in blocks:
            binds = dict(BIND.findall(b))
            for recv, meth in CALL.findall(b):
                scanned += 1
                if meth in names or meth in local:
                    continue
                rel = str(p.relative_to(DOCS))
                rows.append((meth, rel, recv))
                t = binds.get(recv)
                if t and t in types:
                    provable.append((rel, t, recv, meth))
    return rows, provable, scanned


def main() -> int:
    if not DOCS.is_dir():
        print(f"docs directory not found: {DOCS} — set VERUM_DOCS_DIR", file=sys.stderr)
        return 2
    names = core_names()
    if len(names) < 5000:
        print(f"list-doc-absent-methods: indexed only {len(names)} names from "
              "core/ — the declaration pattern or the tree moved. Refusing to "
              "report a list built on that.", file=sys.stderr)
        return 2
    print(f"function/method names declared in core/: {len(names)}")

    bad = 0
    for n in CONTROLS_ABSENT:
        if n in names:
            print(f"  CONTROL FAIL: {n!r} should be absent from core/ and is not",
                  file=sys.stderr)
            bad += 1
    for n in CONTROLS_PRESENT:
        if n not in names:
            print(f"  CONTROL FAIL: {n!r} should be present in core/ and is not",
                  file=sys.stderr)
            bad += 1
    if bad:
        print("Controls failed — the index is wrong and every line below is "
              "suspect.", file=sys.stderr)
        return 2
    print(f"  controls: {len(CONTROLS_ABSENT)} absent + {len(CONTROLS_PRESENT)} "
          "present, all as expected\n")

    rows, provable, scanned = sweep(names)
    per = collections.Counter(m for m, _, _ in rows)
    pages = collections.defaultdict(set)
    for m, pg, _ in rows:
        pages[m].add(pg)
    print(f"lowercase-receiver calls scanned : {scanned}")
    print(f"names absent from core/          : {len(per)} distinct, "
          f"{sum(per.values())} span(s)\n")

    print(f"PROVABLE — the block annotates the receiver with a core type "
          f"({len(provable)}):")
    for pg, t, recv, meth in sorted(set(provable)):
        print(f"   {pg:<38} {recv}: {t} .{meth}()")
    print()
    print("TO READ — receiver type not on the page; each needs a human "
          "before it is believed:")
    for meth, n in per.most_common(40):
        pg = sorted(pages[meth])
        print(f"   {meth:<26} x{n:<3} {pg[0]}{f' +{len(pg)-1} more' if len(pg) > 1 else ''}")
    if len(per) > 40:
        print(f"   … and {len(per) - 40} more distinct name(s)")
    print("\nA nonzero count is NORMAL: a page may define its own types. "
          "Never gate on this number — see the module docstring's floor.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
