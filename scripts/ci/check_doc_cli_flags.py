#!/usr/bin/env python3
"""Every CLI flag the documentation shows must exist in the binary.

Measured 2026-09-05 on the website docs: 17 commands, 40 flag mentions,
FIVE flags that no `--help` lists — `check --tier-report` (three times,
in the memory-safety tutorial, with a per-line output format the tool
never printed), `repl --session`, `repl --no-project`, `doc --search`,
`audit --filter`. One more, `api --signature`, was already marked. A
reader following any of them gets `error: unexpected argument`.

The class this catches is not "a typo". It is a flag that WAS real and
was renamed, and prose that kept the old name because nothing executes
prose. The homepage carried the same class in a different shape: a
sample showing `verum analyze --escape` output that the command does
not print.

WHAT IS DEEMED FINE
  * A line whose comment says NOT IMPLEMENTED / does not exist / not
    supported. Documenting a gap is the correct thing to do and must
    not fail the gate; the gap being documented is the point.
  * `-h`, `--help`, `--version` — universal, and not always echoed in
    a subcommand's own help text.

INSTRUMENT CONTROL
  The check runs `<binary> <cmd> --help` and reads the flags out of it.
  If that produces NO flags at all the command is reported as
  unreadable rather than silently contributing zero findings — a
  `--help` that fails would otherwise make every flag look present by
  making the "does not have it" branch unreachable.
"""
from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
DOCS_ENV = os.environ.get("VERUM_DOCS_DIR")
DOCS = Path(DOCS_ENV) if DOCS_ENV else REPO.parent / "website" / "docs"

# `$ verum <cmd> …` — the flags are every `--flag` on the rest of the
# line, NOT only the ones before the first positional. Caught by this
# file's own self-test: `repl --preload x.vr --skip-verify` reported one
# flag when the pattern required the flags to be contiguous, so a flag
# sitting after a filename was invisible — the exact place a stale flag
# survives longest.
#
# The tail stops at a pipe or `&&`: `verum doc | grep --color` names a
# flag of grep, not of verum.
INVOCATION = re.compile(r"\$ verum ([a-z][a-z0-9-]*)((?:(?! *[|&;#])[^\n])*)")
FLAG = re.compile(r"--[a-z][a-z0-9-]*")
EXCUSED = re.compile(
    r"#[^\n]*\b(NOT IMPLEMENTED|does not exist|not implemented|"
    r"not yet supported|not supported)\b", re.I)
UNIVERSAL = {"--help", "--version"}


def binary() -> str:
    return os.environ.get("VERUM_BIN") or str(REPO / "target" / "debug" / "verum")


def shown(docs: Path) -> dict[str, dict[str, list[str]]]:
    """{command: {flag: [where, …]}} for every flag the docs show."""
    out: dict[str, dict[str, list[str]]] = {}
    for md in sorted(docs.rglob("*.md")):
        try:
            text = md.read_text(errors="ignore")
        except OSError:
            continue
        for line in text.split("\n"):
            m = INVOCATION.search(line)
            if not m or EXCUSED.search(line):
                continue
            cmd = m.group(1)
            tail = m.group(2)
            # `--` ends verum's own arguments: `verum run -- --json a.txt`
            # passes `--json` to the PROGRAM. Measured — without this the
            # gate reports the program's flag as a missing verum flag.
            tail = tail.split(" -- ", 1)[0]
            # Leading bare words are a SUBCOMMAND PATH, not arguments:
            # `verum cog-registry publish --manifest` asks about
            # `cog-registry publish`, whose flags `cog-registry --help`
            # does not list. Measured — treating them as one command
            # reported nine flags that all exist, one level down.
            words = []
            for w in tail.split():
                if w.startswith("-"):
                    break
                if not re.fullmatch(r"[a-z][a-z0-9-]*", w):
                    break
                words.append(w)
            path = " ".join([cmd] + words)
            for fl in FLAG.findall(tail):
                if fl in UNIVERSAL:
                    continue
                out.setdefault(path, {}).setdefault(fl, []).append(
                    md.relative_to(docs).as_posix())
    return out


def real_flags(bin_path: str, cmd: str) -> set[str] | None:
    """Flags `<cmd> --help` lists, or None when the help is unreadable.

    `cmd` may be a subcommand PATH ("cog-registry publish").
    """
    try:
        p = subprocess.run([bin_path, *cmd.split(), "--help"],
                           capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired):
        return None
    found = set(FLAG.findall(p.stdout + p.stderr))
    # Every subcommand's help lists at least --help. Nothing at all means
    # the command does not exist or the help failed — say so, do not
    # quietly report its flags as present.
    return found or None


SELF_TEST = [
    # (line, expected (cmd, flags) or None)
    ("$ verum audit --bundle", ("audit", ["--bundle"])),
    # A subcommand path, not a command plus an argument.
    ("$ verum cog-registry publish --manifest cog.json",
     ("cog-registry publish", ["--manifest"])),
    ("$ verum repl --preload x.vr --skip-verify", ("repl", ["--preload", "--skip-verify"])),
    # An excused line contributes nothing — documenting a gap is correct.
    ("$ verum api --signature \"fn map\"       # NOT IMPLEMENTED", None),
    ("$ verum doc --search x   # does not exist", None),
    # Prose that merely names a command is not an invocation.
    ("Run verum test --workspace to check everything", None),
    # `--help` alone is universal and never reported.
    ("$ verum build --help", ("build", [])),
    # Everything after `--` belongs to the program being run, not verum.
    ("$ verum run -- --json /tmp/a.txt", ("run", [])),
    ("$ verum run --release -- --json a.txt", ("run", ["--release"])),
]


def self_test() -> int:
    bad = 0
    for line, want in SELF_TEST:
        m = INVOCATION.search(line)
        got = None
        if m and not EXCUSED.search(line):
            tail = m.group(2).split(" -- ", 1)[0]
            words = []
            for w in tail.split():
                if w.startswith("-") or not re.fullmatch(r"[a-z][a-z0-9-]*", w):
                    break
                words.append(w)
            flags = [f for f in FLAG.findall(tail) if f not in UNIVERSAL]
            got = (" ".join([m.group(1)] + words), flags)
        if got != want:
            bad += 1
            print(f"FAIL {line!r} -> {got}, expected {want}", file=sys.stderr)
    if bad:
        print(f"self-test: {bad} case(s) FAILED", file=sys.stderr)
        return 1
    print(f"self-test: {len(SELF_TEST)} case(s) OK")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    bin_path = binary()
    if not Path(bin_path).exists():
        print(f"check-doc-cli-flags: no verum binary at {bin_path}, skipped")
        return 0
    if not DOCS.is_dir():
        print(f"check-doc-cli-flags: no docs directory at {DOCS}, skipped")
        return 0

    pairs = shown(DOCS)
    missing: list[tuple[str, str, list[str]]] = []
    unreadable: list[str] = []
    checked = 0
    for cmd, flags in sorted(pairs.items()):
        real = real_flags(bin_path, cmd)
        if real is None:
            unreadable.append(cmd)
            continue
        for fl, where in sorted(flags.items()):
            checked += 1
            if fl not in real:
                missing.append((cmd, fl, sorted(set(where))))

    if unreadable:
        print("[fail] the docs invoke commands whose `--help` says nothing:")
        for cmd in unreadable:
            print(f"    verum {cmd}")
        print("\nEither the subcommand is gone, or `--help` failed. Both are\n"
              "findings: a flag cannot be checked against a help text that is\n"
              "not there, so this is reported rather than counted as clean.")
        return 1

    if missing:
        print(f"[fail] {len(missing)} flag(s) the docs show and the binary rejects:")
        for cmd, fl, where in missing:
            print(f"    verum {cmd} {fl}    — {', '.join(where[:3])}")
        print("\nRun the command to see what it does have. If the flag is gone,\n"
              "name the replacement; if it never existed, say so in the line's\n"
              "own comment (`# NOT IMPLEMENTED`) — a documented gap passes.")
        return 1

    print(f"check-doc-cli-flags: {len(pairs)} command(s), {checked} flag mention(s), 0 unknown")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
