#!/usr/bin/env python3
"""Ring law gate for the standard library's dependency graph.

The law (docs/architecture/core-reference-architecture.md §4):

    A module in ring N may depend only on rings < N.
    Edges upward and sideways between rings do not exist.

Ring 0 is declared as ONE cohesive unit — the language's primitive
layer — so mutual dependencies INSIDE it are expected and allowed
(§6bis.2a: `Maybe.ok_or` / `Result.ok` are paired conversions and must
not be severed). Only edges BETWEEN rings are constrained.

The rings themselves are data, not code: `core/rings.toml` declares
them, this gate reads it. Adding a module without placing it in a ring
is itself a violation — an unplaced module is a module nobody decided
the layer of.

Usage:
    check_core_rings.py            # report; exit 1 on any violation
    check_core_rings.py --census   # report the full edge table, exit 0
    check_core_rings.py --no-root-mounts  # the pre-2026-08-15 narrow view
"""

from __future__ import annotations

import re
import sys
import tomllib
from collections import defaultdict
from functools import lru_cache
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CORE = REPO / "core"
RINGS_TOML = CORE / "rings.toml"

# `mount a.b.c;`, `mount a.b.{X, Y};`, `mount a.b.*;`, `mount .rel.X;`
MOUNT_RE = re.compile(r"\bmount\s+([^;]+);", re.S)


def strip_comments(src: str) -> str:
    """Remove // line comments and /* */ blocks, preserving string bodies."""
    out = []
    i, n = 0, len(src)
    in_str = in_chr = False
    while i < n:
        c = src[i]
        nxt = src[i + 1] if i + 1 < n else ""
        if in_str:
            if c == "\\":
                out.append("  "); i += 2; continue
            if c == '"':
                in_str = False
            out.append(c); i += 1; continue
        if in_chr:
            if c == "\\":
                out.append("  "); i += 2; continue
            if c == "'":
                in_chr = False
            out.append(c); i += 1; continue
        if c == '"':
            in_str = True; out.append(c); i += 1; continue
        if c == "'":
            in_chr = True; out.append(c); i += 1; continue
        if c == "/" and nxt == "/":
            while i < n and src[i] != "\n":
                i += 1
            continue
        if c == "/" and nxt == "*":
            i += 2
            while i + 1 < n and not (src[i] == "*" and src[i + 1] == "/"):
                i += 1
            i += 2
            continue
        out.append(c); i += 1
    return "".join(out)


def module_of(path: Path) -> str | None:
    """The core module a file belongs to, at FULL path granularity.

    `core/intrinsics/lowlevel/aarch64.vr` -> `intrinsics.lowlevel`
    (the containing module, dropping the file name; `mod.vr` names the
    directory itself).

    Sources are resolved as finely as targets deliberately. When only
    targets carried submodule paths, a ring declared for a submodule
    worked when the submodule was DEPENDED ON and did nothing when it
    was DEPENDING — `intrinsics.lowlevel` placed at ring 1 still
    reported `intrinsics(r0) -> sys.linux.auxv(r1)`, because the source
    collapsed back to `intrinsics`. A law that reads one side of an
    edge at a different resolution than the other is not the same law
    on both sides.

    Ring lookup is by longest declared prefix (`ring_for`), so an
    undeclared submodule still inherits its parent's ring and nothing
    falls out of measurement.
    """
    rel = path.relative_to(CORE)
    if len(rel.parts) < 2:
        return None
    # A FILE is a module: `core/security/x509/verifier.vr` declares
    # `module core.security.x509.verifier;`. Dropping the stem made the
    # source coarser than the target one more time — a ring declared for
    # `security.x509.verifier` matched when the file was depended on and
    # never when it was depending.
    parts = list(rel.parts[:-1])
    if rel.name != "mod.vr":
        parts.append(rel.stem)
    return ".".join(parts) if parts else None


def targets(mount_body: str, from_file: Path) -> list[str]:
    """Top-level core module(s) a mount statement points at.

    Resolution mirrors the compiler's: a leading segment naming a
    SIBLING file or subdirectory of the mounting file binds locally and
    is intra-module by construction — `core/sys/windows/mod.vr`'s
    `mount io.{...}` re-exports its own `core/sys/windows/io.vr`, not
    `core/io`. Counting those as cross-module edges would make the law
    report violations that do not exist in the code.

    Relative mounts (`mount .sibling.X;`, `mount super.x.Y;`) are USUALLY
    intra-module — but not always, and the difference is decidable rather
    than assumable.  `super` names the mounting file's own directory and
    each additional `super` one level above it, so `super.super.mem.…`
    written in `core/base/memory.vr` resolves to `core/mem`, a DIFFERENT
    top-level module.  Returning nothing for every relative form hid
    exactly that: measured 2026-08-15, 15 of 581 `super.`-rooted mounts
    leave their module, and one of them — `base` (ring 0) reaching
    `mem.allocator` and `mem.header` (ring 1) — is an upward edge out of
    the foundation itself.
    """
    body = " ".join(mount_body.split())
    if body.startswith("."):
        return []
    if body.startswith("super."):
        return _relative_target(body, from_file)
    head = body.split("{")[0].strip()
    parts = [p for p in head.split(".") if p and p != "*"]
    if not parts:
        return []
    if parts[0] != "core":
        # Local-first: `mount io.{...}` next to `io.vr` / `io/` is local.
        here = from_file.parent
        if (here / f"{parts[0]}.vr").exists() or (here / parts[0]).is_dir():
            return []
        # `mount base.x.Y;` — core-relative shorthand used across the tree.
    else:
        parts = parts[1:]
    if not parts:
        # The mount named the ROOT (`mount core.*;`, `mount core.{X, Y};`).
        # It is not "no dependency" — it is a dependency on everything the
        # root re-exports.  See `root_reexports`.
        if EXPAND_ROOT_MOUNTS:
            return list(root_reexports())
        return []
    return [_longest_module_path(parts)]


# Whether `mount core.*;` / `mount core.{X};` expand to what the root
# re-exports.  ON, and this is the whole point of the gate now.
#
# It shipped OFF for one commit, because at that moment turning it on would
# have decided an architecture question rather than reported one: the law
# saw 5277 edges and reported zero violations while 1267 of 8785 mount
# sites — every `mount core.*;` — were invisible to it, and counting them
# produced 423 upward edges and 15 cycles.
#
# Those are gone.  The prelude now publishes exactly the twelve `base.*`
# submodules §4ter says it should, every other entry having been either
# mounted explicitly by the handful of files that used it or shown by
# measurement to be redundant with a compiler built-in.  With root mounts
# counted the law holds over 20423 edges — four times what it used to
# check — with zero violations.
#
# `--no-root-mounts` restores the narrow view, for comparing against a
# pre-2026-08-15 run.
EXPAND_ROOT_MOUNTS = True

# Edges declared in rings.toml's [exceptions] table: violations that are
# ALLOWED, each with its reason written where the rings themselves are
# declared.  Reported, never silent — an exception the reader cannot see
# is indistinguishable from a gate that missed something.
EXCEPTIONS: set[str] = set()


@lru_cache(maxsize=1)
def root_reexports() -> tuple[str, ...]:
    """The modules `core/mod.vr` publishes — what `mount core.*;` reaches.

    THE HOLE THIS CLOSES.  `mount core.*;` appears in 1195 of core/'s 2560
    files, and `mount core.{X, Y};` in 72 more.  Both name the ROOT, and
    `targets()` used to return NOTHING for them: `parts` becomes `["core"]`,
    the `core` head is stripped, the list is empty, no edge is recorded.  So
    the single most-used dependency statement in the standard library was
    invisible to the law that governs dependencies — 1267 of 8785 mount
    sites, 14 %.

    Today that is harmless by luck rather than by rule: the root re-exports
    only the prelude (base, collections, context, intrinsics, io, math,
    sync, text, time), all of which sit low.  But nothing enforced it.  One
    `public mount super.database.*;` added to `core/mod.vr` would give 1195
    files an edge into ring 5, and this gate would keep reporting zero
    violations, because it cannot see the edge at all.

    Expanding the root mount to what the root actually publishes turns those
    1267 invisible sites into ordinary edges, checked like every other.  The
    prelude's own placement is then what the law tests — which is the
    property the prelude needs to have.
    """
    mod = CORE / "mod.vr"
    if not mod.is_file():
        return ()
    text = strip_comments(mod.read_text(errors="replace"))
    seen: list[str] = []
    for m in re.finditer(r"\bpublic\s+mount\s+super\.([^;{]+)", text):
        # Resolve to the module that OWNS the re-exported symbol, not to its
        # top-level directory.  The distinction is the whole accuracy of this
        # function: the prelude publishes `collections.List`, a single type,
        # NOT the `collections` directory.  Attributing the directory turned
        # 25 named symbols into 9 whole-subtree dependencies and manufactured
        # cycles (`collections -> io -> collections`) that the code does not
        # contain — an over-approximation reported as a finding before it was
        # checked against `core/mod.vr` itself.
        parts = [p for p in m.group(1).split(".") if p and p != "*"]
        if not parts:
            continue
        owner = _longest_module_path(parts)
        if owner and owner not in seen:
            seen.append(owner)
    return tuple(seen)



def _relative_target(body: str, from_file: Path) -> list[str]:
    """The module a `super.`-rooted mount reaches, when it leaves this one.

    `super` is the mounting file's own directory; every additional `super`
    climbs one further.  The result is checked against the filesystem —
    a path that resolves to no directory and no `.vr` yields nothing
    rather than a guessed edge — and is reported ONLY when it lands in a
    different top-level module, because the law constrains edges BETWEEN
    modules and a file reaching its own sibling is not one.
    """
    segs = [seg for seg in body.split("{")[0].split(".") if seg]
    ups = 0
    while ups < len(segs) and segs[ups] == "super":
        ups += 1
    rest = segs[ups:]
    if not rest:
        return []
    base = from_file.parent
    for _ in range(ups - 1):
        base = base.parent
    target = base / rest[0]
    if not (target.is_dir() or target.with_suffix(".vr").is_file()):
        return []
    try:
        t_parts = target.relative_to(CORE).parts
        own = from_file.relative_to(CORE).parts
    except ValueError:
        return []
    if not t_parts or not own or t_parts[0] == own[0]:
        return []
    return [_longest_module_path(list(t_parts))]

def _longest_module_path(parts: list[str]) -> str:
    """The longest prefix of `parts` that is an actual module.

    A mount path is module segments followed by a symbol —
    `core.collections.list.{List}` names module `collections.list`,
    while `core.collections.List` names module `collections` and the
    symbol `List` it re-exports. Nothing in the syntax distinguishes
    them, but the filesystem does: the module tree IS the directory
    tree, so a segment is a module exactly when `<dir>/<seg>.vr` or
    `<dir>/<seg>/` exists.

    Resolving to the deepest real module is what lets a ring be
    declared for a SUBMODULE. That matters because the coarse form —
    one ring per top-level directory — cannot express a directory
    that legitimately spans layers, and forces the whole directory up
    to its highest member, which is how `core/security/hash` (pure
    byte computation, no dependencies beyond core) came to sit in the
    same ring as X.509 policy.
    """
    here, depth = CORE, 0
    for seg in parts:
        # Case-SENSITIVE membership, read from an explicit listing.
        # `Path.is_dir()` / `is_file()` ask the filesystem, and macOS
        # answers case-insensitively: `collections/List.vr` "exists"
        # there because `list.vr` does. That would make this gate
        # resolve `collections.List` (a symbol) to a module on a
        # developer's machine and to nothing on the Linux runner —
        # the same tree, two verdicts.
        entries = _entries(here)
        if seg in entries:
            here = here / seg; depth += 1
        elif f"{seg}.vr" in entries:
            depth += 1; break
        else:
            break
    return ".".join(parts[:depth]) if depth else parts[0]


@lru_cache(maxsize=None)
def _entries(d: Path) -> frozenset[str]:
    try:
        return frozenset(e.name for e in d.iterdir())
    except OSError:
        return frozenset()


def load_rings() -> tuple[dict[str, float], dict[float, str], set[float]]:
    """Read the ring declaration.

    Ring indices are ordered numbers, not consecutive ones: the law
    only ever compares them (`ring(src) < ring(dst)`). Fractional
    indices are therefore legitimate and are written as quoted TOML
    keys — `[ring."1.5"]`. This exists so that discovering a layer
    between two declared rings costs one entry, not a renumbering of
    every ring below it plus every prose reference to them.
    """
    if not RINGS_TOML.exists():
        sys.exit(f"[fail] {RINGS_TOML} missing — the ring law has no declaration to read")
    data = tomllib.loads(RINGS_TOML.read_text())
    global EXCEPTIONS
    EXCEPTIONS = set(data.get("exceptions", {}))
    ring_of: dict[str, float] = {}
    names: dict[float, str] = {}
    cohesive: set[float] = set()
    for key, spec in data.get("ring", {}).items():
        try:
            idx = float(key)
        except ValueError:
            sys.exit(f"[fail] ring key {key!r} is not a number — rings are ordered by index")
        names[idx] = spec.get("name", f"ring{idx}")
        if spec.get("cohesive", False):
            cohesive.add(idx)
        for m in spec.get("modules", []):
            if m in ring_of:
                sys.exit(f"[fail] module '{m}' declared in two rings")
            ring_of[m] = idx
    budget = data.get("ring", {}).get("0", {}).get("inherited_budget")
    return ring_of, names, cohesive, budget


def ring_for(module: str, ring_of: dict[str, float]) -> float | None:
    """The ring of `module`, by LONGEST DECLARED PREFIX.

    `collections.map` inherits `collections`'s ring unless it is itself
    declared. Prefix inheritance is what keeps the declaration small
    AND keeps the gate honest: a target that resolves to a submodule
    can never fall out of the table and stop being measured — it is
    covered by its parent until someone deliberately places it
    elsewhere.
    """
    parts = module.split(".")
    for i in range(len(parts), 0, -1):
        r = ring_of.get(".".join(parts[:i]))
        if r is not None:
            return r
    return None


def placement_of(module: str, ring_of: dict[str, float]) -> str | None:
    """The declared unit that carries `module`'s ring.

    `math.foundations` is placed by the entry for `math` unless it has
    one of its own. Cycle detection runs on these units, not on raw
    module paths: the law speaks about what a ring DECLARES, so
    `collections -> math.foundations -> collections` is the same
    cycle as `collections -> math -> collections` whenever `math` is
    declared as one unit. Comparing raw paths instead would let a
    cycle disappear the moment a mount named a submodule — the finer
    the measurement, the quieter the gate, which is backwards.
    """
    parts = module.split(".")
    for i in range(len(parts), 0, -1):
        cand = ".".join(parts[:i])
        if cand in ring_of:
            return cand
    return None


def main() -> int:
    census = "--census" in sys.argv
    if "--no-root-mounts" in sys.argv:
        global EXPAND_ROOT_MOUNTS
        EXPAND_ROOT_MOUNTS = False
    if EXPAND_ROOT_MOUNTS:
        print(
            "[root-mounts] `mount core.*;` / `mount core.{…};` counted as edges to "
            f"the {len(root_reexports())} module(s) core/mod.vr re-exports: "
            f"{', '.join(root_reexports())}"
        )
    ring_of, ring_names, cohesive_rings, inherited_budget = load_rings()

    edges: dict[tuple[str, str], list[str]] = defaultdict(list)
    for path in sorted(CORE.rglob("*.vr")):
        src_mod = module_of(path)
        if src_mod is None:
            continue
        text = strip_comments(path.read_text(errors="replace"))
        for m in MOUNT_RE.finditer(text):
            line = text[: m.start()].count("\n") + 1
            for dst_mod in targets(m.group(1), path):
                if dst_mod == src_mod:
                    continue
                edges[(src_mod, dst_mod)].append(f"{path.relative_to(REPO)}:{line}")

    present = {module_of(p) for p in CORE.rglob("*.vr")} - {None}
    unplaced = sorted(m for m in present if ring_for(m, ring_of) is None)

    # The law has two independent clauses, and conflating them was a
    # measurement error worth recording: an edge WITHIN a ring
    # (`text -> collections`, `mem -> sys`, `net -> security`) is normal
    # intra-layer structure, not a defect — a layer is not required to
    # be an antichain. What must never exist is (a) an edge to a HIGHER
    # ring, which inverts the architecture, and (b) a CYCLE, which means
    # the two modules are really one and the split is fiction.
    violations: list[tuple[str, str, int, int, list[str]]] = []
    excused: list[tuple[str, str, list[str]]] = []
    for (src, dst), sites in sorted(edges.items()):
        rs, rd = ring_for(src, ring_of), ring_for(dst, ring_of)
        if rs is None or rd is None:
            continue
        if rd > rs:
            # A declared exception is still REPORTED — an allowance the
            # reader cannot see is indistinguishable from a gate that
            # missed something.  It only stops failing the run.
            if f"{src} -> {dst}" in EXCEPTIONS:
                excused.append((src, dst, sites))
            else:
                violations.append((src, dst, rs, rd, sites))

    # Clause (b): cycles among the intra-ring edges.
    intra: dict[str, set[str]] = defaultdict(set)
    for (src, dst) in edges:
        us, ud = placement_of(src, ring_of), placement_of(dst, ring_of)
        if us is None or ud is None or us == ud:
            continue
        r = ring_for(src, ring_of)
        if r != ring_for(dst, ring_of):
            continue
        # A ring may declare itself COHESIVE: one layer whose members
        # are mutually dependent by design. Ring 0 is the case this
        # exists for — `Maybe.ok_or -> Result` and `Result.ok -> Maybe`
        # are paired conversions, `base` needs `List` and `list.vr`
        # needs `base.ordering`, and severing either would be the
        # defect. rings.toml said so in prose from the start; the gate
        # reported those pairs as violations anyway, which is a law and
        # its enforcement disagreeing about the same sentence.
        #
        # Clause (b) still applies everywhere else: a cycle between
        # modules that were declared as SEPARATE layers means the split
        # is fiction.
        if r in cohesive_rings:
            continue
        intra[us].add(ud)
    cycles: list[list[str]] = []
    seen: set[str] = set()
    stack: list[str] = []
    on_stack: set[str] = set()

    def walk(node: str) -> None:
        seen.add(node); stack.append(node); on_stack.add(node)
        for nxt in sorted(intra.get(node, ())):
            if nxt in on_stack:
                cycles.append(stack[stack.index(nxt):] + [nxt])
            elif nxt not in seen:
                walk(nxt)
        stack.pop(); on_stack.discard(node)

    for m in sorted(intra):
        if m not in seen:
            walk(m)

    if census:
        print(f"modules: {len(present)}  edges: {len(edges)}")
        for (src, dst), sites in sorted(edges.items(), key=lambda kv: -len(kv[1])):
            rs, rd = ring_for(src, ring_of), ring_for(dst, ring_of)
            print(f"  {src}(r{rs}) -> {dst}(r{rd})  {len(sites)} site(s)")
        return 0

    ok = True
    if unplaced:
        ok = False
        print(f"[fail] {len(unplaced)} module(s) not placed in any ring — add them to core/rings.toml:")
        for m in unplaced:
            print(f"    {m}")
    if excused:
        print(f"[note] {len(excused)} declared exception(s) — rings.toml [exceptions] carries the reason:")
        for src, dst, sites in excused:
            print(f"    {src} -> {dst}  {len(sites)} site(s)")
        print()

    if cycles:
        ok = False
        print(f"[fail] {len(cycles)} dependency cycle(s) inside a ring — the split is fiction, the modules are one:")
        for c in cycles[:10]:
            print("    " + " -> ".join(c))
    if violations:
        ok = False
        total = sum(len(v[4]) for v in violations)
        print(f"[fail] {len(violations)} UPWARD edge(s) across {total} mount site(s):")
        for src, dst, rs, rd, sites in violations:
            print(f"    {src}(r{rs}) -> {dst}(r{rd})  {len(sites)} site(s)")
            for s in sites[:3]:
                print(f"        {s}")
            if len(sites) > 3:
                print(f"        … {len(sites) - 3} more")
    # RING-0 INHERITANCE RATCHET.
    #
    # A module's ring is resolved by LONGEST DECLARED PREFIX, so a file
    # written into `core/base/` is ring 0 unless someone names it
    # elsewhere.  That is convenient for the twelve modules that really
    # are the language's vocabulary and silently wrong for anything
    # else: on 2026-08-18 ten files had accumulated in ring 0 that way —
    # semver parsing, glob matching, Levenshtein distance — none of them
    # published by the prelude, all of them free to be depended on by
    # the foundation.
    #
    # Forbidding inheritance outright would mean listing all forty
    # legitimate submodules, which is bureaucracy, not architecture.  So
    # the number is a budget: it may shrink freely, and growing it is a
    # decision that has to be written into rings.toml, where its diff
    # gets reviewed.
    inherited0 = sorted(
        m for m in present
        if ring_for(m, ring_of) == 0.0 and m not in ring_of
    )
    if inherited_budget is not None:
        if len(inherited0) > inherited_budget:
            ok = False
            print(
                f"[fail] {len(inherited0)} module(s) land in ring 0 by inheriting a "
                f"parent's entry; the budget in rings.toml is {inherited_budget}."
            )
            print("    Ring 0 is the language's vocabulary — what the grammar, the")
            print("    checker and @derive name, and what the prelude publishes.")
            print("    A new file under core/base/ is NOT that by default.")
            print("    Either name the module's real ring in rings.toml, or raise")
            print("    the budget there with the reason it belongs in the foundation.")
            for m in inherited0[-8:]:
                print(f"        {m}")
        elif len(inherited0) < inherited_budget:
            print(
                f"[note] ring-0 inheritance is down to {len(inherited0)} "
                f"(budget {inherited_budget}) — lower the budget in rings.toml "
                f"to lock the gain in."
            )

    if ok:
        print(f"[ok] ring law holds: {len(present)} modules, {len(edges)} inter-module edges, 0 violations")
        return 0
    print("\nThe law: a module in ring N depends only on rings < N.")
    print("Fix by moving the dependency down, not by widening the ring.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
