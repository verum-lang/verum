#!/usr/bin/env python3
"""Fail when core/ calls a platform module for something that module lacks.

WHY THIS EXISTS
---------------
A call through a module path is not resolved by the compiler (T0806): a
missing leaf or a missing middle segment is accepted silently and evaluates
to `nil`, which satisfies whatever return type the caller declared.  The
platform layer is where that hurts most, because the arm that misses is the
one that does NOT run on the machine you are testing on.

Unlike the general case, the target here is unambiguous:

    sys.darwin.thread.park(...)   ->  core/sys/darwin/thread.vr

so this gate can ask the question the general one cannot — "does the module
the path NAMES actually provide this?" — without guessing across re-export
chains.  Where the path stops at a package (`super.darwin.exit`), the target
is that package's `mod.vr` and re-exports do matter, so both forms are
honoured:

    public mount X.*;               everything X declares
    public mount .X.{ a, b as c };  the listed names, under their new names

WHAT IT FOUND WHEN IT LANDED (T0808, T0807)
-------------------------------------------
    core/sys/init.vr:353-354   super.darwin.write / super.darwin.exit
        The default panic handler. Neither name is among the 159 that
        core/sys/darwin/mod.vr re-exports, though both are declared in
        libsystem.vr. On macOS a panic therefore printed nothing and did
        not abort. The same file spells it correctly at line 613 —
        `sys.darwin.libsystem.exit(1)` — which is what makes the form so
        easy to get wrong.

    core/mem/heap.vr:1265       sys.linux.random.getrandom
        No such module. This is the seeding of the heap free-list pointer
        encoding keys, so the "random" keys stayed at their compile-time
        constants.

    core/sys/common.vr:1252     sys.windows.bcrypt.BCryptGenRandom
        Inside `random_bytes`, the library's one public randomness source.
        The same absent module as heap.vr:1281 — so on Windows the source
        and its most security-sensitive consumer fail together, and fixing
        the module fixes both.

    plus park / unpark / thread_yield / thread_join / get_errno /
    query_performance_counter_ns / flock / GetFileAttributesW /
    SetEndOfFile / GetCommandLineA, each called against a platform module
    that does not provide it.

Thirty-two in all, against 130 that resolve. Note what the ratio means: a
platform call in core/ has better than a one-in-five chance of naming
nothing, and none of them is a compile error.

A private `mount` is not a re-export — the grammar gives that role to
`public mount` alone — so `kernel32.Handle(...)` is counted as unresolved
even though kernel32.vr mounts `Handle` from ntdll for its own use. The
spelling that works from outside is `ntdll.Handle`.

WHAT IT DOES NOT CHECK
----------------------
Arity, which is equally unchecked through a module path (T0806) and equally
silent. A call with the right name and the wrong argument count passes this
gate and still returns nil.
"""

from __future__ import annotations

import re
import sys
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SYS = REPO / "core" / "sys"
CORE = REPO / "core"

PLATFORMS = ("linux", "darwin", "windows", "freebsd")

# `sys.<plat>…`, `core.sys.<plat>…`, and — only from a file sitting directly
# in core/sys/ — `super.<plat>…`, where `super` is `core.sys`.
ABS_CALL = re.compile(
    r"\b(?:core\.sys|sys)\.(linux|darwin|windows|freebsd)"
    r"((?:\.[a-z_][a-z0-9_]*)*)\.([A-Za-z_][A-Za-z0-9_]*)\s*\("
)
SUPER_CALL = re.compile(
    r"\bsuper\.(linux|darwin|windows|freebsd)"
    r"((?:\.[a-z_][a-z0-9_]*)*)\.([A-Za-z_][A-Za-z0-9_]*)\s*\("
)

# A callable name is not always a `fn`: `kernel32.Handle(raw)` is a newtype
# constructor, and a scan that knows only functions reports nine of those as
# missing symbols.  Types and constants therefore count as provided names.
DECL = re.compile(
    r"^[ \t]*(?:(?:public|pub)\s+)?(?:async\s+|unsafe\s+|pure\s+)*fn\s+([A-Za-z_]\w*)"
    r"|^[ \t]*(?:(?:public|pub)\s+)?type\s+(?:(?:affine|linear)\s+)?([A-Za-z_]\w*)"
    r"|^[ \t]*(?:(?:public|pub)\s+)?const\s+([A-Za-z_]\w*)",
    re.M,
)
GLOB_REEXPORT = re.compile(r"^\s*public\s+mount\s+\.?([a-z_]\w*)\s*\.\s*\*\s*;", re.M)
LIST_REEXPORT = re.compile(r"public\s+mount\s+\.?([a-z_]\w*)\.\{(.*?)\}\s*;", re.S)

BASELINE = 29

_declared: dict[Path, set[str]] = {}


def shown_path(path: Path) -> str:
    """Repo-relative when possible. The self-check runs this gate against a
    scratch tree outside the repo, and an unguarded relative_to turns a
    correct finding into a traceback."""
    try:
        return str(path.relative_to(REPO))
    except ValueError:
        return str(path)


def declared_in(path: Path) -> set[str]:
    if path not in _declared:
        _declared[path] = {
            name
            for groups in DECL.findall(path.read_text(errors="ignore"))
            for name in groups
            if name
        }
    return _declared[path]


_visible: dict[Path, set[str]] = {}


def visible_through(path: Path) -> set[str]:
    """Names reachable through `path`: its own declarations plus whatever its
    re-export statements pull up from siblings.  One hop is enough here — a
    platform `mod.vr` re-exports the leaf modules beside it, and those
    declare rather than forward."""
    if path in _visible:
        return _visible[path]
    text = path.read_text(errors="ignore")
    names = {name for groups in DECL.findall(text) for name in groups if name}
    here = path.parent
    for sibling in GLOB_REEXPORT.findall(text):
        target = here / f"{sibling}.vr"
        if target.is_file():
            names |= declared_in(target)
    for sibling, body in LIST_REEXPORT.findall(text):
        # `a, b as c` — the name it is visible AS is what callers write.
        for original, renamed in re.findall(r"\b([A-Za-z_]\w*)\b(?:\s+as\s+(\w+))?", body):
            names.add(renamed or original)
    _visible[path] = names
    return names


def target_module(platform: str, segments: list[str]) -> Path | None:
    base = SYS.joinpath(platform, *[s for s in segments if s])
    for candidate in (base.with_suffix(".vr"), base / "mod.vr"):
        if candidate.is_file():
            return candidate
    return None


def scan() -> tuple[dict, list, int]:
    missing: dict[tuple, list[str]] = defaultdict(list)
    no_module: list[tuple[str, str]] = []
    resolved = 0

    for path in sorted(CORE.rglob("*.vr")):
        text = path.read_text(errors="ignore")
        # `super` means `core.sys` only for a file directly in core/sys/.
        patterns = [ABS_CALL]
        if path.parent == SYS:
            patterns.append(SUPER_CALL)
        for lineno, line in enumerate(text.splitlines(), 1):
            if line.lstrip().startswith("//"):
                continue
            for pattern in patterns:
                for match in pattern.finditer(line):
                    platform, middle, leaf = match.group(1), match.group(2), match.group(3)
                    segments = [s for s in middle.strip(".").split(".") if s]
                    module = target_module(platform, segments)
                    rel = shown_path(path)
                    if module is None:
                        dotted = f"sys.{platform}{middle}"
                        no_module.append((f"{rel}:{lineno}", f"{dotted}.{leaf}"))
                        continue
                    if leaf in visible_through(module):
                        resolved += 1
                    else:
                        key = (platform, "/".join(segments) or "mod", leaf)
                        missing[key].append(f"{rel}:{lineno}")
    return missing, no_module, resolved


def self_test() -> int:
    """The extractor has one failure mode that does not announce itself: a
    `^`-anchored DECL applied to whole-file text without re.M matches only
    the first line, which reports every call as missing.  The first run of
    this scan did exactly that — 0 of 51 calls resolved — and 0-of-N reads
    like a catastrophic finding rather than a broken instrument.  So the
    control is that a known-large platform module yields many names."""
    probe = SYS / "darwin" / "libsystem.vr"
    if not probe.is_file():
        print("self-test: core/sys/darwin/libsystem.vr is missing", file=sys.stderr)
        return 1
    names = declared_in(probe)
    if len(names) < 50:
        print(
            f"self-test: libsystem.vr yielded {len(names)} declarations — "
            "the extractor is broken, not the library",
            file=sys.stderr,
        )
        return 1
    if "arc4random_buf" not in names or "write" not in names:
        print("self-test: known libsystem symbols absent from the extraction", file=sys.stderr)
        return 1
    print(f"[ok] self-test: extractor sees {len(names)} declarations in libsystem.vr")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()

    missing, no_module, resolved = scan()
    total = sum(len(v) for v in missing.values()) + len(no_module)

    if "--list" in sys.argv or total > BASELINE:
        stream = sys.stderr if total > BASELINE else sys.stdout
        print(
            f"platform calls: {resolved} resolved, {total} unresolved "
            f"(baseline {BASELINE})",
            file=stream,
        )
        for (platform, module, leaf), sites in sorted(missing.items()):
            print(f"  core/sys/{platform}/{module}.vr does not provide `{leaf}`", file=stream)
            for site in sites:
                print(f"      {site}", file=stream)
        for site, dotted in no_module:
            print(f"  no such module: {dotted}\n      {site}", file=stream)

    if total > BASELINE:
        print(
            "\nEach of these evaluates to `nil` on the platform it targets, and\n"
            "nothing reports it — including the compiler, and including a test\n"
            "run on a different platform.",
            file=sys.stderr,
        )
        return 1
    if total < BASELINE:
        print(f"platform calls: {total} unresolved, below baseline {BASELINE} — lower BASELINE.")
        return 1
    print(f"[ok] platform-call parity: {resolved} resolved, {total} known-unresolved, none new")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
