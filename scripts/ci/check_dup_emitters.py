#!/usr/bin/env python3
"""DUP-EMITTER-GATE (T0438, design in the T0208 journal): every `verum_*`
symbol has at most ONE defining emitter in crates/verum_codegen/src.

WHY A SOURCE CENSUS, NOT A MODULE WALK (the load-bearing design fact):
the passive `count_basic_blocks() == 0` guard means the LOSING emitter
leaves NO trace in the final LLVM module — a post-IR walk reports zero
duplicates forever. Only reading the Rust emitter source sees both
sites. A silent first-wins between a libc body and a syscall body is
exactly how the no-libc invariant broke unnoticed (T0436).

Census idiom (from the T0208 audit): a DEFINITION site is a binding

    let f = …add_function(…"verum_X"…) / get_or_declare_function(…)
    …
    append_basic_block(f, …)            # in the same enclosing fn

Declarations without a body-append and plain `build_call` uses never
match, so calls cannot false-positive. The census keys on the ENCLOSING
Rust fn, so an intra-fn self-rebuild counts as one definer.

Policy (mirrors the T0424 ratchet): `dup_emitters_known.txt` carries the
audited inventory — each line `symbol<TAB>bucket<TAB>task-ref`. A
duplicate NOT in that file fails the build naming both sites; a listed
symbol that is no longer duplicated asks to be removed from the file.
Bucket [C] entries (intentional last-wins overrides) are permanent
allowlist; buckets [A]/[B]/[D] are debt tracked by their task refs.
"""

import pathlib
import re
import sys
from collections import defaultdict

ROOT = pathlib.Path(__file__).resolve().parents[2]
TARGET = ROOT / "crates" / "verum_codegen" / "src"
KNOWN_FILE = pathlib.Path(__file__).resolve().parent / "dup_emitters_known.txt"

BIND_RE = re.compile(
    r"let\s+(?:mut\s+)?(\w+)(?::\s*[\w:<>'&\s]+)?\s*=\s*[^;]*?"
    r"(?:add_function|get_or_declare_function|get_or_declare_fn|"
    r"get_or_declare_noreturn_function)\s*\([^;]*?\"(verum_\w+)\"",
    re.S,
)
FN_RE = re.compile(r"^\s*(?:pub(?:\([\w:]+\))?\s+)?fn\s+(\w+)", re.M)


def enclosing_fns(text):
    """[(start_offset, fn_name)] sorted — nearest preceding fn is the encloser."""
    return [(m.start(), m.group(1)) for m in FN_RE.finditer(text)]


def fn_for_offset(fns, off):
    name = "<top>"
    for start, fname in fns:
        if start > off:
            break
        name = fname
    return name


def line_of(text, off):
    return text.count("\n", 0, off) + 1


def collect_definers():
    """symbol -> {(file, enclosing_fn, line_of_first_bind)}"""
    definers = defaultdict(set)
    for path in sorted(TARGET.rglob("*.rs")):
        text = path.read_text(encoding="utf-8", errors="replace")
        fns = enclosing_fns(text)
        rel = str(path.relative_to(ROOT))
        for m in BIND_RE.finditer(text):
            var, sym = m.group(1), m.group(2)
            encloser = fn_for_offset(fns, m.start())
            # A definition needs a body-append of the SAME bound variable
            # within the same enclosing fn region — the window ends at the
            # next fn item, so a declaration-fallback binding named `f` /
            # `func` can never borrow an append from a LATER function
            # (the false-positive class the first calibration run hit on
            # verum_os_exit / verum_panic declaration sites).
            next_fn = next((start for start, _ in fns if start > m.end()), len(text))
            window = text[m.end() : next_fn]
            if re.search(r"append_basic_block\(\s*" + re.escape(var) + r"\b", window):
                definers[sym].add((rel, encloser, line_of(text, m.start())))
    return definers


# ARM B (T0438 second arm — SEPARATE by design): a lone libc-only emitter
# with no duplicate trips neither Arm A nor link-inspection (the loser's
# libc call is never emitted into IR; the linked binary shows only
# survivors). Only reading the emitter SOURCE sees "dead today, one flip
# from live". Forbidden set mirrors check_no_libc_link.sh; an emitter
# body referencing one of these as an extern WITHOUT a co-located
# target_is_linux syscall path is flagged. The legit
# `if target_is_linux { syscall } else { libSystem extern }` pattern
# (macOS boundary is ALLOWED per CLAUDE.md) never fires: the
# target_is_linux mention itself clears the emitter.
FORBIDDEN_LIBC = [
    "clock_gettime",
    "nanosleep",
    "access",
    "unlink",
    "getenv",
    "malloc",
    "free",
    "fopen",
    "fread",
    "fwrite",
    "fclose",
]
LIBC_EXTERN_RE = re.compile(
    r"(?:add_function|get_or_declare_function)\s*\([^;]*?\"("
    + "|".join(FORBIDDEN_LIBC)
    + r")\"",
)


def collect_libc_suspects():
    """[(file, enclosing_fn, line, libc_symbol)] for emitter fns that
    reference a forbidden libc extern with NO target_is_linux path."""
    suspects = []
    for path in sorted(TARGET.rglob("*.rs")):
        text = path.read_text(encoding="utf-8", errors="replace")
        fns = enclosing_fns(text)
        rel = str(path.relative_to(ROOT))
        for i, (start, fname) in enumerate(fns):
            end = fns[i + 1][0] if i + 1 < len(fns) else len(text)
            body = text[start:end]
            m = LIBC_EXTERN_RE.search(body)
            if not m:
                continue
            if "target_is_linux" in body or "emit_linux_syscall" in body:
                continue
            suspects.append((rel, fname, line_of(text, start + m.start()), m.group(1)))
    return suspects


def load_known():
    known = {}
    if KNOWN_FILE.exists():
        for raw in KNOWN_FILE.read_text().splitlines():
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            known[parts[0]] = (
                parts[1] if len(parts) > 1 else "?",
                parts[2] if len(parts) > 2 else "?",
            )
    return known


def main():
    definers = collect_definers()
    dups = {s: sites for s, sites in definers.items() if len(sites) > 1}
    known = load_known()

    new_dups = {s: v for s, v in dups.items() if s not in known}
    # libc:* entries belong to Arm B — stale for them means the flagged
    # fn no longer references a forbidden extern.
    libc_fns = {f"libc:{fname}" for (_, fname, _, _) in collect_libc_suspects()}
    stale = [
        s
        for s in known
        if (s.startswith("libc:") and s not in libc_fns)
        or (not s.startswith("libc:") and s not in dups)
    ]

    if new_dups:
        print("check-dup-emitters: FAIL — NEW duplicate verum_* definers:")
        for sym, sites in sorted(new_dups.items()):
            print(f"  {sym}:")
            for rel, fname, line in sorted(sites):
                print(f"    {rel}:{line} (fn {fname})")
        print(
            "\nOne symbol, one definer. If this override is INTENTIONAL "
            "last-wins, add it to scripts/ci/dup_emitters_known.txt with "
            "bucket [C] and a task reference; otherwise delete one body."
        )
        return 1

    libc = collect_libc_suspects()
    new_libc = [
        (rel, fname, line, sym)
        for (rel, fname, line, sym) in libc
        if f"libc:{fname}" not in known
    ]
    if new_libc:
        print(
            "check-dup-emitters: FAIL — emitter bodies reference a "
            "forbidden-on-Linux libc extern with NO target_is_linux "
            "syscall path (Arm B — the dead-body no-libc landmine, T0436 class):"
        )
        for rel, fname, line, sym in new_libc:
            print(f"  {rel}:{line} (fn {fname}) references \"{sym}\"")
        print(
            "\nEmit the Linux leg via direct syscalls (target_is_linux + "
            "emit_linux_syscall) or delete the dead body; a deliberate "
            "exception goes into dup_emitters_known.txt as "
            "libc:<fn-name><TAB>[L]<TAB><task-ref>."
        )
        return 1

    if stale:
        print(
            "check-dup-emitters: OK, but these known entries are no longer "
            "duplicated — remove them from dup_emitters_known.txt:"
        )
        for s in sorted(stale):
            print(f"  {s}")
        return 0

    print(
        f"check-dup-emitters: OK — {len(definers)} verum_* definers, "
        f"{len(dups)} known duplicates (tracked in dup_emitters_known.txt), 0 new"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
