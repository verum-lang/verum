#!/usr/bin/env python3
"""Gate: the shipped stdlib archive must not gain panic-stubs.

WHY THE ARTIFACT AND NOT THE LOG. `check-bake-diagnostics` already counts
panic-stubs — in a BAKE LOG, which it takes as a required argument, so it
runs only when somebody happens to have one. A stub that arrives between
bakes is invisible to it until the next person passes a log.

The archive carries the answer itself. The lenient path writes the stub's
own diagnostic INTO the artifact:

    [lenient] compose_geometric compiled to panic-stub:
        undefined function: compose_functors (in function compose_geometric)

so `strings runtime.vbca` enumerates every stubbed function, by name, with
the reason, from a file that exists after any build. No bake, no log, no
argument.

WHAT A STUB MEANS. The function is in the archive and callable; calling it
panics with the message above. So the name being present proves nothing
about the name working — measured 2026-09-10: `compose_geometric` appears
three times in the archive and is a stub. The INVERSE reads correctly: a
name absent from the archive is genuinely not there (`frame_address`, the
intrinsic behind `StackTrace.capture()`, is absent — zero occurrences).

THE ROSTER IS BY NAME, NOT A COUNT. Two stubs that swap identity keep the
count at two, and a count cannot tell that from nothing happening. Both of
the entries below are the same defect — a functor pair in `core/math` whose
helpers are undefined (T1352, T1366) — and both are permitted until that is
closed, at which point this roster goes empty and stays empty.

Reading the archive as DATA is deliberate and is not the "never trust a
binary in the shared target/" rule: nothing here executes it.
"""
from __future__ import annotations

import pathlib
import re
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
DEFAULT = REPO / "target" / "precompiled-stdlib" / "runtime.vbca"
STUB = re.compile(r"\[lenient\] ([A-Za-z_][A-Za-z0-9_]*) compiled to panic-stub: ([^(]*)")
# A real archive is tens of MB; a truncated or placeholder file must not
# read as "wonderfully free of stubs".
MIN_MB = 5.0

KNOWN: dict[str, str] = {
    "compose_geometric": "undefined function: compose_functors — the functor-composition "
                         "pair that also produces the bake's two [lenient] SKIPs (T1352/T1366)",
    "id_geometric": "undefined function: identity_functor — the same pair",
}


def stubs(path: pathlib.Path) -> dict[str, str]:
    out = subprocess.run(["strings", str(path)], capture_output=True, text=True)
    found: dict[str, str] = {}
    for name, why in STUB.findall(out.stdout):
        found.setdefault(name, why.strip())
    return found


def self_test() -> int:
    bad = 0
    line = ("[lenient] compose_geometric compiled to panic-stub: undefined "
            "function: compose_functors (in function compose_geometric)")
    m = STUB.search(line)
    if not m or m.group(1) != "compose_geometric":
        bad += 1
        print("self-test: the stub line is not parsed", file=sys.stderr)
    elif "compose_functors" not in m.group(2):
        bad += 1
        print("self-test: the REASON is not captured", file=sys.stderr)
    # the name must stop at the word `compiled`, not swallow it
    if m and m.group(1).endswith("compiled"):
        bad += 1
        print("self-test: the name ran into the message", file=sys.stderr)
    # a line that merely mentions a stub is not a stub record
    if STUB.search("a note about compiled to panic-stub behaviour"):
        bad += 1
        print("self-test: prose matched as a stub record", file=sys.stderr)
    print("self-test: ok" if not bad else f"self-test: {bad} FAILED")
    return bad


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    path = pathlib.Path(args[0]) if args else DEFAULT

    if not path.is_file():
        print(f"check-archive-panic-stubs: no archive at {path} — REFUSING to "
              f"report OK. A missing artifact is an unbuilt tree, not a clean "
              f"one. Build the stdlib, or pass the archive path.", file=sys.stderr)
        return 2
    mb = path.stat().st_size / (1024 * 1024)
    if mb < MIN_MB:
        print(f"check-archive-panic-stubs: {path} is {mb:.1f} MB, under the "
              f"{MIN_MB} MB floor — too small to be a real archive, so "
              f"'no stubs' would mean 'nothing read'.", file=sys.stderr)
        return 2

    found = stubs(path)
    print(f"check-archive-panic-stubs: {len(found)} panic-stub(s) in "
          f"{path.name} ({mb:.1f} MB), roster holds {len(KNOWN)}")
    for n in sorted(found):
        mark = "    " if n in KNOWN else "NEW "
        print(f"    {mark} {n}: {found[n]}")

    new = sorted(set(found) - set(KNOWN))
    gone = sorted(set(KNOWN) - set(found))
    rc = 0
    if new:
        rc = 1
        print(f"\n{len(new)} function(s) now ship as a panic-stub and are not on "
              f"the roster: {', '.join(new)}\nA stub is in the archive and "
              f"callable; calling it panics. Fix the undefined name it reports, "
              f"or add it to KNOWN in this file with the reason and a task.",
              file=sys.stderr)
    if gone:
        rc = 1
        print(f"\n{len(gone)} roster entry(ies) no longer stubbed: "
              f"{', '.join(gone)}\nRemove them from KNOWN so the ratchet holds "
              f"the ground gained — by NAME, not by a smaller number.",
              file=sys.stderr)
    return rc


if __name__ == "__main__":
    sys.exit(main())
