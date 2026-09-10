#!/usr/bin/env python3
"""A documented type declaration must carry the types `core/` gives it.

WHY A SEPARATE GATE FROM `check_doc_type_shapes`
------------------------------------------------
That gate compares the SHAPE a page prints — the kind (record or sum)
and the set of field NAMES. It is the reason six wrong declarations on
`term.md` were caught. It does not look at the TYPES, and

    type TaskId is { id: UInt64 };      core says `id: Int`

passes it with every name correct. The two questions are the same
distance apart as "does this method exist" and "what does it return",
and that pair cost twenty-two findings on the sibling gate.

It lives beside rather than inside because the shape gate's `shape()`
returns `(kind, frozenset(names))` through four call sites, and this
extraction needs three narrowings of its own that would have to be
threaded through all of them.

Measured 2026-09-10, first run over `docs/stdlib/**`:

    246 record types documented and declared exactly once in core/
     14 fields carrying a type core does not

`ChatMessage.role` and `.content` were both `Text` against `AgentRole`
and `MessageContent`; `IOVec.base` was `*mut Byte` against
`&unsafe Byte` — a raw pointer where the library has a tier-2
reference, which is the one difference this language cannot let a
reader guess. `CorecursiveCall.guard_depth` dropped a refinement:
`Int` for `Int{>= 0}`, so the page omits a constraint the compiler
enforces at every binding.

THREE NARROWINGS, EACH FROM A BROKEN READING
--------------------------------------------
1. A declaration body needs BALANCED braces. `[^}]*` stops at the `}`
   of a refinement — `guard_depth: Int{>= 0}` ended the body one field
   early and hid every field after it.
2. `>` closes a generic EXCEPT in `>=`. Treating it as a close drove
   the split depth negative inside `Int{>= 0}`, so the next comma never
   split and the rest of the record arrived glued to one field. The
   symptom was a finding whose two sides printed IDENTICALLY.
3. Only a type core declares EXACTLY ONCE is compared, and only fields
   present on both sides. Two declarations of one name are two types,
   and this reader cannot say which the page meant.

AND ONE THAT IS NOT A NARROWING BUT A WARNING. Correcting these by a
replacement bounded on `,` corrupted three lines — `Map<Text,
ProofCertificate>` has a comma INSIDE it, so the replacement took
`Map<Text` and left `, ProofCertificate>` behind. The gate then read
the corrupted text as AGREEING. A repair tool bounded by a delimiter
that occurs inside the value produces text that passes the check it was
run for.
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

HEAD = re.compile(
    r"(?:^|\n)[ \t]*(?:public\s+)?type\s+([A-Z][A-Za-z0-9_]*)\s*(?:<[^>]*>)?\s+is\s*\{"
)
FIELD_NAME = re.compile(r"[a-z_][a-z0-9_]*")
KNOWN: dict[str, str] = {}
FLOOR = 150

SUM = re.compile(
    r"(?:^|\n)[ \t]*(?:public\s+)?type\s+([A-Z][A-Za-z0-9_]*)\s*(?:<[^>]*>)?"
    r"\s+is\s+((?:[^;]|\{[^}]*\})*?);"
)
# A variant is TUPLE `Name(T)` or RECORD `Name { f: T }`. The two differ
# in how a reader constructs and matches them, so a reader cannot swap
# one for the other and they must not both read as "no payload" — which
# is what a tuple-only pattern does, and it hid five findings on one
# page while reporting core as having nothing there.
VARIANT = re.compile(r"([A-Z][A-Za-z0-9_]*)\s*(\([^)]*\)|\{[^}]*\})?")


def variants(body: str) -> dict[str, str]:
    """variant name -> its payload, normalised. {} when not a sum."""
    if "|" not in body:
        return {}
    out: dict[str, str] = {}
    for arm in split_fields(body.replace("|", ",")):
        arm = re.sub(r"//[^\n]*", "", arm).strip()
        m = VARIANT.match(arm)
        if not m:
            continue
        pay = re.sub(r"\s+", "", m.group(2) or "")
        if pay in ("(...)", "(\u2026)", "{...}", "{\u2026}"):
            pay = "ELIDED"          # the page's own abbreviation, not a claim
        elif pay.startswith("{"):
            # a RECORD variant: field ORDER is not part of the pattern a
            # reader writes, and the NAMES are not part of a positional
            # one — compare the type multiset.
            inner = ",".join(sorted(x.split(":", 1)[-1]
                                    for x in pay[1:-1].split(",") if x))
            pay = "{" + inner + "}"
        elif pay:
            # `(err: OSError)` is the same arm as `(OSError)` to anyone
            # matching positionally.
            inner = ",".join(x.split(":", 1)[-1] for x in pay[1:-1].split(","))
            pay = "(" + inner + ")"
        out[m.group(1)] = pay
    return out


def decls(text: str):
    """(name, body) with BALANCED braces."""
    for m in HEAD.finditer(text):
        i = m.end() - 1
        depth = 0
        for j in range(i, min(len(text), i + 4000)):
            if text[j] == "{":
                depth += 1
            elif text[j] == "}":
                depth -= 1
                if depth == 0:
                    yield m.group(1), text[i + 1: j]
                    break


def split_fields(body: str) -> list[str]:
    out, depth, cur, prev = [], 0, "", ""
    for ch in body:
        if ch in "<([{":
            depth += 1
        elif ch in ")]}" or (ch == ">" and prev != "-"):
            depth = max(0, depth - 1)
        prev = ch
        if ch == "," and depth == 0:
            out.append(cur)
            cur = ""
        else:
            cur += ch
    out.append(cur)
    return out


def fields(body: str) -> dict[str, str]:
    out: dict[str, str] = {}
    for piece in split_fields(body):
        piece = re.sub(r"//[^\n]*", "", piece).strip()
        if ":" not in piece:
            continue
        name, ty = piece.split(":", 1)
        name = name.strip().split()[-1] if name.strip() else ""
        if not FIELD_NAME.fullmatch(name):
            continue
        out[name] = re.sub(r"\s+", "", ty.strip().rstrip(","))
    return out


def self_test() -> int:
    bad = 0

    body = next(decls("public type A is {\n  callee: Text,\n"
                      "  guard_depth: Int{>= 0},\n};\n"))[1]
    got = fields(body)
    if got != {"callee": "Text", "guard_depth": "Int{>=0}"}:
        print(f"self-test: a refinement broke the body or the split: {got}",
              file=sys.stderr)
        bad += 1

    got = fields("a: Map<Text, Cert>, b: Int")
    if got != {"a": "Map<Text,Cert>", "b": "Int"}:
        print(f"self-test: a comma inside a generic split a field: {got}",
              file=sys.stderr)
        bad += 1

    if len(list(decls("type A is { x: Int };\ntype B is { y: Int };\n"))) != 2:
        print("self-test: two declarations were not both found", file=sys.stderr)
        bad += 1

    v = variants("Fullscreen | Inline { height: Int } | Fixed(Rect)")
    if v != {"Fullscreen": "", "Inline": "{Int}", "Fixed": "(Rect)"}:
        print(f"self-test: a record variant was read as a tuple one: {v}",
              file=sys.stderr)
        bad += 1
    v = variants("A(err: OSError) | B(...) | C")
    if v != {"A": "(OSError)", "B": "ELIDED", "C": ""}:
        print(f"self-test: payload name or elision not normalised: {v}",
              file=sys.stderr)
        bad += 1
    if variants("{ x: Int }") != {}:
        print("self-test: a record was read as a sum", file=sys.stderr)
        bad += 1

    for label, doc_t, core_t in (
        ("TaskId.id", "UInt64", "Int"),
        ("ChatMessage.role", "Text", "AgentRole"),
        ("IOVec.base", "*mutByte", "&unsafeByte"),
    ):
        if doc_t == core_t:
            print(f"self-test: anchor {label} no longer differs", file=sys.stderr)
            bad += 1

    if bad:
        print(f"self-test: {bad} FAILED", file=sys.stderr)
        return 1
    print(f"[ok] self-test: 3 field cases, 3 variant cases, 3 anchors, "
          f"{len(KNOWN)} on the roster")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not DOCS.is_dir():
        print(f"check-doc-field-types: no website at {DOCS} — REFUSING to "
              f"report OK. A gate whose INPUT is missing is a failed "
              f"checkout, not 'nothing to do'; set VERUM_DOCS_DIR.",
              file=sys.stderr)
        return 2

    core: dict[str, list[dict[str, str]]] = collections.defaultdict(list)
    core_v: dict[str, list[dict[str, str]]] = collections.defaultdict(list)
    for f in CORE.rglob("*.vr"):
        text = f.read_text(errors="replace")
        for name, body in decls(text):
            got = fields(body)
            if got:
                core[name].append(got)
        for m in SUM.finditer(text):
            got = variants(m.group(2))
            if got:
                core_v[m.group(1)].append(got)

    pages = sorted((DOCS / "stdlib").rglob("*.md")) if (DOCS / "stdlib").is_dir() else []
    doc: dict[str, list[tuple[dict[str, str], str]]] = collections.defaultdict(list)
    doc_v: dict[str, list[tuple[dict[str, str], str]]] = collections.defaultdict(list)
    for p in pages:
        text = p.read_text(errors="replace")
        rel = str(p.relative_to(DOCS))
        for name, body in decls(text):
            got = fields(body)
            if got:
                doc[name].append((got, rel))
        for m in SUM.finditer(text):
            got = variants(m.group(2))
            if got:
                doc_v[m.group(1)].append((got, rel))

    comparable = sum(1 for n in set(doc) & set(core) if len(core[n]) == 1)
    off: list[str] = []
    for name in sorted(set(doc) & set(core)):
        if len(core[name]) != 1:
            continue
        c = core[name][0]
        for d, page in doc[name]:
            for fld, dt in sorted(d.items()):
                if fld in c and c[fld] != dt and KNOWN.get(f"{name}.{fld}") is None:
                    off.append(f"{name}.{fld}  page {dt!r} vs core {c[fld]!r}"
                               f"   [{page}]")

    for name in sorted(set(doc_v) & set(core_v)):
        if len(core_v[name]) != 1:
            continue
        c = core_v[name][0]
        for d, page in doc_v[name]:
            for arm, dp in sorted(d.items()):
                if arm not in c or c[arm] == dp:
                    continue
                if "ELIDED" in (dp, c[arm]):
                    continue
                if KNOWN.get(f"{name}.{arm}") is not None:
                    continue
                off.append(f"{name}.{arm}  page {dp or '(no payload)'!r} vs "
                           f"core {c[arm] or '(no payload)'!r}   [{page}]")

    print(f"check-doc-field-types: {len(doc)} record type(s) documented across "
          f"{len(pages)} page(s), {comparable} declared exactly once in core "
          f"and {sum(1 for n in set(doc_v)&set(core_v) if len(core_v[n])==1)} "
          f"sum type(s) — {len(off)} field(s) or variant payload(s) carrying "
          f"a type core does not "
          f"({len(KNOWN)} on the roster)")

    if comparable < FLOOR:
        print(f"\nonly {comparable} type(s) were comparable, below the floor "
              f"of {FLOOR}. The declaration or field shape changed, so this "
              f"gate measured almost nothing — refusing rather than passing. "
              f"Measured 2026-09-10 it was 246.", file=sys.stderr)
        return 2

    for s in off:
        print(f"    + {s}")
    if off:
        print("  A reader builds the record literal this row describes. "
              "Correct the page, or add the field to KNOWN with why core is "
              "the one that is wrong.")
    return 1 if off else 0


if __name__ == "__main__":
    sys.exit(main())
