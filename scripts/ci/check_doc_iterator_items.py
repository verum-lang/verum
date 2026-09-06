#!/usr/bin/env python3
"""Gate: a documentation line that names a `core/` iterator type AND its
item type must agree with what that iterator's `next` actually yields.

WHAT IT ACTUALLY CATCHES — measured against the pre-fix corpus, not
claimed. Eight defects shipped in one day through every other doc gate,
all of them the same sentence: the page promised a BORROW where the
library hands OWNERSHIP. Run against the documentation as it stood
before those were corrected, THIS GATE REPORTS ONE OF THE EIGHT. The
other seven are invisible to it for a structural reason, and the number
is stated here so nobody reads a green run as "the class is covered":

    m.keys()    // Iterator<&K>    MapKeys.next   -> Maybe<K>
    m.values()  // Iterator<&V>    MapValues.next -> Maybe<V>
    m.iter()    // Iterator<(&K, &V)>              -> Maybe<(K, V)>
    s.lines()   // Iterator<&Text> Lines.next     -> Maybe<Text>

the seven wrote `m.keys() // Iterator<&K>`, naming no iterator type at
all, so there is no join key on the line and finding one means inferring
the type of `m` — the type resolution this gate exists to avoid needing.
The one it does catch, `s.lines() -> Lines // Iterator<&Text>`, names
`Lines`.

THE COROLLARY IS THE REAL VALUE, and it is measurable: correcting those
pages took the checkable corpus from NINE claims to TWENTY-TWO, because
writing `MapKeys<K, V>  Item = K` instead of `Iterator<&K>` is both more
accurate and the only form anything can verify. This gate pays a page
back for naming its types. A doc convention that no instrument can read
is a doc convention that drifts.

Nothing existing could see even the one. `check_doc_names_exist` asks
whether the RECEIVER is declared; `check_doc_method_names` asks whether
the METHOD name exists anywhere in `core/` and says so in its own
docstring; `check_doc_blocks_parse` is satisfied by an annotation that
parses. `&K` and `K` both parse, both name declared things, and only one
of them is what the caller gets.

WHY IT CAN BE MECHANICAL WHERE THE OTHERS COULD NOT. The precise
version of the method-name gate needs type resolution, because the doc
writes `a.union(&b)` and the receiver's type is not on the line. Here
the doc writes the ITERATOR TYPE ITSELF — `MapKeys<K, V>`, `Lines`,
`BytesIter` — so the join key is present in the text and `core/` answers
directly with `type Item`.

THE EXTRACTOR ACCUSED THE DOCUMENTATION NINE TIMES BEFORE IT WORKED, and
that is why every one of those nine is a control below. The first
version split the claim on the first `>` or the first comma, so
`Iterator<IoResult<List<Byte>>>` arrived as `IoResult<List<Byte`, and
`Item = (K, V)` as `(K`. Nine of twenty-two lines were reported and all
nine were the instrument's. A gate that mis-parses in the accusing
direction is worse than no gate: it teaches its readers to skip it.

WHAT IT DOES NOT COVER. A page may legitimately name a type `core/` also
declares — a tutorial's own `Lines`, say. The keyed baseline is for
that, and an entry needs a reason. It also does not check the annotation
where the doc names no iterator type: `xs.iter() // Iterator<&T>` alone
carries no join key, and inferring one from `xs` is the type resolution
this gate exists to avoid needing.
"""
import os
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CORE = REPO / "core"
DOCS = Path(os.environ.get("VERUM_DOCS_DIR") or (REPO.parent / "website" / "docs"))

# `implement<...> Iterator for Name<...> { ... }` up to the closing brace
# at column 0. Reading to a column-0 `}` rather than counting braces is
# enough here because `core/` never indents an impl block.
IMPL = re.compile(
    r"implement[^\n]*?\bIterator\s+for\s+([A-Z][A-Za-z0-9_]*)\s*(?:<[^>]*>)?\s*\{(.*?)\n\}",
    re.S)
ITEM_DECL = re.compile(r"type Item\s*=\s*([^;\n]+)")
NEXT_DECL = re.compile(r"fn next\(&mut self\)\s*->\s*Maybe<(.+?)>\s*\{")

# A keyed exception: `page::Type` -> reason. Never a bare page.
BASELINE = {}


def _balanced(text: str, start: int) -> str | None:
    """Read from the `<` at `start` to its matching `>`.

    `Iterator<IoResult<List<Byte>>>` is why this exists: splitting on the
    first `>` reported three io.md lines as defects and all three were
    this function's absence.
    """
    if start >= len(text) or text[start] != "<":
        return None
    depth = 0
    for i in range(start, len(text)):
        if text[i] == "<":
            depth += 1
        elif text[i] == ">":
            depth -= 1
            if depth == 0:
                return text[start + 1:i]
    return None


def _item_after_eq(text: str, start: int) -> str:
    """Read an `Item = ...` claim: to end of line, or to a comma that is
    not inside brackets. `Item = (K, V)` is one claim, not two."""
    depth = 0
    out = []
    for ch in text[start:]:
        if ch in "<([":
            depth += 1
        elif ch in ">)]":
            depth -= 1
            if depth < 0:
                break
        elif ch in ",;" and depth == 0:
            break
        elif ch == "\n":
            break
        out.append(ch)
    return "".join(out).strip()


def core_items() -> dict:
    out = {}
    for f in CORE.rglob("*.vr"):
        s = f.read_text(encoding="utf-8", errors="replace")
        for m in IMPL.finditer(s):
            name, body = m.group(1), m.group(2)
            decl = ITEM_DECL.search(body)
            if not decl:
                continue
            item = decl.group(1).strip()
            # The BODY is the authority when the two disagree; a
            # declaration is a claim and `next` is what runs.
            nxt = NEXT_DECL.search(body)
            if nxt and _norm(nxt.group(1)) != _norm(item):
                item = nxt.group(1).strip()
            out.setdefault(name, item)
    return out


def _norm(t: str) -> str:
    return re.sub(r"\s+", "", t)


def claims(items: dict):
    """(page, line_no, type_name, doc_claim, core_item) for every doc line
    that names a core iterator type AND states an item type."""
    found = []
    scanned = 0
    for p in sorted(DOCS.rglob("*.md")) + sorted(DOCS.rglob("*.mdx")):
        rel = str(p.relative_to(DOCS))
        for n, line in enumerate(
                p.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
            named = [t for t in items if re.search(r"\b" + t + r"\b", line)]
            if not named:
                continue
            # Longest name wins: `MapIterMut` also matches `MapIter`.
            t = max(named, key=len)
            claim = None
            m = re.search(r"\bIterator<", line)
            if m:
                claim = _balanced(line, m.end() - 1)
                if claim is not None and claim.lstrip().startswith("Item"):
                    eq = claim.find("=")
                    claim = _item_after_eq(claim, eq + 1) if eq >= 0 else None
            if claim is None:
                m = re.search(r"\bItem\s*=", line)
                if m:
                    claim = _item_after_eq(line, m.end())
            if not claim:
                continue
            scanned += 1
            found.append((rel, n, t, claim.strip(), items[t]))
    return found, scanned


def self_test() -> int:
    """The nine lines this extractor reported before it worked, plus both
    polarities. A parser that matches nothing passes every gate."""
    bad = 0
    cases = [
        # (line, type, expected claim) — the nine false accusations
        ("type SplitIter<R: BufRead>;   // Iterator<IoResult<List<Byte>>>",
         "SplitIter", "IoResult<List<Byte>>"),
        ("type BytesIter<R: Read>;      // Iterator<IoResult<Byte>>",
         "BytesIter", "IoResult<Byte>"),
        ("type LinesIter<R: BufRead>;   // Iterator<IoResult<Text>>",
         "LinesIter", "IoResult<Text>"),
        ("m.iter()      // MapIter<K, V>       Item = (K, V)",
         "MapIter", "(K, V)"),
        ("m.iter_mut()  // MapIterMut<K, V>    Item = (&K, &mut V)",
         "MapIterMut", "(&K, &mut V)"),
        ("m.into_iter() // MapIntoIter<K, V>   Item = (K, V)",
         "MapIntoIter", "(K, V)"),
        ("m.values_mut()// MapValuesMut<K, V>  Item = &mut V",
         "MapValuesMut", "&mut V"),
        ("m.drain()     // MapDrain<K, V>      Item = (K, V)",
         "MapDrain", "(K, V)"),
        ("xs.iter_mut() // ListIterMut<T>   lazy, Item = &mut T",
         "ListIterMut", "&mut T"),
        # a real defect, in the spelling it shipped in
        ("s.lines()  -> Lines   // Iterator<&Text> (split on '\\n')",
         "Lines", "&Text"),
    ]
    fake = {t: "IRRELEVANT" for _, t, _ in cases}
    import tempfile
    with tempfile.TemporaryDirectory() as d:
        page = Path(d) / "t.md"
        for line, typ, want in cases:
            page.write_text(line + "\n", encoding="utf-8")
            global DOCS
            keep, DOCS = DOCS, Path(d)
            got, _ = claims(fake)
            DOCS = keep
            if len(got) != 1 or got[0][2] != typ or _norm(got[0][3]) != _norm(want):
                print(f"  SELF-TEST FAIL: {line!r} -> {got}", file=sys.stderr)
                bad += 1
        print(f"  [ok] {len(cases) - bad} of {len(cases)} claim(s) parsed exactly")

        # NEGATIVE 1 — a type with no item claim is not a row.
        page.write_text("`MapKeys` iterates the keys.\n", encoding="utf-8")
        keep, DOCS = DOCS, Path(d)
        got, _ = claims(fake)
        DOCS = keep
        if got:
            print(f"  SELF-TEST FAIL: prose without an item claim was read as one: {got}",
                  file=sys.stderr)
            bad += 1
        else:
            print("  [ok] 0 rows from prose that names a type but claims no item")

        # NEGATIVE 2 — THE DOCUMENTED COVERAGE LIMIT, asserted rather than
        # merely described. `m.keys() // Iterator<&K>` is a REAL defect
        # (MapKeys yields an owned K) and this gate cannot see it, because
        # the line names no iterator type and the join key would have to
        # come from inferring `m`. If a later change makes this line a row,
        # the gate has silently grown a type-inference habit and its
        # docstring is lying about what it covers.
        page.write_text("m.keys()        // Iterator<&K>\n", encoding="utf-8")
        keep, DOCS = DOCS, Path(d)
        got, _ = claims(fake)
        DOCS = keep
        if got:
            print(f"  SELF-TEST FAIL: a line with no iterator type became a "
                  f"row — the coverage limit in the docstring is now false: {got}",
                  file=sys.stderr)
            bad += 1
        else:
            print("  [ok] 0 rows where the doc names an item but no type "
                  "(the documented limit)")
    return 1 if bad else 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    # `--check` is accepted for symmetry with the other doc gates and is a
    # no-op: this one is strict by default. A missing docs directory, an
    # empty core index and an empty claim corpus all return 2 rather than
    # OK, because the failure mode these gates keep having is reporting
    # green over an input they never found.
    if not DOCS.is_dir():
        print(f"docs directory not found: {DOCS} — set VERUM_DOCS_DIR", file=sys.stderr)
        return 2
    items = core_items()
    # An instrument that cannot find its input must get STRICTER, never
    # report OK. `core/` has well over a hundred; a collapse to a handful
    # means the impl regex stopped matching.
    if len(items) < 50:
        print(f"doc-iterator-items: FAIL — indexed only {len(items)} iterator "
              "types from core/. The `implement ... Iterator for` spelling "
              "changed, or core/ moved; this gate is measuring nothing.",
              file=sys.stderr)
        return 2
    rows, scanned = claims(items)
    if scanned == 0:
        print("doc-iterator-items: FAIL — 0 doc lines carried both an "
              "iterator type and an item claim. The corpus had 22; a drop to "
              "zero is the extractor, not the docs.", file=sys.stderr)
        return 2
    bad = [r for r in rows if _norm(r[3]) != _norm(r[4])]
    kept = [r for r in bad if f"{r[0]}::{r[2]}" not in BASELINE]
    if kept:
        print(f"doc-iterator-items: FAIL — {len(kept)} of {scanned} item "
              "claim(s) disagree with core/:", file=sys.stderr)
        for pg, n, t, c, real in kept:
            print(f"    {pg}:{n}  {t}: doc says `{c}`, core yields `{real}`",
                  file=sys.stderr)
        print("\nThe body is the authority: read what `next` RETURNS in "
              "core/, not what the page finds natural. A `&` written where "
              "the library yields an owned value promises a view and "
              "delivers a copy.", file=sys.stderr)
        return 1
    print(f"[ok] doc-iterator-items: {scanned} item claim(s) across "
          f"{len({r[0] for r in rows})} page(s) agree with core/ "
          f"({len(items)} iterator types indexed, {len(BASELINE)} keyed)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
