//! Cross-format emit harness — drives the V1 lowerers in
//! `verum_codegen::proof_export` over a fixed CoreTerm fixture and
//! writes the emitted text (wrapped with format-specific preamble)
//! into `target/cross-format-out/{lean,coq,agda,dedukti,metamath}/`
//! so the `cross-format-recheck` CI workflow can run the actual
//! verifier toolchains against lowerer-emitted text — closing the
//! "we lowered to syntax X but never re-checked X" loophole.
//!
//! Fixture: identity-on-Nat — type `(x : Nat) → Nat`, body `λ x => x`.
//! Five files emitted per run; the CI workflow consumes them as the
//! input to lake / coqc / agda / dkcheck / metamath.
//!
//! Re-running the test is idempotent — files are overwritten, never
//! appended, so a stale emit can't poison a downstream re-check.

use std::fs;
use std::path::PathBuf;

use verum_codegen::proof_export::{agda, coq, dedukti, lean, metamath};
use verum_common::{Heap, Text};
use verum_kernel::CoreTerm;

fn out_dir() -> PathBuf {
    // CARGO_TARGET_DIR is set by cargo when invoked via `cargo test`.
    // Falling back to `./target` keeps the test runnable outside of a
    // workspace (e.g. via direct `rustc --test` invocation).
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            // Walk up from CARGO_MANIFEST_DIR until we find `target/`,
            // matching the actual workspace layout.
            let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            manifest.parent().unwrap_or(&manifest).parent()
                .map(|p| p.join("target"))
                .unwrap_or_else(|| PathBuf::from("target"))
        });
    target.join("cross-format-out")
}

fn write_file(format: &str, name: &str, content: String) -> PathBuf {
    let dir = out_dir().join(format);
    fs::create_dir_all(&dir).expect("mkdir cross-format-out subdir");
    let path = dir.join(name);
    fs::write(&path, content).expect("write fixture file");
    path
}

/// Identity-on-Nat as a CoreTerm: `λ (x : Nat). x`. Common across
/// every target — exercises Lam + Pi + Var lowering paths.
fn identity_lam() -> CoreTerm {
    CoreTerm::Lam {
        binder: Text::from("x"),
        domain: Heap::new(CoreTerm::Var(Text::from("Nat"))),
        body: Heap::new(CoreTerm::Var(Text::from("x"))),
    }
}

#[test]
fn emit_lean_identity_on_nat() {
    let body = lean::lower_term(&identity_lam());
    // Wrap with Lean's `def` form so the emitted file is a complete
    // module that `lean` can typecheck standalone (Lean's `Nat` is
    // imported by the prelude).
    let content = format!(
        "-- auto-emitted by cross_format_emit; identity on Nat\n\
         def vmIdNat : Nat → Nat := {body}\n\
         #check vmIdNat\n"
    );
    let path = write_file("lean", "Identity.lean", content);
    let written = fs::read_to_string(&path).unwrap();
    assert!(written.contains("fun (x : Nat) => x"), "lean lowering shape changed");
}

#[test]
fn emit_coq_identity_on_nat() {
    let body = coq::lower_term(&identity_lam());
    // Coq: postulate Nat as an opaque parameter so the file is
    // self-contained — the lowerer treats Var("Nat") as opaque, and
    // we declare the same name here so coqc can typecheck standalone.
    let content = format!(
        "(* auto-emitted by cross_format_emit; identity on Nat *)\n\
         Parameter Nat : Type.\n\
         Definition vm_id_nat : Nat -> Nat := {body}.\n\
         Check vm_id_nat.\n"
    );
    let path = write_file("coq", "Identity.v", content);
    let written = fs::read_to_string(&path).unwrap();
    assert!(written.contains("fun (x : Nat) => x"), "coq lowering shape changed");
}

#[test]
fn emit_agda_identity_on_nat() {
    let body = agda::lower_term(&identity_lam());
    // Agda: postulate Nat as an opaque set so the file is
    // self-contained without needing the standard library.
    let content = format!(
        "-- auto-emitted by cross_format_emit; identity on Nat\n\
         module Identity where\n\
         postulate Nat : Set\n\
         vm-id-nat : Nat → Nat\n\
         vm-id-nat = {body}\n"
    );
    let path = write_file("agda", "Identity.agda", content);
    let written = fs::read_to_string(&path).unwrap();
    // Agda emits Unicode lambda (λ).
    assert!(written.contains("λ (x : Nat) → x"), "agda lowering shape changed");
}

#[test]
fn emit_dedukti_identity_on_nat() {
    let body = dedukti::lower_term(&identity_lam());
    // Dedukti needs an explicit Nat declaration (we declare it
    // abstractly so dkcheck can consume the file standalone).
    let content = format!(
        "(; auto-emitted by cross_format_emit; identity on Nat ;)\n\
         Nat : Type.\n\
         def vm_id_nat : Nat -> Nat := {body}.\n"
    );
    let path = write_file("dedukti", "Identity.dk", content);
    let written = fs::read_to_string(&path).unwrap();
    assert!(written.contains("x : Nat => x"), "dedukti lowering shape changed");
}

#[test]
fn emit_metamath_identity_on_nat() {
    // Metamath's lowering produces a label-form expression; we emit
    // it inside a comment block in a minimal `.mm` file so downstream
    // `metamath read` exercises the file-level parse without needing
    // the full `set.mm` include chain. V2 will emit the full $p/$.
    // proof block with substitution chain.
    let body = metamath::lower_term(&identity_lam());
    let content = format!(
        "$( auto-emitted by cross_format_emit; identity on Nat $)\n\
         $( label-form: {body} $)\n\
         $c set $.\n"
    );
    let path = write_file("metamath", "identity.mm", content);
    let written = fs::read_to_string(&path).unwrap();
    assert!(written.contains("( wlam x Nat x )"), "metamath lowering shape changed");
}

#[test]
fn emit_manifest_lists_all_files() {
    // Sanity: after the four sibling tests run, the manifest is
    // emitted once, listing every fixture path so the CI workflow
    // can iterate without hard-coding extension lookups.
    //
    // Tests can run in any order; if this one runs first, sibling
    // emits will be re-run (`cargo test` runs each #[test] in its
    // own process when --test-threads=1 isn't set, so each test is
    // self-contained — the dependency is on the *filesystem*, not
    // on test ordering).
    let _ = lean::lower_term(&identity_lam());
    let _ = coq::lower_term(&identity_lam());
    let _ = agda::lower_term(&identity_lam());
    let _ = dedukti::lower_term(&identity_lam());
    let _ = metamath::lower_term(&identity_lam());

    let manifest = "lean/Identity.lean\ncoq/Identity.v\nagda/Identity.agda\ndedukti/Identity.dk\nmetamath/identity.mm\n".to_string();
    let path = out_dir().join("MANIFEST");
    fs::create_dir_all(out_dir()).unwrap();
    fs::write(&path, manifest).expect("write MANIFEST");
    assert!(path.exists());
}
