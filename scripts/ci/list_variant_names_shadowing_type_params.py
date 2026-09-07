#!/usr/bin/env python3
"""A LIST TO READ: variant constructor names in `core/` that are ALSO
used as generic type-parameter names somewhere in `core/`.

WHY. T1214. `core/base/iterator.vr` declares

    fn collect<C: FromIterator<Self.Item>>(self) -> C { C.from_iter(self) }

and `core/math/internal.vr` declares

    public type FFIAbi is | C | Stdcall | Fastcall | ...

`extract_expr_type_name` resolves a bare UPPERCASE identifier by asking
which sum type owns a variant of that name, and asks WITHOUT first
checking whether the name is a type parameter in scope. So `C.from_iter`
was typed as a call on `FFIAbi`, emitted as `FFIAbi.from_iter`, and every
receiver-specialisation of `collect` — Args, ByRef, FilterIter,
MappedIter, Range, TransducedIter — carried a Tier-1 abort. Tier 0 was
right throughout, because it resolves from the actual receiver.

THE COUNT THAT MATTERS IS NOT THE INTERSECTION — it is how many owners
each name has. The lookup (`find_variant_parent_type_by_args`) returns
None when two sum types own the same variant name, so:

    UNIQUE owner   → the lookup answers → the name is LIVE ammunition
    2+ owners      → the lookup declines → inert TODAY, and it becomes
                     live the moment one of the owners is deleted or
                     stops being mounted

`C` itself has two owners in `core/` (`FFIAbi` and `ReprKind`), which is
exactly why this hid: it only fires in a program whose mount graph
carries one of them and not the other. So a two-owner row is not a
clean bill; it is a row whose verdict depends on the PROGRAM.

WHY IT IS A LIST AND NOT A GATE. A variant may legitimately be named
`C`, `T` or `E` — a calling convention, a grade, a type tag — and a
generic parameter may legitimately be named after a domain concept. The
collision is only a defect where a compiler path resolves a bare name
without consulting the generic-parameter scope. Gating on the
intersection would forbid ordinary naming to work around one missing
guard.

SISTER INSTRUMENT: `check_barename_collisions.py` gates free-function
(name, arity) collisions. It does not see this family — a variant
constructor is not a free function in its census — which is why the
intersection below went unmeasured until a showcase program stopped.
"""
import collections
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
CORE = REPO / "core"

COMMENT = re.compile(r"//[^\n]*")
TYPE_DECL = re.compile(
    r"^\s*(?:public\s+|pub\s+)?type\s+(?:affine\s+|linear\s+|relevant\s+)?"
    r"([A-Z][A-Za-z0-9_]*)[^\n]*\bis\b(.*?);\s*$",
    re.M | re.S)
VARIANT = re.compile(r"\s*([A-Z][A-Za-z0-9_]*)\s*(?:[({]|$|\s)")
# `fn name<A, B: Bound>` / `type X<T>` / `implement<T>` — the generic
# list only, never the bounds (a BOUND is a protocol name, and counting
# it would report every protocol as a type parameter).
# THE BOUND ITSELF CARRIES ANGLE BRACKETS, and the first version of this
# pattern forbade them — so `fn collect<C: FromIterator<Self.Item>>` , the
# exact declaration this instrument exists for, matched NOTHING. Two
# levels of nesting are allowed; the self-test pins that case by name.
# DECLARATION, NOT USE — and the difference cost 13 false accusations
# on the first run. `implement From<Int16> for Int32` puts a type
# ARGUMENT in the brackets; every "LIVE" row of that run was one of
# these, and each named a real core file as if it declared `Int16` as a
# parameter. Measured against core/: the DECLARING forms are
# `fn name<...>`, `type Name<...>` and `implement<...>` — brackets
# IMMEDIATELY after the keyword, no name between. `implement Name<...>`
# is always a use.
NEST = r"(?:[^<>{}();]|<(?:[^<>]|<[^<>]*>)*>){1,200}"
GENERICS = re.compile(
    r"\b(?:fn|type)\s+(?:affine\s+|linear\s+|relevant\s+)?"
    r"[A-Za-z_][A-Za-z0-9_]*\s*<(" + NEST + r")>"
    r"|\bimplement\s*<(" + NEST + r")>")
# LOOKAHEAD, NOT A CONSUMED DELIMITER. Consuming the trailing comma made
# consecutive parameters invisible: `A, B, R` reported A and R and
# dropped B, because the comma that ended A's match could not also begin
# B's.
PARAM = re.compile(
    r"(?:^|,)\s*(?:const\s+)?([A-Z][A-Za-z0-9_]*)\s*(?=:|,|$)")


def sum_variants():
    """variant name -> [(owning type, file)], from `core/` only."""
    owners = collections.defaultdict(list)
    for f in CORE.rglob("*.vr"):
        text = f.read_text(encoding="utf-8", errors="replace")
        for m in TYPE_DECL.finditer(text):
            name, body = m.group(1), COMMENT.sub("", m.group(2))
            # A record's body opens with `{` before any `|`.
            if "{" in body.split("|")[0] and not body.lstrip().startswith("|"):
                continue
            seen = set()
            for part in body.split("|"):
                mm = VARIANT.match(part)
                if mm:
                    seen.add(mm.group(1))
            if len(seen) >= 2:
                for v in seen:
                    owners[v].append((name, str(f.relative_to(REPO))))
    return owners


def type_params():
    """generic parameter name -> [(file, line)] over `core/`."""
    out = collections.defaultdict(list)
    for f in CORE.rglob("*.vr"):
        text = COMMENT.sub("", f.read_text(encoding="utf-8", errors="replace"))
        for m in GENERICS.finditer(text):
            ln = text[:m.start()].count("\n") + 1
            for p in PARAM.findall(m.group(1) or m.group(2) or ""):
                out[p].append((str(f.relative_to(REPO)), ln))
    return out


def self_test() -> int:
    """Both polarities on every extractor. A census that finds nothing
    passes by finding nothing."""
    bad = 0

    def check(label, got, want):
        nonlocal bad
        if got == want:
            print(f"  [ok] {label}")
        else:
            print(f"  SELF-TEST FAIL: {label} -> {got!r} (wanted {want!r})",
                  file=sys.stderr)
            bad += 1

    check("a bounded parameter is a parameter",
          PARAM.findall("C: FromIterator<Self.Item>"), ["C"])
    check("its BOUND is not — a protocol name must not count",
          "FromIterator" in PARAM.findall("C: FromIterator<Self.Item>"), False)
    check("several parameters",
          PARAM.findall("A, B, R"), ["A", "B", "R"])
    check("a const generic counts",
          PARAM.findall("const N"), ["N"])
    check("a generic LIST is found on a fn",
          bool(GENERICS.search("fn collect<C: FromIterator<Self.Item>>(self)")), True)
    check("`implement<T>` DECLARES",
          bool(GENERICS.search("implement<T> Weak<T> {")), True)
    check("`implement Trait<Arg> for X` does NOT — this is the hole that "
          "produced 13 false accusations",
          bool(GENERICS.search("implement From<Int16> for Int32 {")), False)
    check("`implement Type<Arg> { }` does NOT either",
          bool(GENERICS.search("implement MmioRegister<UInt32, ReadWrite> {")), False)
    check("a generic LIST is found on a type",
          bool(GENERICS.search("public type Result<T, E> is Ok(T) | Err(E);")), True)
    check("a lowercase name is not a parameter",
          PARAM.findall("self, other"), [])
    return 1 if bad else 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    owners = sum_variants()
    params = type_params()
    # DENOMINATORS FIRST. "No collisions" and "no inputs" print the same
    # verdict, and only these two numbers separate them.
    print(f"variant names declared in core/  : {len(owners)}")
    print(f"generic parameter names in core/ : {len(params)}")
    if not owners or not params:
        print("FAIL — one of the two censuses is empty, so the intersection "
              "below means nothing. The extractor broke, not the tree.",
              file=sys.stderr)
        return 2

    shared = sorted(set(owners) & set(params))
    live = [v for v in shared if len({o for o, _ in owners[v]}) == 1]
    ambiguous = [v for v in shared if len({o for o, _ in owners[v]}) > 1]
    print(f"names that are BOTH               : {len(shared)}\n")

    print(f"LIVE — one owning sum type, so the bare-name lookup ANSWERS "
          f"({len(live)}):")
    for v in live:
        # ONE OWNER TYPE can still be declared in more than one file
        # (a `@cfg` twin, a re-declared platform shim), so this is a
        # single owner NAME, not a single (name, file) pair.
        own, where = owners[v][0]
        files = sorted({w for _, w in owners[v]})
        uses = params[v]
        extra = f"  (+{len(files) - 1} more file(s))" if len(files) > 1 else ""
        print(f"   {v:<16} owner {own:<22} {where}{extra}")
        for u, ln in uses[:2]:
            print(f"   {'':<16}   used as a parameter at {u}:{ln}")
    print(f"\nAMBIGUOUS — two or more owners, so the lookup DECLINES today "
          f"({len(ambiguous)}). Not a clean bill: the verdict depends on which "
          f"owners a given program mounts, and deleting one owner makes the "
          f"row live:")
    for v in ambiguous:
        owns = sorted({o for o, _ in owners[v]})
        print(f"   {v:<16} owners {', '.join(owns)}")
        print(f"   {'':<16}   declared as a type parameter in "
              f"{len(params[v])} place(s), e.g.:")
        for u, ln in params[v][:3]:
            print(f"   {'':<16}     {u}:{ln}")
    print("\nNever gate on these numbers: a variant may legitimately be named "
          "`C` or `T`, and the collision is only a defect where a compiler "
          "path resolves a bare name without consulting the generic-parameter "
          "scope. The fix belongs in that path, not in the naming.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
