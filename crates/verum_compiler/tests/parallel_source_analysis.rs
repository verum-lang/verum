//! Ordinary projects and stdlib admission share isolated, ordered source checks.
use verum_compiler::api::{CommonPipelineConfig, SourceFile, run_common_pipeline};
use std::{num::NonZeroUsize, sync::{Arc, Mutex}};
use verum_common::List;

// Subscriber installation changes tracing's process-wide callsite cache. Keep
// that test state isolated; each probe still runs real concurrent source workers.
static TRACING_STATE: Mutex<()> = Mutex::new(());

fn config(workers: usize) -> CommonPipelineConfig {
    CommonPipelineConfig {
        source_threads: NonZeroUsize::new(workers).unwrap(),
        collect_metrics: true,
        ..CommonPipelineConfig::minimal()
    }
}

#[test]
fn independent_failures_retain_every_source_owner() {
    let _trace = TRACING_STATE.lock().unwrap();
    let sources = [
        SourceFile::new("first.vr", "module app.first; fn first() -> Int { true }"),
        SourceFile::new("second.vr", "module app.second; fn second() -> Bool { 7 }"),
        SourceFile::new("valid.vr", "module app.valid; fn valid() -> Int { 73 }"),
    ];
    let mut serial = None;
    for workers in [1, 2, 4] {
        let error = run_common_pipeline(&sources, &config(workers))
            .expect_err("invalid source bodies were admitted");
        assert_eq!(error.diagnostics.len(), 2, "{error:?}");
        let diagnostics = format!("{:?}", error.diagnostics);
        assert!(diagnostics.contains("first") && diagnostics.contains("second"), "{diagnostics}");
        if let Some(serial) = &serial {
            assert_eq!(&diagnostics, serial, "worker count changed diagnostics");
        } else {
            serial = Some(diagnostics);
        }
    }
}

#[test]
fn successful_analysis_retains_lexical_owners_and_work_metrics() {
    let _trace = TRACING_STATE.lock().unwrap();
    let sources = [
        SourceFile::new("first.vr", "module app.first; public type Cell is { public value: Int }; public fn make() -> Cell { Cell { value: 73 } }"),
        SourceFile::new("second.vr", "module app.second; public type Cell is { public value: Bool }; public fn make() -> Cell { Cell { value: true } }"),
        SourceFile::new("consumer.vr", "module app.consumer; mount app.first.make; public fn answer() -> Int { make().value }"),
    ];
    let mut serial = None;
    for workers in [1, 2, 4] {
        let result = run_common_pipeline(&sources, &config(workers)).unwrap();
        assert!(!result.has_errors(), "{:?}", result.diagnostics);
        assert_eq!(result.ast_modules.len(), sources.len());
        assert_eq!(result.source_owners.len(), sources.len());
        let phase = result.metrics.phase_metrics.iter()
            .find(|phase| phase.phase_name.as_str() == "Phase 4: Semantic Analysis").unwrap();
        let limit = phase.custom_metrics.iter()
            .find(|(name, _)| name.as_str() == "source_worker_limit").unwrap();
        assert_eq!(limit.1.as_str(), workers.min(sources.len()).to_string());
        let work: List<_> = phase.custom_metrics.iter()
            .filter(|(name, _)| name.as_str() != "source_worker_limit").cloned().collect();
        if let Some(serial) = &serial {
            assert_eq!(&work, serial, "worker count changed semantic work");
        } else {
            serial = Some(work);
        }
    }
}

struct Capture(Arc<Mutex<List<u8>>>);
impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

#[test]
fn configured_workers_keep_the_callers_trace_subscriber() {
    let _trace = TRACING_STATE.lock().unwrap();
    let output = Arc::new(Mutex::new(List::new()));
    let writer = output.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::NEW)
        .with_ansi(false).without_time()
        .with_writer(move || Capture(writer.clone())).finish();
    tracing::subscriber::with_default(subscriber, || {
        let sources = [
            SourceFile::new("first.vr", "module app.first; fn first() -> Int { 1 }"),
            SourceFile::new("second.vr", "module app.second; fn second() -> Int { 2 }"),
        ];
        run_common_pipeline(&sources, &config(2)).unwrap();
    });
    let output = output.lock().unwrap();
    let output = std::str::from_utf8(&output).unwrap();
    for owner in ["app.first", "app.second"] {
        assert!(output.lines().any(|line| line.contains("source_admission")
            && line.contains(owner) && line.contains("verum-source-")), "{output}");
    }
}
