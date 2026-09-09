#!/usr/bin/env python3
"""Fail when a newtype stops being free across a module boundary.

WHAT THE TREE ALREADY PROMISES
------------------------------
`website:docs/language/types.md` § Newtypes says, of a single-field
newtype:

    "Newtypes cost nothing at runtime; they exist purely for the type
     checker's benefit."

Until 2026-09-10 that sentence was FALSE for any newtype constructed
outside the directory that declares it, and nobody had compared the
sentence with the compiler. This gate is the comparison.

THE DEFECT IT PINS (T1192)
--------------------------
The bake compiles one module per DIRECTORY and builds a fresh codegen for
each, so `newtype_names` — the "this name is transparent" cache — started
empty every time. A newtype from another directory was absent from it and
BOTH sides of codegen took the opaque path: `New`+`Clone`+`SetF` to
construct, `GetF` to read.

That pair is self-consistent, which is exactly why no existing test saw
it: archive-internal code worked, and `safe_read(fd, buf)` returned six
bytes with the very descriptor whose `.0` was broken. User code is
compiled separately and DOES recover transparency from the archive
descriptor, so a value crossing that boundary was built opaque and read
transparently — and yielded its own ADDRESS:

    fd.0 + 0    36066691904
    fd.0 == 3   false
    fd.0 > 0    TRUE          every `if fd >= 0` guard passed

A defect whose two halves agree with each other is invisible to any test
that looks at one half. The only thing that catches it is a comparison
against the DECLARED model, and that model lived in a paragraph.

HOW THIS GATE WORKS
-------------------
Four minimal stdlibs, each a different directory layout, baked with
`verum stdlib precompile --stdlib-path <dir>` (which takes any directory
and finishes in well under a second). The VBC dump of a newtype
constructor must contain no `New` / `SetF`: a transparent wrapper
compiles to a `Mov`.

    same directory, declaration first     transparent
    same directory, declaration LAST      transparent  (order is not the axis)
    parent -> subpackage                  transparent  (was: opaque)
    sibling packages, equal depth         transparent  (was: opaque)

The last two are the regression. The first two are controls: they were
transparent before the fix as well, so a gate that only checked them
would have passed over the defect — which is the shape this file exists
to refuse.
"""

from __future__ import annotations

import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

BOXING = re.compile(r"\b(New|SetF)\s*\{")
DUMPED_FN = re.compile(r"\[vbc-dump\] fn '([^']+)'")

# (label, {relative path: source}, dump filter)
LAYOUTS: list[tuple[str, dict[str, str], str]] = [
    (
        "same directory, declaration first",
        {
            "core/adecl.vr": "module core.adecl;\n\npublic type Nt is (Int);\n",
            "core/buse.vr": (
                "module core.buse;\n\n"
                "mount core.adecl.{Nt};\n\n"
                "public fn make(n: Int) -> Nt { Nt(n) }\n"
                "public fn read(x: Nt) -> Int { x.0 }\n"
            ),
            "core/mod.vr": "public mount adecl.*;\npublic mount buse.*;\n",
        },
        "core.buse.",
    ),
    (
        "same directory, declaration LAST",
        {
            "core/ause.vr": (
                "module core.ause;\n\n"
                "mount core.zdecl.{Nt};\n\n"
                "public fn make(n: Int) -> Nt { Nt(n) }\n"
                "public fn read(x: Nt) -> Int { x.0 }\n"
            ),
            "core/zdecl.vr": "module core.zdecl;\n\npublic type Nt is (Int);\n",
            "core/mod.vr": "public mount zdecl.*;\npublic mount ause.*;\n",
        },
        "core.ause.",
    ),
    (
        "parent -> subpackage",
        {
            "core/decl.vr": "module core.decl;\n\npublic type Nt is (Int);\n",
            "core/pkg/use.vr": (
                "module core.pkg.use;\n\n"
                "mount core.decl.{Nt};\n\n"
                "public fn make(n: Int) -> Nt { Nt(n) }\n"
                "public fn read(x: Nt) -> Int { x.0 }\n"
            ),
            "core/mod.vr": "public mount decl.*;\npublic mount pkg.use.*;\n",
        },
        "core.pkg.use.",
    ),
    (
        "sibling packages, equal depth",
        {
            "core/a/x.vr": "module core.a.x;\n\npublic type Nt is (Int);\n",
            "core/b/y.vr": (
                "module core.b.y;\n\n"
                "mount core.a.x.{Nt};\n\n"
                "public fn make(n: Int) -> Nt { Nt(n) }\n"
                "public fn read(x: Nt) -> Int { x.0 }\n"
            ),
            "core/mod.vr": "public mount a.x.*;\npublic mount b.y.*;\n",
        },
        "core.b.y.",
    ),
]


def bake(verum: pathlib.Path, files: dict[str, str], dump_filter: str) -> str:
    """Bake one layout and return the VBC dump of its module."""
    with tempfile.TemporaryDirectory(prefix="nt-transparency-") as d:
        root = pathlib.Path(d)
        for rel, text in files.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        result = subprocess.run(
            [
                str(verum), "stdlib", "precompile",
                "--stdlib-path", str(root / "core"),
                "-o", str(root / "out.vbca"),
            ],
            capture_output=True,
            text=True,
            timeout=300,
            env={**dict(__import__("os").environ), "VERUM_DUMP_VBC": dump_filter},
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"the fixture itself failed to bake (rc={result.returncode}); "
                f"a gate whose fixture does not compile measures nothing.\n"
                + result.stderr[-800:]
            )
        return result.stderr + result.stdout


def self_test() -> int:
    """The pattern must see boxing where boxing is, and not where it is not.

    A gate whose detector never matches passes over the defect it was
    written for, and reads exactly like a clean tree."""
    boxed = "[vbc-dump]   pc=    4  #1    New { dst: Reg(3), type_id: 0, field_count: 1 }"
    clean = "[vbc-dump]   pc=    0  #0    Mov { dst: Reg(1), src: Reg(0) }"
    if not BOXING.search(boxed):
        print("self-test: the boxing pattern does not match a real `New` line",
              file=sys.stderr)
        return 1
    if BOXING.search(clean):
        print("self-test: the boxing pattern matches a plain `Mov` line",
              file=sys.stderr)
        return 1
    # `Renew`/`SetFoo`-shaped names must not count.
    if BOXING.search("Renewal { } SetFoo { }"):
        print("self-test: the pattern matched a name that merely contains New/SetF",
              file=sys.stderr)
        return 1
    print(f"[ok] self-test: the boxing pattern separates {2} known lines and "
          f"rejects 1 look-alike; {len(LAYOUTS)} layouts configured")
    return 0


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        return self_test()

    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if len(args) != 1:
        print("usage: check_newtype_transparency.py <path to verum>", file=sys.stderr)
        return 2
    verum = pathlib.Path(args[0])
    if not verum.is_file():
        print(f"verum binary not found: {verum}", file=sys.stderr)
        return 2
    if shutil.which(str(verum)) is None and not verum.stat().st_mode & 0o111:
        print(f"not executable: {verum}", file=sys.stderr)
        return 2

    failures: list[tuple[str, list[str]]] = []
    for label, files, dump_filter in LAYOUTS:
        dump = bake(verum, files, dump_filter)
        if not DUMPED_FN.search(dump):
            print(
                f"{label}: the bake produced NO disassembly for {dump_filter!r}. "
                "The fixture compiled but nothing was dumped, so this run "
                "measured nothing — refusing to report it as clean.",
                file=sys.stderr,
            )
            return 2
        boxed = [l.strip() for l in dump.splitlines() if BOXING.search(l)]
        status = "opaque" if boxed else "transparent"
        print(f"  {status:12s} {label}")
        if boxed:
            failures.append((label, boxed))

    if failures:
        print(
            "\nA single-field newtype is documented as free "
            "(`website:docs/language/types.md` § Newtypes: \"Newtypes cost "
            "nothing at runtime\"), and in these layouts it is not:",
            file=sys.stderr,
        )
        for label, lines in failures:
            print(f"  {label}", file=sys.stderr)
            for line in lines[:4]:
                print(f"      {line}", file=sys.stderr)
        print(
            "\nA constructor that allocates makes `.0` in USER code — which "
            "compiles to an identity Mov — return the object's ADDRESS, and "
            "an address compares greater than zero. See T1192.",
            file=sys.stderr,
        )
        return 1

    print(f"[ok] newtype transparency: {len(LAYOUTS)} layout(s), none boxes")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
