#!/usr/bin/env python3
"""Every `@intrinsic("verum.…")` key declared in `core/` that nothing in
`crates/` implements, frozen as a SET rather than a count.

WHAT IS BEING FROZEN, and why it is not a bug list.  A body
`@intrinsic("name", args)` whose key has no registry entry is handled
CORRECTLY by the compiler: it warns at compile time and lowers to a runtime
trap that names the key.

    WARN [intrinsic] unregistered @intrinsic("verum.quic.connect") in
         subject — lowering to a runtime trap; register it or reroute to a
         registered name

    Panic: @intrinsic("verum.quic.connect") is not implemented in this
           build (called from subject); it has no registry entry, so there
           is no value to return

Nothing here asks for that to change.  What this gate freezes is the SIZE
AND MEMBERSHIP of the declared-but-unimplemented surface, so it cannot grow
without a decision, and so that implementing one shows up as a deliberate
edit rather than as silence.

MEASURED 2026-09-10 (T1368): 273 of the 274 distinct `verum.*` keys in
`core/` have no implementing string anywhere in `crates/`.  Only
`verum.process.exit` does.  Thirty-one families, largest first: crypto 43,
libm_deterministic 42, tls 17, quic 17, compress 17, p256 12, k8s 12,
spiffe 11, bpf 11, h3 10.

WHY THIS IS NOT ONLY A SCAFFOLDING QUESTION.  `net/quic` and `net/tls`
carry their status in `core-tests/INVENTORY.md` ("@ignore'd — backend not
yet implemented"), so a reader can find out.  `core/math/ieee754_deterministic`
does not, and it is publicly mountable:

    mount core.math.ieee754_deterministic.{sqrt};  sqrt(2.0)
      -> Panic: @intrinsic("verum.libm_deterministic.sqrt") is not
         implemented in this build

WHY A SET AND NOT A COUNT.  A count agrees while the membership changes:
implement one key, add another, and the number is unmoved.  That blindness
is not hypothetical in this tree — the barename-collision ratchet was
extended to a membership register for exactly this reason after a same-size
swap printed "at baseline".  So the roster is the artefact and the count is
derived from it, never written beside it.

WHY THE COMPILE-TIME WARNING DOES NOT SUBSTITUTE.  It reaches a user
compiling their OWN file.  It does NOT reach anyone for stdlib code: the
bake compiles `core/` inside a cargo build script, whose `tracing` output
cargo hides.  A user meets the trap only as a runtime panic.  So the
warning cannot be the ratchet.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
CORE = REPO / "core"
CRATES = REPO / "crates"
ROSTER = pathlib.Path(__file__).resolve().parent / "intrinsic_keys_unimplemented.txt"

KEY_RE = re.compile(r'@intrinsic\(\s*"(verum\.[A-Za-z0-9_.]+)"')


def declared_keys(root: pathlib.Path) -> set[str]:
    """Every `verum.*` intrinsic key named by a `@intrinsic(...)` in `core/`."""
    keys: set[str] = set()
    for p in sorted(root.rglob("*.vr")):
        try:
            keys.update(KEY_RE.findall(p.read_text(encoding="utf-8", errors="replace")))
        except OSError:
            continue
    return keys


def implemented_keys(keys: set[str], root: pathlib.Path) -> set[str]:
    """Keys that appear as a QUOTED STRING anywhere under `crates/`.

    Read the corpus ONCE.  Asking `grep` per key over 2400 files is the same
    answer at 274x the cost, and a gate slow enough to skip is a gate that
    gets skipped.
    """
    corpus: list[str] = []
    for p in sorted(root.rglob("*.rs")):
        if "target" in p.parts:
            continue
        try:
            corpus.append(p.read_text(encoding="utf-8", errors="replace"))
        except OSError:
            continue
    blob = "\n".join(corpus)
    return {k for k in keys if f'"{k}"' in blob}


def load_roster() -> set[str] | None:
    if not ROSTER.is_file():
        return None
    return {
        line.strip()
        for line in ROSTER.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.startswith("#")
    }


def write_roster(keys: set[str]) -> None:
    header = (
        "# Declared-but-unimplemented `verum.*` intrinsic keys (T1368).\n"
        "# Regenerate with: python3 scripts/ci/check_intrinsic_keys_implemented.py --write\n"
        "# A key LEAVES this list when something in crates/ implements it — that is\n"
        "# progress, and deleting the line is the deliberate act that records it.\n"
        "# A key JOINS it when core/ declares a new @intrinsic nothing backs.\n"
    )
    ROSTER.write_text(header + "\n".join(sorted(keys)) + "\n", encoding="utf-8")


def self_test() -> int:
    """The extractor must find a key, and must not find one that is absent."""
    sample = 'fn f() { @intrinsic("verum.quic.connect", a) }\n// @intrinsic("verum.not.real")\n'
    got = set(KEY_RE.findall(sample))
    # The regex does NOT exempt comments — stated rather than hidden: a key
    # named in a comment is still a key someone may uncomment, and a false
    # PLUS here costs a deletion review, while a false minus costs a silent
    # gap. Both sample keys must be found.
    want = {"verum.quic.connect", "verum.not.real"}
    if got != want:
        print(f"[FAIL] self-test: extractor returned {sorted(got)}, want {sorted(want)}")
        return 2
    if KEY_RE.findall('@intrinsic("add", a, b)'):
        print("[FAIL] self-test: a bare (non-`verum.`) key must not be collected")
        return 2
    print("  [ok] extractor self-test: 2 cases")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--check", action="store_true", help="return non-zero on drift")
    ap.add_argument("--write", action="store_true", help="regenerate the roster")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return self_test()
    rc = self_test()
    if rc:
        return rc

    if not CORE.is_dir() or not CRATES.is_dir():
        print("[FAIL] core/ or crates/ not found — refusing to judge.")
        return 2

    declared = declared_keys(CORE)
    if not declared:
        print("[FAIL] no `verum.*` intrinsic keys found in core/ at all — the "
              "scan did not run, and an empty scan would pass this gate "
              "vacuously.")
        return 2
    implemented = implemented_keys(declared, CRATES)
    unimplemented = declared - implemented

    if args.write:
        write_roster(unimplemented)
        print(f"[write] roster: {len(unimplemented)} unimplemented of {len(declared)} declared")
        return 0

    roster = load_roster()
    if roster is None:
        print(f"[FAIL] roster missing: {ROSTER}")
        print("  A missing register is a REFUSAL, not a pass — without it this "
              "gate cannot tell an implemented key from a deleted one.")
        return 2

    added = sorted(unimplemented - roster)
    gone = sorted(roster - unimplemented)

    print(f"intrinsic keys: {len(declared)} declared in core/, "
          f"{len(implemented)} implemented in crates/, "
          f"{len(unimplemented)} not")

    if not added and not gone:
        print(f"[ok] roster exact: {len(roster)} unimplemented keys, membership unchanged")
        return 0

    if added:
        print(f"\n[FAIL] {len(added)} NEW declared-but-unimplemented key(s):")
        for k in added:
            print(f"    {k}")
        print(
            "\n  Each is a public surface that compiles, warns where nobody\n"
            "  reads it, and traps on first call. Implement it, reroute it to\n"
            "  a registered name, or add it to the roster WITH the reason."
        )
    if gone:
        print(f"\n[stale] {len(gone)} roster entry/entries are no longer "
              f"unimplemented — implemented, or the declaration is gone:")
        for k in gone:
            print(f"    {k}")
        print("\n  Delete their lines (`--write` regenerates), so the ratchet "
              "keeps tightening.")

    return 1 if args.check else 0


if __name__ == "__main__":
    sys.exit(main())
