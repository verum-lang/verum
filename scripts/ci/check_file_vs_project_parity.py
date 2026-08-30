#!/usr/bin/env python3
"""Gate: one source, one verdict — `verum check FILE` vs `verum check` in a project.

The same file gets two answers depending on which command runs. Measured
instances, each a separate pool row:

  * a stdlib CONTEXT resolves in a single file and is `E605: undefined
    context` inside a project (T0974) — the project path received the
    metadata and skipped the registration the single-file path does;
  * variant constructors did the same until T0865 — bare `Ok(x)` was
    "not a function: Ok" in a project, 110 of 334 registry errors;
  * 535 of 2 561 `core/` files fail `verum check` while the bake, which
    compiles the same sources, is green (T0755).

Three symptoms, one shape: two drivers over one language, and a step
present in one of them.

THERE IS A THIRD DRIVER, and it is softer than both. The stdlib BAKE
compiles `core/` and accepts programs the checker refuses — measured
2026-08-30 on a file that ships inside the archive:

    $ verum check core/database/sqlite/native/l0_vfs/mock_vfs.vr
    error<E400>: Type mismatch: expected 'MockBlob', found 'Maybe<MockBlob>'

The checker is RIGHT there; the bake let a real type error through. So a
gate comparing two of the three routes reports "one source, one verdict"
for exactly that case — both compared paths refuse, the third accepts,
and the disagreement is invisible.

That is this gate's own failure mode written down: a comparison covers
the routes it samples, and the conclusion gets stated about the language.
The `bake_accepts` column below closes it for `core/` files, which are
the only ones the bake sees.

This gate writes a handful of small programs, checks each BOTH ways, and
fails when the two verdicts differ. It is deliberately tiny — the point
is the comparison, not coverage.

    scripts/ci/check_file_vs_project_parity.py [--verum PATH] [--keep]
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

# (name, source). Each must be a complete program.
CASES: list[tuple[str, str]] = [
    (
        "stdlib_context",
        """mount core.context.standard.{Logger};

fn emit() using [Logger] {
    Logger.info("hello");
}

fn main() {
    print("ok");
}
""",
    ),
    (
        "variant_ctor",
        """fn wrap(n: Int) -> Maybe<Int> {
    Some(n)
}

fn main() {
    match wrap(1) {
        Some(v) => print(f"got {v}"),
        None    => print("none"),
    }
}
""",
    ),
    (
        "refinement_in_signature",
        """type Pos is n: Int where n > 0;

fn bump(p: Pos) -> Int {
    p + 1
}

fn main() {
    print(f"{bump(1)}");
}
""",
    ),
    # NEGATIVE cases. A program that must be REFUSED catches the failure
    # the positive ones cannot see: a check switched on in one driver and
    # off in the other reads as "clean" on both sides of a positive test.
    # The single-file path enables dependent types, higher-kinded
    # protocols, generic associated types and a protocol-coherence mode
    # that the project path never turns on — so a rule can be live in one
    # command and absent in the other while every valid program passes.
    (
        "NEG_const_generic_mismatch",
        """type Vector<const N: Int> is { items: List<Int> };

fn takes_three(v: Vector<3>) -> Int { 3 }

fn main() {
    let four: Vector<4> = Vector { items: [1, 2, 3, 4] };
    print(f"{takes_three(four)}");
}
""",
    ),
    (
        "NEG_refinement_violated",
        """type Pos is n: Int where n > 0;

fn bump(p: Pos) -> Int { p + 1 }

fn main() {
    let bad: Pos = 0 - 5;
    print(f"{bump(bad)}");
}
""",
    ),
    (
        "NEG_undeclared_name",
        """fn main() {
    print(f"{no_such_function_anywhere(1)}");
}
""",
    ),
]

MANIFEST = """[cog]
name = "parity_probe"
version = "0.1.0"
edition = "2026"
"""


def find_verum(explicit: str | None) -> Path | None:
    if explicit:
        p = Path(explicit)
        return p if p.is_file() else None
    for rel in ("target/release/verum", "target/debug/verum"):
        p = REPO / rel
        if p.is_file():
            return p
    return None


def run(verum: Path, args: list[str], cwd: Path) -> tuple[int, str]:
    proc = subprocess.run(
        [str(verum), *args],
        cwd=cwd,
        capture_output=True,
        text=True,
        timeout=300,
    )
    return proc.returncode, (proc.stdout + proc.stderr)


# `core/` files the checker refuses, in a systematic every-Nth sample.
# The BAKE accepted all of them — it succeeded, and every core/ file is
# its input — so each one is a three-way disagreement the two-route
# comparison above cannot see. A ratchet: it may only go down.
CORE_SAMPLE_STRIDE = 40
# Measured BY THIS GATE, on its own sample. An earlier hand-run of the
# same stride reported 3 because it took only the first 50 of the 64 —
# a baseline has to come from the instrument that will re-measure it,
# or the first run after landing looks like a regression.
CORE_SAMPLE_BASELINE_REFUSED = 6


def measure_bake_vs_checker(verum: Path) -> tuple[int, int, list[str]]:
    """Count core/ files the checker refuses. The bake accepted every one."""
    files = sorted((REPO / "core").rglob("*.vr"))
    sample = files[::CORE_SAMPLE_STRIDE]
    refused: list[str] = []
    for f in sample:
        proc = subprocess.run(
            [str(verum), "check", str(f)],
            cwd=REPO, capture_output=True, text=True, timeout=300,
        )
        if proc.returncode != 0:
            refused.append(str(f.relative_to(REPO)))
    return len(sample), len(files), refused


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--verum")
    ap.add_argument("--keep", action="store_true", help="leave the temp tree in place")
    ap.add_argument(
        "--skip-core",
        action="store_true",
        help="skip the bake-vs-checker sample (it runs the checker ~64 times)",
    )
    args = ap.parse_args()

    verum = find_verum(args.verum)
    if verum is None:
        print(
            "no verum binary at target/{release,debug}/verum — "
            "pass one with --verum PATH. NOT CHECKED.",
            file=sys.stderr,
        )
        return 0

    root = Path(tempfile.mkdtemp(prefix="verum-parity-"))
    mismatches: list[str] = []
    inert: list[str] = []
    checked = 0
    try:
        for name, source in CASES:
            # Each case gets its OWN project: several files declaring
            # `main` in one project currently run an arbitrary one of
            # them (T0979), which would confound this comparison.
            proj = root / name
            (proj / "src").mkdir(parents=True)
            (proj / "verum.toml").write_text(MANIFEST)
            src = proj / "src" / "main.vr"
            src.write_text(source)

            file_rc, file_out = run(verum, ["check", str(src)], root)
            proj_rc, proj_out = run(verum, ["check"], proj)
            checked += 1

            if (file_rc == 0) != (proj_rc == 0):
                worse = "project" if proj_rc != 0 else "file"
                detail = (proj_out if worse == "project" else file_out).strip()
                first = next(
                    (ln for ln in detail.splitlines() if "error" in ln), detail[:120]
                )
                mismatches.append(
                    f"  {name}: file rc={file_rc}, project rc={proj_rc} "
                    f"— {worse} rejects\n      {first.strip()[:120]}"
                )
            elif name.startswith("NEG_") and file_rc == 0:
                # Agreeing is not enough for a program that must be
                # REFUSED: both drivers accepting it means the rule is
                # off everywhere, and comparing verdicts would call that
                # parity. This is the vacuous-gate failure the whole
                # `check_result_free_tests` census is about, one level up.
                inert.append(f"  {name}: accepted by BOTH — the rule is not in force")
    finally:
        if not args.keep:
            shutil.rmtree(root, ignore_errors=True)

    # Positive control: with no case actually run, "no mismatches" would
    # be meaningless.
    if checked != len(CASES):
        print(
            f"REFUSING TO PASS: ran {checked} of {len(CASES)} cases",
            file=sys.stderr,
        )
        return 2

    print(
        f"file-vs-project parity: {checked} programs checked both ways "
        f"({sum(1 for n, _ in CASES if n.startswith('NEG_'))} of them expected to fail)"
    )
    if inert:
        print(
            f"\n{len(inert)} program(s) that must be REFUSED were accepted "
            "by both drivers:\n",
            file=sys.stderr,
        )
        for m in inert:
            print(m, file=sys.stderr)
    # THE THIRD ROUTE. Cheap because the bake's verdict needs no
    # measuring: it SUCCEEDED, and every core/ file is its input, so a
    # core/ file the checker refuses is by construction a disagreement.
    core_bad: list[str] = []
    if not args.skip_core and (REPO / "core").is_dir():
        n_sample, n_total, core_bad = measure_bake_vs_checker(verum)
        print(
            f"bake-vs-checker: {len(core_bad)} of {n_sample} sampled core/ files "
            f"(every {CORE_SAMPLE_STRIDE}th of {n_total}) are REFUSED by the "
            f"checker and were ACCEPTED by the bake (baseline "
            f"{CORE_SAMPLE_BASELINE_REFUSED})"
        )
        for f in core_bad[:5]:
            print(f"    {f}")
        if len(core_bad) > CORE_SAMPLE_BASELINE_REFUSED:
            print(
                f"\n{len(core_bad) - CORE_SAMPLE_BASELINE_REFUSED} more than the "
                f"baseline. The bake is the softest of the three routes, so a "
                f"type error it lets through ships inside the archive.",
                file=sys.stderr,
            )
        elif len(core_bad) < CORE_SAMPLE_BASELINE_REFUSED:
            print(
                f"\nFewer than the baseline. Lower "
                f"CORE_SAMPLE_BASELINE_REFUSED to {len(core_bad)}.",
                file=sys.stderr,
            )

    if not mismatches and not inert and len(core_bad) <= CORE_SAMPLE_BASELINE_REFUSED:
        print("one source, one verdict")
        return 0
    if not mismatches:
        return 1
    print(f"\n{len(mismatches)} program(s) get DIFFERENT verdicts:\n", file=sys.stderr)
    for m in mismatches:
        print(m, file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
