#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! --update-snapshots auto-regenerate drift guard (#69).
//!
//! `vcs/runner/vtest/src/snapshot.rs` implements the golden-file comparison.
//! `compare_or_update(spec_path, name, actual, update)` is the core function.
//!
//! This drift guard pins:
//!   1. snapshot.rs exports `compare_or_update`.
//!   2. compare_or_update has an `update: bool` parameter.
//!   3. snapshot.rs defines `SnapshotResult` enum.
//!   4. SnapshotResult has Created variant.
//!   5. SnapshotResult has Updated variant.
//!   6. SnapshotResult has Mismatch variant.
//!   7. SnapshotResult has Missing variant.
//!   8. snapshot.rs exports `snapshot_path`.
//!   9. VCS spec is `@test: typecheck-pass`.
//!  10. VCS spec uses `@snapshot:` directive.

const SNAPSHOT_SRC: &str = include_str!("../../../vcs/runner/vtest/src/snapshot.rs");
const SPEC: &str = include_str!(
    "../../../vcs/specs/L2-standard/testing/update_snapshots_flag.vr"
);

// ── 1. snapshot.rs exports compare_or_update ─────────────────────────────────

#[test]
fn snapshot_exports_compare_or_update() {
    assert!(
        SNAPSHOT_SRC.contains("pub fn compare_or_update"),
        "snapshot.rs must export 'compare_or_update'"
    );
}

// ── 2. compare_or_update has update: bool param ──────────────────────────────

#[test]
fn compare_or_update_has_update_bool_param() {
    assert!(
        SNAPSHOT_SRC.contains("update: bool"),
        "compare_or_update must have an 'update: bool' parameter"
    );
}

// ── 3. SnapshotResult enum defined ───────────────────────────────────────────

#[test]
fn snapshot_defines_snapshot_result() {
    assert!(
        SNAPSHOT_SRC.contains("SnapshotResult"),
        "snapshot.rs must define 'SnapshotResult'"
    );
}

// ── 4. Created variant ────────────────────────────────────────────────────────

#[test]
fn snapshot_result_has_created_variant() {
    assert!(
        SNAPSHOT_SRC.contains("Created"),
        "SnapshotResult must have 'Created' variant"
    );
}

// ── 5. Updated variant ────────────────────────────────────────────────────────

#[test]
fn snapshot_result_has_updated_variant() {
    assert!(
        SNAPSHOT_SRC.contains("Updated"),
        "SnapshotResult must have 'Updated' variant"
    );
}

// ── 6. Mismatch variant ───────────────────────────────────────────────────────

#[test]
fn snapshot_result_has_mismatch_variant() {
    assert!(
        SNAPSHOT_SRC.contains("Mismatch"),
        "SnapshotResult must have 'Mismatch' variant"
    );
}

// ── 7. Missing variant ────────────────────────────────────────────────────────

#[test]
fn snapshot_result_has_missing_variant() {
    assert!(
        SNAPSHOT_SRC.contains("Missing"),
        "SnapshotResult must have 'Missing' variant"
    );
}

// ── 8. snapshot_path exported ────────────────────────────────────────────────

#[test]
fn snapshot_exports_snapshot_path() {
    assert!(
        SNAPSHOT_SRC.contains("pub fn snapshot_path"),
        "snapshot.rs must export 'snapshot_path'"
    );
}

// ── 9. VCS spec is typecheck-pass ─────────────────────────────────────────────

#[test]
fn spec_is_typecheck_pass() {
    assert!(
        SPEC.contains("@test: typecheck-pass"),
        "update_snapshots_flag.vr must be '@test: typecheck-pass'"
    );
}

// ── 10. VCS spec uses @snapshot directive ────────────────────────────────────

#[test]
fn spec_uses_snapshot_directive() {
    assert!(
        SPEC.contains("@snapshot:"),
        "update_snapshots_flag.vr must use '@snapshot:' directive"
    );
}
