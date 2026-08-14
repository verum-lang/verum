#!/usr/bin/env python3
"""Ratchet: the size of the embedded stdlib archive.

The archive ships inside every `verum` binary, so its size is a product
property, not a build detail — and nothing measured it. A single commit took
it from 20.9 MB to 474.5 MB and every gate stayed green; it surfaced because a
line in a bake log happened to be read for another reason.

The cause is worth stating, because the shape recurs: two module-level tables
(`field_id_to_name`, `type_field_layouts`) are built from the codegen context,
which during a stdlib bake has accumulated EVERY type in the program. Written
per module, they were duplicated across all 574 archive members. Anything else
that is global-in-a-per-module-slot will blow up the same way.

Usage:
    check_archive_size.py <path-to-runtime.vbca>           # report
    check_archive_size.py <path-to-runtime.vbca> --check   # ratchet
"""

from __future__ import annotations

import pathlib
import sys

# Measured 2026-08-14 after the per-module duplication was removed:
# 21398.2 KB. The ceiling leaves room for genuine stdlib growth while still
# catching a duplication bug, which shows up as a MULTIPLE, not a percentage.
MAX_MB = 30.0

# Below this the file cannot be a real archive — a truncated or placeholder
# file must not read as "wonderfully small".
MIN_MB = 5.0


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    check = "--check" in sys.argv[1:]
    if len(args) != 1:
        print(__doc__, file=sys.stderr)
        return 2

    path = pathlib.Path(args[0])
    if not path.is_file():
        print(f"archive not found: {path}", file=sys.stderr)
        return 2

    mb = path.stat().st_size / (1024 * 1024)
    print(f"embedded stdlib archive: {mb:.1f} MB (ceiling {MAX_MB}, floor {MIN_MB})")

    if not check:
        return 0

    if mb > MAX_MB:
        print(
            f"RATCHET: the archive is {mb:.1f} MB, over the {MAX_MB} MB ceiling. "
            f"A jump of this kind is normally one table written per module that "
            f"belongs to the program as a whole — check what the last change "
            f"added to the serializer.",
            file=sys.stderr,
        )
        return 1
    if mb < MIN_MB:
        print(
            f"RATCHET: the archive is {mb:.1f} MB, under the {MIN_MB} MB floor. "
            f"That is not a small archive, that is a broken one — a truncated "
            f"write or a placeholder.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
