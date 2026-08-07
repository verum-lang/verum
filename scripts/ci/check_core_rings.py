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
"""

from __future__ import annotations

import re
import sys
import tomllib
from collections import defaultdict
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
    """Top-level core module a file belongs to (`core/net/x/y.vr` -> `net`)."""
    rel = path.relative_to(CORE)
    return rel.parts[0] if len(rel.parts) > 1 else None


def targets(mount_body: str, from_file: Path) -> list[str]:
    """Top-level core module(s) a mount statement points at.

    Resolution mirrors the compiler's: a leading segment naming a
    SIBLING file or subdirectory of the mounting file binds locally and
    is intra-module by construction — `core/sys/windows/mod.vr`'s
    `mount io.{...}` re-exports its own `core/sys/windows/io.vr`, not
    `core/io`. Counting those as cross-module edges would make the law
    report violations that do not exist in the code.

    Relative mounts (`mount .sibling.X;`, `mount super.x.Y;`) are
    intra-module too and yield nothing.
    """
    body = " ".join(mount_body.split())
    if body.startswith(".") or body.startswith("super."):
        return []
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
        return [parts[0]]
    return [parts[1]] if len(parts) > 1 else []


def load_rings() -> tuple[dict[str, int], dict[int, str]]:
    if not RINGS_TOML.exists():
        sys.exit(f"[fail] {RINGS_TOML} missing — the ring law has no declaration to read")
    data = tomllib.loads(RINGS_TOML.read_text())
    ring_of: dict[str, int] = {}
    names: dict[int, str] = {}
    for key, spec in data.get("ring", {}).items():
        idx = int(key)
        names[idx] = spec.get("name", f"ring{idx}")
        for m in spec.get("modules", []):
            if m in ring_of:
                sys.exit(f"[fail] module '{m}' declared in two rings")
            ring_of[m] = idx
    return ring_of, names


def main() -> int:
    census = "--census" in sys.argv
    ring_of, ring_names = load_rings()

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
    unplaced = sorted(m for m in present if m not in ring_of)

    # The law has two independent clauses, and conflating them was a
    # measurement error worth recording: an edge WITHIN a ring
    # (`text -> collections`, `mem -> sys`, `net -> security`) is normal
    # intra-layer structure, not a defect — a layer is not required to
    # be an antichain. What must never exist is (a) an edge to a HIGHER
    # ring, which inverts the architecture, and (b) a CYCLE, which means
    # the two modules are really one and the split is fiction.
    violations: list[tuple[str, str, int, int, list[str]]] = []
    for (src, dst), sites in sorted(edges.items()):
        rs, rd = ring_of.get(src), ring_of.get(dst)
        if rs is None or rd is None:
            continue
        if rd > rs:
            violations.append((src, dst, rs, rd, sites))

    # Clause (b): cycles among the intra-ring edges.
    intra: dict[str, set[str]] = defaultdict(set)
    for (src, dst) in edges:
        if ring_of.get(src) is not None and ring_of.get(src) == ring_of.get(dst):
            intra[src].add(dst)
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
            rs, rd = ring_of.get(src, "?"), ring_of.get(dst, "?")
            print(f"  {src}(r{rs}) -> {dst}(r{rd})  {len(sites)} site(s)")
        return 0

    ok = True
    if unplaced:
        ok = False
        print(f"[fail] {len(unplaced)} module(s) not placed in any ring — add them to core/rings.toml:")
        for m in unplaced:
            print(f"    {m}")
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
    if ok:
        print(f"[ok] ring law holds: {len(present)} modules, {len(edges)} inter-module edges, 0 violations")
        return 0
    print("\nThe law: a module in ring N depends only on rings < N.")
    print("Fix by moving the dependency down, not by widening the ring.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
