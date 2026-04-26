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
/// satisfies the global-consistency invariants — modulo the
/// intentional `Heap` / `Shared` alias both bound to
/// `TypeId::PTR (14)`.  That alias is by-design (both wrapper
/// types share PTR-level dispatch); resolving it would require
/// multi-name-per-descriptor support in the well-known TypeId
/// map, tracked under #167.
fn assert_type_table_clean(rel_path: &str) {
    let codegen = compile_stdlib_subgraph(rel_path);
    let report = codegen.verify_global_type_table_consistency();

    // Intentional aliases — anything that lands on TypeId(14) and
    // claims one of the well-known PTR-alias names is expected.
    let is_intentional_ptr_alias = |d: &verum_vbc::codegen::DuplicateTypeId| -> bool {
        d.type_id == 14
            && d.descriptor_names.iter().all(|n| {
                matches!(n.as_str(), "Heap" | "Shared")
            })
    };
    let real_dup_ids: Vec<_> = report
        .duplicate_ids
        .iter()
        .filter(|d| !is_intentional_ptr_alias(d))
        .cloned()
        .collect();
    if !real_dup_ids.is_empty()
        || !report.duplicate_names_with_different_ids.is_empty()
        || !report.variant_tag_anomalies.is_empty()
    {
        // Build a filtered report for the failure message.
        let filtered = verum_vbc::codegen::TypeTableHealthReport {
            duplicate_ids: real_dup_ids,
            duplicate_names_with_different_ids: report
                .duplicate_names_with_different_ids
                .clone(),
            variant_tag_anomalies: report.variant_tag_anomalies.clone(),
        };
        panic!("{}", format_report_failure(rel_path, &filtered));
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
/// As of #170/#187 close-out this fixture exposes 1 cross-module
/// hygiene finding (down from 14 across multiple follow-ups):
///
///   1. Added `Channel`/`Deque`/`Tuple`/`Array` to the well-known
///      type-name map (14 → 15, briefly worse — exposed a separate
///      collision class).
///   2. Routed user TypeId allocation through `alloc_user_type_id`
///      so the auto-allocator skips the reserved 256..260 and
///      512..1024 ranges (15 → 13).
///   3. Honoured per-item `@cfg` gates on `mount` declarations
///      inside `resolve_mounts_recursive` (13 → 10).
///   4. Walked `TypeDecl.attributes` from `should_compile_item` —
///      the parser places type-decl `@cfg` attributes on the inner
///      `TypeDecl`, not on `Item` (10 → 7).
///   5. Renamed `RuntimeConfig` → `AsyncRuntimeConfig` in
///      `core/async/executor.vr` (the protocol vs record collision
///      with `core/runtime/config.vr`) (7 → 6).
///   6. Renamed `Task`/`TaskHandle` → `RuntimeTask`/`RuntimeTaskHandle`
///      in `core/runtime/config.vr` (the internal vs public
///      collision with `core/async/{task,nursery}.vr`) (6 → 4).
///   7. Renamed internal `AtomicBool` → `RuntimeAtomicBool` in
///      `core/runtime/config.vr` (the internal AtomicU32-backed
///      version vs the public `core/sync/atomic.vr` version) (4 → 3).
///   8. Renamed internal `YieldNow` → `SelectYieldNow` in
///      `core/async/select.vr`; renamed internal `CallbackEntry` →
///      `CancellationCallback` in `core/async/cancellation.vr`
///      (3 → 1).
///
/// The remaining finding is the `Heap` / `Shared` pair both
/// pointing at `TypeId::PTR (14)` — INTENTIONAL alias case where
/// two type names are deliberately bound to the same id.  Fixing
/// this would require changing the well-known TypeId architecture
/// to allow multi-name aliases without distinct descriptors;
/// out of scope for #187.  See #167 (TypeId / opcode-space
/// extension).
///
/// This test is a *ratchet*: the count must not rise, and must
/// match exactly when it falls (so any improvement gets pinned).
/// When a finding is fixed, lower `RESULT_ISSUE_BASELINE` to lock
/// in the gain.
const RESULT_ISSUE_BASELINE: usize = 1;

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
             so the gain can't silently regress.\n\n{}",
            count, RESULT_ISSUE_BASELINE,
            format_report_failure("base/result.vr (improved — refresh baseline)", &report),
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

/// Diagnostic print of orphan MakeVariant count for visibility.
/// Always passes; informational only.
#[test]
fn orphan_make_variants_dump_for_result() {
    let codegen = compile_stdlib_subgraph("base/result.vr");
    let orphans = codegen.find_orphan_make_variants();
    eprintln!("[#188 dump] base/result.vr — {} orphan MakeVariant(s)", orphans.len());
}

// === Strict-codegen end-to-end test (#166) ============================

/// Verify that `strict_codegen` mode actually halts the build when
/// a bug-class skip would otherwise fire silently.  Uses a synthetic
/// module declaring a function whose body references an undefined
/// symbol — the lenient path warns and continues, the strict path
/// returns `Err(CodegenError)` from the call.
///
/// This pins the contract that `with_strict_codegen()` isn't just
/// a config flag with no observable effect.
#[test]
fn strict_codegen_halts_on_bug_class_skip() {
    use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

    // Source that references an undefined function `nope_undefined_42`.
    // Compiles cleanly under lenient mode (warn-level skip) and
    // returns an error under strict mode.
    let source = "fn caller() -> Int { nope_undefined_42() }";
    let mut parser = verum_parser::Parser::new(source);
    let module = parser
        .parse_module()
        .expect("parse should succeed; the body is the diagnostic");

    // === Lenient (default): compile_module_items_lenient returns Ok
    {
        let mut codegen = VbcCodegen::with_config(
            CodegenConfig::new("test_lenient").with_validation(),
        );
        codegen.collect_protocol_definitions(&module);
        codegen
            .collect_all_declarations(&module)
            .expect("declarations should succeed");
        let result = codegen.compile_module_items_lenient(&module);
        assert!(
            result.is_ok(),
            "lenient mode should warn and continue, not fail: {:?}",
            result.err(),
        );
    }

    // === Strict: compile_module_items_lenient returns Err
    {
        let mut codegen = VbcCodegen::with_config(
            CodegenConfig::new("test_strict")
                .with_validation()
                .with_strict_codegen(),
        );
        codegen.collect_protocol_definitions(&module);
        codegen
            .collect_all_declarations(&module)
            .expect("declarations should succeed");
        let result = codegen.compile_module_items_lenient(&module);
        assert!(
            result.is_err(),
            "strict mode must fail-fast on bug-class skip — `caller` \
             references an undefined function and that's the canonical \
             bug-class signal.  Without this fail, `with_strict_codegen()` \
             is a no-op flag.",
        );
    }
}

/// Print the current findings on result.vr — useful for diagnostic
/// runs (`cargo test -p verum_vbc --features codegen
/// global_type_table_dump_for_result -- --nocapture`).  Always
/// passes; the assertion is purely informational.
#[test]
fn global_type_table_dump_for_result() {
    let codegen = compile_stdlib_subgraph("base/result.vr");
    let report = codegen.verify_global_type_table_consistency();
    eprintln!(
        "[#170 dump] base/result.vr — {} cross-module hygiene finding(s)",
        report.issue_count(),
    );
    for d in &report.duplicate_ids {
        eprintln!(
            "[#170 dump]   duplicate TypeId({}) shared by {:?}",
            d.type_id, d.descriptor_names,
        );
    }
    for d in &report.duplicate_names_with_different_ids {
        eprintln!(
            "[#170 dump]   name `{}` declared with conflicting TypeIds: {:?}",
            d.name, d.type_ids,
        );
    }
    for a in &report.variant_tag_anomalies {
        eprintln!(
            "[#170 dump]   variant tags non-dense in `{}` (TypeId({})): \
             expected {} variants, max tag {}, duplicates {:?}, missing {:?}",
            a.type_name, a.type_id, a.expected_count,
            a.max_tag_seen, a.duplicate_tags, a.missing_tags,
        );
    }
}

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

// === Orphan MakeVariant ratchet (#188) =================================

/// Ratchet test for `find_orphan_make_variants`.
///
/// At a single-module-with-mounts granularity most "orphans" are
/// legitimate — they reference variants whose declaring module
/// wasn't fully transitively loaded by the test harness.  But the
/// COUNT itself is a useful regression signal: if a codegen change
/// introduces NEW orphans (instructions that the runtime can't
/// resolve), the count rises and this ratchet trips.
///
/// When a fix lands that resolves orphans (better mount transitive
/// closure, MakeVariantTyped from #167 Phase 3, etc.), lower
/// `RESULT_ORPHAN_BASELINE` to lock the gain.
const RESULT_ORPHAN_BASELINE: usize = 280;

#[test]
fn orphan_make_variants_baseline_for_result() {
    let codegen = compile_stdlib_subgraph("base/result.vr");
    let orphans = codegen.find_orphan_make_variants();
    let count = orphans.len();
    if count > RESULT_ORPHAN_BASELINE {
        // Print up to 16 orphans for diagnostic so the regressing
        // codegen path is identifiable from a CI log.
        let mut sample = String::new();
        for o in orphans.iter().take(16) {
            sample.push_str(&format!(
                "  - MakeVariant {{ tag: {}, field_count: {} }} in `{}`\n",
                o.tag, o.field_count, o.function_name,
            ));
        }
        if orphans.len() > 16 {
            sample.push_str(&format!(
                "  ... and {} more\n",
                orphans.len() - 16,
            ));
        }
        panic!(
            "orphan MakeVariant count for `base/result.vr` regressed: {} > baseline {}\n\n\
             First {} orphans:\n{}\n\
             A regression here means a codegen change introduced \
             `MakeVariant` instructions that the global type table \
             can't resolve.  See #170 / #188 / #167 Phase 3.",
            count,
            RESULT_ORPHAN_BASELINE,
            16.min(orphans.len()),
            sample,
        );
    }
    if count < RESULT_ORPHAN_BASELINE {
        panic!(
            "orphan MakeVariant count for `base/result.vr` IMPROVED: {} < baseline {}.\n\
             Lower RESULT_ORPHAN_BASELINE in this file to pin the new value, \
             so the gain can't silently regress.",
            count, RESULT_ORPHAN_BASELINE,
        );
    }
}
