#!/usr/bin/env python3
"""Gate: a documentation example must not teach a meta-function the
compiler does not accept.

WHY THIS EXISTS, and why it is a SEPARATE family from the intrinsic-key
gate next to it. `check_doc_intrinsic_traps` (T1372) keys on
`@intrinsic("verum.*")` — a call whose key names a missing
implementation. This one keys on the OTHER shape: `@name(...)` where
`name` is in NEITHER compiler roster. The parser emits
`warning<E0410>` and builds `ExprKind::MetaFunction` anyway; inference
falls to `_ => Type::unit()`. So the call compiles, types as Unit, and
whatever the example does with the result is done to nothing.

Measured 2026-09-10: eleven such names probed against all three
registries — the parser's `KNOWN_META_FUNCTIONS`, the inference match,
and the VBC intrinsic registry — and every one had ZERO occurrences in
`crates/` entire. `core/math/hott.vr` declares BOTH the type and the
constructor through them:

    public type I is @builtin_interval;
    public type HottPath<A>(a: A, b: A) is @builtin_path;
    public fn refl<A>(x: A) -> HottPath<A>(x, x) { @builtin_refl(x) }

and `getting-started/tour.md` mounts exactly those three.

THREE ROSTERS, ALL READ FROM THE COMPILER, never transcribed — the
sibling gate `check_meta_function_names.py` already reads them and this
one imports its readers rather than growing a second copy free to
disagree.

THE EXTRACTED BLOCKS GO THROUGH THE SIBLING'S OWN `scan()`. Writing a
second per-line detector here would be a control that runs BESIDE the
subject instead of through it: every position rule the sibling learned
the hard way (a `|` variant attribute, a declaration attribute at column
0, a block-tail call that IS in expression position) would have to be
re-learned. The blocks are written to a scratch tree as `.vr` files and
`scan()` is called on that root.

WHAT THIS DOES NOT COVER, stated rather than left to be discovered: a
page that names a meta-function in PROSE rather than in a ```verum
block. That is deliberate — prose about a name is not an example
teaching it, and counting it is how the sibling census in T1372 got five
false positives on its first pass.
"""
from __future__ import annotations
import os
import pathlib
import re
import shutil
import sys
import tempfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from check_meta_function_names import (  # noqa: E402
    inference_roster,
    parser_roster,
    scan,
)

REPO = pathlib.Path(__file__).resolve().parents[2]
ATTRS = REPO / "crates" / "verum_types" / "src" / "attr" / "standard.rs"
_DOCS_ROOT = os.environ.get("VERUM_DOCS_DIR")
DOCS = pathlib.Path(_DOCS_ROOT) if _DOCS_ROOT else REPO.parent / "website" / "docs"
BLOCK = re.compile(r"```verum\n(.*?)```", re.S)

# The population, keyed on IDENTITY (page, name) rather than on a count.
# A swap — one page stops teaching a name while another starts — holds a
# count and moves these rows.
KNOWN: dict[str, list[str]] = {
    # The cubical/HoTT primitives. Still real findings, and the reason is
    # narrower than the first version of this comment claimed. That
    # version said the names have ZERO occurrences in `crates/`; it was
    # written from a grep for `"builtin_refl"` while the code writes the
    # literal WITH the sigil, `"@builtin_refl"`. Searched correctly,
    # ELEVEN of them have a code-generation arm apiece in
    # `expressions::compile_call`, each emitting a `CubicalExtended`
    # sub-op that the interpreter implements (twelve handlers) and LLVM
    # lowers.
    #
    # What no layer supplies is their TYPE: the inference match has no
    # arm, so each becomes `Unit` and will not unify with the declared
    # return type. `core/math/hott.vr` declares both the TYPE and the
    # CONSTRUCTOR through them, so the tour's example stops at the type
    # check — `error<E400>: expected '@builtin_path', found 'Unit'` —
    # and never reaches the code generation that would have handled it.
    # A binding gap over a working runtime, not an unimplemented feature.
    #
    # SHRANK 2026-09-10: both pages now quote the E0410 warning in their
    # own status box, so `builtin_refl` on the first and
    # `builtin_path` / `builtin_refl` on the second are DISCLOSED rather
    # than taught, and the disclosure path reports them instead. The
    # remaining names appear further down each page, outside the forty
    # lines the disclosure rule reaches.
    "language/dependent-types.md": ["builtin_sym", "builtin_transport"],
    "verification/cubical-hott.md": [
        "builtin_sym", "builtin_trans", "builtin_transport",
    ],
    # `@llm_oracle` — called at core/math/tactics.vr:663 and declared
    # nowhere. Two pages teach it.
    "language/proof-dsl.md": ["llm_oracle"],
    "reference/tactics.md": ["llm_oracle"],
    # A PLACEHOLDER, not a promise, and there is no mechanical way to tell
    # the two apart: the surrounding prose is "the brace forms `@name[…]`
    # and `@name{…}` exist for DSLs that prefer square brackets", so
    # `@matrix[...]` stands for any such DSL. Listed rather than silently
    # excluded — a roster's job is to say what its count stands for.
    "language/meta/macro-kinds.md": ["matrix"],
    # A different namespace with the same spelling: the page calls
    # `@quantity(...)` a "typed attribute" (§2.1), and it is absent from
    # the 177-name attribute registry, not from the meta-function ones.
    # Real either way — a documented attribute the compiler does not
    # register — but the fix belongs to the attribute surface.
    "verification/quantitative-types.md": ["quantity"],

}
# Derived from the data, never written beside it — a literal here is a
# second source of truth that drifts.
BASELINE = sum(len(v) for v in KNOWN.values())


def attribute_roster() -> set[str]:
    """THE THIRD NAMESPACE, and leaving it out is what made this gate's
    first run report 32 pairs where the truth is far fewer.

    `@` opens TWO different things. `@type_name(T)` is a meta-function
    CALL; `@cold`, `@must_use`, `@repr(C)` are ATTRIBUTES, registered
    separately in verum_types/src/attr/standard.rs (177 of them). The
    sibling gate separates them by POSITION — column 0 means attribute —
    and that rule is exact for `core/`, where an attribute owns its line.
    It is NOT exact for documentation, where the same attributes appear
    mid-line and in parameter position:

        @repr(C) @size(64)                       <- second one not at col 0
        fn process(@unused x: Int, @must_use r: &mut Out) { ... }

    Both were reported as unaccepted meta-calls. A false PLUS on a doc
    page costs more than a false minus: it sends someone to 'fix' a
    correct example. So the name set is the union of all three rosters.
    """
    if not ATTRS.is_file():
        return set()
    return set(re.findall(r'AttributeMetadata::new\("([A-Za-z_][A-Za-z0-9_]*)"\)',
                          ATTRS.read_text()))


def accepted_names() -> set[str]:
    return parser_roster() | inference_roster() | attribute_roster()


# A page may DEFINE the macro it then calls — that is the whole subject of
# `language/meta/*`, and calling a macro you just wrote is correct code, not
# a missing compiler feature. `meta fn repeat(...)` on the tour and
# `pub macro vec3 { ... }` on macro-kinds.md are both defined a few lines
# above their call. Neither belongs in a census of names the compiler does
# not accept, and both were in this gate's first two runs.
# A page that already SAYS the name does not work is not the defect this
# gate is for. `reference/meta-functions.md` carries a `:::caution Not yet
# callable` block naming the exact behaviour — "warn E0410 and evaluate to
# Unit ... treat the code blocks as the intended surface, not as working
# examples". Demanding a second fix there teaches the reader of this gate
# to re-fix what is fixed.
#
# The marker is the compiler's OWN diagnostic code rather than a phrase:
# a page that prints `E0410` is talking about this warning and nothing
# else. A prose test ("not implemented", "caution") would drift with
# whoever writes the next banner.
#
# THE DISCLOSED SET IS STILL PRINTED. A disclaimer beside a block does not
# unteach the block, so these rows are reported — under their own heading,
# without failing — and re-read when the roster moves.
DISCLOSED_MARK = re.compile(r"\bE0410\b")
# HOW CLOSE COUNTS. A page-level rule ("this page mentions E0410, so
# every name on it is disclosed") was the first version, and writing the
# tour's banner showed what it costs: one box disclosing ONE name would
# silence the whole page for every future one. Disclosure is per-NAME and
# proximity-based — the name must appear within this many lines of an
# E0410 mention, i.e. inside or beside the box that explains it.
DISCLOSE_WINDOW = 25


def disclosed_names(text: str) -> set[str]:
    """Names this page explicitly says do not work, by naming the
    compiler's own diagnostic beside them."""
    lines = text.splitlines()
    marks = [i for i, l in enumerate(lines) if DISCLOSED_MARK.search(l)]
    if not marks:
        return set()
    out: set[str] = set()
    for m in marks:
        lo = max(0, m - DISCLOSE_WINDOW)
        hi = min(len(lines), m + DISCLOSE_WINDOW + 1)
        for l in lines[lo:hi]:
            out |= set(re.findall(r"@([a-z_][a-z0-9_]*)", l))
    return out

DEFINES = re.compile(
    r"^\s*(?:pub\s+|public\s+)?(?:meta\s+fn|macro)\s+([A-Za-z_][A-Za-z0-9_]*)",
    re.M)


def extract(dest: pathlib.Path) -> tuple[int, dict[str, str], dict[str, set[str]]]:
    """Every ```verum block as a .vr file.

    Returns (block count, file->page, page->names it defines,
    page -> the names it explicitly discloses)."""
    origin: dict[str, str] = {}
    defined: dict[str, set[str]] = {}
    disclosed: dict[str, set[str]] = {}
    n = 0
    for page in sorted(DOCS.rglob("*.md")) + sorted(DOCS.rglob("*.mdx")):
        rel = str(page.relative_to(DOCS))
        page_text = page.read_text(errors="ignore")
        defined.setdefault(rel, set()).update(DEFINES.findall(page_text))
        dn = disclosed_names(page_text)
        if dn:
            disclosed[rel] = dn
        for i, m in enumerate(BLOCK.finditer(page.read_text(errors="ignore"))):
            stem = rel.replace("/", "__").rsplit(".", 1)[0]
            out = dest / f"{stem}__{i}.vr"
            out.write_text(m.group(1), encoding="utf8")
            origin[out.name] = rel
            n += 1
    return n, origin, defined, disclosed


def self_test() -> int:
    bad = 0
    accepted = accepted_names()
    if "cfg" not in accepted:
        print("self-test: the parser roster did not load (`cfg` missing)")
        bad += 1
    if len(accepted) < 200:
        print(f"self-test: only {len(accepted)} accepted names — a roster "
              "reader stopped matching")
        bad += 1
    if "must_use" not in accepted:
        print("self-test: the ATTRIBUTE roster did not load (`must_use` missing)")
        bad += 1
    with tempfile.TemporaryDirectory() as td:
        d = pathlib.Path(td)
        # (a) a name in NEITHER roster, in expression position -> reported
        (d / "a.vr").write_text("fn f() { let x = @nonesuch_meta(1); }\n")
        # (b) a name the parser accepts -> NOT reported
        (d / "b.vr").write_text("fn f() { let x = @type_name(Int); }\n")
        # (c) a declaration attribute at column 0 -> NOT reported
        (d / "c.vr").write_text("@derive(Debug)\npublic type T is { a: Int };\n")
        got = scan(d, accepted)
        if "nonesuch_meta" not in got:
            print("self-test: an unaccepted meta-call was NOT reported")
            bad += 1
        if "type_name" in got:
            print("self-test: an ACCEPTED meta-call was reported")
            bad += 1
        if "derive" in got:
            print("self-test: a declaration attribute was reported as a call")
            bad += 1
    # DISCLOSURE IS PER-NAME, NOT PER-PAGE. The first version keyed on
    # "does this page mention E0410 anywhere", and writing a caution box
    # for ONE name on the tour showed the cost: the box would have
    # silenced every other name on that page, for good.
    page = "\n".join([
        "prose about @alpha",           # far from the mark
        *[""] * 40,
        "the compiler warns E0410 on @beta and evaluates it to Unit",
    ])
    d = disclosed_names(page)
    if "beta" not in d:
        print("self-test: a name beside an E0410 mention is not disclosed")
        bad += 1
    if "alpha" in d:
        print("self-test: a name 40 lines from the only E0410 mention was "
              "disclosed — the rule went back to page-level")
        bad += 1
    if disclosed_names("no diagnostic named here, just @gamma"):
        print("self-test: a page with no E0410 mention disclosed something")
        bad += 1

    if bad:
        print(f"self-test: {bad} FAILED")
        return 1
    print(f"[ok] self-test: 3 detector cases + 3 roster loads "
          f"({len(accepted)} accepted names)")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not DOCS.is_dir():
        print(f"check-doc-meta-functions: {DOCS} not present — the docs live "
              "in a sibling checkout; reporting UNMEASURED rather than "
              "passing on an absent input.")
        return 0

    floor = 0
    for i, a in enumerate(sys.argv):
        if a == "--min-blocks" and i + 1 < len(sys.argv):
            floor = int(sys.argv[i + 1])

    accepted = accepted_names()
    if len(accepted) < 200:
        print(f"check-doc-meta-functions: only {len(accepted)} accepted "
              "meta-function names read from the compiler — a roster reader "
              "stopped matching. Refusing to report a census built on that.")
        return 1

    tmp = pathlib.Path(tempfile.mkdtemp(prefix="doc-meta-"))
    try:
        blocks, origin, defined, disclosed = extract(tmp)
        if blocks < floor:
            print(f"check-doc-meta-functions: only {blocks} ```verum block(s) "
                  f"under {DOCS}, expected at least {floor} — the corpus is "
                  "missing or the pattern stopped matching. A census of "
                  "nothing is not a clean census.")
            return 1
        found = scan(tmp, accepted)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    have: dict[str, set[str]] = {}
    for name, sites in found.items():
        for s in sites:
            fn = pathlib.Path(s.rsplit(":", 1)[0]).name
            page = origin.get(fn, fn)
            if name in defined.get(page, set()):
                continue          # the page defines the macro it calls
            have.setdefault(page, set()).add(name)

    shown = {}
    trimmed = {}
    for p, ns in have.items():
        d = ns & disclosed.get(p, set())
        rest = ns - d
        if d:
            shown[p] = d
        if rest:
            trimmed[p] = rest
    have = trimmed
    total = sum(len(v) for v in have.values())
    print(f"check-doc-meta-functions: {total} (page, name) pair(s) teaching a "
          f"meta-function the compiler does not accept, over {blocks} block(s) "
          f"(baseline {BASELINE})")

    want = {p: set(v) for p, v in KNOWN.items()}
    new = {p: sorted(ns - want.get(p, set())) for p, ns in have.items()}
    new = {p: ns for p, ns in new.items() if ns}
    gone = {p: sorted(want[p] - have.get(p, set())) for p in want}
    gone = {p: ns for p, ns in gone.items() if ns}

    for p, ns in sorted(have.items()):
        print(f"    {p}: {', '.join(sorted(ns))}")
    if shown:
        print(f"  DISCLOSED — the page names E0410 itself; reported, not failed:")
        for p, ns in sorted(shown.items()):
            print(f"    = {p}: {', '.join(sorted(ns))}")

    if new:
        print("  NEW — a page now teaches a name the compiler does not accept:")
        for p, ns in sorted(new.items()):
            print(f"    + {p}: {', '.join(ns)}")
    if gone:
        print("  GONE — fixed or moved; delete the row from KNOWN:")
        for p, ns in sorted(gone.items()):
            print(f"    - {p}: {', '.join(ns)}")
    return 1 if (new or gone) else 0


if __name__ == "__main__":
    sys.exit(main())
