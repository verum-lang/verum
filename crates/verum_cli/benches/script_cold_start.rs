//! Script-mode cold-start latency budget.
//!
//! Two layers:
//!
//! 1. `script_first_run` — full pipeline: parse, typecheck, verify,
//!    codegen, interpret. Dominant cost is stdlib loading (~2 K
//!    files) plus per-script frontend work. This is the figure that
//!    matters for "first ever run of this script" — and the headline
//!    number we want to drive down.
//!
//! 2. `script_warm_cache` — subsequent run with the persistent VBC
//!    cache populated. Skips every front-end phase: just deserialise
//!    + execute. The 40-60× speedup over the first run is the value
//!    proposition of `ScriptContext::cache_lookup` / `cache_store`.
//!
//! Both shell out to the built `verum` binary so the measurement
//! includes process spawn, stdlib decompression from the embedded
//! archive, allocator warmup, and every other step a real user
//! invocation pays. That's the honest number to track.
//!
//! Run with: `cargo bench -p verum_cli --bench script_cold_start`.
//! Compare runs with `--save-baseline` / `--baseline`.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};

fn verum_binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn temp_script(body: &str, tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = std::env::temp_dir().join(format!(
        "verum-bench-{tag}-{}-{}.vr",
        std::process::id(),
        nanos
    ));
    std::fs::write(&p, body).expect("write bench script");
    p
}

/// Best-effort cache wipe so the "first run" measurement isn't
/// polluted by leftover entries from a previous bench session.
/// Uses the same default location as `ScriptCache::at_default`.
fn clear_cache() {
    if let Some(home) = std::env::var_os("HOME") {
        let dir = PathBuf::from(home).join(".verum").join("script-cache");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

fn run_script(path: &std::path::Path) {
    let status = Command::new(verum_binary())
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("verum spawn");
    // Scripts may exit non-zero by design (e.g., the bench fixture
    // could `exit(0)` or have any tail value); we don't gate on the
    // status here — the timing is what we measure.
    let _ = status;
}

fn bench_cold_start(c: &mut Criterion) {
    // Tiny representative script — just enough to exercise parser,
    // entry detection, and one print statement. Dominated by stdlib
    // bootstrap, not by user code.
    let script = temp_script(
        "#!/usr/bin/env verum\nprint(\"hello\");\n",
        "cold",
    );

    let mut g = c.benchmark_group("script_cold_start");
    // Cold-start runs the full pipeline (~10s on a fresh build);
    // give criterion a longer measurement window to keep the sample
    // count statistically meaningful without exploding runtime.
    g.measurement_time(Duration::from_secs(60));
    g.sample_size(10);

    g.bench_function("first_run_no_cache", |b| {
        b.iter(|| {
            clear_cache();
            run_script(&script);
        });
    });

    g.bench_function("warm_cache_hit", |b| {
        // Prime the cache once outside the timed loop.
        clear_cache();
        run_script(&script);
        b.iter(|| {
            run_script(&script);
        });
    });

    g.finish();
    let _ = std::fs::remove_file(&script);
}

criterion_group!(benches, bench_cold_start);
criterion_main!(benches);
