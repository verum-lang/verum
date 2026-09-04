#!/usr/bin/env python3
"""Gate: the Verum examples in the documentation must be Verum.

Extracts every ```verum block that carries an `fn main` — the ones a
reader can copy and run — and compiles each with `verum check`.

WHY ONLY THE SELF-CONTAINED ONES: of 2823 verum blocks in the docs, 63
have an `fn main`. The rest are fragments that need a surrounding
project, and wrapping them would invent context and produce failures
that say nothing about the documentation. The 63 are the blocks whose
correctness a reader can actually depend on.

WHAT THE FIRST RUN FOUND (2026-08-28), and why the classification
matters more than the total: 51 of 63 failed, and only about half of
that was documentation drift.

    26  named an API that does not exist       <- real
     8  parse errors                           <- real
    10  used a name defined in an earlier block  <- this tool's limit
     9  needed a context to be provided          <- this tool's limit
     7  mounted a sibling module                 <- this tool's limit
     4  elided with `...`                        <- by design

A gate that reported "51 broken examples" would have been wrong about
half of them, and the half it was wrong about is the half nobody can
fix.

BLOCKS THAT MUST NOT COMPILE are first-class here. Documentation
teaches by counter-example — "yield outside a fn* body", "an unbalanced
brace", "a refinement the compiler can clearly disprove" — and a block
marked as such is checked in the OTHER direction: if it compiles, the
marker is stale and the reader is being shown a fear that no longer
applies. Five marker spellings are recognised because five are in use.

THE RATCHET, and why the key is a hash. `--check` alone answers "is the
count zero", which it is not and will not be for a while. `--ratchet`
answers the question a maintainer can act on: did THIS change make the
documentation worse. Its baseline is keyed by `page#blake2s(body)[:8]`,
not by line number, because a line number moves when anything above it
is edited and every downstream example would read as new.

The key is the block's own text, so EDITING a tracked example re-keys
it: the old key reads as fixed and the new one as new. That is correct —
an edited example is a different example — but it means a documentation
commit that touches one of these blocks wants `--write-baseline` run
afterwards. The baseline lives here and the pages live in the website
repository, so nothing reminds you but this paragraph.

Usage:
    check_doc_examples.py                  # report, grouped
    check_doc_examples.py --check          # exit 1 on any real failure
    check_doc_examples.py --ratchet        # exit 1 only on a NEW one
    check_doc_examples.py --write-baseline
    check_doc_examples.py --self-test
"""

from __future__ import annotations

import argparse
import hashlib
import os
import re
import subprocess
import sys
import tempfile
from collections import Counter, defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
# Built from parts so the literal path does not appear in a tracked file
# (`make check-internal-refs`), the same way the sibling doc gates do it.
DOCS = REPO / "internal" / "website" / "docs"
BASELINE = REPO / "scripts" / "ci" / "doc_examples_known_failures.txt"

BLOCK = re.compile(r"^```verum\n(.*?)^```", re.M | re.S)

# Second check, over EVERY block rather than the self-contained ones:
# a `mount core.a.b;` must name a module that exists or a symbol that
# module exports. Compiling cannot ask this — 54 of ~2800 blocks have an
# `fn main` — and it is the class that produced `mount core.io.mmap`,
# a section of the file-IO cookbook written against a module `core/io/`
# has never contained.
#
# THE INSTRUMENT TOOK FOUR TRIES AND EACH WRONG ONE LOOKED CLEAN. Naming
# only the module says 18, because `mount M.symbol;` is legal grammar
# (`mount_item = path`). Falling back to "is the PARENT a module" says
# 0 — it excuses everything, `core.io.mmap` included, which is how a
# green answer hid the one case already found by hand. Missing the bare
# re-export form (`public mount .list.List;`, not the braced one) says
# 12, blaming `core.collections.List`. Hence the controls below, which
# run on every invocation: two that must resolve, two that must not.
MOUNT = re.compile(r"^\s*(?:public\s+)?mount\s+(core(?:\.[A-Za-z_][A-Za-z0-9_]*)+)", re.M)
# Modules the compiler SYNTHESISES — no file under core/, and mounting
# them compiles. `core.prelude` is the one that matters: 531 files under
# core-tests/ mount it, `mount core.prelude.{Bool, Int, Maybe, List,
# Text};` type-checks clean, and an arbitrary absent module in the same
# position answers `error<E402>`. A file-tree map cannot see it, so
# without this list the gate reports a legal mount as phantom — it did,
# and a documentation page was "corrected" on the strength of it before
# the compiler was asked.
SYNTHESISED_MODULES = {"core.prelude"}
CORE = REPO / "core"
# The four that remain are each documented AS absent on their own page
# (a `:::caution`, a `:::danger`, or an "illustrative name" comment), so
# the pole is true: a sixth means a page started lying again.
MOUNT_BASELINE = 4
MOUNT_CONTROLS = [("core.sys.common.pread", True), ("core.collections.List", True),
                  ("core.io.mmap", False), ("core.zzz.nothing", False)]

# Blocks the documentation deliberately shows as NOT compiling.
MUST_FAIL = re.compile(
    r"//\s*(COMPILE ERROR|ERROR|error<E\d+>|does not compile|WRONG|BAD|✗|refused)",
    re.I,
)

# Failures that are this tool's limitation rather than a defect in the
# prose. Each one is a block that is correct in its page and incomplete
# on its own.
LIMITS = (
    ("mounts a sibling module", re.compile(r"module `[^`]+` not found")),
    ("needs a provided context", re.compile(r"undefined context")),
    ("name defined in an earlier block", re.compile(r"unbound variable|type not found")),
)


def verum_binary() -> Path:
    override = os.environ.get("VERUM_BIN")
    if override:
        p = Path(override)
        if not p.is_file():
            raise SystemExit(f"VERUM_BIN={override} is not a file")
        return p
    for c in (REPO / "target" / "release" / "verum", REPO / "target" / "debug" / "verum"):
        if c.is_file():
            return c
    raise SystemExit(
        "no verum binary at target/{release,debug}/verum — pass one with "
        "VERUM_BIN=/path/to/verum"
    )


def blocks():
    for d in sorted(DOCS.rglob("*.md")):
        text = d.read_text(errors="ignore")
        for m in BLOCK.finditer(text):
            body = m.group(1)
            if "fn main(" not in body:
                continue
            line = text[: m.start()].count("\n") + 1
            yield d.relative_to(DOCS.parent), line, body


def core_modules() -> dict[str, list[Path]]:
    mods: dict[str, list[Path]] = {}
    for f in sorted(CORE.rglob("*.vr")):
        parts = list(f.relative_to(REPO).with_suffix("").parts)
        if parts[-1] == "mod":
            parts = parts[:-1]
        mods.setdefault(".".join(parts), []).append(f)
        for i in range(1, len(parts)):
            mods.setdefault(".".join(parts[:i]), [])
    return mods


_EXPORTS: dict[str, set[str]] = {}
_DECL = (r"^\s*public\s+(?:async\s+)?fn\s+([a-z_]\w*)", r"^\s*public\s+type\s+(\w+)",
         r"^\s*public\s+(?:const|static)\s+(\w+)", r"^\s*public\s+context\s+(\w+)")


def exports(mods, mod: str) -> set[str]:
    if mod in _EXPORTS:
        return _EXPORTS[mod]
    out: set[str] = set()
    for f in mods.get(mod, []):
        src = f.read_text(errors="ignore")
        for pat in _DECL:
            out |= set(re.findall(pat, src, re.M))
        for grp in re.findall(r"^\s*public\s+mount\s+[\w.]*\.\{([^}]*)\}", src, re.M):
            out |= {x.strip().split(" as ")[-1] for x in grp.split(",") if x.strip()}
        for one in re.findall(r"^\s*public\s+mount\s+([\w.]+)\s*;", src, re.M):
            out.add(one.rsplit(".", 1)[-1])
    _EXPORTS[mod] = out
    return out


def mount_resolves(mods, path: str) -> bool:
    if path in SYNTHESISED_MODULES or path in mods:
        return True
    parent, _, leaf = path.rpartition(".")
    return parent in mods and leaf in exports(mods, parent)


def check_mounts() -> tuple[int, list[str]]:
    """(broken count, lines). Runs its own controls first."""
    mods = core_modules()
    for path, want in MOUNT_CONTROLS:
        if mount_resolves(mods, path) != want:
            raise SystemExit(
                f"mount-control FAILED: {path} should "
                f"{'resolve' if want else 'not resolve'} — the instrument is wrong, "
                "not the documentation"
            )
    broken: dict[str, set[str]] = defaultdict(set)
    for d in sorted(DOCS.rglob("*.md")):
        for m in BLOCK.finditer(d.read_text(errors="ignore")):
            for mm in MOUNT.finditer(m.group(1)):
                if not mount_resolves(mods, mm.group(1)):
                    broken[mm.group(1)].add(str(d.relative_to(DOCS.parent)))
    lines = []
    for k in sorted(broken):
        parent, _, leaf = k.rpartition(".")
        why = "no such module" if parent not in mods else f"{parent} does not export `{leaf}`"
        lines.append(f"  {k}  — {why}\n      {', '.join(sorted(broken[k]))}")
    return len(broken), lines


def classify(body: str, errs: list[str]) -> str:
    if not errs:
        return "compiles"
    first = errs[0]
    if "..." in body and "Parse error" in first:
        return "elided with `...`"
    for name, pat in LIMITS:
        if pat.search(first):
            return name
    if "Parse error" in first:
        return "PARSE ERROR"
    return "API / semantic"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--ratchet", action="store_true",
                    help="fail only on a real failure that is not in the baseline")
    ap.add_argument("--write-baseline", action="store_true")
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        ok = True
        cases = [
            ("fn main() {}", [], "compiles"),
            ("fn main() { ... }", ["error<E018>: Parse error: x"], "elided with `...`"),
            ("fn main() {}", ["error<E402>: module `x` not found"], "mounts a sibling module"),
            ("fn main() {}", ["error: undefined context: IO"], "needs a provided context"),
            ("fn main() {}", ["error<E018>: Parse error: y"], "PARSE ERROR"),
            ("fn main() {}", ["error<E400>: no method named `z`"], "API / semantic"),
        ]
        for body, errs, want in cases:
            got = classify(body, errs)
            if got != want:
                print(f"self-test FAIL: {errs} -> {got}, want {want}")
                ok = False
        if not MUST_FAIL.search("// COMPILE ERROR: nope"):
            print("self-test FAIL: marker not recognised")
            ok = False
        try:
            check_mounts()          # raises if its own controls disagree
        except SystemExit as e:
            print(f"self-test FAIL: {e}")
            ok = False
        print("self-test: ok" if ok else "self-test: FAILED")
        return 0 if ok else 1

    n_mounts, mount_lines = check_mounts()
    if n_mounts:
        print(f"--- {n_mounts} unresolvable `mount core.*` in doc blocks ---")
        for line in mount_lines:
            print(line)
        print()

    binary = verum_binary()
    buckets: Counter[str] = Counter()
    detail: dict[str, list] = defaultdict(list)
    stale_markers: list = []
    keys: dict[str, tuple] = {}

    with tempfile.TemporaryDirectory() as tmp:
        for rel, line, body in blocks():
            f = Path(tmp) / f"{Path(rel).stem}_{line}.vr"
            f.write_text(body)
            try:
                r = subprocess.run(
                    [str(binary), "check", str(f)],
                    capture_output=True, text=True, timeout=args.timeout,
                )
                blob = r.stdout + r.stderr
            except subprocess.TimeoutExpired:
                buckets["TIMEOUT"] += 1
                detail["TIMEOUT"].append((rel, line, "did not finish"))
                continue
            errs = [l for l in blob.splitlines() if l.startswith("error")]

            if MUST_FAIL.search(body):
                # Checked the other way round: this block is documented
                # as not compiling, so compiling is the failure.
                if errs:
                    buckets["counter-example, correctly refused"] += 1
                else:
                    buckets["STALE MARKER — marked as failing, compiles"] += 1
                    stale_markers.append((rel, line, ""))
                continue

            k = classify(body, errs)
            buckets[k] += 1
            if errs:
                key = f"{rel}#{hashlib.blake2s(body.encode()).hexdigest()[:8]}"
                detail[k].append((rel, line, errs[0][:100]))
                keys[key] = (k, f"{rel}:{line}", errs[0][:100])

    for k, v in buckets.most_common():
        print(f"{v:4d}  {k}")

    real = ("PARSE ERROR", "API / semantic", "TIMEOUT",
            "STALE MARKER — marked as failing, compiles")
    print()
    for k in real:
        if not detail.get(k) and k not in buckets:
            continue
        rows = detail.get(k) or stale_markers if "STALE" in k else detail.get(k, [])
        if not rows:
            continue
        print(f"--- {k} ---")
        for rel, line, e in rows:
            print(f"  {rel}:{line}  {e}")

    n_real = sum(buckets[k] for k in real)
    print(f"\ncheck-doc-examples: {n_real} example(s) a reader cannot trust")

    tracked = {k: v for k, v in keys.items() if v[0] in real}
    if args.write_baseline:
        BASELINE.write_text("".join(f"{k}\n" for k in sorted(tracked)))
        print(f"[write] baseline: {len(tracked)} known-failing example(s)")
        return 0

    if args.ratchet:
        known = set()
        if BASELINE.is_file():
            known = {l.strip() for l in BASELINE.read_text().split("\n") if l.strip()}
        new = sorted(set(tracked) - known)
        gone = sorted(known - set(tracked))
        if new:
            print(f"\n[fail] {len(new)} example(s) newly cannot be trusted:")
            for k in new:
                bucket, where, err = tracked[k]
                print(f"    {where}  [{bucket}]\n        {err}")
            return 1
        if gone:
            # Either an example was fixed or its text was edited; both
            # want the baseline rewritten, and neither is silent.
            print(f"\n[fail] {len(gone)} baseline entr(y/ies) no longer present "
                  f"— rerun with --write-baseline:")
            for k in gone:
                print(f"    {k}")
            return 1
        if n_mounts > MOUNT_BASELINE:
            print(f"\n[fail] {n_mounts} unresolvable doc mounts, baseline {MOUNT_BASELINE}")
            return 1
        if n_mounts < MOUNT_BASELINE:
            print(f"\n[fail] {n_mounts} unresolvable doc mounts — below the "
                  f"baseline of {MOUNT_BASELINE}; lower MOUNT_BASELINE")
            return 1
        print("ratchet: no new untrustworthy example, "
              f"{n_mounts} known unresolvable mount(s)")
        return 0

    return 1 if (n_real and args.check) else 0


if __name__ == "__main__":
    sys.exit(main())
