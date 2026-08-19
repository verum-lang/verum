#!/usr/bin/env python3
"""Census of ambient name visibility in core/ (T0780).

Every public stdlib symbol is visible bare, without a `mount`. This
script measures how much core/ depends on that, per file, and prints
the `mount` statements each file would need to stand without it.

The compiler reports one line per ambient use under
`VERUM_REPORT_AMBIENT=1`:

    [ambient] <consumer-module>\t<name>\t<owner-module>

A use is ambient only when the consuming file neither DECLARED the name
nor MOUNTED it — a declaration owns its name, and a mount IS the
request. Type parameters are excluded at the source: they resolve to a
type variable, not a nominal type.

Usage:
    census_ambient_visibility.py [N] [verum]      # sample N files (0 = all)
    census_ambient_visibility.py --mounts FILE    # print the mounts one file needs

Sampling is deterministic — every Nth file across a sorted list — so two
runs over an unchanged tree report the same number.
"""

import os
import subprocess
import sys
from collections import Counter, defaultdict

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CORE = os.path.join(ROOT, "core")

# Owners whose names are legitimately ambient today: the prelude
# re-exports them into every module by design. They stop being
# exceptions once the prelude is an explicit enumerable list, which is
# the other half of T0780's acceptance.
def looks_like_a_type_parameter(name):
    """`I`, `K`, `V`, `T2` — a parameter, not a symbol someone imports.

    Type parameters are already excluded at the source (they resolve to
    a type VARIABLE), but a stdlib module can also declare a REAL type
    with a one-letter name — `core/math/hott.vr` declares `type I is
    @builtin_interval` — and a file writing `fn from_iter<I: Iterator>`
    then shows up as reaching for it. The language resolves that
    correctly (measured: a user `fn f<I>(…)` binds its own `I`), so the
    report is what is wrong, not the program.

    Counted separately rather than silently dropped: a one-letter name
    declared as a public type IS a hazard, just not this file's.
    """
    return len(name) <= 2 and name[:1].isupper()


PRELUDE_OWNERS = {
    "core.base",
    "core.base.maybe",
    "core.base.result",
    "core.base.ordering",
    "core.base.protocols",
    "core.base.ops",
    "core.base.error",
    "core.base.iterator",
    "core.base.panic",
    "core.base.memory",
    "core.base.cell",
    "core.base.primitives",
    "core.base.coercion",
}


def core_files():
    out = []
    for dirpath, _dirnames, filenames in os.walk(CORE):
        for fn in sorted(filenames):
            if fn.endswith(".vr"):
                out.append(os.path.join(dirpath, fn))
    return sorted(out)


def sample(files, n):
    if n <= 0 or n >= len(files):
        return files
    step = len(files) / n
    return [files[int(i * step)] for i in range(n)]


def module_of(path):
    """core/time/duration.vr -> core.time.duration

    `mod.vr` names the directory's own module, so it drops the leaf.
    """
    rel = os.path.relpath(os.path.abspath(path), os.path.dirname(CORE))
    stem = rel[:-3] if rel.endswith(".vr") else rel
    parts = [p for p in stem.split(os.sep) if p]
    if parts and parts[-1] == "mod":
        parts.pop()
    return ".".join(parts)


def report_for(verum, path):
    """Returns [(consumer, name, owner)] for one file.

    Checking a file also checks what it depends on, and those modules
    report their OWN ambient uses. Only the rows whose consumer is this
    file's module belong to this file — without that filter,
    `core/time/duration.vr` was credited with wanting
    `TypeSchemaBuilder` and `BenchTimer`, which it never mentions.
    """
    env = {**os.environ, "VERUM_REPORT_AMBIENT": "1"}
    try:
        proc = subprocess.run(
            [verum, "check", path],
            env=env,
            capture_output=True,
            text=True,
            timeout=300,
        )
    except subprocess.TimeoutExpired:
        return []
    own = module_of(path)
    rows = []
    for line in proc.stderr.splitlines():
        if not line.startswith("[ambient] "):
            continue
        parts = line[len("[ambient] "):].split("\t")
        if len(parts) == 3 and parts[0] == own:
            rows.append(tuple(parts))
    return rows


def main():
    args = sys.argv[1:]
    if args and args[0] == "--mounts":
        verum = args[2] if len(args) > 2 else "verum"
        rows = report_for(verum, args[1])
        needed = defaultdict(set)
        for consumer, name, owner in rows:
            if owner in PRELUDE_OWNERS or owner.startswith("<"):
                continue
            # A module does not mount itself. The metadata attributes a
            # type to the module that declares it, and a file may use a
            # sibling declaration from its own module freely.
            if owner == consumer or looks_like_a_type_parameter(name):
                continue
            needed[owner].add(name)
        if not needed:
            print("# nothing to mount — this file already stands on its own")
            return 0
        for owner in sorted(needed):
            names = ", ".join(sorted(needed[owner]))
            print(f"mount {owner}.{{{names}}};")
        return 0

    count = int(args[0]) if args else 40
    verum = args[1] if len(args) > 1 else "verum"

    files = sample(core_files(), count)
    per_file = Counter()
    per_owner = Counter()
    total = 0
    real = 0
    params = 0
    for path in files:
        rows = report_for(verum, path)
        rel = os.path.relpath(path, ROOT)
        for consumer, _name, owner in rows:
            total += 1
            per_owner[owner] += 1
            if looks_like_a_type_parameter(_name):
                params += 1
            elif (
                owner not in PRELUDE_OWNERS
                and not owner.startswith("<")
                and owner != consumer
            ):
                real += 1
                per_file[rel] += 1

    print(f"files checked:            {len(files)} of {len(core_files())}")
    print(f"ambient uses (all):       {total}")
    print(f"ambient uses (non-prelude): {real}")
    print(f"one-letter names (parameters, reported separately): {params}")
    print()
    print("top owners reached without a mount:")
    for owner, n in per_owner.most_common(12):
        mark = "  (prelude)" if owner in PRELUDE_OWNERS else ""
        print(f"  {n:6d}  {owner}{mark}")
    print()
    print("files depending most on ambient visibility:")
    for rel, n in per_file.most_common(10):
        print(f"  {n:6d}  {rel}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
