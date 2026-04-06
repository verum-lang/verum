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
// Unit tests for profile_cmd.rs
//
// Migrated from src/profile_cmd.rs to comply with CLAUDE.md test organization.

use verum_compiler::profile_cmd::{CbgrStats, FunctionProfile, ProfileReport};
use verum_common::Text;

#[test]
fn test_profile_report() {
    let mut report = ProfileReport::new();
    report.add_function(
        Text::from("test"),
        FunctionProfile {
            stats: CbgrStats {
                num_cbgr_refs: 5,
                num_ownership_refs: 0,
                num_checks: 50,
                total_time_ns: 1_000_000,
                cbgr_time_ns: 150_000,
            },
            overhead_pct: 15.0,
            is_hot: true,
        },
    );
    assert_eq!(report.num_hot_paths(), 1);
}

#[test]
fn test_cbgr_stats() {
    let stats = CbgrStats {
        num_cbgr_refs: 10,
        num_ownership_refs: 5,
        num_checks: 100,
        total_time_ns: 2_000_000,
        cbgr_time_ns: 300_000,
    };

    // Verify overhead calculation
    let overhead_pct = (stats.cbgr_time_ns as f64 / stats.total_time_ns as f64) * 100.0;
    assert_eq!(overhead_pct, 15.0);
}

#[test]
fn test_function_profile_hot_path_detection() {
    let hot_profile = FunctionProfile {
        stats: CbgrStats {
            num_cbgr_refs: 100,
            num_ownership_refs: 0,
            num_checks: 1000,
            total_time_ns: 10_000_000,
            cbgr_time_ns: 3_000_000,
        },
        overhead_pct: 30.0,
        is_hot: true,
    };

    assert!(hot_profile.is_hot);
    assert!(hot_profile.overhead_pct > 20.0);

    let cold_profile = FunctionProfile {
        stats: CbgrStats {
            num_cbgr_refs: 2,
            num_ownership_refs: 5,
            num_checks: 10,
            total_time_ns: 1_000_000,
            cbgr_time_ns: 10_000,
        },
        overhead_pct: 1.0,
        is_hot: false,
    };

    assert!(!cold_profile.is_hot);
    assert!(cold_profile.overhead_pct < 5.0);
}
