#!/usr/bin/env python3
"""Ratchet on simple-type-name collisions across `core/`.

The compiler's layout registry keys types by their SIMPLE name.  Two
modules declaring `JoinPair` therefore do not get two types: the second
declarer resolves against the first one's fields.  That is T0458, and it
has produced real miscompiles — `ColumnSchema`, `Executor`, `JoinPair`
were each fixed by renaming one side.

WHY A RATCHET, AND WHY NOW.  T0458's spec recorded 99 colliding public
type names on 2026-07-19, and the existing guardrail
(`stdlib_unique_type_names`) is PERMANENTLY RED — it demands zero, it
has never been zero, and a gate that cannot go green is read as broken
rather than as a finding.  Nothing was watching the growth.

WHAT THE NUMBER ACTUALLY IS.  The first version of this gate counted
every line starting `type <name>`, which in Verum introduces three
different things and only one of them declares a type — see
`declares_a_type`.  It therefore reported 150 names / 446 declarations
on 2026-08-15, of which the largest entries were the associated-type
vocabulary of the protocol library (`Output` in 39 files, `Item` in 32,
`IntoIter` in 13) — bindings that name no layout and can collide with
nothing.  Measured through the grammar instead: 134 names / 284
declarations.  A gate whose baseline is a third noise gets read as
broken too.

The eventual fix is not renaming 134 types: two modules declaring the
same simple name is legitimate, and the registry is what should carry
the qualifier.  Until that lands, this stops the bleeding.

TWO DELIBERATE DIFFERENCES FROM THE OLD GUARDRAIL:

  * It counts ALL declarations, not just `public` ones.  The layout
    registry does not carry visibility, so a private type collides
    exactly as hard.  A public-only census was the blind spot that let
    the `SinkInner` pair through after `Modifier` had already been
    found — the measure was built on a property of the SOURCE while the
    defect lives in a property of the IMPLEMENTATION.

  * The baseline is a list of `name<TAB>path` pairs, not a count and
    not a set of names.  A count is satisfied by deleting a file.  A
    set of names misses a THIRD module joining an existing pair.  The
    pair list makes the diff itself the review.

GRAMMAR NOTE (verum.ebnf:569):

    type_def = visibility , 'type' , [ 'affine' | 'linear' ] , identifier

A regex of the form `type (\\w+)` captures the MODIFIER, not the type:
`public type affine ArenaScope` reads as a type named "affine", which
then appears as a forty-way collision across core/database/.  The two
modifiers are skipped explicitly below.

Usage:
    check_type_name_collisions.py                  # gate
    check_type_name_collisions.py --list           # print every collision
    check_type_name_collisions.py --write-baseline # re-record
"""

from __future__ import annotations

import re
import sys
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CORE = REPO / "core"
KNOWN = REPO / "scripts" / "ci" / "type_name_collisions_known.txt"

# `visibility , 'type' , [ 'affine' | 'linear' ] , identifier , [generics]`
DECL = re.compile(r"^\s*(?:public\s+)?type\s+(?:(?:affine|linear)\s+)?([A-Za-z_]\w*)")


def declares_a_type(line: str) -> str | None:
    """The declared name, if this line is a `type_def` — else `None`.

    `type` introduces THREE different things in Verum and only one of
    them is a nominal declaration:

        type Point is { x: Float };          # type_def — declares `Point`
        type Item;                           # protocol associated type
        type Output = Result<T, RecvError>;  # associated-type BINDING

    The grammar's discriminator is the `is` keyword (`type_def =
    visibility, 'type', [...], identifier, [generics], 'is', ...`), and
    without it this gate counted every `implement Future for X { type
    Output = ...; }` as a declaration of a type called `Output`.  That
    was most of what it reported: 39 files "declaring" `Output`, 32
    `Item`, 13 `IntoIter`, 10 `Target` — the associated-type vocabulary
    of the protocol library, none of which can collide in a layout
    registry because none of them names a layout.
    """
    m = DECL.match(line)
    if not m:
        return None
    rest = line[m.end() :]
    # Skip a balanced generic parameter list, which may nest
    # (`type Wrapper<T: Into<U>> is ...`), so a `[^>]*` scan will not do.
    if rest.lstrip().startswith("<"):
        rest = rest.lstrip()
        depth = 0
        for i, ch in enumerate(rest):
            if ch == "<":
                depth += 1
            elif ch == ">":
                depth -= 1
                if depth == 0:
                    rest = rest[i + 1 :]
                    break
        else:
            return None
    return m.group(1) if re.match(r"\s+is\b", rest) else None

# Platform twins are legitimate BY DESIGN — one Stat per OS, selected by
# module.  They are NOT exempted: T0435 reports precisely these names
# miscompiling (core/sys/darwin/mod.vr re-exports bare `Stat` and the
# LINUX layout answers).  The "legitimate" bucket is where the live
# miscompile already is, so it stays in the count and is only labelled.
PLATFORM = re.compile(r"^core/sys/(darwin|linux|windows|freebsd)/")


def strip_noise(text: str) -> list[str]:
    """Blank out string literals and line comments, keeping line count.

    Both have produced false positives in this repo before: a `"NAN"`
    literal read as a math mount, a `max` inside a comment read as a
    declaration.
    """
    out = []
    for line in text.split("\n"):
        line = re.sub(r'"(?:[^"\\]|\\.)*"', '""', line)
        line = re.sub(r"//.*$", "", line)
        out.append(line)
    return out


def collisions() -> dict[str, list[str]]:
    """`{type name: [repo-relative paths]}` for names declared in >1 file."""
    where: dict[str, set[str]] = defaultdict(set)
    for path in sorted(CORE.rglob("*.vr")):
        rel = str(path.relative_to(REPO))
        for line in strip_noise(path.read_text(errors="replace")):
            name = declares_a_type(line)
            if name:
                where[name].add(rel)
    return {n: sorted(p) for n, p in where.items() if len(p) > 1}


def as_pairs(coll: dict[str, list[str]]) -> list[str]:
    return sorted(f"{name}\t{path}" for name, paths in coll.items() for path in paths)


def read_known() -> set[str]:
    if not KNOWN.exists():
        return set()
    return {
        ln.rstrip("\n")
        for ln in KNOWN.read_text().splitlines()
        if ln.strip() and not ln.startswith("#")
    }


def main() -> int:
    coll = collisions()
    pairs = as_pairs(coll)
    plat = {n for n, ps in coll.items() if all(PLATFORM.match(p) for p in ps)}

    if "--list" in sys.argv:
        for name, paths in sorted(coll.items()):
            tag = "  [platform twin]" if name in plat else ""
            print(f"{name}{tag}")
            for p in paths:
                print(f"    {p}")
        print(f"\n{len(coll)} colliding type name(s), {len(pairs)} declaration(s)")
        print(f"{len(plat)} of them are pure platform twins (see T0435)")
        return 0

    if "--write-baseline" in sys.argv:
        preamble = [
            "# Simple type names declared in more than one core/ file, as",
            "# `name<TAB>path` pairs — the KNOWN set.  The compiler's layout",
            "# registry keys by simple name, so each pair beyond the first is",
            "# a type whose fields may be answered by another module (T0458).",
            "#",
            "# A line REMOVED is a collision resolved.  A line ADDED is a",
            "# decision to ship one more, and belongs in a commit message",
            "# that says so.",
            "#",
            "# Generated by: scripts/ci/check_type_name_collisions.py --write-baseline",
        ]
        KNOWN.write_text("\n".join(preamble + pairs) + "\n")
        print(f"[ok] baseline written: {len(coll)} name(s), {len(pairs)} declaration(s)")
        return 0

    known = read_known()
    now = set(pairs)
    new = sorted(now - known)
    gone = sorted(known - now)

    if gone:
        print(f"[ok] {len(gone)} declaration(s) no longer collide:")
        for g in gone[:20]:
            print(f"    {g}")
        if len(gone) > 20:
            print(f"    … and {len(gone) - 20} more")
        print("    scripts/ci/check_type_name_collisions.py --write-baseline")

    if new:
        print(f"\n[fail] {len(new)} new colliding type declaration(s):")
        for n in new:
            name, path = n.split("\t", 1)
            others = [p for p in coll[name] if p != path]
            print(f"    {name}  in  {path}")
            for o in others:
                print(f"        already declared in  {o}")
        print(
            "\nThe layout registry keys types by SIMPLE name, so the second\n"
            "declarer resolves against the first one's fields.  Rename one\n"
            "side, or — if this is deliberate — say so in the commit and\n"
            "re-record the baseline."
        )
        return 1

    print(
        f"[ok] type-name ratchet holds: {len(coll)} colliding name(s), "
        f"{len(pairs)} declaration(s), none new"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
