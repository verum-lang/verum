#!/usr/bin/env python3
"""The three FFI sites that consult `repr_c_types` through a reference must
match ALL THREE CBGR reference tiers.

CBGR spells a reference three ways and the AST gives each its own variant:

    TypeKind::Reference        &T          tier 0, CBGR-checked
    TypeKind::CheckedReference &checked T  tier 1, compiler-proven
    TypeKind::UnsafeReference  &unsafe T   tier 2, manual proof

A `match` that lists only `Reference` answers a question about ONE THIRD of
the reference forms and reads as though it covered them.  This gate exists
because that cost a P0 and because the same class had already been fixed
ONCE and read as closed.

WHAT IT COST, measured (T1192).  `ffi_referenced_struct_name` matched
`Reference | Pointer`, so `fstat(fd: Int32, buf: &unsafe DarwinStat)`
produced no `FfiStructLayout`, the parameter collapsed to `CType::Ptr`, and
the marshaller's pointer branch handed C the raw Verum heap-object pointer.
`fstat(2)` then wrote 144 bytes of `struct stat` starting at the OBJECT
HEADER.  The proof was a number rather than an argument: the panic reported
`type_id=16777230`, and `stat -f %d` on the file reports `st_dev=16777230`
— `st_dev` is the first field of `struct stat`, `type_id` the first field
of `ObjectHeader`, and they are the same bytes.

WHY A PIN AND NOT A CENSUS.  Eighty-four production match arms across the
workspace handle `Reference` with no sibling tier arm (T1358), and most are
almost certainly harmless — a ratchet over that population would train a
reader to wave it through.  These THREE were fixed because their
consequence was measured, so these three are what gets pinned.  Widening
the gate is a decision that belongs after T1358's triage, not before it.

WHY NOT JUST THE SPEC.  `File.size()` returning 5 covers this today, but it
covers it THROUGH the whole stdlib; a refactor that drops a tier from the
extractor would surface as a distant runtime failure, and the panic it
produces sends a reader after a layout bug — the detour this defect already
caused twice.  A source pin names the line.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
CODEGEN = REPO / "crates" / "verum_vbc" / "src" / "codegen" / "mod.rs"

TIERS = ("Reference", "CheckedReference", "UnsafeReference")

# Each site is identified by an ANCHOR that names what it decides — never by
# a line number, because a file that grows by ten lines must not move this
# table. The window is the match arm's own head plus the lines that follow.
# Each site is a FUNCTION, named. Not a line number (a file that grows must
# not move this table) and not a comment near the fix (anchoring on the fix's
# own words would make the gate pass only while that wording survives, which
# proves nothing about the code).
SITES = [
    (
        "ffi_referenced_struct_name",
        "the name extractor that decides whether a record gets an FfiStructLayout",
    ),
    (
        "verum_type_to_ctype",
        "the ctype mapper that decides StructPtr vs a bare Ptr",
    ),
    (
        "get_struct_layout_index",
        "the layout-index lookup that drives FFI write-back",
    ),
]



def self_test() -> int:
    """Pin the detector: it must fail on a two-tier arm and pass on a
    three-tier one, and it must refuse when its anchor is missing."""
    good = "\n".join(
        [
            "fn anchor_here() {",
            "    match x {",
            "        TypeKind::Reference { inner, .. }",
            "        | TypeKind::CheckedReference { inner, .. }",
            "        | TypeKind::UnsafeReference { inner, .. } => y,",
            "    }",
            "}",
        ]
    )
    bad = good.replace("        | TypeKind::UnsafeReference { inner, .. } => y,", "        => y,")
    failures = 0
    if missing_tiers(good, "anchor_here"):
        print("  SELF-TEST FAIL: a three-tier body was reported as missing tiers")
        failures += 1
    if missing_tiers(bad, "anchor_here") != ["UnsafeReference"]:
        print("  SELF-TEST FAIL: the dropped tier was not named exactly")
        failures += 1
    if missing_tiers(good, "no_such_function") != list(TIERS):
        print("  SELF-TEST FAIL: an absent function did not report as fully missing")
        failures += 1
    if failures:
        print(f"[FAIL] detector self-test: {failures} case(s)")
        return 2
    print("  [ok] detector self-test: 3 cases")
    return 0


def function_body(text: str, name: str) -> str | None:
    """The body of `fn <name>`, by brace balance. Returns None when the
    function is absent — the caller turns that into a refusal, never a pass."""
    lines = text.splitlines()
    start = next(
        (i for i, l in enumerate(lines) if re.search(rf"\bfn {re.escape(name)}\s*[(<]", l)),
        None,
    )
    if start is None:
        return None
    depth, seen, out = 0, False, []
    for line in lines[start:]:
        out.append(line)
        depth += line.count("{") - line.count("}")
        if "{" in line:
            seen = True
        if seen and depth <= 0:
            break
    return "\n".join(out)


def missing_tiers(text: str, name: str) -> list[str]:
    """Tiers absent from that function's body. An absent function reports
    every tier missing — an instrument that cannot find its input gets
    STRICTER, never softer."""
    body = function_body(text, name)
    if body is None:
        return list(TIERS)
    return [t for t in TIERS if not re.search(rf"TypeKind::{t}\b", body)]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return self_test()
    rc = self_test()
    if rc:
        return rc

    if not CODEGEN.is_file():
        print(f"[FAIL] {CODEGEN} not found — refusing to judge.")
        return 2
    text = CODEGEN.read_text()

    bad = []
    for name, what in SITES:
        missing = missing_tiers(text, name)
        if missing:
            bad.append((name, what, missing))

    shown = CODEGEN.relative_to(REPO) if CODEGEN.is_relative_to(REPO) else CODEGEN
    print(f"FFI reference-tier pins checked: {len(SITES)} site(s) in {shown}")
    if bad:
        print(f"\n[FAIL] {len(bad)} site(s) no longer cover all three tiers:")
        for name, what, missing in bad:
            print(f"    fn {name} — {what}")
            print(f"      missing: {', '.join('TypeKind::' + m for m in missing)}")
        print(
            "\n  A reference that reaches C is a C pointer whatever its CBGR\n"
            "  tier. Dropping a tier here does not produce a type error — it\n"
            "  produces `CType::Ptr`, and the marshaller then hands C the\n"
            "  Verum OBJECT HEADER address. Measured cost: `fstat(2)` wrote\n"
            "  `struct stat` over the header, and the panic that followed\n"
            "  reads like a layout bug."
        )
        return 1

    print(f"[ok] all {len(SITES)} sites list "
          f"{', '.join('TypeKind::' + t for t in TIERS)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
