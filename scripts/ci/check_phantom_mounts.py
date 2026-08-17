#!/usr/bin/env python3
"""Fail when `mount M.{X}` names an X that module M does not export.

WHY THIS EXISTS
---------------
A mount naming a symbol its module does not have is not rejected when the
name happens to exist in some OTHER module: every public stdlib symbol is
visible bare, and that global resolution satisfies the import.  Worse, the
mistake LAUNDERS ITSELF through the bake — measured as a 2x2:

    name exists elsewhere?   baked into the archive?   result
    no                       no                        E401
    no                       yes                       E401
    yes                      no                        E401
    yes                      yes                       ACCEPTED, 0 errors

Only the last row passes, and the last row is what ships: the first bake
accepts the mount, records the module with that resolution, and every later
`verum check` consults the recorded answer instead of re-deriving it.  So
checking core/ in place cannot see this class at all — the archive it
consults was produced by the same acceptance.

What it cost once: `core/database/sqlite/native/l5_sql/parser/token_stream.vr`
mounted `Token` from a lexer that declares `SqlToken`.  `Token` exists —
`core/meta/token.vr`, a RECORD belonging to the metaprogramming lexer — so
the whole SQL token stream was typed as that record, every `TIdent(s)`
pattern matched a variant against a record, and the binding silently
vanished.  The reported error was `unbound variable: s`, three lines away
from the cause and in a different file from the mistake.

HOW IT AVOIDS THE TRAP
----------------------
Probes are written OUTSIDE core/, so the module under test is not the module
being checked and the archive cannot vouch for the mount.  That is row three
of the table, where the compiler answers correctly.

A textual scan cannot replace this.  One was tried: 2070 hits, then 1030
after fixing variant extraction, and a hand-check of five found FOUR false
positives (a variant followed by a comment, a no-brace `public mount`
re-export, `public pure fn`, an over-long declaration body).  A regex cannot
model a module's export surface in this language; the compiler already
knows it, and `E401` is exactly that knowledge.

COST
----
One `verum check` per target module — 842 of them, roughly a quarter of an
hour.  Too slow for every push; run it in the nightly/periodic lane, or by
hand after touching mounts.  It is NOT part of `make gates-source`, which is
source-only and finishes in seconds.

Usage:
    check_phantom_mounts.py <path-to-verum> [--write-baseline]
"""

from __future__ import annotations

import collections
import pathlib
import re
import subprocess
import sys
import tempfile

BASELINE = pathlib.Path(__file__).with_name("phantom_mounts_known.txt")

# `mount core.a.b.{X, Y as Z}` — only the explicit group form is checkable:
# a glob mount names nothing in particular, and a relative path (`super.x`,
# `.child`) is resolved against the importing file, which a probe elsewhere
# cannot reproduce.
MOUNT_RE = re.compile(
    r"(?m)^\s*(?:public\s+|pub\s+)?mount\s+(core\.[\w.]+)\.\{([^}]*)\}", re.S
)
NOT_FOUND_RE = re.compile(r"cannot find `([^`]+)` in module `([^`]+)`")


def wanted_names(root: pathlib.Path) -> dict[str, set[str]]:
    """module path -> every name any file imports from it."""
    want: dict[str, set[str]] = collections.defaultdict(set)
    for path in sorted(root.rglob("*.vr")):
        text = path.read_text(errors="ignore")
        for module, items in MOUNT_RE.findall(text):
            for part in items.split(","):
                # `X as Y` is still an import OF X; the alias is local.
                name = part.strip().split(" as ")[0].strip()
                if re.fullmatch(r"[A-Za-z_]\w*", name):
                    want[module].add(name)
    return want


def probe_once(verum: str, workdir: pathlib.Path, module: str, names: list[str]):
    """One compiler run; returns the names it reported as missing."""
    src = workdir / (module.replace(".", "_") + ".vr")
    src.write_text(
        "mount %s.{%s};\n\nfn main() { print(\"ok\"); }\n" % (module, ", ".join(names))
    )
    try:
        run = subprocess.run(
            [verum, "check", str(src)], capture_output=True, timeout=300, text=True
        )
    except subprocess.TimeoutExpired:
        return []
    return [
        (found_module, name)
        for name, found_module in NOT_FOUND_RE.findall(run.stdout + run.stderr)
    ]


def probe(verum: str, workdir: pathlib.Path, module: str, names: list[str]):
    """Every name in `names` the module does not export.

    The compiler reports only the FIRST missing name in a mount group and
    stops — measured: a group with two deliberate phantoms yields one E401.
    A single run per module would therefore report "one phantom per module"
    and read as a total.  So each hit is removed and the module re-probed
    until it comes back clean; the extra cost is one run per phantom found,
    not one per name checked.
    """
    remaining = list(names)
    hits: list[tuple[str, str]] = []
    for _ in range(len(names)):
        found = probe_once(verum, workdir, module, remaining)
        if not found:
            break
        hits.extend(found)
        missing = {name for _, name in found}
        remaining = [n for n in remaining if n not in missing]
        if not remaining:
            break
    return hits


def self_test(verum: str, workdir: pathlib.Path) -> bool:
    """The instrument must report a phantom it is given deliberately.

    Without this the whole run degrades to "no output means clean", which is
    the shape every silent gate in this repo has had.
    """
    lexer = "core.database.sqlite.native.l5_sql.lexer"
    hits = probe(verum, workdir, lexer, ["SqlToken", "ZzDeliberatePhantomZz"])
    names = {n for _, n in hits}
    if "ZzDeliberatePhantomZz" not in names:
        print(
            "check-phantom-mounts: SELF-TEST FAILED — a deliberate phantom was "
            "not reported, so a clean run would prove nothing",
            file=sys.stderr,
        )
        return False
    if "SqlToken" in names:
        print(
            "check-phantom-mounts: SELF-TEST FAILED — a real export was "
            "reported as missing",
            file=sys.stderr,
        )
        return False
    return True


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    verum = sys.argv[1]
    write_baseline = "--write-baseline" in sys.argv

    root = pathlib.Path("core")
    if not root.is_dir():
        print("check-phantom-mounts: run from the repository root", file=sys.stderr)
        return 2

    with tempfile.TemporaryDirectory(prefix="verum-mount-probe-") as tmp:
        workdir = pathlib.Path(tmp)
        if not self_test(verum, workdir):
            return 1

        want = wanted_names(root)
        found: set[str] = set()
        for module in sorted(want):
            for owner, name in probe(verum, workdir, module, sorted(want[module])):
                found.add(f"{owner}\t{name}")

    if write_baseline:
        BASELINE.write_text(
            "# Mounts naming a symbol the module does not export.\n"
            "# Regenerate: scripts/ci/check_phantom_mounts.py <verum> --write-baseline\n"
            "# A line removed is a mount repaired.  A line added is a decision to\n"
            "# ship one more import that resolves to an unrelated same-named symbol.\n"
            + "\n".join(sorted(found))
            + "\n"
        )
        print(f"check-phantom-mounts: baseline written, {len(found)} entries")
        return 0

    known = {
        line.strip()
        for line in BASELINE.read_text().splitlines()
        if line.strip() and not line.startswith("#")
    } if BASELINE.exists() else set()

    new = sorted(found - known)
    gone = sorted(known - found)
    for entry in new:
        owner, name = entry.split("\t")
        print(f"  NEW phantom mount: {name!r} is not exported by {owner}")
    if gone:
        print(f"check-phantom-mounts: {len(gone)} repaired since the baseline")
    if new:
        print(
            f"check-phantom-mounts: FAIL — {len(new)} new phantom mount(s); "
            f"{len(known)} known",
            file=sys.stderr,
        )
        return 1
    print(f"check-phantom-mounts: OK ({len(found)} == baseline)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
