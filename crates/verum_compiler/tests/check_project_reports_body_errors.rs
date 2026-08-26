//! T0884 — `verum check` on a PROJECT reports what `verum check <file>`
//! reports.
//!
//! One source, two commands, two verdicts:
//!
//! ```text
//! fn builtin_case() { let b: Int = "x"; }
//!
//! verum check src/main.vr   ->  error<E400>: expected 'Int', found 'Text'
//! verum check               ->  0 errors
//! ```
//!
//! and the project form is the one every real program uses.
//!
//! The two commands did not disagree about the TYPES. They disagreed
//! about which channel they read. The checker reports on two: a hard
//! `Err` stops the walk, and a statement it can RECOVER from pushes a
//! diagnostic and continues. An annotation mismatch is the second shape,
//! so a pass that reads only `Err` — as the project path did — throws
//! away everything the checker found and kept going from.
//!
//! What this pins is the CHANNEL, not one diagnostic: the fixture's
//! errors are all recovered ones, so a regression that re-narrows the
//! pass to `Err` alone takes the count straight back to zero.

use std::path::{Path, PathBuf};

fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create project dir");
    }
    std::fs::write(path, body).expect("write project file");
}

/// A project with a known number of RECOVERED type errors, spread over
/// the entry file and a sibling so neither position can pass by itself.
fn scaffold(dir: &str, entry_body: &str, sibling_body: &str) -> PathBuf {
    let root = std::env::temp_dir()
        .join("verum-check-project-body-errors")
        .join(dir);
    let _ = std::fs::remove_dir_all(&root);

    write(
        &root.join("verum.toml"),
        "[cog]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2026\"\n",
    );
    write(&root.join("mod.vr"), "module demo;\n");
    write(
        &root.join("src").join("lib.vr"),
        &format!("module demo.lib;\n{sibling_body}"),
    );
    write(
        &root.join("src").join("main.vr"),
        &format!("module demo.main;\n{entry_body}fn main() {{ print(\"p\"); }}\n"),
    );
    root
}

fn errors_for(root: &Path) -> usize {
    use verum_compiler::{CompilationPipeline, CompilerOptions, Session};

    let options = CompilerOptions {
        input: root.join("src"),
        output: std::env::temp_dir().join("verum-check-project-body-out"),
        ..Default::default()
    };
    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new_check(&mut session);
    pipeline
        .check_project()
        .expect("check_project should complete")
        .user_errors
}

#[test]
fn a_recovered_annotation_mismatch_is_reported_in_project_mode() {
    let root = scaffold(
        "three-errors",
        // Two in the entry file: one against a BUILT-IN type and one
        // against a standard-library type, because the project path
        // resolves those differently and a fix for one is not a fix for
        // the other.
        "fn builtin_case() { let b: Int = \"x\"; print(\"a\"); }\n\
         fn stdlib_case()  { let c: Text = 6;  print(\"b\"); }\n",
        // And one in the sibling, so a pass that checks only the entry
        // file cannot reach the expected count.
        "public fn sibling_case() { let a: Text = 5; print(\"c\"); }\n",
    );
    let found = errors_for(&root);
    assert_eq!(
        found, 3,
        "every recovered annotation mismatch must be reported: expected 3, got {found}"
    );
}

/// The negative pole. Without it, a change that emitted a diagnostic for
/// every statement would satisfy the assertion above.
#[test]
fn a_clean_project_reports_nothing() {
    let root = scaffold(
        "clean",
        "fn fine() { let b: Int = 1; print(f\"{b}\"); }\n",
        "public fn also_fine() -> Int { 2 }\n",
    );
    let found = errors_for(&root);
    assert_eq!(
        found, 0,
        "a project with no type errors must report none, got {found}"
    );
}
