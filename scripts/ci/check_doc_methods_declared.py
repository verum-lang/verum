#!/usr/bin/env python3
"""Gate: a method a doc example CALLS should be declared somewhere in core/.

WHY THIS EXISTS, measured 2026-09-08 and not hypothetical. Five pages
showed

    Style.new().fg(color).add_modifier(Modifier.Bold)

    error<E400>: no method named `add_modifier` found for type `Style`

Three method names were invented and so was the constant casing.
`add_modifier` and `sub_modifier` are FIELDS of the `Style` record —
`Style.bold()` is `Style { add_modifier: …union(Modifier.BOLD), ..self }`
— which is why the field name reads like an API and is not one.

WHY NO EXISTING GATE SAW IT. The ladder asks three questions and this is
not among them:

    parse    every ```verum block parses
    names    a CAPITALISED receiver exists in core/   <- TYPES, not methods
    check    `verum check` type-checks the block
    run      only blocks with an `fn main`
    exercised whether anything EXECUTES the method     (a separate gate)

`Style` and `Modifier` are both real, so the names gate is silent; the
call is in a fragment, so nothing runs it. A documented method that
exists NOWHERE passed every rung.

WHAT THIS COUNTS. Method names called as `.name(` inside a ```verum
block, against every `fn name` declared in core/.

TWO PATTERN TRAPS, both of which bit me while writing this:

  * `fn NAME(` misses every GENERIC declaration, which is written
    `fn NAME<T: …>(`. A first version reported `shuffle_vec` and `choice`
    absent on that basis and the claim reached a commit message before
    the control refuted it.
  * asking "is it in the MISSING set" is not asking "is it declared" —
    a name no longer USED is in neither set, and the first control read
    that as declared. The control below asks `declared` directly.

UNDECLARED IS NOT ALWAYS WRONG. A reader's own example type has its own
methods (`find_user`, `from_row`, `build_request`), and those are the
floor — the same shape as the type-name gate's twenty-one reader-owned
types. The number goes down by fixing real ones, and the floor is
whatever survives.
"""
from __future__ import annotations
import os
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
DOCS = Path(os.environ.get("VERUM_STDLIB_DOCS") or (REPO.parent / "website" / "docs"))
CORE = REPO / "core"
BASELINE = 111  # Lowered by FIXING, never by argument.
                #   118 -> 114  the Postgres/MySQL config builders
                #               (with_host, with_port, with_user,
                #                with_database, with_password_from_env)
                #   114 -> 111  H3Response.with_body, OpenOptions.open_async,
                #               ServerOptions.with_cert_pem/with_key_pem

BLOCK = re.compile(r"```verum\n(.*?)```", re.S)
CALL = re.compile(r"\.([a-z_][a-z0-9_]*)\s*\(")
# `[<(]` — a generic declaration has `<` where a plain one has `(`.
DECL = re.compile(r"\bfn\s+([a-z_][a-z0-9_]*)\s*[<(]")

# Measured on 2026-09-08. Absent ones were removed from the docs the same
# day; they stay here because a census with no known answer cannot be
# trusted, and this one caught two of my own mistakes before it ran.
CONTROL = {
    "add_modifier": False, "remove_modifier": False,   # fields, never functions
    "shuffle_vec": True, "choice": True,               # generic free functions
    "uniform_01": True, "bold": True,
}


def declared_names() -> set[str]:
    if not CORE.is_dir():
        return set()
    return set(DECL.findall(
        "\n".join(f.read_text(errors="ignore") for f in CORE.rglob("*.vr"))))


def self_test() -> int:
    bad = 0
    if DECL.findall("public fn shuffle_vec<T: Copy>(key: RandomKey)") != ["shuffle_vec"]:
        print("self-test: the declaration pattern misses a GENERIC fn"); bad += 1
    if DECL.findall("public fn bold(self) -> Style {") != ["bold"]:
        print("self-test: the declaration pattern misses a plain fn"); bad += 1
    if DECL.findall("    add_modifier: Modifier,") != []:
        print("self-test: a FIELD must not read as a declaration"); bad += 1
    if CALL.findall("Style.new().fg(c).add_modifier(M.BOLD)") != ["new", "fg", "add_modifier"]:
        print("self-test: the call pattern misses a chained call"); bad += 1
    print("self-test: OK" if not bad else f"self-test: {bad} FAILED")
    return bad


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not DOCS.is_dir():
        print(f"check-doc-methods-declared: {DOCS} not present — the docs live "
              "in a sibling checkout; UNMEASURED rather than passing on an "
              "absent input.")
        return 0

    declared = declared_names()
    if not declared:
        print("check-doc-methods-declared: core/ is not present — UNMEASURED.")
        return 0

    # THE CONTROL, asked as "is it declared", never as "is it missing":
    # a name that is no longer used is in neither set, and reading that
    # as declared is how the first version of this passed while wrong.
    wrong = [n for n, want in CONTROL.items() if (n in declared) != want]
    if wrong:
        print("control FAILED for: " + ", ".join(wrong))
        print("  A census whose known answers are wrong says nothing about "
              "its unknowns. Fix the patterns before reading any count.")
        return 1
    print(f"control: {len(CONTROL)}/{len(CONTROL)} known answers correct")

    pages: dict[str, set[str]] = {}
    for f in sorted(list(DOCS.rglob("*.md")) + list(DOCS.rglob("*.mdx"))):
        for m in BLOCK.finditer(f.read_text(errors="ignore")):
            for name in CALL.findall(m.group(1)):
                if name not in declared:
                    pages.setdefault(name, set()).add(
                        f.relative_to(DOCS).as_posix())

    total = len(pages)
    print(f"check-doc-methods-declared: {total} method name(s) called by a doc "
          f"example are declared nowhere in core/ (baseline {BASELINE})")
    for name, ps in sorted(pages.items(), key=lambda kv: -len(kv[1]))[:10]:
        print(f"  {len(ps):>2} page(s)  .{name:<22} e.g. {sorted(ps)[0]}")

    if total > BASELINE:
        print(f"  ABOVE BASELINE by {total - BASELINE}. A method that exists "
              "nowhere is one a reader cannot call.")
        return 1
    if total < BASELINE:
        print(f"  BELOW baseline by {BASELINE - total} — lower it.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
