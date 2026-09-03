#!/usr/bin/env python3
"""Find correctness mechanisms that are BUILT and never CONSULTED.

The recurring shape, seen four times in one day: a check computes the
right answer, stores it, and hands it to nobody — `take_variant_collisions`
(zero callers), `variant_signature_admits` (three call sites, all in one
arm while a second consumer read the wrong thing), `validate_module_aliases`
("the entry point for module-level alias validation", zero callers), and the
adaptive-CBGR triple `record_clean_validation` / `record_violation` /
`violation_rate` — all three unused, so the counters `should_validate` reads
are permanently zero and `should_validate` itself has no callers either.

Grepping the SYMPTOM never finds these; grepping the mechanism's own NAME
and counting callers does, in about ninety seconds.

Deliberately NOT a gate.  A public function with no in-tree caller is
often legitimate API surface, so a threshold here would be noise with a
number attached.  Run it, read it, and file what it finds.

    python3 scripts/ci/census_unconsumed_mechanisms.py [--all]

`--all` drops the correctness filter and prints every uncalled public fn.
"""

import collections
import os
import re
import sys

ROOTS = [
    "crates/verum_vbc/src",
    "crates/verum_types/src",
    "crates/verum_codegen/src",
]
# Names whose zero-caller count says nothing: trait-required or ubiquitous.
SKIP = {
    "new", "default", "fmt", "clone", "from", "into", "len", "get",
    "insert", "iter", "push", "next",
}
# The shape that matters: the function's NAME or DOC says it decides
# something about correctness.
CORRECTNESS = re.compile(
    r"collision|conflict|diagnos|unsound|violat|mismatch|drift|"
    r"inconsist|ambigu|take_|report_|surface|validate|verify",
    re.I,
)
CALL = re.compile(r"(?:[.:]|\b)([a-z_][a-z0-9_]*)\s*\(")
# Mentions WITHOUT a paren — a field read, a doc reference, a match arm.
# Counted separately because a name with zero calls but many bare
# mentions is usually a getter over a live field, not dead code: keying
# the census on `name(` alone reported `take_new_type_descriptors` as
# draining an empty vector when `specializer.rs:1033` pushes to that
# field on every specialised descriptor (T1073).
BARE = re.compile(r"\b([a-z_][a-z0-9_]*)\b")
DEFN = re.compile(r"^\s*pub(?:\([a-z:]+\))?\s+fn\s+([a-z_][a-z0-9_]*)\s*[(<]")


def collect_definitions():
    defs = {}
    for root in ROOTS:
        for dirpath, _, files in os.walk(root):
            for name in files:
                if not name.endswith(".rs"):
                    continue
                path = os.path.join(dirpath, name)
                lines = open(path, encoding="utf-8", errors="replace").read().split("\n")
                for i, line in enumerate(lines):
                    m = DEFN.match(line)
                    if not m or m.group(1) in SKIP:
                        continue
                    doc, j = [], i - 1
                    while j >= 0 and lines[j].strip().startswith(("///", "#[")):
                        doc.append(lines[j].strip())
                        j -= 1
                    defs.setdefault(m.group(1), (path, i + 1, " ".join(reversed(doc))))
    return defs


def collect_bare_mentions(names: set[str]) -> collections.Counter:
    """How often each name appears at all, parens or not.

    The difference between this and the call count is the signal: a
    mechanism nobody CALLS whose name still appears elsewhere is
    usually reachable through a field or a re-export, and deleting it
    on the call count alone strands whatever produces into it.
    """
    seen = collections.Counter()
    for dirpath, _, files in os.walk("crates"):
        for name in files:
            if not name.endswith(".rs"):
                continue
            text = open(os.path.join(dirpath, name), encoding="utf-8", errors="replace").read()
            for m in BARE.findall(text):
                if m in names:
                    seen[m] += 1
    return seen


def collect_calls():
    """One pass over every .rs file, definition lines stripped so that
    `fn name(` is not miscounted as a call to `name`."""
    calls = collections.Counter()
    for dirpath, _, files in os.walk("crates"):
        for name in files:
            if not name.endswith(".rs"):
                continue
            text = open(os.path.join(dirpath, name), encoding="utf-8", errors="replace").read()
            text = re.sub(r"^\s*(?:pub(?:\([a-z:]+\))?\s+)?fn\s+\w+", "", text, flags=re.M)
            calls.update(CALL.findall(text))
    return calls


def main():
    show_all = "--all" in sys.argv
    defs = collect_definitions()
    calls = collect_calls()
    uncalled = [n for n in defs if calls[n] == 0]
    rows = [
        (n,) + defs[n]
        for n in uncalled
        if show_all or CORRECTNESS.search(n) or CORRECTNESS.search(defs[n][2])
    ]
    print(f"public fns scanned: {len(defs)}   with no in-tree caller: {len(uncalled)}")
    print(f"of those, correctness-shaped: {len(rows)}" if not show_all else "")
    # Also look for the FIELD a getter drains: `take_x` / `get_x` /
    # `x_mut` all front a field named `x`, and it is the field's
    # mentions that say whether anything produces into it. Keying only
    # on the function's own name is what let `take_new_type_descriptors`
    # look empty while `new_type_descriptors` was pushed to on every
    # specialised descriptor.
    def backing(n: str) -> str:
        for pre in ("take_", "get_", "drain_"):
            if n.startswith(pre):
                return n[len(pre):]
        for suf in ("_mut", "_ref"):
            if n.endswith(suf):
                return n[: -len(suf)]
        return n

    bare = collect_bare_mentions({r[0] for r in rows} | {backing(r[0]) for r in rows})
    for name, path, line, _doc in sorted(rows):
        b = backing(name)
        n = bare[name] + (bare[b] if b != name else 0)
        # 1 == the definition itself. Anything above that is a mention
        # the call-count did not see, and a reason to read before
        # deleting.
        # 1 == the definition itself; a backing field adds its own.
        hint = "" if n <= 1 else f"   [{n} mentions incl. `{b}` — read before deleting]"
        print(f"  {name:44} {path}:{line}{hint}")
    # A census that can only report findings measures nothing: say so when
    # the answer is none.
    if not rows:
        print("  (none — every correctness-shaped mechanism has a consumer)")


if __name__ == "__main__":
    main()
