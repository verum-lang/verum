#!/usr/bin/env python3
"""A documented method signature must be the one `core/` declares.

WHY THIS AND NOT ITS FIVE SIBLINGS
----------------------------------
`check_doc_method_names`, `check_doc_methods_declared`,
`check_doc_receiver_methods` and `check_doc_type_shapes` ask whether a
documented NAME exists. `check_doc_call_arity` asks whether a call could
run. None asks what the call gives BACK, and a wrong return type is the
error a reader cannot see until they write the `match`.

Measured 2026-09-10, first run over `docs/stdlib/**`:

    22 method signatures returned something core/ does not
    11 of them named a type declared NOWHERE — `DbError`,
       `Transaction`, `BuildError`, `SubmissionId`

`docs/stdlib/database.md` gave nine `Database` methods as
`Result<…, DbError>`; core says `SqliteApiDbError`, and `DbError` is not
a type. `docs/stdlib/sys.md` reproduced the `IOEngine` protocol with
three methods, one of which (`shutdown`) the protocol does not have,
and a `submit` returning `Result<SubmissionId, IoError>` against
`Result<Int{>= 0}, EngineIoError>`.

FOUR NARROWINGS, EACH FROM A FALSE ACCUSATION THIS PROBE MADE
-------------------------------------------------------------
1. KEY ON THE OWNER, not the bare method name. Matching `randomness`
   across all of core/ found a same-named method on an unrelated type
   and reported the page wrong. The owner is the nearest enclosing
   `implement X` / `type X is protocol`.
2. RESET THE OWNER AT COLUMN ZERO — and only there. Without a reset the
   owner outlives its block and the next method is judged against a
   type that does not have it. Resetting on any `}` after `strip()`
   also matches every METHOD BODY's closing brace, which took the
   comparable population from 264 to 108: a denominator halved by a
   fix, which is the shape to watch for.
3. `Self` AND `Self.X` ARE NOT COMPARABLE. `Self` is the impl's own
   type; `Self.X` is an ASSOCIATED type, and a page substituting a
   concrete one (`Self.File` -> `SqliteFile`) is documenting a real
   implementation, not contradicting the protocol.
4. AN UNBALANCED `)` IN THE RETURN means the `->` was found inside the
   parameter list — `fn spawn<T>(self, f: fn() -> T)` matches the INNER
   arrow. Counted, never reported.

Only a method core declares with EXACTLY ONE return type is compared:
two declarations mean two types of the same name, and this reader
cannot say which the page meant.
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

OWNER = re.compile(
    r"^\s*(?:public\s+)?(?:type\s+([A-Z][A-Za-z0-9_]*)[^\n]*\bis\s+protocol"
    r"|implement(?:\s*<[^>]*>)?\s+(?:([A-Z][A-Za-z0-9_]*)\s+for\s+([A-Z][A-Za-z0-9_]*)"
    r"|([A-Z][A-Za-z0-9_]*)))"
)
METH = re.compile(
    r"^\s*(?:public\s+)?fn\s+([a-z_][a-z0-9_]*)\s*(?:<[^(>]*>)?\s*\(\s*"
    r"(&(?:mut\s+|checked\s+|unsafe\s+)?self|mut\s+self|self)\s*"
    r"(?:,([^)]{0,200}))?\)"
    r"\s*(?:->\s*([^{;\n]+))?"
)


def receiver(s: str) -> str:
    return re.sub(r"\s+", " ", s.strip())


def param_types(s: str | None) -> str:
    """TYPES only, positionally.

    A parameter NAME is not part of a positional call, and comparing
    names made 23 of 27 findings cosmetic — `min` against `min_level`,
    `f` against `frame`, `cb` against `callback`. Splitting on commas
    at depth zero keeps `Result<A, B>` and `(A, B)` in one piece.
    """
    if not s:
        return ""
    s = s.split("//")[0].strip().rstrip(",")
    pieces, depth, cur = [], 0, ""
    for ch in s:
        if ch in "<([":
            depth += 1
        elif ch in ">)]":
            depth -= 1
        if ch == "," and depth == 0:
            pieces.append(cur)
            cur = ""
        else:
            cur += ch
    pieces.append(cur)
    out = []
    for piece in pieces:
        ty = piece.split(":", 1)[1] if ":" in piece else piece
        ty = re.sub(r"\s+", "", ty)
        if "Self." in ty:
            return "ASSOC"
        out.append(ty)
    return ",".join(out)
# Findings the page is right about. Empty, and that is the finished
# state: the twenty-two it was built from are corrected, not tolerated.
KNOWN: dict[tuple[str, str], str] = {}
FLOOR = 150


def norm(t: str) -> str:
    t = t.split("//")[0].strip().rstrip(";,")
    if "Self." in t:
        return "ASSOC"
    t = re.sub(r"\s+", "", t)
    if t.count(")") > t.count("("):
        return "UNPARSED"
    return t


def harvest(text: str) -> dict[tuple[str, str], set[str]]:
    out: dict[tuple[str, str], set[str]] = {}
    owner = None
    for line in text.split("\n"):
        if line[:1] in ("}", ")"):
            owner = None
        o = OWNER.match(line)
        if o:
            owner = o.group(1) or o.group(3) or o.group(4) or o.group(2)
            continue
        m = METH.match(line)
        if m and owner:
            out.setdefault((owner, m.group(1)), set()).add(
                (receiver(m.group(2)), param_types(m.group(3)),
                 norm(m.group(4)) if m.group(4) else "NORETURN")
            )
    return out


SKIP = {"Self", "ASSOC", "UNPARSED"}


def self_test() -> int:
    bad = 0

    owner_survives = harvest(
        "implement Registry {\n"
        "    public fn counter(&self, c: C) -> CounterHandle {\n"
        "        0\n"
        "    }\n"
        "    public fn gauge(&self, c: C) -> GaugeHandle {\n"
        "        0\n"
        "    }\n"
        "}\n"
    )
    if set(owner_survives) != {("Registry", "counter"), ("Registry", "gauge")}:
        print(f"self-test: a method body's closing brace reset the owner: "
              f"{sorted(owner_survives)}", file=sys.stderr)
        bad += 1

    ends = harvest(
        "implement A {\n    fn f(&self) -> X {\n        0\n    }\n}\n"
        "implement B {\n    fn g(&self) -> Y {\n        0\n    }\n}\n"
    )
    if set(ends) != {("A", "f"), ("B", "g")}:
        print(f"self-test: the owner outlived its block: {sorted(ends)}",
              file=sys.stderr)
        bad += 1

    for label, src, want in (
        ("associated type", "Self.File", "ASSOC"),
        ("own type", "Self", "Self"),
        ("arrow inside the parameter list", "T)", "UNPARSED"),
        ("ordinary", "Result<Int, E>", "Result<Int,E>"),
        ("trailing comment", "Maybe<Backtrace>;   // default impl",
         "Maybe<Backtrace>"),
    ):
        got = norm(src)
        if got != want:
            print(f"self-test: {label}: {src!r} -> {got!r}, wanted {want!r}",
                  file=sys.stderr)
            bad += 1

    # THE THREE ANCHORS, as the pairs they were when found. A detector
    # that stops seeing these has stopped measuring.
    for owner, meth, doc_t, core_t in (
        ("ErrorProtocol", "backtrace", "Maybe<&Backtrace>", "Maybe<Backtrace>"),
        ("VfsProtocol", "current_time", "Result<Timestamp,VfsError>", "Timestamp"),
        ("Database", "execute", "Result<(),DbError>", "Result<(),SqliteApiDbError>"),
    ):
        if norm(doc_t) == norm(core_t) or norm(doc_t) in SKIP:
            print(f"self-test: anchor {owner}.{meth} no longer differs",
                  file=sys.stderr)
            bad += 1

    if bad:
        print(f"self-test: {bad} FAILED", file=sys.stderr)
        return 1
    for label, src, want in (
        ("names ignored, types kept", "min: ContextLogLevel", "ContextLogLevel"),
        ("depth-zero split", "a: Result<A, B>, b: Int", "Result<A,B>,Int"),
        ("associated parameter", "msg: Self.Msg", "ASSOC"),
        ("no parameters", None, ""),
    ):
        got = param_types(src)
        if got != want:
            print(f"self-test: param {label}: {src!r} -> {got!r}, wanted {want!r}",
                  file=sys.stderr)
            bad += 1
    if receiver("&mut  self") != "&mut self":
        print("self-test: receiver normalisation", file=sys.stderr)
        bad += 1
    if bad:
        print(f"self-test: {bad} FAILED", file=sys.stderr)
        return 1
    print(f"[ok] self-test: 2 owner-tracking cases, 5 return normalisations, "
          f"4 parameter cases, 1 receiver case, 3 anchors, "
          f"{len(KNOWN)} on the roster")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not DOCS.is_dir():
        print(
            f"check-doc-method-signatures: no website at {DOCS} — REFUSING "
            f"to report OK. A gate whose INPUT is missing is a failed "
            f"checkout, not 'nothing to do'; set VERUM_DOCS_DIR.",
            file=sys.stderr,
        )
        return 2

    core: dict[tuple[str, str], set[str]] = collections.defaultdict(set)
    for f in CORE.rglob("*.vr"):
        for k, v in harvest(f.read_text(errors="replace")).items():
            core[k] |= v

    doc: dict[tuple[str, str], set[tuple[str, str]]] = collections.defaultdict(set)
    pages = sorted((DOCS / "stdlib").rglob("*.md")) if (DOCS / "stdlib").is_dir() else []
    for p in pages:
        for k, v in harvest(p.read_text(errors="replace")).items():
            for r in v:
                doc[k].add((r, str(p.relative_to(DOCS))))

    off: dict[str, list[str]] = {"receiver": [], "parameters": [], "return": []}
    for k in sorted(set(doc) & set(core)):
        if len(core[k]) != 1:
            continue
        cr, cp, ct = next(iter(core[k]))
        for (dr, dp, dt), page in sorted(doc[k]):
            if KNOWN.get(k):
                continue
            where = f"{k[0]}.{k[1]}"
            if dr != cr:
                off["receiver"].append(
                    f"{where}  page {dr!r} vs core {cr!r}   [{page}]")
            elif dp != cp and "ASSOC" not in (dp, cp):
                off["parameters"].append(
                    f"{where}  page ({dp}) vs core ({cp})   [{page}]")
            elif (dt != ct and dt not in SKIP and ct not in SKIP
                  and "NORETURN" not in (dt, ct)):
                off["return"].append(
                    f"{where}  page {dt!r} vs core {ct!r}   [{page}]")

    total = sum(1 for k in set(doc) & set(core) if len(core[k]) == 1)
    found = sum(len(v) for v in off.values())
    print(f"check-doc-method-signatures: {len(doc)} documented (owner, "
          f"method) pair(s) across {len(pages)} page(s), {total} comparable "
          f"against core — {len(off['receiver'])} receiver, "
          f"{len(off['parameters'])} parameter and {len(off['return'])} return "
          f"disagreement(s) ({len(KNOWN)} on the roster)")

    if total < FLOOR:
        print(f"\nonly {total} pair(s) were comparable, below the floor of "
              f"{FLOOR}. The owner tracking or the signature shape changed, "
              f"so this gate measured almost nothing — refusing rather than "
              f"passing. Measured 2026-09-10 it was 248, and a narrowing "
              f"once took it to 108 while still reporting a clean run.",
              file=sys.stderr)
        return 2

    for axis in ("receiver", "parameters", "return"):
        for s in off[axis]:
            print(f"    + [{axis}] {s}")
    if found:
        print("  A reader copies the signature and the compiler refuses it. "
              "Correct the page, or add the pair to KNOWN with why core is "
              "the one that is wrong.")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
