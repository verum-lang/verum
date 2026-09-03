#!/usr/bin/env python3
"""Gate: the parser must not call a registered attribute unknown.

Two lists name Verum's attributes, in two crates, and they disagreed:

  verum_types/src/attr/standard.rs        the registry, 177 entries
  verum_fast_parser/src/attr_validation.rs a "quick check" match arm

The quick check's own comment says it "is not exhaustive - full
validation is done by verum_types registry". The DIAGNOSTIC it produces
is not tentative: `warning<W0400>: unknown attribute `@trusted``. And
because it is a warning, the attribute is a silent no-op as well, so the
author is told the name does not exist AND nothing acts on it.

Measured 2026-09-03: thirteen names registered in `standard.rs` —
`kernel`, `link_name`, `link_section`, `linkage`, `ownership`,
`property`, `register_block`, `section`, `test_case`, `trusted`,
`unsafe_fn`, `visibility`, `weak` — were reported exactly as
`@zzq_nonsense` was, character for character.

A check may be incomplete about an attribute's TARGET. It may not be
incomplete about its EXISTENCE while phrasing the answer as if it were
complete. This gate pins the containment that makes the phrasing true.

DIRECTION: registry ⊆ parser. The reverse is allowed — the parser knows
names that come from elsewhere (`cfg`, `const`, `framework`,
`llvm_only`, `multiversion`, `universe_poly` are built in ahead of the
registry), and demanding equality would fail on them for no reason.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
REGISTRY = REPO / "crates" / "verum_types" / "src" / "attr" / "standard.rs"
PARSER = REPO / "crates" / "verum_fast_parser" / "src" / "attr_validation.rs"

REG = re.compile(r'AttributeMetadata::new\("([a-z_0-9]+)"\)')
LIT = re.compile(r'"([a-z_0-9]+)"')


def registered(path: Path) -> set[str]:
    return set(REG.findall(path.read_text(errors="ignore")))


def parser_names(path: Path) -> set[str]:
    """Names in the match that DECIDES existence.

    The first version of this read every string literal in the file, on
    the reasoning that a generous set keeps the gate off the file's
    internal shape. It could not go red: removing `"trusted"` from its
    match arm left the word in a comment, the set still contained it,
    and the gate stayed green. A gate that cannot fail for the case it
    was built for is not a gate — so the extraction is narrowed to the
    one construct whose contents are the answer.

    Bounded by `let valid = match name {` and the `};` that closes it,
    so comments before and after are out of scope by construction.
    """
    src = path.read_text(errors="ignore")
    start = src.index("let valid = match name {")
    end = src.index("\n        };", start)
    body = src[start:end]
    # Blank out comments: a name mentioned in a `//` line inside the
    # match is discussion, not an arm.
    body = re.sub(r"//[^\n]*", "", body)
    return set(LIT.findall(body))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--registry", type=Path, default=REGISTRY)
    ap.add_argument("--parser", type=Path, default=PARSER)
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        ok = True
        try:
            reg, par = registered(args.registry), parser_names(args.parser)
        except OSError as e:
            print(f"self-test FAIL: {e}")
            return 1
        # Both sides must parse to a plausible size, or containment is free.
        if len(reg) < 100:
            print(f"self-test FAIL: registry parsed to {len(reg)} names")
            ok = False
        if len(par) < 50:
            print(f"self-test FAIL: parser parsed to {len(par)} names")
            ok = False
        # Known anchors on each side.
        for n in ("inline", "cold", "derive"):
            if n not in reg:
                print(f"self-test FAIL: `{n}` missing from the registry parse")
                ok = False
            if n not in par:
                print(f"self-test FAIL: `{n}` missing from the parser parse")
                ok = False
        # The motivating name must be present on BOTH sides now.
        if "trusted" not in reg:
            print("self-test FAIL: `trusted` is no longer registered — "
                  "the motivating case is gone, revisit this gate")
            ok = False
        print("self-test: ok" if ok else "self-test: FAILED")
        return 0 if ok else 1

    reg, par = registered(args.registry), parser_names(args.parser)
    if len(reg) < 100 or len(par) < 50:
        print(f"check-parser-attrs: parse looks wrong ({len(reg)} registered, "
              f"{len(par)} in the parser) — refusing to pass vacuously",
              file=sys.stderr)
        return 1

    missing = sorted(reg - par)
    print(f"check-parser-attrs: {len(reg)} registered, {len(par)} known to the "
          f"parser, {len(missing)} would be called unknown")
    if missing:
        print("\nRegistered attributes the parser calls unknown — each one is "
              "a silent no-op AND a diagnostic telling the author the name "
              "does not exist:", file=sys.stderr)
        for m in missing:
            print(f"  @{m}", file=sys.stderr)
        print("\nAdd them to the match in "
              "crates/verum_fast_parser/src/attr_validation.rs, with the "
              "target `standard.rs` registers them for.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
