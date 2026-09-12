#!/usr/bin/env python3
"""Gate: `implement <Generic> { }` must carry its type parameter.

WHY THIS EXISTS, measured 2026-09-12 and not hypothetical.
`core/sys/no_runtime.vr` wrote

    public type SyncChannel<T> is Empty | Full(T);
    implement SyncChannel {            # <- no <T>
        public fn new() -> SyncChannel<T> { … }
        public fn send(&mut self, value: T) -> Bool { … }
    }

and the block half-applied: `SyncChannel.new()` CONSTRUCTS, while
`c.send(7)` panics with `method 'SyncChannel.send' not found on
receiver`. The same shape on `NoOpMutex<T>` in the same file refused
`m.lock()` at check time. Forty-three tests across four core-tests files
were compile-dead behind it, and nothing else complained: a type whose
constructor works looks wired.

WHAT IT CHECKS. For every `implement <Name> {` with no `<…>` after the
name, whether THAT FILE declares `<Name>` with type parameters. The
same-file restriction is not laziness — it is the whole correctness of
the check. A scan that looked repo-wide reported five offenders and
three were name COLLISIONS: `Command` is non-generic in
`core/io/process.vr` and generic in `core/term/app/command.vr`,
`Database` and `WatchEvent` likewise. An `implement` block applies to
the declaration in scope, so the file is the right unit.

NOT CHECKED, deliberately: `implement Protocol for Type`, which has its
own parameter rules, and blocks whose type is declared in another module
— there the mount decides and this gate would be guessing.
"""
from __future__ import annotations
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CORE = REPO / "core"

GENERIC_DECL = re.compile(r"^(?:public\s+)?type\s+([A-Z][A-Za-z0-9]*)\s*<", re.M)
BARE_IMPL = re.compile(r"^implement\s+([A-Z][A-Za-z0-9]*)\s*\{", re.M)


def offenders() -> list[tuple[str, str]]:
    out: list[tuple[str, str]] = []
    for path in sorted(CORE.rglob("*.vr")):
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        generics = set(GENERIC_DECL.findall(text))
        if not generics:
            continue
        for name in BARE_IMPL.findall(text):
            if name in generics:
                out.append((name, str(path.relative_to(REPO))))
    return out


def self_test() -> int:
    bad = 0
    same_file = (
        "public type Box<T> is { v: T };\n"
        "implement Box {\n"
        "    public fn new(v: T) -> Box<T> { Box { v: v } }\n"
        "}\n"
    )
    if "Box" not in set(GENERIC_DECL.findall(same_file)):
        print("self-test: the generic-declaration pattern missed `type Box<T>`")
        bad += 1
    if "Box" not in BARE_IMPL.findall(same_file):
        print("self-test: the bare-implement pattern missed `implement Box {`")
        bad += 1
    # The CORRECT spelling must not be reported.
    if BARE_IMPL.findall("implement<T> Box<T> {\n}\n"):
        print("self-test: `implement<T> Box<T> {` was read as a bare implement")
        bad += 1
    # A non-generic type with the same name in another file is the
    # collision case this gate must NOT report; the same-file rule is
    # what prevents it, so pin that the declaration set is per-file.
    other_file = "public type Box is { v: Int };\nimplement Box {\n}\n"
    if GENERIC_DECL.findall(other_file):
        print("self-test: a NON-generic declaration was read as generic — "
              "the collision cases would be reported")
        bad += 1
    print("self-test: OK" if not bad else f"self-test: {bad} FAILED")
    return bad


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    found = offenders()
    if not found:
        print("check-implement-generics: no `implement <Generic> {` without "
              "its type parameter")
        return 0
    print(f"check-implement-generics: {len(found)} `implement` block(s) name a "
          "type this file declares WITH type parameters and give none:")
    for name, path in found:
        print(f"    implement {name} {{   in {path}")
    print("\nThe block half-applies: static methods resolve and every `&self` "
          "method is lost, silently. Write `implement<T> Name<T> {`.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
