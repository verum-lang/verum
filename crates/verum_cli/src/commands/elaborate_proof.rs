//! `verum elaborate-proof <file.vr>` — walk Verum source and emit
//! kernel-checkable certificates.
//!
//! ## What this command does
//!
//! Walks every theorem / lemma / corollary in a `.vr` source file.
//! For each declaration with a supported proof body, runs
//! [`verum_kernel::tactic_elaborator::elaborate_theorem`] to construct
//! a [`Certificate`] and writes it to disk as a `.vproof` file.
//!
//! The emitted `.vproof` files are kernel-checked at construction
//! time — the de Bruijn criterion is enforced before the file is
//! written.  Independent re-verification is available via
//! `verum check-proof <file.vproof>`.
//!
//! Together with `verum check-proof`, this command closes the
//! round-trip from source theorem to kernel verdict: the elaborator
//! exercises the tactic_elaborator on real Verum source rather than
//! the hand-built ASTs the unit tests use.
//!
//! ## Output
//!
//! Per-theorem `.vproof` files in `<source-dir>/elaborated/` (the
//! `--output-dir` flag overrides the destination).  Stdout reports
//! per-theorem outcomes: `✓ verified` / `✗ FAILED <reason>` /
//! `⊘ skipped <reason>`.  Exit code is non-zero only on
//! [`ElabError::KernelRejection`] (the elaborator produced an
//! ill-typed term — a contract violation); UnsupportedTactic and
//! UndeclaredApplyTarget are graceful skips.

use crate::error::{CliError, Result};
use crate::ui;
use std::path::{Path, PathBuf};
use verum_ast::decl::ItemKind;
use verum_kernel::tactic_elaborator::{
    elaborate_theorem, register_kernel_bridge_dispatchers, register_kernel_v0_lemmas,
    register_propositional_connectives, ElabContext, ElabError,
};

/// One row in the elaboration report — what happened for one
/// theorem.
#[derive(Debug, Clone)]
pub struct ElaborationRow {
    /// Theorem / lemma / corollary name.
    pub name: String,
    /// Status: Verified / Skipped / Failed.
    pub status: ElaborationStatus,
}

/// Per-theorem outcome of elaboration.
#[derive(Debug, Clone)]
pub enum ElaborationStatus {
    /// Certificate produced + re-verified.  Path to the emitted
    /// `.vproof` file.
    Verified { vproof_path: PathBuf },
    /// Tactic form not yet supported by the elaborator (graceful
    /// skip — `UnsupportedTactic`, `UndeclaredApplyTarget`, or
    /// `UnsupportedExpression`).  Reason carries the diagnostic.
    Skipped { reason: String },
    /// Elaborator produced a term that the kernel rejected.  This
    /// is a CONTRACT VIOLATION (the elaborator is supposed to
    /// produce well-typed terms).  Non-graceful failure.
    Failed { reason: String },
}

impl ElaborationStatus {
    /// Whether this row contributes to the non-zero exit verdict.
    pub fn is_failure(&self) -> bool {
        matches!(self, ElaborationStatus::Failed { .. })
    }
}

/// Entry point for `verum elaborate-proof <file>`.
///
/// `output_dir` is the destination directory for `.vproof` files.
/// When `None`, defaults to `<source-dir>/elaborated/`.
pub fn execute(path: &str, output_dir: Option<&str>) -> Result<()> {
    let source_path = PathBuf::from(path);
    if !source_path.exists() {
        return Err(CliError::InvalidArgument(format!(
            ".vr source file not found: {}",
            path,
        )));
    }
    if source_path.extension().and_then(|s| s.to_str()) != Some("vr") {
        return Err(CliError::InvalidArgument(format!(
            "expected a .vr file, got: {}",
            path,
        )));
    }

    let out_dir = match output_dir {
        Some(d) => PathBuf::from(d),
        None => source_path
            .parent()
            .map(|p| p.join("elaborated"))
            .unwrap_or_else(|| PathBuf::from("./elaborated")),
    };

    ui::step(&format!(
        "Elaborating proofs from {} → {}",
        source_path.display(),
        out_dir.display(),
    ));

    let rows = walk_and_elaborate(&source_path, &out_dir)?;

    // Render report.
    let mut total_verified = 0usize;
    let mut total_skipped = 0usize;
    let mut total_failed = 0usize;
    for row in &rows {
        match &row.status {
            ElaborationStatus::Verified { vproof_path } => {
                total_verified += 1;
                ui::success(&format!(
                    "{}: certificate verified → {}",
                    row.name,
                    vproof_path.display(),
                ));
            }
            ElaborationStatus::Skipped { reason } => {
                total_skipped += 1;
                println!("  ⊘ {}: skipped — {}", row.name, reason);
            }
            ElaborationStatus::Failed { reason } => {
                total_failed += 1;
                println!("  ✗ {}: FAILED — {}", row.name, reason);
            }
        }
    }
    println!();
    println!(
        "  {} verified, {} skipped (unsupported), {} FAILED (kernel rejection).",
        total_verified, total_skipped, total_failed,
    );

    if total_failed > 0 {
        return Err(CliError::VerificationFailed(format!(
            "{} theorem(s) elaborated to ill-typed terms — kernel rejected",
            total_failed,
        )));
    }
    Ok(())
}

/// Walk one `.vr` source file and elaborate every theorem-shaped
/// declaration.  Writes `.vproof` files for verified theorems;
/// returns one [`ElaborationRow`] per theorem.
fn walk_and_elaborate(
    source_path: &Path,
    out_dir: &Path,
) -> Result<Vec<ElaborationRow>> {
    use verum_common::span::FileId;
    use verum_fast_parser::FastParser;

    let source = std::fs::read_to_string(source_path).map_err(|e| {
        CliError::custom(format!("read {}: {}", source_path.display(), e))
    })?;

    let parser = FastParser::new();
    let file_id = FileId::new(0);
    let module = parser.parse_module_str(&source, file_id).map_err(|e| {
        CliError::InvalidArgument(format!(
            "parse {} failed: {:?}",
            source_path.display(),
            e,
        ))
    })?;

    // Ensure output directory exists.
    std::fs::create_dir_all(out_dir).map_err(|e| {
        CliError::custom(format!("mkdir {}: {}", out_dir.display(), e))
    })?;

    let mut rows = Vec::new();
    for item in module.items.iter() {
        let (theorem, name) = match &item.kind {
            ItemKind::Theorem(t) | ItemKind::Lemma(t) | ItemKind::Corollary(t) => {
                (t, t.name.name.to_string())
            }
            // Axioms have no proof body and are explicitly trusted
            // — not elaborator candidates.
            _ => continue,
        };

        let mut ctx = ElabContext::new();
        // Pre-register the canonical axiom families so theorems can
        // resolve their apply-targets without per-call wiring.
        //
        //   - propositional connectives — `a == b`, `a && b`, `!x`
        //     translate to opaque connective-axiom App chains.
        //   - kernel_v0 lemma stubs — `apply
        //     core.verify.kernel_v0.lemmas.beta.church_rosser_confluence`
        //     and friends resolve to the registered axiom slots
        //     carrying their `@framework(<system>, "<path>")`
        //     citations.
        //   - kernel bridge dispatchers — `apply kernel_<rule>_strict`
        //     resolves to the registered bridge axiom; the audit
        //     gate's apply-graph walker classifies the leaf as
        //     `kernel_strict`.
        register_propositional_connectives(&mut ctx);
        register_kernel_v0_lemmas(&mut ctx);
        register_kernel_bridge_dispatchers(&mut ctx);
        let row = match elaborate_theorem(theorem, &mut ctx) {
            Ok(cert) => {
                let vproof_path = out_dir.join(format!("{}.vproof", name));
                let json = serde_json::to_string_pretty(&cert).map_err(|e| {
                    CliError::custom(format!("serialise certificate: {}", e))
                })?;
                std::fs::write(&vproof_path, json).map_err(|e| {
                    CliError::custom(format!(
                        "write {}: {}",
                        vproof_path.display(),
                        e,
                    ))
                })?;
                ElaborationRow {
                    name,
                    status: ElaborationStatus::Verified { vproof_path },
                }
            }
            Err(ElabError::KernelRejection(reason)) => ElaborationRow {
                name,
                status: ElaborationStatus::Failed { reason },
            },
            Err(other) => ElaborationRow {
                name,
                status: ElaborationStatus::Skipped {
                    reason: format!("{}", other),
                },
            },
        };
        rows.push(row);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Pin: a `.vr` file with a single theorem `apply` body whose
    /// target axiom is undeclared cleanly skips with the
    /// UndeclaredApplyTarget message.
    #[test]
    fn elaborate_proof_skips_undeclared_apply_targets() {
        let source = "module test_elab;\n\
                      \n\
                      public theorem foo()\n\
                          ensures true\n\
                      {\n\
                          proof { apply some_undeclared_axiom; };\n\
                      }\n";
        let mut src = tempfile::NamedTempFile::new().expect("tempfile");
        src.write_all(source.as_bytes()).expect("write");
        // Rename to .vr extension by copying.
        let vr_path = src.path().with_extension("vr");
        std::fs::copy(src.path(), &vr_path).expect("copy to .vr");

        let out_dir = tempfile::tempdir().expect("tempdir");
        let result = execute(
            vr_path.to_string_lossy().as_ref(),
            Some(out_dir.path().to_string_lossy().as_ref()),
        );
        // Skipped (not failed) → exit 0.
        assert!(result.is_ok(), "expected ok with skipped row, got {:?}", result);
        // No .vproof files emitted.
        let entries: Vec<_> = std::fs::read_dir(out_dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(entries.is_empty(), "no .vproof should be emitted on skip");

        std::fs::remove_file(&vr_path).ok();
    }

    /// Pin: missing source file produces a clear error.
    #[test]
    fn elaborate_proof_missing_file_errors_cleanly() {
        match execute("/tmp/nonexistent_elab_source.vr", None) {
            Err(CliError::InvalidArgument(msg)) => {
                assert!(msg.contains("not found"), "got: {}", msg);
            }
            other => panic!("expected InvalidArgument, got {:?}", other),
        }
    }

    /// Pin: non-.vr extension is rejected.
    #[test]
    fn elaborate_proof_rejects_wrong_extension() {
        let mut src = tempfile::NamedTempFile::new().expect("tempfile");
        src.write_all(b"module x;\n").expect("write");
        match execute(src.path().to_string_lossy().as_ref(), None) {
            Err(CliError::InvalidArgument(msg)) => {
                assert!(msg.contains("expected a .vr file"), "got: {}", msg);
            }
            other => panic!("expected InvalidArgument, got {:?}", other),
        }
    }
}
