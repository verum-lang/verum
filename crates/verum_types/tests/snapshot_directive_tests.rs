#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! @snapshot directive drift guard (#61).
//!
//! `vcs/runner/vtest/src/snapshot.rs` implements golden-file snapshot testing.
//! `vcs/runner/vtest/src/directive.rs` wires the `@snapshot:` directive.
//!
//! This drift guard pins:
//!   1. snapshot.rs defines `SnapshotResult` with Match/Created/Updated/Mismatch/Missing.
//!   2. snapshot.rs exports `compare_or_update` function.
//!   3. snapshot.rs exports `snapshot_path` function.
//!   4. `compare_or_update` accepts an `update: bool` parameter.
//!   5. `SnapshotResult::Mismatch` carries `expected` and `actual` fields.
//!   6. `SnapshotResult::Missing` carries a `path` field.
//!   7. directive.rs documents `@snapshot:` directive.
//!   8. `TestDirectives` has `snapshot: Option<Text>` field.
//!   9. Parser handles `@snapshot:` prefix.
//!  10. The VCS spec uses `@snapshot: greeting_output`.

const SNAPSHOT_RS: &str = include_str!("../../../vcs/runner/vtest/src/snapshot.rs");
const DIRECTIVE_RS: &str = include_str!("../../../vcs/runner/vtest/src/directive.rs");
const SNAPSHOT_SPEC: &str = include_str!(
    "../../../vcs/specs/L2-standard/testing/snapshot_golden_file.vr"
);

// ── 1. SnapshotResult variants ────────────────────────────────────────────────

#[test]
fn snapshot_result_match_variant_exists() {
    assert!(
        SNAPSHOT_RS.contains("Match,") || SNAPSHOT_RS.contains("Match\n"),
        "SnapshotResult must have a Match variant in snapshot.rs"
    );
}

#[test]
fn snapshot_result_created_variant_exists() {
    assert!(
        SNAPSHOT_RS.contains("Created,") || SNAPSHOT_RS.contains("Created\n"),
        "SnapshotResult must have a Created variant in snapshot.rs"
    );
}

#[test]
fn snapshot_result_updated_variant_exists() {
    assert!(
        SNAPSHOT_RS.contains("Updated,") || SNAPSHOT_RS.contains("Updated\n"),
        "SnapshotResult must have an Updated variant in snapshot.rs"
    );
}

#[test]
fn snapshot_result_mismatch_variant_exists() {
    assert!(
        SNAPSHOT_RS.contains("Mismatch"),
        "SnapshotResult must have a Mismatch variant in snapshot.rs"
    );
}

#[test]
fn snapshot_result_missing_variant_exists() {
    assert!(
        SNAPSHOT_RS.contains("Missing"),
        "SnapshotResult must have a Missing variant in snapshot.rs"
    );
}

// ── 2. compare_or_update exported ────────────────────────────────────────────

#[test]
fn compare_or_update_fn_exists() {
    assert!(
        SNAPSHOT_RS.contains("pub fn compare_or_update"),
        "snapshot.rs must export 'pub fn compare_or_update'"
    );
}

// ── 3. snapshot_path exported ────────────────────────────────────────────────

#[test]
fn snapshot_path_fn_exists() {
    assert!(
        SNAPSHOT_RS.contains("pub fn snapshot_path"),
        "snapshot.rs must export 'pub fn snapshot_path'"
    );
}

// ── 4. compare_or_update accepts update: bool ────────────────────────────────

#[test]
fn compare_or_update_has_update_bool_param() {
    assert!(
        SNAPSHOT_RS.contains("update: bool"),
        "compare_or_update must accept 'update: bool' parameter"
    );
}

// ── 5. Mismatch carries expected and actual ───────────────────────────────────

#[test]
fn mismatch_has_expected_field() {
    assert!(
        SNAPSHOT_RS.contains("expected:"),
        "SnapshotResult::Mismatch must have an 'expected' field"
    );
}

#[test]
fn mismatch_has_actual_field() {
    assert!(
        SNAPSHOT_RS.contains("actual:"),
        "SnapshotResult::Mismatch must have an 'actual' field"
    );
}

// ── 6. Missing carries path ───────────────────────────────────────────────────

#[test]
fn missing_has_path_field() {
    assert!(
        SNAPSHOT_RS.contains("path:"),
        "SnapshotResult::Missing must have a 'path' field"
    );
}

// ── 7. directive.rs documents @snapshot: ─────────────────────────────────────

#[test]
fn directive_module_doc_lists_snapshot() {
    assert!(
        DIRECTIVE_RS.contains("@snapshot:"),
        "directive.rs module doc must document '@snapshot: <name>'"
    );
}

// ── 8. TestDirectives has snapshot field ─────────────────────────────────────

#[test]
fn test_directives_struct_has_snapshot_field() {
    assert!(
        DIRECTIVE_RS.contains("pub snapshot: Option<Text>"),
        "TestDirectives must have 'pub snapshot: Option<Text>' field"
    );
}

// ── 9. Parser handles @snapshot: prefix ──────────────────────────────────────

#[test]
fn parser_handles_snapshot_prefix() {
    assert!(
        DIRECTIVE_RS.contains("strip_prefix(\"@snapshot:\")"),
        "directive.rs parser must handle '@snapshot:' via strip_prefix"
    );
}

#[test]
fn parser_assigns_snapshot_to_directives() {
    assert!(
        DIRECTIVE_RS.contains("directives.snapshot"),
        "directive.rs must assign 'directives.snapshot' when @snapshot: is found"
    );
}

// ── 10. VCS spec uses @snapshot: greeting_output ─────────────────────────────

#[test]
fn snapshot_spec_uses_snapshot_directive() {
    assert!(
        SNAPSHOT_SPEC.contains("@snapshot: greeting_output"),
        "snapshot_golden_file.vr must use '@snapshot: greeting_output'"
    );
}

#[test]
fn snapshot_spec_is_typecheck_pass() {
    assert!(
        SNAPSHOT_SPEC.contains("@test: typecheck-pass"),
        "snapshot_golden_file.vr must be '@test: typecheck-pass'"
    );
}
