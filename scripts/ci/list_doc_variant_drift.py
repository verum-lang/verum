#!/usr/bin/env python3
"""A LIST TO READ, not a number to gate on: `Sum.Variant` spans in the
documentation naming a variant `core/` does not declare.

READ THIS BEFORE WIRING IT INTO CI. It must not be a gate, and the
reason is measured rather than cautious — see "the floor" below. It is
`list_` and not `check_` on purpose.

WHY IT EXISTS AT ALL. The site's other name gate reads ```verum blocks.
Every defect this found was in a MARKDOWN TABLE or in PROSE, inside
inline backticks, where no gate looks:

    stdlib/cli.md          6 of 10 exit-code names the library does not
                           have, one wrong code, 14 real variants omitted
                           — while the prose one paragraph BELOW the
                           table already said the truth
    stdlib/signal.md       `Signal.Chld`; the variant is `Child`
    stdlib/protobuf.md     `WireType.StartGroup` / `EndGroup` in a table
                           the ```verum block DIRECTLY BELOW disproves
    weft/spiffe.md         `WeftError.Forbidden`; the function returns
                           `AuthRejection`
    quic/transport-params  `TransportError.ProtocolViolation`; the real
                           thing is the wire CODE, not a variant
    cookbook/json.md       `Data.Number`; `Data` has `Int` and `Float`
                           separately, which is the opposite claim
    architecture-types ×4  `Capability.Custom { tag, schema }`; the
                           variant is `CustomCapability(Text)`

THE EXTRACTOR HAD FOUR HOLES AND EVERY ONE ACCUSED THE DOCUMENTATION.
That history is why the controls below run on every invocation:

    per-line anchoring       missed `| Black | DarkGrey`      4 false hits
    comment before variant 1 missed `TlsError.ProtocolVersion` 1
    struct-shaped variants   missed `OutputTooLarge { limit: Int }`  2
    schema notation          `KernelRule.K<Name>` is a PLACEHOLDER —
                             `core/verify/kernel_soundness/theorems.vr:10`
                             writes it the same way in its own comment  1

Eight accusations, all the instrument's. A hole now shows up as a
control mismatch instead of as a documentation "defect".

THE FLOOR, and it is why this can never gate. A page that TEACHES that a
name is invalid must write that name:

    "`Tier.TierCheck` will fail with `UnknownVariant{kind:"Tier"…}`"
    "Invalid values (`Foundation.UnknownProfile(...)`) …"

No syntactic instrument distinguishes a use from a denial in general.
The denial detector below catches some phrasings and will never catch
all of them, so a nonzero count is normal and a zero count would mean
the sweep had stopped working. Gating on it would punish pages for being
explicit — it would make the documentation worse.
"""
import os
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
DOCS = pathlib.Path(os.environ.get("VERUM_DOCS_DIR") or (REPO.parent / "website" / "docs"))
CORE = REPO / "core"

COMMENT = re.compile(r"//[^\n]*")
TYPE_DECL = re.compile(
    r"^\s*(?:public\s+|pub\s+)?type\s+([A-Z][A-Za-z0-9_]*)[^\n]*\bis\b(.*?);\s*$",
    re.M | re.S)
VARIANT = re.compile(r"\s*([A-Z][A-Za-z0-9_]*)\s*(?:[({]|$|\s)")
FENCE = re.compile(r"^```.*?^```", re.M | re.S)
INLINE = re.compile(r"`([A-Z][A-Za-z0-9_]*)\.([A-Z][A-Za-z0-9_]*)([^`]*)`?")
# The denial word can sit on EITHER side of the name, and measuring the
# site's own phrasings is what says so:
#
#   "Invalid values (`Foundation.UnknownProfile(...)`)"        before
#   "`Tier.TierCheck` will fail with `UnknownVariant`"          after
#   "The Verum-side variant `Tier.TierCheck` did not match"     after
#
# A window on one side only excused the first and reported the other
# three. This is noise reduction in a LIST, not a verdict, so looking
# both ways is the right trade: an over-broad exclusion costs a line
# somebody would have read, while an under-broad one costs the list its
# readability, which is the only thing it has.
DENIAL = re.compile(
    r"(no|not|does not|never|absent|missing|invalid|unknown|fail|removed|"
    r"renamed|deprecated)\b", re.I)

# Four that must resolve and three that must not. Each was a measured
# false positive or a measured defect; together they cover all four hole
# classes above.
CONTROLS = [
    ("Color", "DarkGrey", True),            # two variants on one line
    ("TlsError", "ProtocolVersion", True),  # comment before variant 1
    ("CompressError", "OutputTooLarge", True),   # struct-shaped variant
    ("Http2Error", "ConnectionError", True),     # struct-shaped variant
    ("WireType", "EndGroup", False),        # a real absence, fixed on the site
    ("WeftError", "Forbidden", False),      # a real absence, fixed on the site
    ("Signal", "Child", True),              # the correct spelling of a fixed typo
    # A KNOWN false positive, asserted as such: a tutorial's own `Expr`
    # against core's two unrelated ones. See the docstring for why
    # core-only scope is the right trade.
    ("Expr", "Lambda", False),
]


def _harvest(text, out):
    for m in TYPE_DECL.finditer(text):
        name, body = m.group(1), COMMENT.sub("", m.group(2))
        # A record's body opens with `{` before any `|`.
        if "{" in body.split("|")[0] and not body.lstrip().startswith("|"):
            continue
        variants = set()
        for part in body.split("|"):
            mm = VARIANT.match(part)
            if mm:
                variants.add(mm.group(1))
        if len(variants) >= 2:
            out.setdefault(name, set()).update(variants)


def sum_variants():
    """type name -> its variant names, from `core/` ONLY. `core/` is the
    authority and the docs deliberately are not.

    THIS WAS TRIED THE OTHER WAY AND THE CONTROL KILLED IT. Admitting
    doc-declared sums as well silences a real defect: `stdlib/protobuf.md`
    DECLARES `public type WireType is | Varint | Fixed64 | LengthDelim |
    StartGroup | EndGroup | Fixed32` in a ```verum block, where `core/`
    has four variants and neither group. Under corpus scope that
    declaration answers for itself and the page's fiction goes quiet —
    the `WireType.EndGroup` control flipped to True the moment the docs
    were admitted, which is what caught it.

    The cost of core-only scope is one known false positive:
    `tutorials/pattern-matching.md` builds a toy interpreter with its own
    `type Expr is … | Lambda { … }`, and `core/` has two unrelated `Expr`
    types (a SQL AST and a STARK AIR), so the page's `Expr.Lambda` reads
    as a miss. That is deliberate. NO SYNTACTIC RULE SEPARATES the two
    cases — a tutorial declaring its own type and a reference page
    contradicting core's look identical — so the choice is which error to
    take, and a false positive costs a line of reading while a false
    negative hides a fiction on a reference page.
    """
    out = {}
    for f in CORE.rglob("*.vr"):
        text = f.read_text(encoding="utf-8", errors="replace")
        _harvest(text, out)
    return out


def main() -> int:
    if not DOCS.is_dir():
        print(f"docs directory not found: {DOCS} — set VERUM_DOCS_DIR", file=sys.stderr)
        return 0
    sums = sum_variants()
    print(f"sum types indexed in core/: {len(sums)}\n")

    ok = True
    for typ, var, want in CONTROLS:
        got = var in sums.get(typ, set())
        ok &= got == want
        mark = "OK" if got == want else "MISMATCH"
        print(f"  control {typ}.{var:<18} expect {str(want):<5} got {str(got):<5} {mark}")
    if not ok:
        print("\nCONTROLS FAILED — the extractor has a hole and every finding "
              "below is suspect. Fix the extractor before reading the list.",
              file=sys.stderr)
        return 2
    print("  all controls pass\n")

    findings, denials, schemas = [], 0, 0
    for page in sorted(DOCS.rglob("*.md")) + sorted(DOCS.rglob("*.mdx")):
        text = FENCE.sub("", page.read_text(encoding="utf-8", errors="replace"))
        for m in INLINE.finditer(text):
            recv, mem, tail = m.group(1), m.group(2), m.group(3)
            if recv not in sums or mem in sums[recv]:
                continue
            # `KernelRule.K<Name>` / `LemmaStatus.<Status>` are schema
            # notation; core/ writes them the same way in its own comments.
            if tail.startswith("<") or "<" in mem:
                schemas += 1
                continue
            window = text[max(0, m.start() - 70):m.end() + 70]
            if DENIAL.search(window):
                denials += 1
                continue
            findings.append((f"{recv}.{mem}", str(page.relative_to(DOCS))))

    print(f"excluded as schema notation : {schemas}")
    print(f"excluded as denials         : {denials}")
    # COUNT THE THING THAT IS PRINTED. The first version counted the list
    # (7, with a page listed twice) and printed the set (5), so the number
    # and the list disagreed on the same line of output.
    unique = sorted(set(findings))
    print(f"\nTO READ ({len(unique)} of {len(findings)} spans) — each needs "
          f"checking against `core/` BY NAME before it is believed:")
    for name, page in unique:
        print(f"   {name:<34} {page}")
    print("\nA nonzero count is normal: a page teaching that a name is "
          "invalid must write that name. Never gate on this number.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
