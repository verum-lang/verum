//! T0904 — a theorem parameter typed by a MOUNTED type can be stated.
//!
//! A parameter got a declared SORT only when its type's shape was known,
//! and the shapes came from scanning the module under verification. A
//! mounted type is not in that module's items, so its parameters kept
//! the `Int` default while the predicate they were passed to was
//! declared over the type's own sort, and the application was refused:
//!
//!     role_holds (argument 0 is Int, declared "Verum!Role")
//!
//! Not "unproved" — UNSTATEABLE. The claim never reached the solver,
//! which reads from the outside exactly like a claim that failed to
//! prove, and is a different defect.
//!
//! The fix reads the session's MODULE REGISTRY rather than its
//! parsed-file cache. That distinction is the whole finding and is why
//! this test exists at the project level rather than as a single-file
//! spec: the cache holds only the file the command was pointed at —
//! measured, it reported ONE module for a project whose mount had just
//! resolved — while the registry is what cross-file resolution was
//! built against and holds every module the check actually loaded.
//!
//! The local type is the control. It worked before, and a "fix" that
//! gave every unrecognised type name its own sort would satisfy the
//! mounted cases and break `n + 0 == n` for `n: Nat`, which is how an
//! earlier attempt failed.

use std::path::{Path, PathBuf};

fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create project dir");
    }
    std::fs::write(path, body).expect("write project file");
}

/// A project whose entry file mounts a record and a variant from a
/// sibling module, and states a theorem over each.
fn scaffold() -> PathBuf {
    let root = std::env::temp_dir().join("verum-mounted-type-shape-test");
    let _ = std::fs::remove_dir_all(&root);

    write(
        &root.join("verum.toml"),
        "[cog]\nname = \"mountedshape\"\nversion = \"0.1.0\"\nedition = \"2026\"\n",
    );
    write(&root.join("mod.vr"), "module mountedshape;\n");
    write(
        &root.join("src").join("shapes").join("mod.vr"),
        "module mountedshape.shapes;\n",
    );
    write(
        &root.join("src").join("shapes").join("kinds.vr"),
        "module mountedshape.shapes.kinds;\n\
         public type Role is Reader | Writer;\n\
         public type Pair is { a: Int, b: Int };\n",
    );
    write(
        &root.join("src").join("main.vr"),
        "module mountedshape.main;\n\
         mount mountedshape.shapes.kinds.{Role, Pair};\n\
         \n\
         public type Local is { n: Int };\n\
         public fn local_holds(l: Local) -> Bool { l.n == l.n }\n\
         @verify(formal)\n\
         public theorem local_is_stateable(l: Local) ensures local_holds(l) proof by auto;\n\
         \n\
         public fn role_holds(r: Role) -> Bool { r == r }\n\
         @verify(formal)\n\
         public theorem mounted_variant_is_stateable(r: Role) ensures role_holds(r) proof by auto;\n\
         \n\
         public fn pair_commutes(p: Pair) -> Bool { p.a + p.b == p.b + p.a }\n\
         @verify(formal)\n\
         public theorem mounted_record_is_stateable(p: Pair) ensures pair_commutes(p) proof by auto;\n\
         \n\
         fn main() { print(\"mounted-shapes\"); }\n",
    );
    root
}

/// Verify the project's entry file and return the report.
fn verify_entry(root: &Path) -> verum_compiler::verify_cmd::VerificationReport {
    use verum_compiler::{CompilerOptions, Session, verify_cmd::VerifyCommand};

    let options = CompilerOptions {
        input: root.join("src").join("main.vr"),
        output: std::env::temp_dir().join("verum-mounted-type-shape-out"),
        ..Default::default()
    };
    let mut session = Session::new(options);
    VerifyCommand::new(&mut session)
        .run_to_report(None)
        .expect("verification should complete, whatever its verdict")
}

#[test]
fn a_theorem_over_a_mounted_variant_is_stateable() {
    let root = scaffold();
    let report = verify_entry(&root);
    let failed: Vec<&str> = report
        .failed_names()
        .into_iter()
        .filter(|n| n.contains("mounted_variant"))
        .collect();
    assert!(
        failed.is_empty(),
        "a theorem over a MOUNTED variant must be stateable; failed: {failed:?}"
    );
}

#[test]
fn a_theorem_over_a_mounted_record_is_stateable() {
    let root = scaffold();
    let report = verify_entry(&root);
    let failed: Vec<&str> = report
        .failed_names()
        .into_iter()
        .filter(|n| n.contains("mounted_record"))
        .collect();
    assert!(
        failed.is_empty(),
        "a theorem over a MOUNTED record must be stateable; failed: {failed:?}"
    );
}

#[test]
fn a_theorem_over_a_local_type_still_works() {
    // The control. A fix that gave every unrecognised type name its own
    // sort would satisfy the two tests above and break this one's kin.
    let root = scaffold();
    let report = verify_entry(&root);
    let failed: Vec<&str> = report
        .failed_names()
        .into_iter()
        .filter(|n| n.contains("local_is_stateable"))
        .collect();
    assert!(
        failed.is_empty(),
        "a theorem over a LOCAL type must keep working; failed: {failed:?}"
    );
}
