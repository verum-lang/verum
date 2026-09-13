#!/usr/bin/env python3
"""Gate: a `public mount .sub.{A, B, C};` may not name something `sub` does not declare.

A RE-EXPORT OF A NON-EXISTENT NAME IS SILENT AT THE PRODUCING END.
Nothing refuses `core/sys/mod.vr`; the failure surfaces only in whatever
mounts the umbrella, as `cannot find MemoryOrdering in module core.sys`,
and it surfaces there for every consumer at once. Measured 2026-09-12:
`core/sys/mod.vr` re-exported `MemoryOrdering` while `core/sys/common.vr`
declares `SysMemoryOrdering` — one word, and it took all 17 tests in
`core-tests/sys/mod/unit_test.vr` with it (A135).

Scope: file-relative re-exports only (`public mount .leaf.{…}`), because
those name a sibling file whose declarations this script can read without
resolving the module graph. That is also where the defect lives: the
umbrella pattern is what makes the producing end silent.

The declaration forms are taken from the grammar's own modifier list
rather than guessed — `public pure fn`, `public unsafe fn` and
`public async fn` are all real, and a first version of this scan that
knew only `public fn` reported four false positives in one file.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# RATCHET, not a clean sheet. Measured 2026-09-13 at 195 after the `core/sys`
# umbrella was repaired (28 stale names, every one of them a PRE-RENAME
# spelling: `Duration` for `EngineDuration`, `SocketAddr` for
# `RawSocketAddr`, `Mutex` for `LinuxMutex` / `DarwinMutex` …). The renames
# had been done to AVOID the A120 collisions — `core/sys/io_engine.vr:74`
# says so in as many words — and the umbrella kept advertising the old,
# colliding names, because nothing at the producing end asked.
#
# What remains is three distinct repairs, none of them verifiable from a
# macOS host and all of them larger than the gate that found them:
#
#   * `core/sys/windows/mod.vr` — 55: twelve renames (`Mutex` ->
#     `WindowsMutex`), eight names re-exported from the WRONG leaf
#     (`GENERIC_READ` is in `ntdll.vr`, listed under `kernel32`), and 35
#     that exist nowhere in the tree (`WriteConsoleA`, `HeapAlloc`, …).
#     `@cfg(target_os = "windows")` means none of it compiles here.
#   * `core/database/**` — about 70, mostly renamed façade types.
#   * `core/term`, `core/mem`, `core/math` — about 28.
#
# The 195 is the count AFTER the detector learned the dotless spelling
# (`public mount thread.{…}`, no leading dot). Before that it read 164 and
# reported ZERO findings in the windows umbrella, which carries 55 — the
# shape of a detector that has measured nothing. Anchored against the
# pre-fix `core/sys/mod.vr`, where it does find A135's `MemoryOrdering`.
#
# T1320 fought this same class in this same windows file and wrote the
# method down: rename where a same-meaning replacement exists one spelling
# away, DROP where nothing exists, and never leave a name that reads like a
# real one.
BASELINE = 195  # 196 before `core/mem/mod.vr` stopped re-exporting the renamed `Capability`
CORE = ROOT / "core"

# `public mount .leaf.{A, B as C, D};` — possibly spanning many lines.
#
# THE LEADING DOT IS OPTIONAL IN PRACTICE and a first version of this scan
# required it. `core/sys/windows/mod.vr:408` writes `public mount thread.{…}`
# with no dot, re-exporting eight names — `Mutex`, `Once`, `RwLock` … — that
# `windows/thread.vr` does not declare (it declares `WindowsMutex`,
# `WindowsOnce`, …). The gate reported zero findings in that file, which is
# the shape of a detector that has measured nothing.
#
# A dotless path that is NOT a sibling (`core.base.protocols`) simply fails
# to resolve to a file below and is skipped, so accepting both spellings
# cannot widen the gate beyond file-relative re-exports.
REEXPORT = re.compile(
    r"public\s+mount\s+\.?([A-Za-z_][A-Za-z0-9_.]*)\.\{(.*?)\}\s*;", re.S
)

# Everything that can introduce a name. `pure`, `unsafe`, `async`, `meta`
# and `const` may appear between `public` and the keyword.
MODIFIERS = r"(?:pub|public)\s+(?:(?:pure|unsafe|async|meta|extern|inline)\s+)*"
DECL = re.compile(
    MODIFIERS + r"(?:type|fn|const|static|module|context|protocol)\s+([A-Za-z_][A-Za-z0-9_]*)",
)
# A re-export inside the LEAF counts as a declaration for our purposes:
# the name is reachable through it.
LEAF_REEXPORT = re.compile(r"public\s+mount\s+[^;{]*\{(.*?)\}\s*;", re.S)

# A VARIANT IS A DECLARED NAME. `type Maybe<T> is None | Some(T);` makes
# `None` and `Some` re-exportable, and `core/base/mod.vr` re-exports both.
# A first version of this scan knew only the type's own name and reported
# 541 findings, nearly all of them variants — the kind of number that is
# "красивее или дико больше ожидаемого" and means the instrument is wrong.
TYPE_BODY = re.compile(
    MODIFIERS + r"type\s+[A-Za-z_][A-Za-z0-9_]*\s*(?:<[^>]*>)?\s+is\b(.*?);",
    re.S,
)
VARIANT_NAME = re.compile(r"\b([A-Z][A-Za-z0-9_]*)\s*(?:\{|\(|\||$)", re.M)


def strip_comments(text: str) -> str:
    """A name inside a COMMENT is not a re-exported item.

    The brace body of a `public mount` routinely carries `// …` notes, and
    reading them as items produced findings whose "name" was a sentence.
    """
    out = []
    for line in text.split("\n"):
        idx = line.find("//")
        out.append(line if idx < 0 else line[:idx])
    return "\n".join(out)


def declared_names(path: Path) -> set[str]:
    if not path.is_file():
        return set()
    text = strip_comments(path.read_text(encoding="utf-8", errors="replace"))
    names = {m.group(1) for m in DECL.finditer(text)}
    for m in TYPE_BODY.finditer(text):
        body = m.group(1)
        # Only a SUM type's body carries variants; a record body is `{ … }`
        # whose field names are lowercase and never match the regex anyway.
        for v in VARIANT_NAME.finditer(body):
            names.add(v.group(1))
    for m in LEAF_REEXPORT.finditer(text):
        for raw in m.group(1).split(","):
            item = raw.strip()
            if not item:
                continue
            if " as " in item:
                item = item.split(" as ")[-1].strip()
            if item:
                names.add(item)
    return names


def leaf_sources(mod_file: Path, leaf: str) -> list[Path]:
    """`.a.b` names `a/b.vr` or `a/b/mod.vr` relative to the mod file."""
    rel = leaf.replace(".", "/")
    base = mod_file.parent
    return [base / f"{rel}.vr", base / rel / "mod.vr"]


def main() -> int:
    findings: list[str] = []
    checked = 0
    for mod_file in sorted(CORE.rglob("mod.vr")):
        text = strip_comments(
            mod_file.read_text(encoding="utf-8", errors="replace")
        )
        for m in REEXPORT.finditer(text):
            leaf, body = m.group(1), m.group(2)
            sources = leaf_sources(mod_file, leaf)
            if not any(p.is_file() for p in sources):
                continue  # not a file-relative leaf we can read
            declared: set[str] = set()
            for p in sources:
                declared |= declared_names(p)
            if not declared:
                continue
            line_no = text.count("\n", 0, m.start()) + 1
            for raw in body.split(","):
                item = raw.strip()
                if not item:
                    continue
                source_name = item.split(" as ")[0].strip()
                if not source_name or not source_name[0].isalpha():
                    continue
                checked += 1
                if source_name not in declared:
                    findings.append(
                        f"{mod_file.relative_to(ROOT)}:{line_no}: "
                        f"`public mount .{leaf}.{{…}}` re-exports `{source_name}`, "
                        f"which `{leaf}` does not declare"
                    )
    if len(findings) > BASELINE:
        print(f"check-reexport-names: {len(findings)} finding(s), baseline {BASELINE}:\n")
        for f in findings:
            print(f"  {f}")
        print(
            "\nA re-exported name that the source module does not declare is "
            "silent here and fails at EVERY consumer of the umbrella."
        )
        return 1
    if findings:
        print(
            f"check-reexport-names: {len(findings)} finding(s), baseline {BASELINE} "
            f"— at or under baseline, not failing. Lower BASELINE as they are fixed; "
            f"{checked} re-exported name(s) checked."
        )
        return 0
    print(f"check-reexport-names: OK — {checked} re-exported name(s) all declared")
    return 0


if __name__ == "__main__":
    sys.exit(main())
