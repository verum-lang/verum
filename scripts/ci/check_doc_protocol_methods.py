#!/usr/bin/env python3
"""A documented protocol must offer the method set `core/` declares.

WHY THIS RUNG
-------------
`check_doc_type_shapes` compares record fields and variant names.
`check_doc_method_signatures` compares a method that appears on BOTH
sides. Neither notices a page that INVENTS a method or drops a REQUIRED
one — and a protocol is the one declaration a reader retypes in full,
because implementing it means writing every method it names.

Both of this campaign's largest finds were exactly that, and both were
caught by hand rather than by a gate:

    Styled      page gave it two methods; core has one
    IOEngine    page gave three; core has six, and one of the three
                (`shutdown`) core did not have at all

Measured 2026-09-10, first clean run: 72 protocols documented, 59
compared against core (11 declared in another module under the same
name, 4 eliding with `...`), and by then the seven the probe found had
been corrected — `ExecutableTool`
documented as `fn call(args)` against core's `schema` / `execute`,
`RuntimeConfig` given six getters against core's five entirely
different methods, `TrustBundleProvider` given `current_bundle` and
`rotation_signal` against `x509_bundle` / `jwt_bundle` / `reload`.

FOUR NARROWINGS, EACH FROM A FALSE ACCUSATION
---------------------------------------------
1. `async fn` is a protocol method. A pattern without it read core's
   `HealthProbe` as having NO methods and reported the page as
   inventing the one it correctly documents.
2. SCOPE TO THE PAGE'S MODULE. `math.md`'s `Tokenizer` is an ML
   tokenizer; core's is FTS5's, in `core/database`. Two concepts under
   one name, and comparing them called a correct page wrong.
3. A method with a DEFAULT BODY is not required of an implementor, so
   omitting it is abbreviation. Four of ten "omissions" were defaults.
   The comparison is therefore ASYMMETRIC: the page may not name a
   method core has under no form, and may not omit one core requires.
4. READ ONE-LINE BODIES, and treat `...` as elision only in CODE.
   `type Zero is protocol { fn zero() -> Self; fn is_zero(..) -> Bool; }`
   is common, and a line-anchored reader saw none of it. Acting on that
   absence DUPLICATED two methods that were already there. Stripping
   comments before the elision test matters too: core's
   `WeftTransport` says "see `write_res...`" in a doc comment, and
   reading that as an elision emptied core's set and reported every
   method the page correctly documents as invented.

Narrowing 4 is the one to remember. The first three produced wrong
REPORTS; the fourth produced a wrong EDIT, because absence read as a
finding is acted on.
"""

from __future__ import annotations

import collections
import os
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
CORE = REPO / "core"
_DOCS_ROOT = os.environ.get("VERUM_DOCS_DIR")
DOCS = pathlib.Path(_DOCS_ROOT) if _DOCS_ROOT else REPO.parent / "website" / "docs"

PROTO = re.compile(
    r"(?:^|\n)[ \t]*(?:public\s+)?type\s+([A-Z][A-Za-z0-9_]*)\s*(?:<[^>]*>)?"
    r"\s+is\s+protocol\s*\{"
)
FN = re.compile(r"^(?:public\s+)?(?:async\s+)?fn\s+([a-z_][a-z0-9_]*)")
# A DEFAULT BODY opens with `{` after whitespace: `) -> Int { 0 }` and
# `) -> Result<…> {` both match. A REFINEMENT does not — `Int{>= 0}`
# has no space before the brace — so the two are told apart without
# knowing the type grammar. Caught by the self-test before shipping:
# a one-line default `fn helper(&self) -> Int { 0 }` was being counted
# as a required method, because the earlier rule only looked at whether
# the fragment ENDED with `{`.
HAS_BODY = re.compile(r"\)\s*(?:->[^{]*?)?\s\{")
KNOWN: dict[str, str] = {}
FLOOR = 40


def protocols(text: str) -> dict[str, tuple[set[str], set[str], bool]]:
    """name -> (required methods, all methods, the body elides with `...`)."""
    out: dict[str, tuple[set[str], set[str], bool]] = {}
    for m in PROTO.finditer(text):
        i = m.end() - 1
        depth = 0
        body = None
        for j in range(i, min(len(text), i + 8000)):
            if text[j] == "{":
                depth += 1
            elif text[j] == "}":
                depth -= 1
                if depth == 0:
                    body = text[i + 1: j]
                    break
        if body is None:
            continue
        code = re.sub(r"//[^\n]*", "", body)
        if "..." in code or "…" in code:
            out.setdefault(m.group(1), (set(), set(), True))
            continue
        req, full = set(), set()
        for frag in re.split(r"[;\n]", code):
            for fn in FN.findall(frag.strip()):
                full.add(fn)
                if not HAS_BODY.search(frag):
                    req.add(fn)
        cur = out.setdefault(m.group(1), (set(), set(), False))
        cur[0].update(req)
        cur[1].update(full)
    return out


def self_test() -> int:
    bad = 0

    got = protocols("public type P is protocol {\n"
                    "    async fn probe(&self, u: &U) -> Bool;\n"
                    "    fn helper(&self) -> Int { 0 }\n"
                    "};\n")
    if "P" not in got or got["P"][0] != {"probe"} or got["P"][1] != {"probe", "helper"}:
        print(f"self-test: async or default handling wrong: {got}", file=sys.stderr)
        bad += 1

    got = protocols("type Zero is protocol { fn zero() -> Self; fn is_zero(&self) -> Bool; }\n")
    if got.get("Zero", (set(),))[0] != {"zero", "is_zero"}:
        print(f"self-test: a one-line body was not read: {got}", file=sys.stderr)
        bad += 1

    got = protocols("type E is protocol { ... };\n")
    if not got.get("E", (None, None, False))[2]:
        print("self-test: an elided body was not recognised", file=sys.stderr)
        bad += 1

    got = protocols("type W is protocol {\n"
                    "    /// see `write_res...` in connection.vr\n"
                    "    fn peer_addr(&self) -> Int;\n"
                    "};\n")
    if got.get("W", (set(), set(), True))[2] or got["W"][0] != {"peer_addr"}:
        print(f"self-test: `...` in a COMMENT was read as elision: {got}",
              file=sys.stderr)
        bad += 1

    # THE ANCHOR, as the pair it was when found.
    page, core = {"call"}, ({"schema", "execute"}, {"schema", "execute"})
    if not (page - core[1]) or not (core[0] - page):
        print("self-test: the ExecutableTool anchor no longer differs",
              file=sys.stderr)
        bad += 1

    if bad:
        print(f"self-test: {bad} FAILED", file=sys.stderr)
        return 1
    print(f"[ok] self-test: 4 extraction cases, 1 anchor, "
          f"{len(KNOWN)} on the roster")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not DOCS.is_dir():
        print(f"check-doc-protocol-methods: no website at {DOCS} — REFUSING "
              f"to report OK. A gate whose INPUT is missing is a failed "
              f"checkout, not 'nothing to do'; set VERUM_DOCS_DIR.",
              file=sys.stderr)
        return 2

    core: dict[str, list[tuple[tuple[set[str], set[str], bool], str]]] = \
        collections.defaultdict(list)
    for f in CORE.rglob("*.vr"):
        module = f.relative_to(CORE).parts[0]
        for name, sets in protocols(f.read_text(errors="replace")).items():
            core[name].append((sets, module))

    stdlib = DOCS / "stdlib"
    pages = sorted(stdlib.rglob("*.md")) if stdlib.is_dir() else []
    doc: dict[str, list[tuple[tuple[set[str], set[str], bool], str, str]]] = \
        collections.defaultdict(list)
    for p in pages:
        rel = p.relative_to(stdlib)
        page_module = rel.parts[0][:-3] if len(rel.parts) == 1 else rel.parts[0]
        for name, sets in protocols(p.read_text(errors="replace")).items():
            doc[name].append((sets, str(p.relative_to(DOCS)), page_module))

    comparable = skipped = elided = 0
    off: list[str] = []
    for name in sorted(set(doc) & set(core)):
        if len(core[name]) != 1:
            continue
        (creq, cfull, _), cmod = core[name][0]
        for (dreq, dfull, delided), page, pmod in doc[name]:
            if pmod != cmod:
                skipped += 1
                continue
            if delided:
                elided += 1
                continue
            comparable += 1
            if KNOWN.get(name) is not None:
                continue
            invented = sorted(dfull - cfull)
            dropped = sorted(creq - dfull)
            if invented:
                off.append(f"{name} names {', '.join(invented)} — core has no "
                           f"such method   [{page}]")
            if dropped:
                off.append(f"{name} omits {', '.join(dropped)} — required, so "
                           f"an implementor written from this page will not "
                           f"compile   [{page}]")

    print(f"check-doc-protocol-methods: {len(doc)} protocol(s) documented "
          f"across {len(pages)} page(s), {comparable} compared against core "
          f"({skipped} out of the page's module, {elided} eliding with `...`) "
          f"— {len(off)} disagreement(s) ({len(KNOWN)} on the roster)")

    if comparable < FLOOR:
        print(f"\nonly {comparable} protocol(s) were comparable, below the "
              f"floor of {FLOOR}. The declaration shape or the module "
              f"mapping changed, so this gate measured almost nothing — "
              f"refusing rather than passing. Measured 2026-09-10 it was 59.",
              file=sys.stderr)
        return 2

    for s in off:
        print(f"    + {s}")
    return 1 if off else 0


if __name__ == "__main__":
    sys.exit(main())
