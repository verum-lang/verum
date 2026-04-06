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
//! Tests for compilation profiling and metrics infrastructure
//!
//! This test file validates the CompilationProfileReport, PhasePerformanceMetrics,
//! and related types for tracking compilation performance.

use std::time::Duration;
use verum_compiler::compilation_metrics::{
    Bottleneck, BottleneckKind, CompilationProfileReport, CompilationStats, ModuleMetrics,
    PhasePerformanceMetrics,
};
use verum_common::Text;

#[test]
fn test_compilation_profile_report_creation() {
    let report = CompilationProfileReport::new();

    assert_eq!(report.phase_metrics.len(), 0);
    assert_eq!(report.module_metrics.len(), 0);
    assert_eq!(report.total_duration.as_millis(), 0);
    assert_eq!(report.total_memory_bytes, 0);
    assert_eq!(report.bottlenecks.len(), 0);
}

#[test]
fn test_record_phase() {
    let mut report = CompilationProfileReport::new();

    report.record_phase("Parsing", Duration::from_millis(50), 1024 * 512);
    report.record_phase("Type Checking", Duration::from_millis(100), 1024 * 1024);

    assert_eq!(report.phase_metrics.len(), 2);
    assert_eq!(report.total_duration.as_millis(), 150);
    assert_eq!(report.total_memory_bytes, 1024 * 512 + 1024 * 1024);
}

#[test]
fn test_add_module() {
    let mut report = CompilationProfileReport::new();

    report.add_module("main.vr", Duration::from_millis(100), 10);
    report.add_module("utils.vr", Duration::from_millis(50), 5);

    assert_eq!(report.module_metrics.len(), 2);
    assert_eq!(report.stats.modules_compiled, 2);
    assert_eq!(report.stats.functions_compiled, 15);
}

#[test]
fn test_phase_percentage_calculation() {
    let mut report = CompilationProfileReport::new();

    // Add two phases with equal time
    report.record_phase("Phase A", Duration::from_millis(100), 1024);
    report.record_phase("Phase B", Duration::from_millis(100), 1024);

    report.finalize();

    // Each phase should be 50% of total time
    for phase in &report.phase_metrics {
        assert!(
            (phase.time_percentage - 50.0).abs() < 0.1,
            "Expected ~50%, got {}%",
            phase.time_percentage
        );
    }
}

#[test]
fn test_bottleneck_detection_slow_phase() {
    let mut report = CompilationProfileReport::new();

    // Add a slow phase (>20% threshold)
    report.record_phase("Slow Phase", Duration::from_millis(250), 1024);
    report.record_phase("Fast Phase", Duration::from_millis(50), 512);

    report.finalize();

    // Should detect bottleneck
    assert!(
        !report.bottlenecks.is_empty(),
        "Expected bottleneck detection"
    );

    let has_slow_phase = report
        .bottlenecks
        .iter()
        .any(|b| b.kind == BottleneckKind::SlowPhase && b.location == "Slow Phase");

    assert!(has_slow_phase, "Expected slow phase bottleneck");
}

#[test]
fn test_bottleneck_detection_high_memory() {
    let mut report = CompilationProfileReport::new();

    // Add phase with high memory usage (>25% threshold)
    report.record_phase("Memory Heavy", Duration::from_millis(100), 3 * 1024 * 1024);
    report.record_phase("Memory Light", Duration::from_millis(100), 1024 * 1024);

    report.finalize();

    let has_memory_bottleneck = report
        .bottlenecks
        .iter()
        .any(|b| b.kind == BottleneckKind::HighMemory);

    assert!(has_memory_bottleneck, "Expected high memory bottleneck");
}

#[test]
fn test_bottleneck_detection_slow_module() {
    let mut report = CompilationProfileReport::new();

    // Add slow module (>100ms threshold)
    report.add_module("slow.vr", Duration::from_millis(150), 10);
    report.add_module("fast.vr", Duration::from_millis(50), 5);

    report.finalize();

    // Check that slow module is marked
    let slow_module = report
        .module_metrics
        .iter()
        .find(|m| m.module_name == "slow.vr");

    assert!(
        slow_module.is_some() && slow_module.unwrap().is_slow,
        "Expected slow module to be marked"
    );

    let has_slow_module_bottleneck = report
        .bottlenecks
        .iter()
        .any(|b| b.kind == BottleneckKind::SlowModule);

    assert!(
        has_slow_module_bottleneck,
        "Expected slow module bottleneck"
    );
}

#[test]
fn test_compilation_stats_calculation() {
    let mut report = CompilationProfileReport::new();

    report.record_phase("Phase 1", Duration::from_millis(100), 1024 * 1024);
    report.add_module("mod1.vr", Duration::from_millis(100), 10);
    report.add_module("mod2.vr", Duration::from_millis(100), 15);

    report.stats.total_loc = 10_000;
    report.finalize();

    // Check compilation speed calculation
    let expected_speed = 10_000.0 / report.total_duration.as_secs_f64();
    assert!(
        (report.stats.compilation_speed_loc_per_sec - expected_speed).abs() < 1.0,
        "Compilation speed mismatch"
    );

    // Check average time per module
    assert!(
        (report.stats.avg_time_per_module_ms - 50.0).abs() < 1.0,
        "Average time per module mismatch"
    );
}

#[test]
fn test_summary_generation() {
    let mut report = CompilationProfileReport::new();

    report.record_phase("Parsing", Duration::from_millis(50), 512 * 1024);
    report.record_phase("Analysis", Duration::from_millis(100), 1024 * 1024);
    report.add_module("main.vr", Duration::from_millis(80), 5);

    report.stats.total_loc = 1000;
    report.finalize();

    let summary = report.summary();

    // Should contain key information
    assert!(summary.as_str().contains("Compilation Profile Report"));
    assert!(summary.as_str().contains("Parsing"));
    assert!(summary.as_str().contains("Analysis"));
    assert!(summary.as_str().contains("Phase Breakdown"));
}

#[test]
fn test_json_serialization_roundtrip() {
    let mut report = CompilationProfileReport::new();

    report.record_phase("Test Phase", Duration::from_millis(100), 2048);
    report.add_module("test.vr", Duration::from_millis(50), 3);
    report.stats.total_loc = 500;
    report.finalize();

    // Serialize to JSON
    let json = report.to_json().expect("Failed to serialize");

    // Deserialize back
    let deserialized =
        CompilationProfileReport::from_json(json.as_str()).expect("Failed to deserialize");

    // Verify key fields match
    assert_eq!(deserialized.phase_metrics.len(), report.phase_metrics.len());
    assert_eq!(
        deserialized.module_metrics.len(),
        report.module_metrics.len()
    );
    assert_eq!(
        deserialized.stats.modules_compiled,
        report.stats.modules_compiled
    );
    assert_eq!(deserialized.stats.total_loc, report.stats.total_loc);
}

#[test]
fn test_module_metrics() {
    let module = ModuleMetrics {
        module_name: "test.vr".into(),
        duration: Duration::from_millis(75),
        function_count: 8,
        lines_of_code: 250,
        memory_bytes: 1024 * 512,
        is_slow: false,
    };

    assert_eq!(module.module_name, Text::from("test.vr"));
    assert_eq!(module.function_count, 8);
    assert_eq!(module.lines_of_code, 250);
}

#[test]
fn test_phase_performance_metrics() {
    use verum_common::Map;

    let mut custom = Map::new();
    custom.insert(Text::from("files_parsed"), Text::from("15"));

    let phase = PhasePerformanceMetrics {
        phase_name: "Lexical Parsing".into(),
        duration: Duration::from_millis(45),
        memory_allocated: 512 * 1024,
        items_processed: 15,
        time_percentage: 15.0,
        memory_percentage: 10.0,
        custom_metrics: custom,
    };

    assert_eq!(phase.phase_name, Text::from("Lexical Parsing"));
    assert_eq!(phase.duration.as_millis(), 45);
    assert_eq!(phase.items_processed, 15);
    assert_eq!(phase.time_percentage, 15.0);
}

#[test]
fn test_bottleneck_structure() {
    let bottleneck = Bottleneck {
        kind: BottleneckKind::SlowPhase,
        location: "Type Checking".into(),
        description: "Takes 40% of compilation time".into(),
        severity: 40.0,
        suggestion: "Enable incremental compilation".into(),
    };

    assert_eq!(bottleneck.kind, BottleneckKind::SlowPhase);
    assert_eq!(bottleneck.location, Text::from("Type Checking"));
    assert_eq!(bottleneck.severity, 40.0);
}

#[test]
fn test_empty_report_finalization() {
    let mut report = CompilationProfileReport::new();
    report.finalize();

    // Should not panic and should have zero stats
    assert_eq!(report.stats.compilation_speed_loc_per_sec, 0.0);
    assert_eq!(report.stats.avg_time_per_module_ms, 0.0);
    assert_eq!(report.bottlenecks.len(), 0);
}

#[test]
fn test_peak_memory_tracking() {
    let mut report = CompilationProfileReport::new();

    report.record_phase("Phase 1", Duration::from_millis(50), 1024);
    report.record_phase("Phase 2", Duration::from_millis(50), 4096);
    report.record_phase("Phase 3", Duration::from_millis(50), 2048);

    // Peak should be 4096
    assert_eq!(report.peak_memory_bytes, 4096);
}

#[test]
fn test_compilation_stats_default() {
    let stats = CompilationStats::default();

    assert_eq!(stats.modules_compiled, 0);
    assert_eq!(stats.functions_compiled, 0);
    assert_eq!(stats.total_loc, 0);
    assert_eq!(stats.compilation_speed_loc_per_sec, 0.0);
}

#[test]
fn test_multiple_bottlenecks() {
    let mut report = CompilationProfileReport::new();

    // Add slow phase
    report.record_phase("Slow Phase", Duration::from_millis(250), 1024);
    report.record_phase("Normal Phase", Duration::from_millis(50), 512);

    // Add high memory phase
    report.record_phase("Memory Heavy", Duration::from_millis(50), 3 * 1024 * 1024);

    // Add slow module
    report.add_module("slow.vr", Duration::from_millis(120), 5);

    report.finalize();

    // Should detect multiple bottlenecks
    assert!(
        report.bottlenecks.len() >= 2,
        "Expected multiple bottlenecks"
    );

    let bottleneck_kinds: Vec<_> = report.bottlenecks.iter().map(|b| b.kind).collect();

    // Should have at least slow phase and high memory
    assert!(bottleneck_kinds.contains(&BottleneckKind::SlowPhase));
    assert!(bottleneck_kinds.contains(&BottleneckKind::HighMemory));
}
