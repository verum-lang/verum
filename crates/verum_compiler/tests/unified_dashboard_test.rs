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
//! Test for unified performance dashboard
//!
//! Demonstrates the unified dashboard functionality with sample data.

use std::time::Duration;
use verum_compiler::compilation_metrics::{CompilationProfileReport, PhasePerformanceMetrics};
use verum_compiler::profile_cmd::{CbgrStats, FunctionProfile, ProfileReport};
use verum_compiler::unified_dashboard::UnifiedDashboard;
use verum_common::Map;

#[test]
fn test_unified_dashboard_creation() {
    let mut compilation_report = CompilationProfileReport::new();
    compilation_report.total_duration = Duration::from_secs(45);

    // Add phase metrics
    compilation_report.phase_metrics = vec![
        PhasePerformanceMetrics {
            phase_name: "Parsing".into(),
            duration: Duration::from_millis(2100),
            memory_allocated: 1024 * 1024,
            items_processed: 100,
            time_percentage: 4.6,
            memory_percentage: 10.0,
            custom_metrics: Map::new(),
        },
        PhasePerformanceMetrics {
            phase_name: "Type Checking".into(),
            duration: Duration::from_millis(8700),
            memory_allocated: 2 * 1024 * 1024,
            items_processed: 200,
            time_percentage: 19.2,
            memory_percentage: 20.0,
            custom_metrics: Map::new(),
        },
        PhasePerformanceMetrics {
            phase_name: "Verification".into(),
            duration: Duration::from_millis(28300),
            memory_allocated: 4 * 1024 * 1024,
            items_processed: 50,
            time_percentage: 62.6,
            memory_percentage: 40.0,
            custom_metrics: Map::new(),
        },
        PhasePerformanceMetrics {
            phase_name: "Codegen".into(),
            duration: Duration::from_millis(6100),
            memory_allocated: 3 * 1024 * 1024,
            items_processed: 150,
            time_percentage: 13.5,
            memory_percentage: 30.0,
            custom_metrics: Map::new(),
        },
    ]
    .into();

    let mut profile_report = ProfileReport::new();

    // Add function profiles
    profile_report.add_function(
        "complex_algorithm".into(),
        FunctionProfile {
            stats: CbgrStats {
                num_cbgr_refs: 50,
                num_ownership_refs: 10,
                num_checks: 500,
                total_time_ns: 30_000_000_000, // 30s
                cbgr_time_ns: 28_300_000_000,  // 28.3s
            },
            overhead_pct: 94.3,
            is_hot: true,
        },
    );

    profile_report.add_function(
        "process_matrix".into(),
        FunctionProfile {
            stats: CbgrStats {
                num_cbgr_refs: 20,
                num_ownership_refs: 5,
                num_checks: 200,
                total_time_ns: 45_300_000, // 45.3ms
                cbgr_time_ns: 28_700_000,  // 28.7ms
            },
            overhead_pct: 63.4,
            is_hot: true,
        },
    );

    profile_report.add_function(
        "safe_parse".into(),
        FunctionProfile {
            stats: CbgrStats {
                num_cbgr_refs: 2,
                num_ownership_refs: 8,
                num_checks: 10,
                total_time_ns: 1_000_000, // 1ms
                cbgr_time_ns: 1_000,      // 0.001ms
            },
            overhead_pct: 0.1,
            is_hot: false,
        },
    );

    // Create unified dashboard
    let dashboard = UnifiedDashboard::from_data(&compilation_report, &profile_report);

    // Verify compilation metrics
    assert_eq!(dashboard.compilation.total_time, Duration::from_secs(45));
    assert!(dashboard.compilation.verification.is_slow);
    assert_eq!(dashboard.compilation.verification.percentage, 62.6);

    // Verify runtime metrics
    assert!(dashboard.runtime.cbgr_overhead_pct > 0.0);

    // Verify hot spots were identified
    assert!(!dashboard.hot_spots.is_empty());

    // Verify recommendations were generated
    assert!(!dashboard.recommendations.is_empty());

    // Display dashboard (for manual inspection)
    dashboard.display();

    // Test JSON export
    let json = dashboard.to_json().expect("Failed to export JSON");
    assert!(!json.is_empty());
    println!("\nJSON Export:\n{}", json);

    // Test HTML export
    let html = dashboard.to_html();
    assert!(!html.is_empty());
    assert!(html.contains("Verum Performance Analysis"));
    assert!(html.contains("Compilation Time"));
    assert!(html.contains("Runtime Performance"));
}

#[test]
fn test_dashboard_without_hot_spots() {
    let mut compilation_report = CompilationProfileReport::new();
    compilation_report.total_duration = Duration::from_secs(10);

    // Use 15% for each phase (< 20% threshold for "slow")
    compilation_report.phase_metrics = vec![
        PhasePerformanceMetrics {
            phase_name: "Parsing".into(),
            duration: Duration::from_millis(1500),
            memory_allocated: 1024 * 1024,
            items_processed: 100,
            time_percentage: 15.0,
            memory_percentage: 25.0,
            custom_metrics: Map::new(),
        },
        PhasePerformanceMetrics {
            phase_name: "Type Checking".into(),
            duration: Duration::from_millis(1500),
            memory_allocated: 1024 * 1024,
            items_processed: 100,
            time_percentage: 15.0,
            memory_percentage: 25.0,
            custom_metrics: Map::new(),
        },
        PhasePerformanceMetrics {
            phase_name: "Verification".into(),
            duration: Duration::from_millis(1500),
            memory_allocated: 1024 * 1024,
            items_processed: 100,
            time_percentage: 15.0,
            memory_percentage: 25.0,
            custom_metrics: Map::new(),
        },
        PhasePerformanceMetrics {
            phase_name: "Codegen".into(),
            duration: Duration::from_millis(1500),
            memory_allocated: 1024 * 1024,
            items_processed: 100,
            time_percentage: 15.0,
            memory_percentage: 25.0,
            custom_metrics: Map::new(),
        },
    ]
    .into();

    let profile_report = ProfileReport::new();

    let dashboard = UnifiedDashboard::from_data(&compilation_report, &profile_report);

    // No slow phases (each is 15% which is < 20% threshold)
    assert!(!dashboard.compilation.parsing.is_slow);
    assert!(!dashboard.compilation.type_checking.is_slow);
    assert!(!dashboard.compilation.verification.is_slow);
    assert!(!dashboard.compilation.codegen.is_slow);

    // No hot spots (no function profiles provided)
    assert!(dashboard.hot_spots.is_empty());

    dashboard.display();
}

#[test]
fn test_phase_metrics() {
    use verum_compiler::unified_dashboard::DashboardPhaseMetrics;

    let fast_phase = DashboardPhaseMetrics {
        duration: Duration::from_millis(100),
        percentage: 5.0,
        is_slow: false,
    };

    let slow_phase = DashboardPhaseMetrics {
        duration: Duration::from_millis(5000),
        percentage: 45.0,
        is_slow: true,
    };

    assert!(!fast_phase.is_slow);
    assert!(slow_phase.is_slow);
}

#[test]
fn test_hot_spot_ranking() {
    use verum_compiler::unified_dashboard::{HotSpot, HotSpotKind};

    let hot_spot_1 = HotSpot {
        rank: 1,
        function_name: "complex_algorithm".into(),
        kind: HotSpotKind::SlowVerification,
        cost: "28.3s verification".into(),
        target: "reduce to <5s".into(),
    };

    let hot_spot_2 = HotSpot {
        rank: 2,
        function_name: "process_matrix".into(),
        kind: HotSpotKind::HighCbgrOverhead,
        cost: "28.7ms CBGR".into(),
        target: "convert to &checked".into(),
    };

    assert_eq!(hot_spot_1.rank, 1);
    assert_eq!(hot_spot_2.rank, 2);
    assert_eq!(hot_spot_1.kind, HotSpotKind::SlowVerification);
    assert_eq!(hot_spot_2.kind, HotSpotKind::HighCbgrOverhead);
}
