#!/usr/bin/env python3
"""Gate: a `type X is …` in the docs must match the one in `core/`.

THE RUNG THIS FILLS. The ladder asked whether a TYPE NAME exists
(`check-doc-names-exist`) and whether a METHOD is declared
(`check-doc-methods-declared` / `check-doc-method-names`). Nothing asked
whether the SHAPE a page prints is the shape the library has, and a
reader copies the shape.

WHY IT EXISTS, measured 2026-09-09 on `website:docs/stdlib/term.md` and
its reference pages, where six declarations in a row were wrong:

    TerminalMode is Raw | Cooked | CBreak
        — a RECORD { original, fd, is_raw, is_alternate }
    TerminalSize is { cols, rows }
        — the field is `columns`, and there are two more
    CursorShape is Block | Line | Underline | BlinkingBlock | …
        — every shape is a BLINKING/STEADY pair; two of six names exist
    ClearMode is Entire | AfterCursor | BeforeCursor | Line | …
        — three of six wrong
    KeyEvent is { code, modifiers, kind, state }
        — three fields; there is no `state`
    Breakpoint is Mobile | Tablet | Desktop | Wide
        — a RECORD { min_width, name } with four CONSTANTS

Every one passed every existing gate: the type names are real and the
methods around them are declared.

WHAT IT COMPARES. For a name declared in BOTH the docs and `core/`:
the SET of variant names for a sum, the SET of field names for a record.
Not order, not payload types, not generics — those drift for good
reasons (a doc may abbreviate `Maybe<Int>` to `Int`), and a name that is
simply absent is the defect worth catching.

A doc-only type is the floor and is skipped: an example type is the
reader's, and `check-doc-names-exist` already counts those.
"""
from __future__ import annotations
import os
import re
import sys
from pathlib import Path

BASELINE = 99  # Lowered by FIXING a page, never by argument.
               #
               # 101 -> 99 on 2026-09-09: `stdlib/async.md`'s
               # `RetryConfig` had four fields and every one of the four
               # names was wrong (`max_attempts` for `max_retries`,
               # `initial_backoff_ms` for `initial_delay_ms`,
               # `max_backoff_ms` for `max_delay_ms`, plus an invented
               # `jitter`), with two constructors that do not exist; and
               # `RecoveryStrategy.None` is `NoRecovery`.
               #
               # The remaining 99 are a real backlog, not noise — spot-
               # checked against `core/`: `stdlib/architecture.md`
               # documents `Capability` with TEN variants and core/ has
               # nine completely different ones, not a single name in
               # common. This rung has never been checked, which is why
               # the number starts high.

REPO = Path(__file__).resolve().parents[2]
DOCS = Path(os.environ.get("VERUM_STDLIB_DOCS")
            or os.environ.get("VERUM_DOCS_DIR")
            or (REPO.parent / "website" / "docs"))
CORE = REPO / "core"

BLOCK = re.compile(r"```verum\n(.*?)```", re.S)
# `type Name is` … up to the terminating `;`
DECL = re.compile(
    r"(?:^|\n)\s*(?:public\s+|pub\s+)?type\s+([A-Z][A-Za-z0-9_]*)"
    r"(?:<[^>]*>)?\s+is\b(.*?);", re.S)
FIELD = re.compile(r"(?:^|,)\s*([a-z_][a-z0-9_]*)\s*:", re.M)
VARIANT = re.compile(r"(?:^|\|)\s*([A-Z][A-Za-z0-9_]*)")


def shape(body: str) -> tuple[str, frozenset[str]]:
    """('record'|'sum'|'other', names) for a declaration body."""
    stripped = re.sub(r"//[^\n]*", "", body)
    if "protocol" in stripped:
        return ("other", frozenset())
    if "{" in stripped and "|" not in stripped.split("{", 1)[0]:
        inner = stripped[stripped.index("{") + 1: stripped.rindex("}")] \
            if "}" in stripped else stripped
        return ("record", frozenset(FIELD.findall(inner)))
    if "|" in stripped:
        # a sum: names at the head of each arm, ignoring record payloads
        arms = re.sub(r"\{[^}]*\}", "", stripped)
        return ("sum", frozenset(VARIANT.findall(arms)))
    return ("other", frozenset())


def module_for(rel: str) -> str | None:
    """The `core/` subtree a docs page speaks for, or None.

    Only `stdlib/<module>…` pages name a module. Everything else — the
    cookbook, the tutorials, the language reference — declares example
    types whose names collide with real ones (`Node`, `Counter`,
    `Command`, `Store` all exist in `core/` as something unrelated), and
    comparing those by bare name is the wrong-table trap: the first
    version of this gate reported 184 mismatches, almost none real.
    """
    if not rel.startswith("stdlib/"):
        return None
    rest = rel[len("stdlib/"):]
    head = rest.split("/", 1)[0]
    if head.endswith(".md") or head.endswith(".mdx"):
        head = head.rsplit(".", 1)[0]
    # `database-postgres` documents `core/database`; take the head word.
    head = head.split("-", 1)[0]
    return head or None


def collect(text: str) -> dict[str, tuple[str, frozenset[str]]]:
    out: dict[str, tuple[str, frozenset[str]]] = {}
    for m in DECL.finditer(text):
        name, body = m.group(1), m.group(2)
        kind, names = shape(body)
        if kind == "other" or not names:
            continue
        out.setdefault(name, (kind, names))
    return out


def self_test() -> int:
    bad = 0
    k, n = shape(" { a: Int, b: Text }")
    if (k, n) != ("record", frozenset({"a", "b"})):
        bad += 1; print("self-test: a record is not read")
    k, n = shape("\n    | Foo\n    | Bar(Int)\n    | Baz { x: Int }")
    if (k, n) != ("sum", frozenset({"Foo", "Bar", "Baz"})):
        bad += 1; print(f"self-test: a sum is not read — got {n}")
    k, _ = shape(" protocol { fn x(); }")
    if k != "other":
        bad += 1; print("self-test: a protocol is not skipped")
    d = collect("```\ntype A is { p: Int };\n```")
    if "A" not in d:
        bad += 1; print("self-test: a declaration is not collected")
    k, n = shape(" { cols: Int, rows: Int }")
    if n != frozenset({"cols", "rows"}):
        bad += 1; print(f"self-test: fields on ONE line are missed — got {n}")
    if module_for("stdlib/term/reference/api-raw.md") != "term":
        bad += 1; print("self-test: a nested stdlib page loses its module")
    if module_for("stdlib/database-postgres.md") != "database":
        bad += 1; print("self-test: a hyphenated stdlib page loses its module")
    if module_for("cookbook/arenas.md") is not None:
        bad += 1; print("self-test: a cookbook page was given a module")
    # the measured case that motivated the gate
    _, cs = shape("\n | BlinkingBlock | SteadyBlock | BlinkingBar;")
    if cs != frozenset({"BlinkingBlock", "SteadyBlock", "BlinkingBar"}):
        bad += 1; print("self-test: CursorShape variants are not read")
    print("self-test: OK" if not bad else f"self-test: {bad} FAILED")
    return bad


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not DOCS.is_dir() or not CORE.is_dir():
        print("check-doc-type-shapes: docs or core/ not present — UNMEASURED.")
        return 0

    floor = 0
    for i, a in enumerate(sys.argv):
        if a == "--min-types" and i + 1 < len(sys.argv):
            floor = int(sys.argv[i + 1])

    per_module: dict[str, dict[str, tuple[str, frozenset[str]]]] = {}

    def shapes_of(module: str) -> dict[str, tuple[str, frozenset[str]]]:
        if module not in per_module:
            root = CORE / module
            per_module[module] = collect("\n".join(
                f.read_text(errors="ignore") for f in root.rglob("*.vr")
            )) if root.is_dir() else {}
        return per_module[module]

    mismatches: list[str] = []
    compared = 0
    for f in sorted(list(DOCS.rglob("*.md")) + list(DOCS.rglob("*.mdx"))):
        rel = f.relative_to(DOCS).as_posix()
        module = module_for(rel)
        if module is None:
            continue
        core_shapes = shapes_of(module)
        for m in BLOCK.finditer(f.read_text(errors="ignore")):
            for name, (kind, names) in collect(m.group(1)).items():
                if name not in core_shapes:
                    continue           # reader-owned example type: the floor
                ckind, cnames = core_shapes[name]
                compared += 1
                if kind != ckind:
                    mismatches.append(
                        f"{rel}: `{name}` is a {kind} here and a {ckind} in core/")
                    continue
                missing = sorted(names - cnames)
                if missing:
                    label = "variant" if kind == "sum" else "field"
                    mismatches.append(
                        f"{rel}: `{name}` names {label}(s) core/ does not have: "
                        + ", ".join(missing))

    if compared < floor:
        print(f"check-doc-type-shapes: only {compared} declaration(s) compared, "
              f"expected at least {floor} — the corpus or the pattern is gone.")
        return 1

    print(f"check-doc-type-shapes: {len(mismatches)} of {compared} documented "
          f"type declaration(s) disagree with core/ (baseline {BASELINE})")
    for x in mismatches[:20]:
        print(f"  {x}")
    if len(mismatches) > 20:
        print(f"  … and {len(mismatches) - 20} more")

    if len(mismatches) > BASELINE:
        print(f"  ABOVE BASELINE by {len(mismatches) - BASELINE}. A record "
              "literal copied from a page whose field names are wrong does "
              "not compile, and no other gate asks about a type's SHAPE.")
        return 1
    if len(mismatches) < BASELINE:
        print(f"  BELOW baseline by {BASELINE - len(mismatches)} — lower it.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
