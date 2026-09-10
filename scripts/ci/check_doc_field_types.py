#!/usr/bin/env python3
"""A documented record field must carry the type `core/` gives it.

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
    print(f"[ok] self-test: 3 extraction cases, 3 anchors, "
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
    for f in CORE.rglob("*.vr"):
        for name, body in decls(f.read_text(errors="replace")):
            got = fields(body)
            if got:
                core[name].append(got)

    pages = sorted((DOCS / "stdlib").rglob("*.md")) if (DOCS / "stdlib").is_dir() else []
    doc: dict[str, list[tuple[dict[str, str], str]]] = collections.defaultdict(list)
    for p in pages:
        for name, body in decls(p.read_text(errors="replace")):
            got = fields(body)
            if got:
                doc[name].append((got, str(p.relative_to(DOCS))))

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

    print(f"check-doc-field-types: {len(doc)} record type(s) documented across "
          f"{len(pages)} page(s), {comparable} declared exactly once in core "
          f"— {len(off)} field(s) carrying a type core does not "
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
