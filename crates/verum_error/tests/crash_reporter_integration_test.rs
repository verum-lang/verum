//! Integration tests for the industrial crash reporter.
//!
//! These tests exercise the full install → panic → write path in an
//! isolated temp directory. They do NOT install the fatal-signal
//! handlers (that process-wide side effect would break rayon and other
//! tests), only the panic hook half.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use verum_error::breadcrumb;
use verum_error::crash::{self, CrashReporterConfig};

// The crash reporter installs process-wide state (panic hook + global
// config). All tests that touch it must run serially so they don't
// clobber each other's config.
static SERIAL: Mutex<()> = Mutex::new(());

fn unique_dir() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("verum-crash-reporter-test-{}-{}-{}", pid, ts, n))
}

fn install_for_test(dir: &PathBuf) {
    let _ = fs::create_dir_all(dir);
    crash::install(CrashReporterConfig {
        app_name: "verum-test".into(),
        app_version: "0.0.0-test".into(),
        report_dir: Some(dir.clone()),
        retention: 50,
        capture_backtrace: false,
        install_signal_handlers: false,
        redact_sensitive_env: true,
        issue_tracker_url: "https://example.invalid/issue".into(),
    });
}

/// Induce a controlled panic on the main thread and assert that a
/// report lands in the configured directory.
#[test]
fn panic_writes_structured_report() {
    let _g = SERIAL.lock().unwrap();
    let dir = unique_dir();
    install_for_test(&dir);
    // Note: once installed, install() is a no-op. Subsequent tests keep
    // writing to this dir. We scan for reports produced *during* this
    // test by filename uniqueness.

    crash::set_command("test");
    crash::set_input_file("unit-test.vr");

    let before: Vec<_> = crash::list_reports().unwrap_or_default();

    // Panic on a secondary thread so the process survives. The panic
    // hook still runs on that thread and writes the report.
    let jh = std::thread::Builder::new()
        .name("crash-reporter-panicker".into())
        .spawn(|| {
            let _b = breadcrumb::enter("test.section", "inside the panic guard");
            panic!("intentional test panic — ignore");
        })
        .unwrap();
    let _ = jh.join(); // Err on panic, that's fine.

    // Wait briefly — in case the hook flushes asynchronously.
    std::thread::sleep(std::time::Duration::from_millis(50));
    let after: Vec<_> = crash::list_reports().unwrap_or_default();

    // List_reports uses the initially-installed report_dir globally,
    // so we check that at least one report grew.
    assert!(
        after.len() > before.len(),
        "expected a new report in {}; before={}, after={}",
        crash::report_dir().unwrap().display(),
        before.len(),
        after.len()
    );

    // The report directory that we installed above should contain at
    // least one .log and one .json file.
    let entries: Vec<_> = fs::read_dir(crash::report_dir().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        entries.iter().any(|n| n.ends_with(".log")),
        "no .log file in {entries:?}"
    );
    assert!(
        entries.iter().any(|n| n.ends_with(".json")),
        "no .json file in {entries:?}"
    );

    // The latest log should mention the panic message and the breadcrumb.
    let newest = crash::list_reports().unwrap().into_iter().next().unwrap();
    let text = fs::read_to_string(&newest).unwrap();
    assert!(
        text.contains("intentional test panic"),
        "log missing panic message: {text}"
    );
    assert!(
        text.contains("test.section"),
        "log missing breadcrumb phase: {text}"
    );
    assert!(text.contains("unit-test.vr"), "log missing input context");
}

#[test]
fn env_snapshot_is_populated() {
    let _g = SERIAL.lock().unwrap();
    let dir = unique_dir();
    install_for_test(&dir);

    let snap = crash::env_snapshot().expect("installed");
    assert!(!snap.os.is_empty());
    assert!(!snap.arch.is_empty());
    assert!(snap.cpu_cores >= 1);
    assert!(snap.pid > 0);
    // VERUM_BUILD_* flow through build.rs → build profile should be
    // non-default here since we're running under `cargo test`.
    assert!(!snap.build_profile.is_empty());
    assert!(!snap.build_target.is_empty());
}

#[test]
fn reports_are_rotated_against_retention_limit() {
    let _g = SERIAL.lock().unwrap();
    let dir = unique_dir();
    let _ = fs::create_dir_all(&dir);
    // Seed >retention fake reports and verify rotation happens when we
    // write a new real one.
    for i in 0..60 {
        let stem = dir.join(format!("verum-fake-{:03}", i));
        fs::write(stem.with_extension("log"), "fake log").unwrap();
        fs::write(stem.with_extension("json"), "{}").unwrap();
    }
    install_for_test(&dir);

    // Trigger a panic so the reporter runs its rotation pass.
    let jh = std::thread::spawn(|| {
        panic!("rotate me");
    });
    let _ = jh.join();
    std::thread::sleep(std::time::Duration::from_millis(50));

    let files: Vec<_> = fs::read_dir(crash::report_dir().unwrap())
        .unwrap()
        .flatten()
        .collect();
    // retention = 50, times 2 (.log + .json) = 100 max allowed; allow
    // a little slack (±2) for race with file creation.
    assert!(
        files.len() <= 104,
        "expected rotation to cap entries, got {}",
        files.len()
    );
}
