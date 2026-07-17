#![cfg(test)]

//! T0231 (PRINT-POLY-PRELUDE-SHADOW-1) regression gate.
//!

//! `print` / `println` / `eprint` / `eprintln` are polymorphic
//! compiler intrinsics (`fn<T>(T) -> Unit`); the runtime renders any
//! value. Before this gate the implicit-prelude import replay
//! clobbered the polymorphic env schemes with the concrete stdlib
//! `&Text` signatures through an unguarded insert channel, so every
//! bare `print(42)` failed with a false E400 on BOTH tiers.
//!

//! Related code:
//! - `verum_types/src/infer/env.rs` — `insert_fn_scheme_guarded`,
//!  the ONE authority every registration channel routes through.
//! - `verum_types/src/infer/modules.rs` — the prelude import replay
//!  sites now routed through the guard.
//! - `verum_vbc/src/codegen/expressions.rs` — `emit_value_to_text`
//!  (shared conversion authority) + the eprint/eprintln lowering.

use std::io::Write;
use std::path::PathBuf;
use tempfile::NamedTempFile;
use verum_compiler::{CompilationPipeline, CompilerOptions, Session, VerifyMode};

fn check_ok(source: &str) -> (bool, Vec<String>) {
    let mut file = NamedTempFile::new().expect("temp");
    write!(file, "{}", source).expect("write");
    let opts = CompilerOptions {
        input: file.path().to_path_buf(),
        output: PathBuf::from("/tmp/print_poly_test.out"),
        verify_mode: VerifyMode::Runtime,
        ..Default::default()
    };
    let mut session = Session::new(opts);
    let mut pipeline = CompilationPipeline::new(&mut session);
    let ok = pipeline.run_check_only().is_ok() && !session.has_errors();
    let msgs = session
        .diagnostics()
        .iter()
        .map(|d| d.message().to_string())
        .collect();
    (ok, msgs)
}

/// The exact pre-fix failure shape: a bare Int through every
/// print-family builtin.
#[test]
fn print_family_accepts_int() {
    let source = r#"
        fn main() {
            let x = 42;
            print(x);
            println(x);
            eprint(x);
            eprintln(x);
        }
    "#;
    let (ok, msgs) = check_ok(source);
    assert!(
        ok,
        "print family must accept Int (polymorphic intrinsics); diagnostics: {:?}",
        msgs
    );
}

/// Floats and Bools flow through the same polymorphic schemes.
#[test]
fn print_family_accepts_float_and_bool() {
    let source = r#"
        fn main() {
            print(1.5);
            println(true);
            eprintln(2.25);
        }
    "#;
    let (ok, msgs) = check_ok(source);
    assert!(ok, "diagnostics: {:?}", msgs);
}

/// Control: the concrete Text path keeps working.
#[test]
fn print_family_accepts_text() {
    let source = r#"
        fn main() {
            print("out");
            eprintln("err");
        }
    "#;
    let (ok, msgs) = check_ok(source);
    assert!(ok, "diagnostics: {:?}", msgs);
}
