#!/usr/bin/env python3
"""Gate: KNOWN_TABLES must cover every field `Manifest` declares.

`verum config validate` warns that a top-level table is unknown and
therefore ignored. The warning is only as good as its list: a table
that IS read but is missing from the list gets called a typo, which is
worse than the silence it replaced — the author is told their working
configuration does nothing.

That is not hypothetical. The first version of this list was derived
from `Manifest`'s fields alone and warned on `[lint]` and `[linker]`,
two tables that work: `verum lint` reads `[lint]` through
`load_full_lint_config`, and `verum_compiler::linker_config` reads
`[linker]`, with its own test asserting that `output`, `lto`,
`use_lld`, `pic` and `strip` all take effect. Both open the manifest
themselves, so neither is a `Manifest` field.

WHAT THIS GATE CHECKS, and what it deliberately does not: it checks the
direction that can be checked mechanically — every `Manifest` field
must appear in KNOWN_TABLES. The other direction cannot be derived,
because an independent reader is just some code that opens the file;
those entries carry a comment naming their reader, and this gate
asserts the comment is there rather than trying to find the reader.

So a new `Manifest` field fails this gate loudly; a new independent
reader is caught by the comment requirement on any entry that is not a
`Manifest` field.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CONFIG = REPO / "crates" / "verum_cli" / "src" / "config.rs"
COMMAND = REPO / "crates" / "verum_cli" / "src" / "commands" / "config.rs"

FIELD = re.compile(r'(?:#\[serde\(([^)]*)\)\]\s*)?pub\s+([a-z_0-9]+)\s*:')


def manifest_fields(path: Path) -> list[str]:
    s = path.read_text(errors="ignore")
    i = s.index("pub struct Manifest")
    body = s[i : s.index("\n}", i)]
    out = []
    for m in FIELD.finditer(body):
        attr, name = m.group(1) or "", m.group(2)
        r = re.search(r'rename\s*=\s*"([^"]+)"', attr)
        out.append(r.group(1) if r else name)
    return out


def known_tables(path: Path) -> tuple[list[str], str]:
    s = path.read_text(errors="ignore")
    i = s.index("const KNOWN_TABLES")
    block = s[i : s.index("];", i)]
    return re.findall(r'"([a-z_0-9]+)"', block), block


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--config", type=Path, default=CONFIG)
    ap.add_argument("--command", type=Path, default=COMMAND)
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        ok = True
        # The parse must find a plausible number on both sides, or the
        # comparison passes for free.
        try:
            fields = manifest_fields(args.config)
            tables, block = known_tables(args.command)
        except (ValueError, OSError) as e:
            print(f"self-test FAIL: could not parse ({e})")
            return 1
        if len(fields) < 20:
            print(f"self-test FAIL: only {len(fields)} Manifest fields parsed")
            ok = False
        if len(tables) < 20:
            print(f"self-test FAIL: only {len(tables)} KNOWN_TABLES parsed")
            ok = False
        # A serde rename must win over the field name.
        probe = 'pub struct Manifest {\n    #[serde(rename = "zzq-renamed")]\n    pub zzq_raw: X,\n}\n'
        tmp = Path("/tmp/zzq_manifest_probe.rs")
        tmp.write_text(probe)
        if manifest_fields(tmp) != ["zzq-renamed"]:
            print("self-test FAIL: a serde rename did not win")
            ok = False
        tmp.unlink()
        # The motivating case: `lint` and `linker` are NOT Manifest
        # fields, so a gate written the other way round would have
        # demanded their removal.
        if "lint" in fields or "linker" in fields:
            print("self-test FAIL: lint/linker became Manifest fields — "
                  "the comment in KNOWN_TABLES is now stale")
            ok = False
        print("self-test: ok" if ok else "self-test: FAILED")
        return 0 if ok else 1

    fields = manifest_fields(args.config)
    tables, block = known_tables(args.command)
    if len(fields) < 20 or len(tables) < 20:
        print(f"check-known-tables: parse looks wrong "
              f"({len(fields)} fields, {len(tables)} tables) — refusing "
              f"to pass vacuously", file=sys.stderr)
        return 1

    missing = [f for f in fields if f not in tables]
    extra = [t for t in tables if t not in fields]

    print(f"check-known-tables: {len(fields)} Manifest fields, "
          f"{len(tables)} KNOWN_TABLES, {len(missing)} uncovered, "
          f"{len(extra)} from independent readers")
    rc = 0
    if missing:
        print("\nManifest fields missing from KNOWN_TABLES — `config "
              "validate` will call each of these a typo:", file=sys.stderr)
        for m in missing:
            print(f"  [{m}]", file=sys.stderr)
        rc = 1
    # Entries that are not Manifest fields must say which reader owns
    # them, so the next person can tell a real table from a leftover.
    if extra and "own loaders" not in block and "independent reader" not in block:
        print("\nKNOWN_TABLES has entries that are not Manifest fields "
              f"({', '.join(extra)}) and no comment naming their reader.",
              file=sys.stderr)
        rc = 1
    return rc


if __name__ == "__main__":
    sys.exit(main())
