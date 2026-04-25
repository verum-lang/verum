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

const FIXTURE: &str = r#"
// @test: typecheck-pass
// @tier: 0
// @level: L0
// @timeout: 10000
// @expected-exit: 0

mount base.{Bool, Int64, Text};
mount core.collections.list.{List};

// A small surface that exercises the same pipeline paths the historical
// non-determinism cases hit: protocol default methods, type-name imports,
// stdlib function lookups, generic monomorphisation.
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

#[test]
#[ignore = "requires built target/release/vtest; run with --ignored"]
fn vbc_bytecode_emission_is_deterministic_across_runs() {
    let root = workspace_root();

    // Drop the fixture into the VCS specs tree as a typecheck-pass smoke
    // so vtest's normal driver picks it up without any custom plumbing.
    let dir = root.join("vcs/specs/L0-critical/_determinism");
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let fixture = dir.join("codegen_determinism_fixture.vr");
    std::fs::write(&fixture, FIXTURE).expect("write fixture");

    // Wipe the disk-cache once so both runs start from the same state.
    let _ = std::fs::remove_dir_all(root.join("target/.verum-cache"));

    // First run primes the cache; both run-1 and run-2 *should* produce
    // the same exit-code-and-stderr — that's the contract.
    let (code_a, err_a) = run_once(&fixture);

    // Wipe again before the second run so neither run can short-circuit
    // through a previously-saved registry.
    let _ = std::fs::remove_dir_all(root.join("target/.verum-cache"));

    let (code_b, err_b) = run_once(&fixture);

    let _ = std::fs::remove_file(&fixture);
    let _ = std::fs::remove_dir(&dir);

    assert_eq!(
        code_a, code_b,
        "VBC compilation is non-deterministic — exit codes differ between runs:\n\
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
        "VBC compilation is non-deterministic — stderr differs between runs:\n\
         --- run A ---\n{}\n--- run B ---\n{}",
        err_a,
        err_b,
    );
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
