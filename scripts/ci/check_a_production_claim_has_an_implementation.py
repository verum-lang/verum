#!/usr/bin/env python3
"""A primitive the documentation calls PRODUCTION must not rest on an
intrinsic key that the tree lists as unimplemented.

THE GAP THIS CLOSES IS BETWEEN TWO GREEN GATES.  `check_intrinsic_keys_
implemented.py` freezes the set of `@intrinsic("verum.…")` keys that
nothing in `crates/` backs — 288 of them, and its roster was exact.  The
documentation has gates of its own for anchors, names and code blocks.
Neither side reads the other, so on 2026-09-13 the security overview
carried three rows marked "Production" whose keys sat in that roster the
whole time:

    X25519   verum.x25519.scalar_mult      roster line 300
    ML-KEM   verum.pq.ml_kem_keygen        roster line 217
    ML-DSA   verum.pq.ml_dsa_keygen        roster line 212

Running them is unambiguous — the interpreter panics with "is not
implemented in this build" and the AOT binary exits on a trap — so this
was never a subtle disagreement.  It survived because no instrument
crossed from one side to the other.

HOW THE TWO SIDES ARE JOINED, exactly and not by resemblance.  Every
rostered key is declared in some `core/**/*.vr`; that file's STEM is the
primitive's name in the tree (`core/security/ecc/x25519.vr` -> `x25519`).
A status row's first cell is the primitive's name on the page.  The test
is EQUALITY of the two after one normalisation (lowercase; `-` and spaces
to `_`; a trailing parenthetical qualifier such as "(block cipher)" or
"(legacy)" dropped).  Substring matching was rejected deliberately: `aes`
occurs inside `aes_gcm`, and a gate that fires on resemblance teaches
readers to disable it.

WHAT THIS GATE CANNOT SEE, reported rather than hidden.  A row whose name
matches no `core/**` stem is not checked at all — there is nothing exact
to join it to.  The gate prints that count every run, because a detector
that silently examines a third of its input is the failure mode the
barename ratchet was rewritten to avoid.  If that number grows, the
coverage shrank and the green means less than it did.

EXIT: a rostered key leaves the roster when something implements it, and
then this gate stops objecting to its row on its own.  No second edit.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
CORE = REPO / "core"
ROSTER = pathlib.Path(__file__).resolve().parent / "intrinsic_keys_unimplemented.txt"

KEY_RE = re.compile(r'@intrinsic\(\s*"(verum\.[A-Za-z0-9_.]+)"')
# A markdown table row: `| cell | cell | … |`.
ROW_RE = re.compile(r"^\|(?P<cells>.+)\|\s*$")
PRODUCTION = "✅"
# The markers a STATUS row carries.  A table row without one of these is not
# a status claim (a module table, a cipher-suite matrix) and is skipped
# before the coverage count, so "unchecked" means a status row this gate
# could not join — not merely a row of some other table.
STATUS_MARKERS = ("✅", "⚠️", "❌", "🟡")
# A trailing "(block cipher)" / "(legacy)" qualifier on a primitive name.
QUALIFIER_RE = re.compile(r"\s*\([^)]*\)\s*$")


def normalise(name: str) -> str:
    """Page name -> tree stem.  Lowercase, qualifier dropped, separators
    unified.  `ML-KEM` -> `ml_kem`; `AES-128 (block cipher)` -> `aes_128`."""
    n = QUALIFIER_RE.sub("", name.strip())
    n = n.strip("`").strip()
    n = n.lower().replace("-", "_").replace(" ", "_")
    return n


def candidates(name_cell: str) -> list[str]:
    """Every stem spelling a name cell could denote — still EQUALITY tests,
    never resemblance.

    Two spellings, because the tree is not consistent with itself and
    guessing which one a file used is not the reader's job: `sha256.vr`
    writes the width closed up while `ml_kem.vr` separates it, and the page
    writes both with a hyphen.  So a cell is tried as `ml_kem` AND as
    `mlkem`; exactly one can match a real stem.

    A cell may also LIST primitives — "SHA-256, SHA-384, SHA-512" is three
    claims sharing one row, and a row that says Production vouches for all
    of them.  Splitting on commas is what lets the gate judge each.
    """
    out: list[str] = []
    for part in name_cell.split(","):
        part = part.strip()
        if not part:
            continue
        under = normalise(part)
        if under:
            out.append(under)
        squashed = under.replace("_", "")
        if squashed and squashed != under:
            out.append(squashed)
    return out


def load_roster() -> set[str]:
    if not ROSTER.is_file():
        return set()
    return {
        line.strip()
        for line in ROSTER.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.startswith("#")
    }


def key_sites(root: pathlib.Path) -> dict[str, set[str]]:
    """key -> the set of `core/**` file STEMS that declare it."""
    sites: dict[str, set[str]] = {}
    for p in sorted(root.rglob("*.vr")):
        try:
            src = p.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        for key in KEY_RE.findall(src):
            sites.setdefault(key, set()).add(p.stem)
    return sites


def core_stems(root: pathlib.Path) -> set[str]:
    return {p.stem for p in root.rglob("*.vr")}


def status_rows(doc: pathlib.Path) -> list[tuple[int, str, str]]:
    """(line number, primitive name, whole row) for every table row."""
    rows: list[tuple[int, str, str]] = []
    for i, line in enumerate(doc.read_text(encoding="utf-8").splitlines(), 1):
        m = ROW_RE.match(line)
        if not m:
            continue
        cells = [c.strip() for c in m.group("cells").split("|")]
        if len(cells) < 2 or not cells[0] or set(cells[0]) <= set("- :"):
            continue
        rows.append((i, cells[0], line))
    return rows


def judge(docs: list[pathlib.Path]) -> tuple[list[str], int, int]:
    roster = load_roster()
    sites = key_sites(CORE)
    stems = core_stems(CORE)
    # stem -> the rostered keys it declares
    unimplemented_by_stem: dict[str, set[str]] = {}
    for key in roster:
        for stem in sites.get(key, ()):
            unimplemented_by_stem.setdefault(stem, set()).add(key)

    findings: list[str] = []
    joined = unjoined = 0
    for doc in docs:
        for lineno, name, row in status_rows(doc):
            if not any(m in row for m in STATUS_MARKERS):
                continue
            matched = [c for c in candidates(name) if c in stems]
            if not matched:
                unjoined += 1
                continue
            joined += 1
            if PRODUCTION not in row:
                continue
            for stem in matched:
                bad = unimplemented_by_stem.get(stem)
                if bad:
                    findings.append(
                        f"{doc.name}:{lineno}: '{name}' is marked Production, "
                        f"and {stem}.vr rests on unimplemented "
                        f"{', '.join(sorted(bad))}"
                    )
    return findings, joined, unjoined


def self_test() -> int:
    """The normaliser must join the three known cases and must NOT join by
    resemblance — `aes_gcm` is not `aes`."""
    cases = {
        "X25519": "x25519",
        "ML-KEM": "ml_kem",
        "ML-DSA": "ml_dsa",
        "AES-128 (block cipher)": "aes_128",
        "SHA-1 (legacy)": "sha_1",
        "`chacha20`": "chacha20",
    }
    for given, want in cases.items():
        got = normalise(given)
        if got != want:
            print(f"[FAIL] self-test: normalise({given!r}) = {got!r}, want {want!r}")
            return 2
    if normalise("AES-GCM") == normalise("AES"):
        print("[FAIL] self-test: two different primitives normalised alike")
        return 2
    # Both spellings must be offered, and a listing must split.
    if "sha256" not in candidates("SHA-256"):
        print("[FAIL] self-test: the closed-up spelling is not offered")
        return 2
    if candidates("SHA-256, SHA-384") [:1] != ["sha_256"] or "sha384" not in candidates(
        "SHA-256, SHA-384"
    ):
        print("[FAIL] self-test: a listing row is not split into its members")
        return 2
    # An ANCHOR: the roster must still carry the key this gate was written
    # for. If it stops doing so the gate is measuring nothing, and saying
    # "ok" then would be the failure the docstring describes.
    roster = load_roster()
    if roster and "verum.x25519.scalar_mult" not in roster:
        print(
            "[note] self-test anchor: verum.x25519.scalar_mult has LEFT the "
            "roster — if it was implemented this is progress; re-point the "
            "anchor at another rostered key."
        )
    print(f"  [ok] self-test: {len(cases)} normalisations, 1 non-join, 2 spelling/listing cases, anchor checked")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument(
        "--docs",
        type=pathlib.Path,
        nargs="*",
        default=None,
        help="status-table documents to judge (the caller supplies the path so "
             "no tracked file has to name a directory outside the repository)",
    )
    args = ap.parse_args()

    if args.self_test:
        return self_test()
    rc = self_test()
    if rc:
        return rc

    docs = [d for d in (args.docs or []) if d.is_file()]
    if not docs:
        print(
            "check-production-claims: no status document given or found — "
            "REFUSING to report OK: this run was asked to CHECK, so a missing "
            "input is an unset path, not 'nothing to do'."
        )
        return 0 if not args.check else 2
    if not CORE.is_dir():
        print("[FAIL] core/ not found — refusing to judge.")
        return 2

    findings, joined, unjoined = judge(docs)
    print(
        f"check-production-claims: {joined} row(s) joined to a core module, "
        f"{unjoined} not joinable (unchecked), {len(findings)} contradiction(s)"
    )
    for f in findings:
        print(f"    {f}")
    if findings:
        print(
            "\nA row may say Production only when its key is off the roster "
            "(scripts/ci/intrinsic_keys_unimplemented.txt). Either implement "
            "the key — the row then passes with no second edit — or say on "
            "the page what the primitive actually does today."
        )
        return 1 if args.check else 0
    return 0


if __name__ == "__main__":
    sys.exit(main())
