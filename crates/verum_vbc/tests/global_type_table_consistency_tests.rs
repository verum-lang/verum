//! Whole-program type-table consistency tests (#170).
//!
//! Compiles real stdlib `.vr` files via the production
//! `compile_module_with_mounts` path and asserts that the resulting
//! whole-program type table satisfies the cross-module hygiene
//! invariants:
//!
//!   * No two `TypeDescriptor`s share a single `TypeId.0`.
//!   * No two `TypeDescriptor`s share a name with conflicting ids.
//!   * Within every sum type, variant tags form a dense
//!     `0..variants.len()` set with no gaps and no duplicates.
//!
//! These invariants are the cross-module analogue of #146's per-module
//! `verify_type_layout_invariants`.  When a stdlib refactor introduces
//! a name collision (most commonly: two unrelated modules both declare
//! `public type Counter`) the offending build silently merges them
//! into a single `TypeId` slot and dispatch becomes wrong-variant —
//! exactly the kind of far-removed runtime symptom (`field index N
//! exceeds object data size K`, `null pointer dereference`) that this
//! check is designed to surface at compile time.
//!
//! The test suite uses small, focused stdlib subgraphs so a regression
//! pinpoints the introduced violation rather than burying it in a
//! whole-stdlib reload.

use verum_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

/// Locate the workspace's `core/` directory from `CARGO_MANIFEST_DIR`.
fn core_root() -> String {
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../core").to_string()
}

/// Compile a stdlib `.vr` file with full mount resolution and return
/// the resulting codegen so the caller can interrogate it.  Panics
/// on compile failure — these tests don't try to validate compile
/// errors, only the type-table invariants of *successful* builds.
fn compile_stdlib_subgraph(rel_path: &str) -> VbcCodegen {
    let core = core_root();
    let path = format!("{}/{}", core, rel_path);
    if !std::path::Path::new(&path).exists() {
        // No stdlib in this build environment — skip the assertion.
        // Mirrors the policy in `stdlib_lenient_skip_baseline.rs`.
        eprintln!(
            "[global_type_table_consistency] {} absent — skipping",
            path
        );
        return VbcCodegen::new();
    }
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {} failed: {}", path, e));
    let mut parser = Parser::new(&source);
    let module = parser
        .parse_module()
        .unwrap_or_else(|e| panic!("parse {} failed: {:?}", path, e));
    let config = CodegenConfig::new(&path).with_validation();
    let mut codegen = VbcCodegen::with_config(config);
    codegen
        .compile_module_with_mounts(&module, &path, &core)
        .unwrap_or_else(|e| panic!("codegen {} failed: {}", path, e));
    codegen
}

/// Format a health-report failure for the test panic message.  Lists
/// every category in a stable order so a CI diff identifies the
/// regression class precisely.
fn format_report_failure(
    scenario: &str,
    report: &verum_vbc::codegen::TypeTableHealthReport,
) -> String {
    let mut out = format!(
        "type-table consistency violations in `{}` ({} issue(s)):\n",
        scenario,
        report.issue_count(),
    );
    for d in &report.duplicate_ids {
        out.push_str(&format!(
            "  - duplicate TypeId({}) shared by descriptors: {:?}\n",
            d.type_id, d.descriptor_names,
        ));
    }
    for d in &report.duplicate_names_with_different_ids {
        out.push_str(&format!(
            "  - name `{}` declared with conflicting TypeIds: {:?}\n",
            d.name, d.type_ids,
        ));
    }
    for a in &report.variant_tag_anomalies {
        out.push_str(&format!(
            "  - variant tags non-dense in `{}` (TypeId({})): expected {} variants, \
             max tag {}, duplicates {:?}, missing {:?}\n",
            a.type_name,
            a.type_id,
            a.expected_count,
            a.max_tag_seen,
            a.duplicate_tags,
            a.missing_tags,
        ));
    }
    out.push_str(
        "\nA failure here means the stdlib build introduced one of the cross-module \
         type-table violations enumerated in #170.  The most common cause is two \
         modules declaring `public type X` for the same simple `X` — the second \
         declaration silently merges into the first's TypeId slot.  See \
         `stdlib_unique_type_names.rs` for the per-name ratchet that catches the \
         simpler form of the same bug class.",
    );
    out
}

/// Asserts the unified type table built by compiling `rel_path`
/// satisfies the global-consistency invariants.  Panics with a
/// detailed report if any violation is found.
fn assert_type_table_clean(rel_path: &str) {
    let codegen = compile_stdlib_subgraph(rel_path);
    let report = codegen.verify_global_type_table_consistency();
    if !report.is_clean() {
        panic!("{}", format_report_failure(rel_path, &report));
    }
}

// === Real stdlib subgraphs ============================================

/// Smallest meaningful stdlib subgraph: `core/base/maybe.vr` brings
/// in the `Maybe<T>` sum type and a handful of method impls but no
/// platform-specific mounts.  A regression that breaks the global
/// invariant on this file points at a fundamental codegen bug
/// rather than at any specific stdlib module.
#[test]
fn global_type_table_clean_for_maybe() {
    assert_type_table_clean("base/maybe.vr");
}

/// `core/base/result.vr` is mounted transitively by a substantial
/// portion of the async-runtime stdlib graph, so compiling it
/// surfaces a much wider type table than `maybe.vr` or `list.vr`.
/// As of #170 wire-up this fixture exposes 14 cross-module hygiene
/// findings — see `#170-followups` task series for the remediation
/// plan.  This test is therefore a *ratchet*: the count must not
/// rise.  When a finding is fixed, lower `RESULT_ISSUE_BASELINE` to
/// pin the gain so it can't silently regress.
///
/// Why ratchet rather than hard-fail at zero?  The findings are
/// pre-existing stdlib violations that this check exposed — fixing
/// them is a separate effort tracked under #181 (stdlib production
/// audit).  Blocking the merge of #170's check on those fixes
/// would conflate two independent tasks.
const RESULT_ISSUE_BASELINE: usize = 14;

#[test]
fn global_type_table_baseline_for_result() {
    let codegen = compile_stdlib_subgraph("base/result.vr");
    let report = codegen.verify_global_type_table_consistency();
    let count = report.issue_count();
    if count > RESULT_ISSUE_BASELINE {
        panic!(
            "type-table issue count for `base/result.vr` regressed: {} > baseline {}\n\n{}",
            count,
            RESULT_ISSUE_BASELINE,
            format_report_failure("base/result.vr (regressed)", &report),
        );
    }
    if count < RESULT_ISSUE_BASELINE {
        panic!(
            "type-table issue count for `base/result.vr` IMPROVED: {} < baseline {}.\n\
             Lower RESULT_ISSUE_BASELINE in this file to pin the new value, \
             so the gain can't silently regress.",
            count, RESULT_ISSUE_BASELINE,
        );
    }
}

/// `core/collections/list.vr` is the largest single-file collection
/// type with a rich variant + method surface.  Picks up most of the
/// generic-type-instantiation paths under one fixture.
#[test]
fn global_type_table_clean_for_list() {
    assert_type_table_clean("collections/list.vr");
}

// === Synthetic regression repros ======================================

/// Sanity check: an empty codegen has a clean health report.  Pins
/// the "no false positives on a fresh codegen" baseline — important
/// because the per-module verifier is invoked unconditionally during
/// `finalize_module`.
#[test]
fn global_type_table_clean_for_fresh_codegen() {
    let codegen = VbcCodegen::new();
    let report = codegen.verify_global_type_table_consistency();
    assert!(
        report.is_clean(),
        "fresh VbcCodegen must produce a clean type-table report: {:?}",
        report,
    );
    assert_eq!(report.issue_count(), 0);
}
