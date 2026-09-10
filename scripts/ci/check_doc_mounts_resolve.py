#!/usr/bin/env python3
"""A `mount core.a.b.{Name}` or `core.a.b.fn(...)` must name something real.

WHY THIS AND NOT `check_doc_names_exist`
----------------------------------------
That gate asks "does this name occur anywhere in core/". This one asks
"does THIS module export THIS name", and the two questions have very
different answers. A reader does not grep the tree — they paste the
mount line, and the compiler asks the second question.

The distinction is not theoretical here. verum-35 measured the same
split on the compiler side (T1380): a bake log names 207 cross-module
identifiers that do not resolve, 101 of them dotted module paths — the
exact domain of a gate that reads zero, because that gate asks whether
the LEAF is declared somewhere.

Measured 2026-09-10, first full run over the site:

    637 distinct (module, name) pairs mounted in the docs
    617 the module exports it
      5 reached through a glob re-export (not decided here)
      7 THE MODULE DOES NOT EXIST
      8 the module exists and does not have the name

Ten of those fourteen were a WRONG MODULE rather than a missing name —
`Path` lives in `core.io.path`, `Rng` in `core.random.deterministic`,
`SentPacketInfo` in `...recovery.pn_space` — so the fix was to point
the mount at the module that declares the thing. That is the shape this
gate exists to find: the name is real, the path a reader copies is not,
and a gate asking "does this leaf exist somewhere" says yes to all ten.

FIVE CORRECTIONS BEFORE THE FIRST NUMBER WAS WORTH PRINTING, each found
by checking a suspected finding against the tree rather than believing
the count. They are the self-test:

  1. `public mount .map.Map;` — a brace-less single re-export, the
     dominant form in every `mod.vr`. Missing it accused 25 correct
     pages.
  2. a sum type's VARIANTS are exported names. `HalfEven` is reachable
     through `mount core.text.numeric.decimal.{HalfEven}` and is
     declared nowhere a `fn|type|const` scan looks.
  3. `public type affine LockHandle is {…}` — a qualifier may sit
     between the keyword and the name, and the first version read
     `affine` as the declared name.
  4. multi-line mount lists must stay IN the denominator. Forbidding
     newlines inside the braces removed the prose artefacts and took
     the population from 681 to 353 with them — a denominator traded
     for a cleaner-looking numerator. The runaway is bounded by length
     instead, and every item must be a whole identifier.
  5. a mount annotated with its own expected diagnostic is a
     DEMONSTRATION, not a claim: `mount core.collections.list.{NoSuchSymbol};
     // error<E401>: cannot find` is a page teaching what failure looks
     like. Flagging it would ask an author to break a correct example.

WHAT IT DOES NOT DECIDE
    A module that re-exports by glob (`public mount .x.*;`) is not
    resolved further — the name may or may not be there, and following
    the chain needs the resolver, not a regex. Those are counted and
    printed, never failed.
"""

from __future__ import annotations

import os
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
CORE = REPO / "core"

# The site is a SEPARATE checkout beside this one; a tracked file may not
# name the working copy's private path.  Gate: `make check-internal-refs`.
_DOCS_ROOT = os.environ.get("VERUM_DOCS_DIR")
DOCS = pathlib.Path(_DOCS_ROOT) if _DOCS_ROOT else REPO.parent / "website" / "docs"

MOUNT_LIST = re.compile(
    r"mount\s+((?:core|cog)(?:\.[a-z_][a-z0-9_]*)+)\s*\.\s*\{([^}]{0,400})\}"
)
DECL = re.compile(
    r"^\s*(?:public\s+|pub\s+)?(?:unsafe\s+)?(?:fn|type|const|context)\s+"
    r"(?:(?:affine|linear|unique|shared|mut)\s+)*([A-Za-z_][A-Za-z0-9_]*)",
    re.M,
)
VARIANTS = re.compile(r"type\s+[A-Za-z_][A-Za-z0-9_<>, ]*\s+is\s+([^;{]+(?:\{[^}]*\}[^;]*)*);", re.S)
VNAME = re.compile(r"\b([A-Z][A-Za-z0-9_]*)")
REEXPORT_LIST = re.compile(r"public\s+mount\s+\.?[a-z0-9_.]*\{([^}]*)\}")
REEXPORT_ONE = re.compile(
    r"public\s+mount\s+\.?[a-z0-9_.]*\.([A-Za-z_][A-Za-z0-9_]*)\s*"
    r"(?:as\s+([A-Za-z_][A-Za-z0-9_]*)\s*)?;"
)
REEXPORT_MOD = re.compile(r"public\s+module\s+([a-z_][a-z0-9_]*)\s*;")
REEXPORT_GLOB = re.compile(r"public\s+mount\s+\.?([a-z0-9_.]+)\s*\.\s*\*\s*;")
DEMONSTRATION = re.compile(r"//.*error<E\d+>")
# A dotted CALL is the same claim in the other syntax: `core.a.b.fn(...)`
# says module `core.a.b` has a function `fn`.  Two segments minimum after
# the root, so a method call on a value (`x.stats().pretty()`) cannot
# masquerade as one.  Small population — 12 across the whole site — and
# both of its first findings were real: `stats_prometheus.listener_exporter`
# and `cli.plugin.discover`, neither declared anywhere in core/.
CALL_PATH = re.compile(
    r"\b((?:core|cog)(?:\.[a-z_][a-z0-9_]*){2,})\.([a-z_][a-z0-9_]*)\s*\("
)
IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")

# Known, each with why it is still here.  Identity, not a count: a swap
# holds the number and moves these rows.
# Ten of the first fourteen were a WRONG MODULE, not a missing name, and
# were fixed by pointing the mount at the module that declares the thing:
#
#   core.io.fs.Path                 -> core.io.path
#   core.random.Rng                 -> core.random.deterministic
#   core.net.http2.StreamEvent      -> core.net.http2.stream (the page's
#                                      own module table already said so)
#   ...loss_detection.SentPacketInfo -> ...pn_space (loss_detection mounts
#                                      it PRIVATELY, which is not a re-export)
#   core.net.h3.qpack.HeaderField   -> the name is `QpackHeaderField`
#   core.prelude.{Bool,Int,Maybe,List,Text} — built-in; no mount needed and
#                                      no `core/prelude` to mount from
#
# The four below are different in kind: the name is declared NOWHERE in
# core/. Each is marked at the line a reader copies — a note further down
# the page does not stop anybody pasting the block — but a marker makes
# the page honest, not correct, so they stay here until the declarations
# exist.
KNOWN: dict[str, list[str]] = {
    # `core/security/x509/` has neither a `parse` nor a `sign` submodule.
    "core.security.x509.parse": ["parse_cert_chain_pem"],
    "core.security.x509.sign": ["FileSigner"],
    # `core/signal/mod.vr` shows `ctrl_c()` in its own doc comment and
    # declares no such function.
    "core.signal": ["ctrl_c"],
    # The tour's framework-axiom example. `Site` is real; the axiom is not.
    "core.math.frameworks.lurie_htt": ["sheafification_is_infinity_topos"],
}


def module_files(path: str) -> list[pathlib.Path]:
    parts = path.split(".")[1:]
    p = CORE.joinpath(*parts)
    return [q for q in (p.with_suffix(".vr"), p / "mod.vr") if q.is_file()]


def exported(path: str) -> tuple[set[str], bool] | None:
    """Names the module offers, and whether it also re-exports by glob."""
    files = module_files(path)
    if not files:
        return None
    names: set[str] = set()
    glob = False
    for f in files:
        text = f.read_text(errors="replace")
        names |= set(DECL.findall(text))
        for m in VARIANTS.finditer(text):
            body = m.group(1)
            if "|" in body or "(" in body:
                names |= set(VNAME.findall(body))
        for m in REEXPORT_LIST.finditer(text):
            for item in m.group(1).split(","):
                item = item.strip()
                if item:
                    names.add(item.split(" as ")[-1].strip())
        for m in REEXPORT_ONE.finditer(text):
            names.add(m.group(2) or m.group(1))
        names |= set(REEXPORT_MOD.findall(text))
        if REEXPORT_GLOB.search(text):
            glob = True
    return names, glob


def scan(text: str) -> list[tuple[str, str]]:
    """(module, name) for every mount that is a CLAIM, not a demonstration."""
    out = []
    for m in MOUNT_LIST.finditer(text):
        line_end = text.find("\n", m.end())
        line = text[m.start(): line_end if line_end != -1 else len(text)]
        if DEMONSTRATION.search(line):
            continue
        for item in m.group(2).split(","):
            item = item.strip().split(" as ")[0].strip()
            if IDENT.fullmatch(item):
                out.append((m.group(1), item))
    for m in CALL_PATH.finditer(text):
        line_end = text.find("\n", m.end())
        line = text[m.start(): line_end if line_end != -1 else len(text)]
        if DEMONSTRATION.search(line):
            continue
        out.append((m.group(1), m.group(2)))
    return out


def self_test() -> int:
    bad = 0
    cases = [
        ("brace-less re-export",
         "public mount .map.Map;\npublic mount .map.MapIter as Iter;\n",
         {"Map", "Iter"}),
        ("sum-type variants",
         "public type Rounding is HalfEven | HalfUp | Down;\n",
         {"Rounding", "HalfEven", "HalfUp", "Down"}),
        ("qualifier before the name",
         "public type affine LockHandle is { fd: Int };\n",
         {"LockHandle"}),
        ("plain declarations",
         "public fn open(p: Text) -> Int { 0 }\nconst LIMIT = 4;\n",
         {"open", "LIMIT"}),
    ]
    for label, src, want in cases:
        got = set(DECL.findall(src))
        for m in VARIANTS.finditer(src):
            if "|" in m.group(1) or "(" in m.group(1):
                got |= set(VNAME.findall(m.group(1)))
        for m in REEXPORT_ONE.finditer(src):
            got.add(m.group(2) or m.group(1))
        if not want <= got:
            print(f"self-test: {label}: missing {want - got}", file=sys.stderr)
            bad += 1

    multi = scan("mount core.net.http2.{\n    FrameHeader,\n    Settings as S,\n};\n")
    if multi != [("core.net.http2", "FrameHeader"), ("core.net.http2", "Settings")]:
        print(f"self-test: multi-line mount not read: {multi}", file=sys.stderr)
        bad += 1

    demo = scan("mount core.collections.list.{NoSuchSymbol};  // error<E401>: cannot find\n")
    if demo:
        print(f"self-test: an annotated failure example was read as a claim: {demo}",
              file=sys.stderr)
        bad += 1

    prose = scan("mount core.intrinsics.runtime.time.{ works on both tiers\n"
                 "and returns realtime_secs }\n")
    if prose:
        print(f"self-test: prose was read as a mount list: {prose}", file=sys.stderr)
        bad += 1

    call = scan("Rendering goes through `core.net.quic.stats.render_endpoint(s)`.\n")
    if call != [("core.net.quic.stats", "render_endpoint")]:
        print(f"self-test: a dotted call path was not read: {call}", file=sys.stderr)
        bad += 1

    # A method chain on a VALUE is not a module path, and one segment
    # after the root is a module, not a function in one.
    for src in ("accepted.stats().snapshot().pretty();\n",
                "core.time.now();\n"):
        if scan(src):
            print(f"self-test: read a module call out of {src!r}", file=sys.stderr)
            bad += 1

    # `exported` carries all five extractor corrections and was reached
    # by NO self-test case until 2026-09-10 — verified by hand at build
    # time and never again. A function whose narrowings are exercised
    # only once is a comment with a return value.
    import tempfile
    with tempfile.TemporaryDirectory() as tmp:
        mod = pathlib.Path(tmp) / "probe"
        mod.mkdir()
        (mod / "mod.vr").write_text(
            "public mount .map.Map;\n"
            "public mount .map.Iter as Cursor;\n"
            "public mount .other.{Alpha, Beta as Gamma};\n"
            "public module submod;\n"
            "public type Rounding is HalfEven | HalfUp;\n"
            "public type affine LockHandle is { fd: Int };\n"
            "public fn open(p: Text) -> Int { 0 }\n"
        )
        global CORE
        saved, CORE = CORE, pathlib.Path(tmp)
        try:
            got = exported("core.probe")
        finally:
            CORE = saved
    if got is None:
        print("self-test: exported() did not find the probe module",
              file=sys.stderr)
        bad += 1
    else:
        names, glob = got
        want = {"Map", "Cursor", "Alpha", "Gamma", "submod", "Rounding",
                "HalfEven", "HalfUp", "LockHandle", "open"}
        if not want <= names:
            print(f"self-test: exported() lost {sorted(want - names)}",
                  file=sys.stderr)
            bad += 1
        if glob:
            print("self-test: a glob re-export was seen where there is none",
                  file=sys.stderr)
            bad += 1

    if bad:
        print(f"self-test: {bad} FAILED", file=sys.stderr)
        return 1
    print(f"[ok] self-test: {len(cases)} declaration form(s), "
          f"1 multi-line list, 1 dotted call, 4 non-claims rejected, "
          f"10 exported names over 6 re-export forms")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not DOCS.is_dir():
        print(
            f"check-doc-mounts-resolve: no website at {DOCS} — REFUSING to report OK. "
            f"A gate whose INPUT is missing is a failed checkout, not "
            f"'nothing to do'; set VERUM_DOCS_DIR. Measured 2026-09-10: "
            f"this gate sat in the source-only aggregate, whose CI job has "
            f"no website, and reported OK on every run.",
            file=sys.stderr,
        )
        return 2

    want = {(m, n) for m, ns in KNOWN.items() for n in ns}
    have: dict[tuple[str, str], str] = {}
    seen: set[tuple[str, str]] = set()
    ok = globbed = 0
    for page in sorted(DOCS.rglob("*.md")):
        for mod, name in scan(page.read_text(errors="replace")):
            if (mod, name) in seen:
                continue
            seen.add((mod, name))
            e = exported(mod)
            if e is None:
                have[(mod, name)] = f"{page.name}: no such module"
                continue
            names, glob = e
            if name in names:
                ok += 1
            elif glob:
                globbed += 1
            else:
                have[(mod, name)] = f"{page.name}: module has no `{name}`"

    print(f"check-doc-mounts-resolve: {len(seen)} distinct (module, name) "
          f"pair(s) mounted or called across the docs — {ok} exported, "
          f"{globbed} behind "
          f"a glob re-export (undecided), {len(have)} unresolved "
          f"({len(want)} on the roster)")

    new = sorted(set(have) - want)
    gone = sorted(want - set(have))
    for k in sorted(have):
        print(f"    {k[0]}.{k[1]}  —  {have[k]}")
    if new:
        print("  NEW — a page mounts a name its module does not have:")
        for mod, name in new:
            print(f"    + {mod}.{name}")
    if gone:
        print("  GONE — fixed or moved; delete the row from KNOWN:")
        for mod, name in gone:
            print(f"    - {mod}.{name}")
    return 1 if (new or gone) else 0


if __name__ == "__main__":
    sys.exit(main())
