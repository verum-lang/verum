#!/usr/bin/env python3
"""Landed-markers fence: assert every manifest marker is present.

Manifest: docs/architecture/landed-markers.txt
Line format: `<repo-relative-path> :: <literal substring>`

A missing marker means a LANDED mechanism was reverted or clobbered
(the 2026-07-12 multi-session incident silently erased 237 committed
lines through a botched staged merge). This check turns that silent
regression class into a loud sub-second failure, suitable for CI and
for `make check-markers` before any bake/acceptance run.

Exit codes: 0 all present; 1 missing markers (listed); 2 manifest or
file unreadable.
"""

from __future__ import annotations

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
MANIFEST = REPO / "docs" / "architecture" / "landed-markers.txt"


def main() -> int:
    try:
        lines = MANIFEST.read_text(encoding="utf-8").splitlines()
    except OSError as e:
        print(f"check_landed_markers: cannot read manifest: {e}", file=sys.stderr)
        return 2

    missing: list[str] = []
    unreadable: list[str] = []
    checked = 0
    cache: dict[str, str] = {}

    for raw in lines:
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if " :: " not in line:
            print(
                f"check_landed_markers: malformed manifest line: {line!r}",
                file=sys.stderr,
            )
            return 2
        rel, marker = (part.strip() for part in line.split(" :: ", 1))
        if rel not in cache:
            try:
                cache[rel] = (REPO / rel).read_text(encoding="utf-8")
            except OSError:
                unreadable.append(rel)
                cache[rel] = ""
        checked += 1
        if marker not in cache[rel]:
            missing.append(f"{rel} :: {marker}")

    if unreadable:
        for rel in sorted(set(unreadable)):
            print(f"UNREADABLE {rel}", file=sys.stderr)
    if missing:
        print(
            f"check_landed_markers: {len(missing)} of {checked} markers MISSING "
            "— a landed mechanism was reverted or clobbered:",
            file=sys.stderr,
        )
        for m in missing:
            print(f"  MISSING {m}", file=sys.stderr)
        return 1

    print(f"check_landed_markers: OK ({checked} markers present)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
