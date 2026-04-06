#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
// Tests for audit module
// Migrated from src/audit.rs per CLAUDE.md standards

use verum_cli::audit::*;

#[test]
fn test_audit_options_default() {
    let options = AuditOptions::default();
    assert!(options.verify_checksums);
}
