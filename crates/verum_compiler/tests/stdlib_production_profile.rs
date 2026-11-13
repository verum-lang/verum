//! Profiling describes the actual archive producer, including sidecars and refusals.
use std::time::Duration;
use verum_compiler::precompile::{PrecompileConfig, precompile_stdlib};
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};

#[test]
fn complete_production_reports_publication_and_non_overlapping_intervals() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    std::fs::create_dir(&core).unwrap();
    std::fs::write(core.join("mod.vr"), "public fn value() -> Int { 41 }").unwrap();
    let output = root.path().join("runtime.vbca");
    let started = std::time::Instant::now();
    let result = precompile_stdlib(&PrecompileConfig {
        stdlib_path: core,
        output_path: output.clone(),
        target_triple: None,
        threads: None,
        verbose: false,
    })
    .unwrap();
    assert!(result.errors.is_empty());
    assert_eq!(result.profile.total_duration, result.total_time);
    assert!(result.total_time <= started.elapsed());
    let intervals: Duration = result
        .profile
        .phase_metrics
        .iter()
        .map(|p| p.duration)
        .sum();
    assert!(intervals <= result.total_time, "phase intervals overlap");
    let phases: Vec<_> = result
        .profile
        .phase_metrics
        .iter()
        .map(|p| p.phase_name.as_str())
        .collect();
    for expected in [
        "Stdlib source snapshot",
        "Stdlib file mounts",
        "Stdlib target selection",
        "Stdlib source admission",
        "Stdlib materialization",
        "Stdlib body compilation",
        "Stdlib core metadata",
        "Stdlib symbol graph",
        "Stdlib publication and verification",
    ] {
        assert_eq!(
            phases.iter().filter(|p| **p == expected).count(),
            1,
            "{phases:?}"
        );
    }
    assert_eq!(
        result.profile.stats.modules_compiled,
        result.modules_compiled
    );
    assert_eq!(
        result.profile.stats.functions_compiled,
        result.functions_compiled
    );
    assert_eq!(result.profile.module_metrics.len(), result.modules_compiled);
    assert!(output.is_file());
    assert!(output.with_extension("core_metadata").is_file());
    assert!(output.with_extension("symbol_graph").is_file());
    assert!(output.with_added_extension("receipt").is_file());
    let json = result.profile.to_json().unwrap();
    let decoded: verum_compiler::compilation_metrics::CompilationProfileReport =
        serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.stats.functions_compiled, result.functions_compiled);
}

#[test]
fn refusal_keeps_the_active_phase_without_publishing_an_archive() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("mod.vr"),
        "type Broken is { value: Missing };",
    )
    .unwrap();
    let output = root.path().join("refused.vbca");
    let mut session = Session::new(CompilerOptions::default());
    let result = CompilationPipeline::new_core(
        &mut session,
        CoreConfig::new(root.path()).with_output(&output),
    )
    .compile_core();
    assert!(result.is_err(), "an unresolved carrier must be refused");
    assert!(!output.exists());
    let report = session.current_metrics();
    assert_eq!(
        report.phase_metrics.last().unwrap().phase_name.as_str(),
        "Stdlib source admission"
    );
    assert!(
        !report.phase_metrics.is_empty(),
        "early return lost measured work"
    );
    assert!(
        !report
            .phase_metrics
            .iter()
            .any(|p| p.phase_name.as_str() == "Stdlib archive serialization")
    );
    assert_eq!(report.stats.modules_compiled, 0);
}
