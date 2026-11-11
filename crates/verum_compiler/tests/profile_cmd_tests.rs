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

// Migrated from src/profile_cmd.rs to comply with AGENTS.md test organization.

use verum_common::Text;
use verum_compiler::profile_cmd::{CbgrStats, FunctionProfile, ProfileReport};

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

#[test]
fn static_model_counts_reference_declarations_separately_from_possible_accesses() {
    let source = r#"
        fn unused(n: &Int) -> Int { 1 }
        fn accessed(n: &Int) -> Int { match true { true => *n, false => 0 } }
        fn looped(n: &Int) -> Int { while false { let x = *n; } *n }
    "#;
    let module = verum_fast_parser::Parser::new(source)
        .parse_module()
        .unwrap();
    let report = ProfileReport::estimate(&module, 5.0).unwrap();
    for (name, checks) in [("unused", 0), ("accessed", 1), ("looped", 11)] {
        let profile = report.functions.get(&Text::from(name)).unwrap();
        assert_eq!(profile.stats.num_cbgr_refs, 1, "{name}");
        assert_eq!(profile.stats.num_checks, checks, "{name}");
    }
}

#[test]
fn unrepresentable_loop_cost_is_explicit() {
    let source = format!(
        "fn nested(n: &Int) {{ {} let value = *n; {} }}",
        "while false { ".repeat(35),
        "}".repeat(35)
    );
    let module = verum_fast_parser::Parser::new(&source)
        .parse_module()
        .unwrap();
    let error = ProfileReport::estimate(&module, 5.0).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("exceeds the report range for nested"),
        "{error}"
    );
}
