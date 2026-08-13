#!/usr/bin/env python3
"""Ratchet: FIELD-GUESS-HARD-1 diagnostics and panic stubs in a stdlib bake.

Both counts are the visible surface of defects that do NOT fail the build:

  * `error[FIELD-GUESS-HARD-1]` — a field's position was GUESSED by scanning
    every type in the program and taking the most-fields candidate. The
    message says "error", the bake proceeds, and the emitted index may be a
    foreign slot or out of bounds. 167 of these were real defects: a variant
    the type never declared, a field renamed at the declaration and not at the
    call, a type name resolving nowhere.

  * `[lenient] SKIP` — a function that FAILED to compile and was replaced by a
    stub that panics at runtime. 31 of these shipped in the archive.

Neither number can be read off the source tree; both come from running a bake.
The gate therefore takes a bake log rather than re-baking (a bake is ~12
minutes), and CI is expected to pass the log it already produced.

Usage:
    check_bake_diagnostics.py <bake-log>            # report
    check_bake_diagnostics.py <bake-log> --check    # ratchet, exit 1 on drift
"""

from __future__ import annotations

import pathlib
import sys

# Frozen at the measured counts. What remains is ONE defect class: an API
# sketched against a PROTOCOL and never written — `SheafInfinityTopos` built
# with a record literal (the field guess), and `compose_geometric` /
# `id_geometric` over `InfinityFunctor` (the two stubs). The floor is not zero
# until those types exist, and writing them from a call site would be
# inventing the design nobody wrote.
#
# `QuicStream` used to be counted here and was NOT that class: api/stream.vr
# declares `QuicApiStream` with exactly the three fields the call sites build,
# and the bare name resolved to the unrelated transport-level record. Renaming
# the uses took FIELD-GUESS from 2 to 1 — hence this baseline moving in the
# same commit that earned it.
# Lower these in the same commit that earns it.
BASELINE_FIELD_GUESS = 1
BASELINE_PANIC_STUBS = 2


def counts(log_text: str) -> tuple[int, int]:
    guesses = sum(1 for line in log_text.splitlines() if "FIELD-GUESS-HARD-1" in line)
    stubs = sum(1 for line in log_text.splitlines() if "[lenient] SKIP" in line)
    return guesses, stubs


def main() -> int:
    args = [a for a in sys.argv[1:] if a != "--check"]
    check = "--check" in sys.argv[1:]
    if len(args) != 1:
        print(__doc__, file=sys.stderr)
        return 2

    path = pathlib.Path(args[0])
    if not path.is_file():
        print(f"bake log not found: {path}", file=sys.stderr)
        return 2

    text = path.read_text(encoding="utf-8", errors="ignore")
    guesses, stubs = counts(text)

    # A log that contains NEITHER marker is far more likely to be the wrong
    # file — or a bake that died early — than a perfect bake. Refuse to report
    # a clean sheet we cannot distinguish from an empty one.
    if "Modules compiled" not in text and "Archive size" not in text:
        print(
            f"{path} does not look like a completed bake log (no 'Modules compiled' "
            f"or 'Archive size' line). Refusing to read 0/0 as success.",
            file=sys.stderr,
        )
        return 2

    print(f"FIELD-GUESS-HARD-1 : {guesses} (baseline {BASELINE_FIELD_GUESS})")
    print(f"[lenient] SKIP     : {stubs} (baseline {BASELINE_PANIC_STUBS})")

    if not check:
        return 0

    failed = False
    for name, got, want in (
        ("FIELD-GUESS-HARD-1", guesses, BASELINE_FIELD_GUESS),
        ("panic stubs", stubs, BASELINE_PANIC_STUBS),
    ):
        if got > want:
            print(
                f"RATCHET: {name} rose to {got} (baseline {want}). Each one is a "
                f"defect the build does NOT fail on — a guessed field index or a "
                f"function replaced by a runtime panic.",
                file=sys.stderr,
            )
            failed = True
        elif got < want:
            print(
                f"RATCHET: {name} dropped to {got} (baseline {want}). Lower the "
                f"baseline in the same commit that earns it — a silently improving "
                f"number is how a gate stops measuring.",
                file=sys.stderr,
            )
            failed = True

    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
