//! Determinism contract for the VBC bytecode emission pipeline.
//!
//! Compiles the same fixed Verum source twice in a row inside a single
//! process and asserts the resulting bytecode is byte-identical.  Without
//! this guardrail, any new HashMap iteration inserted into the codegen
//! pipeline would silently leak Rust's per-process random hasher seed
//! into FunctionId / TypeId assignment, producing the symptom matrix
//! that motivated commits 0723ad43 + 82303f94 (task #143):
//!
//!     * `method 'X.Y' not found on value`
//!     * `Null pointer dereference`
//!     * `Division by zero`
//!     * `field index N (offset M) exceeds object data size K`
//!
//! Each of those happens when the runtime resolves a function-id /
//! variant-tag / field-offset that was assigned to a *different* symbol
//! in the run that produced the bytecode — which is exactly what
//! non-deterministic iteration causes.
//!
//! Mechanism: spawn the same vtest invocation twice, compare the
//! exit-code-and-stderr signature.  If the underlying bytecode differs
//! between runs, the panic surface (or the absence of panic) differs
//! too.
//!
//! Why two child-processes rather than two in-process compiles: each
//! process gets a fresh random hasher seed.  In-process repeats reuse
//! the same seed and would fail to detect HashMap-iteration leaks.

use std::path::PathBuf;
use std::process::Command;

/// Fixture A — record type, while loop, assert_eq.
///
/// Covers the original symptom matrix from commits 0723ad43 +
/// 82303f94: protocol default methods, type-name imports, stdlib
/// function lookups, generic monomorphisation around a small
/// composite record.
const FIXTURE_RECORD_LOOP: &str = r#"
// @test: typecheck-pass
// @tier: 0
// @level: L0
// @timeout: 10000
// @expected-exit: 0

mount base.{Bool, Int64, Text};
mount core.collections.list.{List};

public type Pair is { fst: Int64, snd: Text };

public fn make(n: Int64) -> Pair {
    Pair { fst: n, snd: "v".clone() }
}

public fn sum_first_n(n: Int64) -> Int64 {
    let mut i: Int64 = 0 as Int64;
    let mut acc: Int64 = 0 as Int64;
    while i < n {
        let p = make(i);
        acc = acc + p.fst;
        i = i + (1 as Int64);
    }
    acc
}

fn main() {
    let p = make(7 as Int64);
    assert_eq(p.fst, 7 as Int64);
    assert(p.snd == "v".clone());
    assert_eq(sum_first_n(5 as Int64), 10 as Int64);
}
"#;

/// Fixture B — variant-constructor disambiguation (#76 family).
///
/// Two user-declared sum types each have a `V4` variant with
/// identical structure. Without deterministic codegen, the simple
/// name `V4` can collapse onto whichever type registered first,
/// then `MakeVariant` flows the wrong tag through to runtime
/// pattern matching → "field index N OOB" or "Unknown variant
/// constructor 'V4'" — both stochastic across runs.
const FIXTURE_VARIANT_DISAMBIG: &str = r#"
// @test: typecheck-pass
// @tier: 0
// @level: L0
// @timeout: 10000
// @expected-exit: 0

mount base.{Bool, Int64};

public type Inner is { p: Int64 };

public type Net is V4(Inner) | V6(Int64);
public type Mac is V4(Inner) | V8(Int64);

public fn net_port(n: Net) -> Int64 {
    match n {
        V4(i) => i.p,
        V6(p) => p,
    }
}

public fn mac_port(m: Mac) -> Int64 {
    match m {
        V4(i) => i.p,
        V8(p) => p,
    }
}

fn main() {
    let n: Net = V4(Inner { p: 80 as Int64 });
    let m: Mac = V4(Inner { p: 22 as Int64 });
    assert_eq(net_port(n), 80 as Int64);
    assert_eq(mac_port(m), 22 as Int64);
}
"#;

/// Fixture C — stdlib method dispatch (#79 family).
///
/// Multiple stdlib carrier types (Result, Maybe, …) all expose a
/// method named `unwrap` with different panic-on-nothing semantics.
/// The dispatcher must route by receiver type, not by first-
/// registered candidate. Pre-determinism-fix, repeated runs would
/// sometimes resolve the wrong `unwrap` and either silently corrupt
/// the value or panic with the wrong message.
const FIXTURE_METHOD_DISPATCH: &str = r#"
// @test: typecheck-pass
// @tier: 0
// @level: L0
// @timeout: 10000
// @expected-exit: 0

mount base.{Bool, Int64};
mount core.base.maybe.{Maybe};
mount core.base.result.{Result};

public fn double_some(x: Int64) -> Int64 {
    let m: Maybe<Int64> = Some(x);
    m.unwrap() + m.unwrap()
}

public fn triple_ok(x: Int64) -> Int64 {
    let r: Result<Int64, Int64> = Ok(x);
    r.unwrap() + r.unwrap() + r.unwrap()
}

fn main() {
    assert_eq(double_some(5 as Int64), 10 as Int64);
    assert_eq(triple_ok(7 as Int64), 21 as Int64);
}
"#;

/// Fixture D — generic monomorphisation across two carriers.
///
/// `List<Pair>` inside a `Maybe<...>` exercises two layers of
/// generic specialisation in the codegen. Historical
/// non-determinism in mono-key ordering (HashMap iteration) would
/// produce different per-instantiation FunctionIds across runs,
/// later misresolving when the list iterator dispatched on the
/// inner pair's `.fst` field.
const FIXTURE_GENERIC_MONO: &str = r#"
// @test: typecheck-pass
// @tier: 0
// @level: L0
// @timeout: 10000
// @expected-exit: 0

mount base.{Bool, Int64};
mount core.base.maybe.{Maybe};
mount core.collections.list.{List};

public type Cell is { fst: Int64, snd: Int64 };

public fn build(n: Int64) -> Maybe<List<Cell>> {
    let mut xs: List<Cell> = List.new();
    let mut i: Int64 = 0 as Int64;
    while i < n {
        xs.push(Cell { fst: i, snd: i + (1 as Int64) });
        i = i + (1 as Int64);
    }
    Some(xs)
}

public fn sum_first(xs_opt: Maybe<List<Cell>>) -> Int64 {
    match xs_opt {
        Some(xs) => {
            let mut s: Int64 = 0 as Int64;
            let mut i: Int64 = 0 as Int64;
            let n = xs.len();
            while i < n {
                s = s + xs[i].fst;
                i = i + (1 as Int64);
            }
            s
        },
        None => -1 as Int64,
    }
}

fn main() {
    let m = build(4 as Int64);
    // Sum of fst over Cells {0,1,2,3} = 0+1+2+3 = 6.
    assert_eq(sum_first(m), 6 as Int64);
}
"#;

/// All fixtures the determinism contract pins.  Each is run in two
/// separate vtest subprocesses with a wiped disk-cache; the
/// (exit_code, stderr) tuple must match across runs.
const FIXTURES: &[(&str, &str)] = &[
    ("record_loop", FIXTURE_RECORD_LOOP),
    ("variant_disambig", FIXTURE_VARIANT_DISAMBIG),
    ("method_dispatch", FIXTURE_METHOD_DISPATCH),
    ("generic_mono", FIXTURE_GENERIC_MONO),
];

/// Locate the workspace root containing core/ and target/release/vtest.
fn workspace_root() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for ancestor in crate_dir.ancestors() {
        if ancestor.join("target/release/vtest").is_file() {
            return ancestor.to_path_buf();
        }
    }
    panic!(
        "workspace root with target/release/vtest not found from {}",
        crate_dir.display()
    );
}

/// Run vtest on the fixture and capture (exit_code, stderr).  We use
/// stderr alone — stdout from successful runs is the same VCS report
/// boilerplate, while stderr carries the panic message that varied
/// run-to-run before the determinism fix.
fn run_once(fixture_path: &std::path::Path) -> (Option<i32>, String) {
    let root = workspace_root();
    let vtest = root.join("target/release/vtest");
    let output = Command::new(&vtest)
        .args(["run", fixture_path.to_str().unwrap()])
        .current_dir(&root)
        .output()
        .expect("failed to run vtest");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Wipe the on-disk stdlib cache so each subprocess starts from
/// the same state (no short-circuit through a previously-saved
/// registry).
fn wipe_disk_cache(root: &std::path::Path) {
    let _ = std::fs::remove_dir_all(root.join("target/.verum-cache"));
}

/// Drive one fixture through the contract: compile twice in
/// separate processes with the disk-cache wiped between runs,
/// assert (exit_code, normalised_stderr) matches.
///
/// `name` is used as the fixture file's basename and for failure
/// reporting.
fn run_determinism_contract(name: &str, source: &str) {
    let root = workspace_root();

    // Drop the fixture into the VCS specs tree as a typecheck-pass smoke
    // so vtest's normal driver picks it up without any custom plumbing.
    let dir = root.join("vcs/specs/L0-critical/_determinism");
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let fixture = dir.join(format!("codegen_determinism_{name}.vr"));
    std::fs::write(&fixture, source).expect("write fixture");

    wipe_disk_cache(&root);
    let (code_a, err_a) = run_once(&fixture);

    wipe_disk_cache(&root);
    let (code_b, err_b) = run_once(&fixture);

    let _ = std::fs::remove_file(&fixture);
    // dir cleanup deferred — sibling fixtures may still occupy it
    // when tests run in parallel (`cargo test` default).

    assert_eq!(
        code_a, code_b,
        "[{name}] VBC compilation is non-deterministic — exit codes differ between runs:\n\
         run A: {:?}\n\
         run B: {:?}\n\
         If a HashMap iteration was recently added to the codegen path, sort \
         it by a stable key (function-id / module path / name).  See commit \
         0723ad43 + 82303f94 for prior fixes.",
        code_a,
        code_b,
    );
    assert_eq!(
        normalise_stderr(&err_a),
        normalise_stderr(&err_b),
        "[{name}] VBC compilation is non-deterministic — stderr differs between runs:\n\
         --- run A ---\n{}\n--- run B ---\n{}",
        err_a,
        err_b,
    );
}

#[test]
#[ignore = "requires built target/release/vtest; run with --ignored"]
fn vbc_bytecode_emission_is_deterministic_across_runs() {
    // Run every fixture in the contract.  Each probes a distinct
    // historical non-determinism vector — see the const docs above.
    for (name, source) in FIXTURES {
        run_determinism_contract(name, source);
    }

    // Final cleanup: remove the fixture directory if empty.
    let root = workspace_root();
    let dir = root.join("vcs/specs/L0-critical/_determinism");
    let _ = std::fs::remove_dir(&dir);
}

#[test]
#[ignore = "requires built target/release/vtest; run with --ignored"]
fn vbc_record_loop_is_deterministic() {
    run_determinism_contract("record_loop", FIXTURE_RECORD_LOOP);
}

#[test]
#[ignore = "requires built target/release/vtest; run with --ignored"]
fn vbc_variant_disambig_is_deterministic() {
    run_determinism_contract("variant_disambig", FIXTURE_VARIANT_DISAMBIG);
}

#[test]
#[ignore = "requires built target/release/vtest; run with --ignored"]
fn vbc_method_dispatch_is_deterministic() {
    run_determinism_contract("method_dispatch", FIXTURE_METHOD_DISPATCH);
}

#[test]
#[ignore = "requires built target/release/vtest; run with --ignored"]
fn vbc_generic_mono_is_deterministic() {
    run_determinism_contract("generic_mono", FIXTURE_GENERIC_MONO);
}

/// Strip non-deterministic-by-design lines (timestamps, durations, etc.)
/// from the stderr capture so the comparison only flags real bytecode
/// drift.
fn normalise_stderr(s: &str) -> String {
    s.lines()
        .filter(|line| {
            !(line.contains("Date:")
                || line.contains("Duration:")
                || line.contains("[")) // strips ANSI INFO/WARN lines
        })
        .collect::<Vec<_>>()
        .join("\n")
}
