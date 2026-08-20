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
    "register_declared_type_aliases": (
        "bootstrap",
        "Runs from `run_unit_declaration_prepasses`, which the shared "
        "collector calls for every file of the unit — so both paths get it "
        "by construction (T0692).",
    ),
    "collect_protocol_definitions": (
        "bootstrap",
        "The shared collector runs it across the whole unit before any item "
        "is collected; the bake's Pass 1a did the same thing and is gone.",
    ),
    "pregenerate_ffi_struct_layouts": (
        "bootstrap",
        "Pass 1a.6 in compile_core_module_from_ast calls "
        "run_unit_declaration_prepasses over every file of the module "
        "(T0692, f1c27ff62 + this commit). Was a GAP under T0362/T0640: the "
        "naive enable shifted Ordering Display rendering for Int8/Int64/"
        "Float/Text but not Int. That split had its own cause — a primitive "
        "receiver carried no type NAME, so `f\"{a.cmp(b)}\"` never learned "
        "its result is an Ordering — and with it fixed all five render "
        "identically.",
    ),
    "claim_user_type_name": (
        "bootstrap",
        "Same Pass 1a.6 (T0692). OWN-DECL-LAYOUT-EVICT-1 (T0125) is "
        "therefore live for the bake: a stdlib declaration now claims the "
        "simple type key and evicts a stale archive layout, as the "
        "single-file path has always done.",
    ),
}


def prepasses_in_collect_all() -> set[str]:
    """Names of `self.<method>(` calls inside the shared collector.

    Reads `collect_unit_declarations` — the ONE entry both compile paths
    use since T0692. It used to read `collect_all_declarations`, which
    was the single-file path's own sequence; the bake had a second one
    beside it, and this gate existed to keep the two comparable. They
    are now the same function, so the question this gate asks has
    changed shape: not "does the bake mirror each pass" but "does the
    bake still call only the shared entry" — which `bake_drives_shared_collector`
    checks below.
    """
    src = CODEGEN.read_text(encoding="utf-8")
    start = src.index("pub fn collect_unit_declarations")
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
    calls = {m.group(1) for m in re.finditer(r"self\.([a-z_][a-z0-9_]*)\(", body)}

    # `run_unit_declaration_prepasses` is a grouping, not a pass: expand
    # it so the individual passes stay individually classified. Without
    # this, wrapping a pass in the helper would hide it from the gate.
    if "run_unit_declaration_prepasses" in calls:
        calls.discard("run_unit_declaration_prepasses")
        helper_start = src.index("pub fn run_unit_declaration_prepasses")
        helper_tail = src[helper_start:]
        helper_end = len(helper_tail)
        for m in re.finditer(r"\n    \}\n", helper_tail):
            after = helper_tail[m.end() : m.end() + 400]
            if re.match(r"\s*(///|#\[|pub fn |fn )", after):
                helper_end = m.end()
                break
        calls |= {
            m.group(1)
            for m in re.finditer(
                r"self\.([a-z_][a-z0-9_]*)\(", helper_tail[:helper_end]
            )
        }
    return calls


def bake_drives_shared_collector() -> list[str]:
    """Complaints if the bake collects declarations on its own again.

    The defect this whole gate is about is a SECOND collection sequence
    living in the bootstrap. Now that one exists, the cheapest way to
    keep it one is to name the calls that would start a second: any
    `collect_*` on the codegen from the bootstrap other than the shared
    entry.
    """
    src = BOOTSTRAP.read_text(encoding="utf-8")
    allowed = {"collect_unit_declarations"}
    found = {
        m.group(1)
        for m in re.finditer(r"codegen\.(collect_[a-z_]+)\(", src)
    }
    return sorted(found - allowed)


def main() -> int:
    try:
        found = prepasses_in_collect_all()
        bootstrap_src = BOOTSTRAP.read_text(encoding="utf-8")
    except (OSError, ValueError) as e:
        print(f"check_bake_prepass_parity: cannot read sources: {e}", file=sys.stderr)
        return 2

    prepasses = found - NOT_A_PREPASS
    failures: list[str] = []

    # The stronger property, and the one that actually prevents the
    # defect: the bake must not start a collection sequence of its own.
    for stray in bake_drives_shared_collector():
        failures.append(
            f"the bake calls `codegen.{stray}(...)` directly.\n"
            f"    Declaration collection has ONE entry — "
            f"`collect_unit_declarations(files)` — precisely so the bake's "
            f"sequence cannot drift from the user path's again (T0692).\n"
            f"    Pass the files to that instead."
        )

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
