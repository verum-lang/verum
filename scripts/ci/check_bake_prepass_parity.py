"""Gate: every pre-pass in `collect_all_declarations` is accounted for on the
stdlib-bake path.

WHY THIS EXISTS
---------------
`VbcCodegen::collect_all_declarations` is where the declaration-order
pre-passes live — the passes whose whole purpose is to defeat "the AST visited
this before that".  The stdlib bootstrap does NOT call it: it drives
`collect_protocol_definitions` + `collect_non_protocol_declarations` per file
(`stdlib_bootstrap.rs::compile_core_module_from_ast`).

So a pre-pass added to `collect_all_declarations` is INERT for the shipped
`.vbca` while staying green in every single-module unit test, because those
tests DO call it.  The failure mode is invisible to the test suite by
construction.  Three separate correctness defects have already traced to this
one gap:

  * variant-form type aliases  -> worked around by bootstrap Phase 2.9
  * blanket impls (T0625)      -> fixed by Pass 1a.5 + a global registry
  * FFI struct layouts (T0362) -> found 2026-07-25, see COVERAGE below

Each cost a session to rediscover.  This gate does not fix the divergence; it
makes a NEW instance impossible to introduce silently.  Every pre-pass must be
listed in COVERAGE with an explicit verdict, and an unlisted one fails the
build.

Exit codes: 0 all pre-passes classified; 1 unclassified or stale entries; 2 a
source file could not be read.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CODEGEN = REPO / "crates" / "verum_vbc" / "src" / "codegen" / "mod.rs"
BOOTSTRAP = REPO / "crates" / "verum_compiler" / "src" / "pipeline" / "stdlib_bootstrap.rs"

# Helpers that are not pre-passes: control flow, per-item work driven by the
# enclosing loop, or predicates.  Listing them here keeps the gate's signal
# about *pre-passes* rather than about every method call in the function.
NOT_A_PREPASS = {
    "collect_declarations",  # the per-item walk the pre-passes exist to precede
    "should_compile_item",  # a predicate
}

# Every pre-pass, with how the stdlib bake covers it.  "bootstrap" means the
# bake performs equivalent work (possibly under a different name — record
# WHICH); "GAP" means it does not, and names the tracking task.
COVERAGE = {
    "collect_blanket_impls": (
        "bootstrap",
        "Pass 1a.5 in compile_core_module_from_ast calls this same method over "
        "every file of the module, plus global_blanket_impl_registry for the "
        "cross-module half (T0625, e29d52eaa).",
    ),
    "declared_alias_target_name": (
        "bootstrap",
        "Phase 2.9 does the equivalent work inline via "
        "VbcCodegen::detect_variant_form_alias over all parsed modules; it does "
        "not call this helper by name.",
    ),
    "pregenerate_ffi_struct_layouts": (
        "GAP",
        "T0362/T0640: the bake never runs this, so forward-declared record "
        "types in extern signatures do not resolve to StructPtr in the "
        "archive. A naive fix (calling it from the bootstrap) BAKES CLEAN but "
        "changes Display dispatch — see T0640.",
    ),
    "claim_user_type_name": (
        "GAP",
        "T0640: same unreachable call-site pattern; OWN-DECL-LAYOUT-EVICT-1 "
        "(T0125) is therefore inert for the bake. Candidate cause of T0408. "
        "A naive bootstrap-side call bakes clean but shifts Ordering Display "
        "dispatch inconsistently — see T0640.",
    ),
}


def prepasses_in_collect_all() -> set[str]:
    """Names of `self.<method>(` calls inside `collect_all_declarations`."""
    src = CODEGEN.read_text(encoding="utf-8")
    start = src.index("pub fn collect_all_declarations")
    # The function ends at the next top-level `    }` followed by a blank line
    # and another `    ///` or `    pub fn` / `    fn` at the same indent.
    tail = src[start:]
    end = len(tail)
    for m in re.finditer(r"\n    \}\n", tail):
        after = tail[m.end() : m.end() + 400]
        if re.match(r"\s*(///|#\[|pub fn |fn )", after):
            end = m.end()
            break
    body = tail[:end]
    return {m.group(1) for m in re.finditer(r"self\.([a-z_][a-z0-9_]*)\(", body)}


def main() -> int:
    try:
        found = prepasses_in_collect_all()
        bootstrap_src = BOOTSTRAP.read_text(encoding="utf-8")
    except (OSError, ValueError) as e:
        print(f"check_bake_prepass_parity: cannot read sources: {e}", file=sys.stderr)
        return 2

    prepasses = found - NOT_A_PREPASS
    failures: list[str] = []

    unclassified = sorted(prepasses - COVERAGE.keys())
    for name in unclassified:
        failures.append(
            f"  UNCLASSIFIED pre-pass `{name}` in collect_all_declarations.\n"
            f"    The stdlib bake does NOT call collect_all_declarations, so this\n"
            f"    pass is probably inert for the shipped archive. Verify whether\n"
            f"    compile_core_module_from_ast covers it, then add it to COVERAGE\n"
            f"    in {Path(__file__).name} with the verdict and the reason."
        )

    stale = sorted(COVERAGE.keys() - prepasses)
    for name in stale:
        failures.append(
            f"  STALE COVERAGE entry `{name}` — it is no longer called in\n"
            f"    collect_all_declarations. Remove it from COVERAGE."
        )

    # A "bootstrap" verdict that names a concrete bake-side symbol is checked.
    for name, (verdict, reason) in sorted(COVERAGE.items()):
        if verdict == "bootstrap" and name in prepasses and name in bootstrap_src:
            continue  # called directly — nothing to check
        if verdict == "bootstrap" and name in prepasses:
            cited = re.findall(r"\b(Phase \d+\.\d+|[A-Za-z_]+::[a-z_]+|[a-z_]{6,})\b", reason)
            if not cited:
                failures.append(
                    f"  `{name}` is marked covered by the bootstrap but the reason\n"
                    f"    names no concrete mechanism. Say WHAT does the work."
                )

    gaps = sorted(n for n, (v, _) in COVERAGE.items() if v in ("GAP", "UNAUDITED") and n in prepasses)

    if failures:
        print("check_bake_prepass_parity: FAILED\n")
        print("\n".join(failures))
        return 1

    print(f"check_bake_prepass_parity: OK — {len(prepasses)} pre-pass(es) classified")
    if gaps:
        print(f"  known gaps/unaudited (tracked by T0640): {', '.join(gaps)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
