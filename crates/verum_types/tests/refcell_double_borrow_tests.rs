#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! RefCell double-borrow-mut panic drift guard (#71).
//!
//! `core/base/cell.vr` defines RefCell<T> with runtime borrow checking.
//! A second call to borrow_mut() while a mutable borrow is live must panic.
//!
//! This drift guard pins:
//!   1. cell.vr defines `RefCell<T>` type.
//!   2. cell.vr defines `BorrowMutError` type with a `message` field.
//!   3. cell.vr has `borrow_mut` method on RefCell.
//!   4. cell.vr has `try_borrow_mut` method returning Result.
//!   5. cell.vr has `borrow_count` method.
//!   6. borrow_mut panics — either `panic(` call or "already borrowed" message.
//!   7. try_borrow_mut returns Err on conflict.
//!   8. The VCS spec uses `RefCell.new`, `borrow_mut`, `try_borrow_mut`.
//!   9. The VCS spec uses `catch_unwind` to verify the panic contract.

const CELL_VR: &str = include_str!("../../../core/base/cell.vr");
const REFCELL_SPEC: &str = include_str!(
    "../../../vcs/specs/L2-standard/testing/refcell_double_borrow_mut_panic.vr"
);

// ── 1. RefCell<T> type exists ─────────────────────────────────────────────────

#[test]
fn cell_vr_defines_refcell_type() {
    assert!(
        CELL_VR.contains("RefCell<T>") || CELL_VR.contains("type RefCell"),
        "cell.vr must define RefCell<T>"
    );
}

// ── 2. BorrowMutError with message field ─────────────────────────────────────

#[test]
fn cell_vr_defines_borrow_mut_error() {
    assert!(
        CELL_VR.contains("BorrowMutError"),
        "cell.vr must define BorrowMutError"
    );
}

#[test]
fn borrow_mut_error_has_message_field() {
    assert!(
        CELL_VR.contains("message:"),
        "BorrowMutError in cell.vr must have a 'message' field"
    );
}

// ── 3. borrow_mut method ──────────────────────────────────────────────────────

#[test]
fn cell_vr_has_borrow_mut_method() {
    assert!(
        CELL_VR.contains("fn borrow_mut"),
        "cell.vr must define a 'borrow_mut' method on RefCell"
    );
}

// ── 4. try_borrow_mut method ──────────────────────────────────────────────────

#[test]
fn cell_vr_has_try_borrow_mut_method() {
    assert!(
        CELL_VR.contains("fn try_borrow_mut"),
        "cell.vr must define a 'try_borrow_mut' method on RefCell"
    );
}

// ── 5. borrow_count method ────────────────────────────────────────────────────

#[test]
fn cell_vr_has_borrow_count_method() {
    assert!(
        CELL_VR.contains("fn borrow_count"),
        "cell.vr must define a 'borrow_count' method on RefCell"
    );
}

// ── 6. borrow_mut panics on conflict ─────────────────────────────────────────

#[test]
fn borrow_mut_references_already_borrowed() {
    assert!(
        CELL_VR.contains("already borrowed"),
        "cell.vr borrow_mut must reference 'already borrowed' for the panic message"
    );
}

// ── 7. try_borrow_mut returns Err on conflict ─────────────────────────────────

#[test]
fn try_borrow_mut_returns_err() {
    assert!(
        CELL_VR.contains("Err(BorrowMutError"),
        "cell.vr try_borrow_mut must return Err(BorrowMutError ...) on conflict"
    );
}

// ── 8. VCS spec uses RefCell API ──────────────────────────────────────────────

#[test]
fn refcell_spec_uses_refcell_new() {
    assert!(
        REFCELL_SPEC.contains("RefCell.new"),
        "refcell_double_borrow_mut_panic.vr must use 'RefCell.new'"
    );
}

#[test]
fn refcell_spec_uses_borrow_mut() {
    assert!(
        REFCELL_SPEC.contains("borrow_mut"),
        "refcell_double_borrow_mut_panic.vr must use 'borrow_mut'"
    );
}

#[test]
fn refcell_spec_uses_try_borrow_mut() {
    assert!(
        REFCELL_SPEC.contains("try_borrow_mut"),
        "refcell_double_borrow_mut_panic.vr must use 'try_borrow_mut'"
    );
}

// ── 9. VCS spec uses catch_unwind ─────────────────────────────────────────────

#[test]
fn refcell_spec_uses_catch_unwind() {
    assert!(
        REFCELL_SPEC.contains("catch_unwind"),
        "refcell_double_borrow_mut_panic.vr must use 'catch_unwind' to verify panic contract"
    );
}

#[test]
fn refcell_spec_is_typecheck_pass() {
    assert!(
        REFCELL_SPEC.contains("@test: typecheck-pass"),
        "refcell_double_borrow_mut_panic.vr must be '@test: typecheck-pass'"
    );
}
