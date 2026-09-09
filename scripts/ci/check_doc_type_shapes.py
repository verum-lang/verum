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

BASELINE = 35  # Lowered by FIXING a page, never by argument.
               #
               # 101 -> 99 on 2026-09-09: `stdlib/async.md`'s
               # `RetryConfig` had four fields and every one of the four
               # names was wrong (`max_attempts` for `max_retries`,
               # `initial_backoff_ms` for `initial_delay_ms`,
               # `max_backoff_ms` for `max_delay_ms`, plus an invented
               # `jitter`), with two constructors that do not exist; and
               # `RecoveryStrategy.None` is `NoRecovery`.
               #
               # 99 -> 92 was NOT a fix: the collector delimited a
               # declaration at the first `;`, and
               # `core/compress/mod.vr`'s `Algorithm` carries the doc
               # line "(headerless; used inside gzip/zlib/pkzip)". Four
               # variants of a correct page were reported absent from a
               # type that has them. Comments are stripped BEFORE the
               # delimiter is looked for now, and the self-test carries
               # that case. Caught by spot-checking the third page
               # rather than the first two.
               #
               # 92 -> 86 by fixing `stdlib/architecture.md`: its
               # `Capability`, `Foundation`, `MsfsStratum`, `Lifecycle`,
               # `ArchMetric` and `CounterfactualReport` were a
               # different vocabulary from `core/architecture/` — of the
               # ten capability variants it listed, not one exists.
               #
               # 86 -> 82 by fixing `stdlib/cog.md`: the manifest's
               # identity table is `[cog]` not `[package]`,
               # dependencies are a MAP keyed by name, the archive keeps
               # its index and payloads as parallel lists, and
               # `ResolveError` has two arms where the page listed three
               # different ones.
               #
               # 82 -> 80: `stdlib/configuration.md`'s `ConfigValue`
               # (eight bare variants for fourteen `Config`-prefixed
               # ones, with TOML's four date/time shapes collapsed into
               # one) and `stdlib/compress.md`'s `IoError` for
               # `IoFailure`.
               #
               # 80 -> 78, and a gate correction inside it: the
               # sum-vs-record test asked whether a `|` appeared before
               # the first `{`, so `core/context/error.vr`'s
               # `ContextError` — whose FIRST arm omits the leading pipe
               # — read as a record and a correct page was reported as
               # disagreeing about the KIND of the type. The test now
               # looks for a `|` outside any brace group.
               #
               # The fixes were `stdlib/action.md`: `Primitive` (eight
               # `Epsilon`-prefixed variants, not seven bare ones),
               # `Enactment`, `Articulation`, `EffectKind`, `LazyDesign`
               # and a `GaugeCanonical` type that does not exist.
               #
               # 78 -> 76, and a second gate correction: the
               # collector kept only the FIRST declaration per name, so
               # `stdlib/database.md`'s loom section was compared
               # against the cross-adapter `DbError` and twenty-one
               # variants were reported absent. It keeps every
               # declaration now and a page matching ANY of them passes.
               # The page was still wrong, differently: loom's type is
               # `SqliteApiDbError`, and the comment in
               # `core/database/sqlite/error.vr` claiming the two SHARE
               # the bare name had gone stale at the rename.
               #
               # The remaining 76 are a real backlog, not noise — spot-
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
DECL_HEAD = re.compile(
    r"(?:^|\n)[ \t]*(?:public\s+|pub\s+)?type\s+([A-Z][A-Za-z0-9_]*)"
    r"(?:<[^>]*>)?\s+is\b")


def declarations(text: str):
    """(name, body) for every `type X is …` in `text`.

    The body ends at the first `;` OUTSIDE a brace group, or — when the
    declaration is a record whose closing `}` carries no `;` — at that
    `}`. Delimiting on the first `;` alone was wrong for 668 of core/'s
    records, which close with a bare `}`:

        public type FsEvent is {
            pub path: Text,
            pub kind: FsEventKind,
        }

    `grammar/verum.ebnf`'s `type_definition_body` mandates the `;` and
    the parser accepts its absence; whichever of those is the defect,
    the census has to read the tree as it IS. Running past that `}`
    swallowed the NEXT declaration's body, so `FsEvent` was measured
    with `FsWatcher`'s field and a page naming `path`/`kind` was
    reported as naming fields core/ does not have.
    """
    for m in DECL_HEAD.finditer(text):
        i, depth, saw_brace = m.end(), 0, False
        while i < len(text):
            c = text[i]
            if c == "{":
                depth += 1; saw_brace = True
            elif c == "}":
                depth -= 1
                if depth == 0:
                    j = i + 1
                    while j < len(text) and text[j] in " \t\r\n":
                        j += 1
                    # `};` ends here; so does a bare `}` that is not
                    # followed by more of the same declaration.
                    if j >= len(text) or text[j] != "|":
                        yield m.group(1), text[m.end():i + 1]
                        break
            elif c == ";" and depth == 0:
                yield m.group(1), text[m.end():i]
                break
            i += 1
        else:
            if saw_brace or depth:
                yield m.group(1), text[m.end():]
# A field may carry a visibility modifier — 35 in core/ do, e.g.
# `core/sys/fs_watch.vr`'s `FsEvent { pub path: Text, pub kind: … }`.
# Without skipping it the collector saw NO fields there and the gate
# reported a page listing exactly `path` and `kind` as wrong.
FIELD = re.compile(
    r"(?:^|,)\s*(?:public\s+|pub\s+)?([a-z_][a-z0-9_]*)\s*:", re.M)
# A variant may carry attributes: `variant = { attribute } , identifier
# , [ variant_data ] , …` (grammar/verum.ebnf:815, whose own example
# is `@default Ok | @deprecated Legacy | Error(Text)`). Skipping them
# was not optional — `core/runtime/supervisor.vr` writes
# `| @default EscalateToParent`, the collector saw no variant there,
# and the gate then reported a page naming `EscalateToParent` as
# naming a variant core/ does not have. One such variant exists in
# core/ today, and the gate was wrong about exactly it.
VARIANT = re.compile(
    r"(?:^|\|)\s*(?:@[a-z_][A-Za-z0-9_]*(?:\([^)]*\))?\s+)*"
    r"([A-Z][A-Za-z0-9_]*)")


def shape(body: str) -> tuple[str, frozenset[str]]:
    """('record'|'sum'|'other', names) for a declaration body."""
    stripped = re.sub(r"//[^\n]*", "", body)
    if "protocol" in stripped:
        return ("other", frozenset())

    # Sum-or-record is decided by a `|` OUTSIDE any brace group, not by
    # one before the first `{`. `core/context/error.vr`'s `ContextError`
    # writes its FIRST arm without a leading pipe —
    #
    #     public type ContextError is
    #         NotFound { context_name: Text }
    #         | NotProvided { … }
    #
    # so "no `|` before the first `{`" read a five-variant sum as a
    # record, and the gate reported a correct page as disagreeing about
    # the KIND of the type.
    outside = re.sub(r"\{[^{}]*\}", "", stripped)
    if "|" in outside:
        return ("sum", frozenset(VARIANT.findall(outside)))
    if "{" in stripped:
        inner = stripped[stripped.index("{") + 1: stripped.rindex("}")] \
            if "}" in stripped else stripped
        return ("record", frozenset(FIELD.findall(inner)))
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
    # Comments are stripped BEFORE the declaration is delimited, not
    # after. `core/compress/mod.vr`'s `Algorithm` carries the doc line
    # "RFC 1951 raw deflate (headerless; used inside gzip/zlib/pkzip)",
    # and that semicolon ended the declaration four variants early —
    # the gate then reported Deflate, Zlib, Brotli and Zstd as absent
    # from a type that has all four. A census whose delimiter can appear
    # inside a comment measures the comment.
    text = re.sub(r"//[^\n]*", "", text)
    out: dict[str, tuple[str, frozenset[str]]] = {}
    for name, body in declarations(text):
        kind, names = shape(body)
        if kind == "other" or not names:
            continue
        out.setdefault(name, (kind, names))
    return out


def collect_all(text: str) -> dict[str, list[tuple[str, frozenset[str]]]]:
    """Every declaration per name, not the first.

    One module subtree can declare a name twice on purpose.
    `core/database` has TWO `DbError`s and says so in a comment: "Loom's
    own SQLite-specific DbError lives in a sibling namespace and shares
    the bare name with our common surface." A first-wins collector
    compared the loom section of the page against the common type and
    reported twenty-one variants as absent from a page that is right.
    """
    text = re.sub(r"//[^\n]*", "", text)
    out: dict[str, list[tuple[str, frozenset[str]]]] = {}
    for name, body in declarations(text):
        kind, names = shape(body)
        if kind == "other" or not names:
            continue
        out.setdefault(name, []).append((kind, names))
    return out


def self_test() -> int:
    bad = 0
    k, n = shape(" { a: Int, b: Text }")
    if (k, n) != ("record", frozenset({"a", "b"})):
        bad += 1; print("self-test: a record is not read")
    k, n = shape("\n    | Foo\n    | Bar(Int)\n    | Baz { x: Int }")
    if (k, n) != ("sum", frozenset({"Foo", "Bar", "Baz"})):
        bad += 1; print(f"self-test: a sum is not read — got {n}")
    # The measured false positive: a first arm with NO leading pipe.
    k, n = shape("\n    NotFound { a: Text }\n    | NotProvided { b: Text }")
    if k != "sum" or n != frozenset({"NotFound", "NotProvided"}):
        bad += 1
        print(f"self-test: a sum whose first arm omits `|` reads as {k} {n}")
    k, _ = shape(" protocol { fn x(); }")
    if k != "other":
        bad += 1; print("self-test: a protocol is not skipped")
    d = collect("```\ntype A is { p: Int };\n```")
    if "A" not in d:
        bad += 1; print("self-test: a declaration is not collected")
    # The measured false positive: a `;` inside a doc comment must not
    # end the declaration.
    d = collect("type A is\n  /// a; b\n  | X\n  | Y;\n")
    if d.get("A", (None, frozenset()))[1] != frozenset({"X", "Y"}):
        bad += 1
        print(f"self-test: a semicolon in a COMMENT ends the declaration — "
              f"got {d.get('A')}")
    k, n = shape(" { cols: Int, rows: Int }")
    if n != frozenset({"cols", "rows"}):
        bad += 1; print(f"self-test: fields on ONE line are missed — got {n}")
    if module_for("stdlib/term/reference/api-raw.md") != "term":
        bad += 1; print("self-test: a nested stdlib page loses its module")
    if module_for("stdlib/database-postgres.md") != "database":
        bad += 1; print("self-test: a hyphenated stdlib page loses its module")
    if module_for("cookbook/arenas.md") is not None:
        bad += 1; print("self-test: a cookbook page was given a module")
    # An ATTRIBUTED variant is still a variant.
    _, n = shape("\n | RestartSubtree\n | @default EscalateToParent\n | Terminate")
    if n != frozenset({"RestartSubtree", "EscalateToParent", "Terminate"}):
        bad += 1
        print(f"self-test: an @attributed variant is dropped — got {n}")
    _, n = shape("\n | @serde(rename = \"a\") Alpha\n | Beta")
    if n != frozenset({"Alpha", "Beta"}):
        bad += 1
        print(f"self-test: an attribute WITH ARGS eats its variant — got {n}")
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

    per_module: dict[str, dict[str, list[tuple[str, frozenset[str]]]]] = {}

    def shapes_of(module: str) -> dict[str, list[tuple[str, frozenset[str]]]]:
        if module not in per_module:
            root = CORE / module
            per_module[module] = collect_all("\n".join(
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
                candidates = core_shapes[name]
                compared += 1
                # A page matching ANY declaration of that name is right;
                # a module may hold two types under one name on purpose.
                if any(kind == ck and not (names - cn) for ck, cn in candidates):
                    continue
                same_kind = [cn for ck, cn in candidates if ck == kind]
                if not same_kind:
                    kinds = "/".join(sorted({ck for ck, _ in candidates}))
                    mismatches.append(
                        f"{rel}: `{name}` is a {kind} here and a {kinds} in core/")
                    continue
                missing = sorted(min((names - cn for cn in same_kind), key=len))
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
