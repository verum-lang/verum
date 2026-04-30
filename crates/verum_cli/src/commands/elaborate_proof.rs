//! `verum elaborate-proof <file.vr>` — elaborate Verum theorems to
//! kernel-checkable certificates (#164 Phase-3).
//!
//! ## What this command does
//!
//! Walks every theorem/lemma/corollary in a Verum source file
//! (`.vr`).  For each declaration with a supported proof body
//! (Phase-1+2: `proof { apply <lemma>(args); }`), runs
//! [`verum_kernel::tactic_elaborator::elaborate_theorem`] to construct
//! a [`Certificate`] and writes it to disk as a `.vproof` file.
//!
//! The emitted `.vproof` files are **kernel-checked at construction
//! time** by the elaborator — the de Bruijn criterion is enforced
//! before the file is written.  Independent re-verification is
//! available via the existing `verum check-proof <file.vproof>`
//! command (#157 follow-up).
//!
//! ## The fundamental closure
//!
//! Pre-this-command: the `tactic_elaborator` module existed in
//! isolation.  Tests proved it could elaborate hand-built ASTs;
//! nothing ran on real Verum source.
//!
//! Post-this-command: `verum elaborate-proof core/verify/kernel_v0/lemmas/beta.vr`
//! walks the file, finds `church_rosser_confluence`, and either
//!
//!   - succeeds → emits `church_rosser_confluence.vproof` that
//!     `verum check-proof` re-verifies, OR
//!   - fails with a concrete `ElabError` (UnsupportedTactic /
//!     UndeclaredApplyTarget / KernelRejection) pinpointing the gap.
//!
//! Together with `verum check-proof`, this completes the round-trip
//! from source theorem to kernel verdict.
//!
//! ## Output
//!
//! Per-theorem `.vproof` files written next to the source file
//! (in `<source-dir>/elaborated/<theorem-name>.vproof`).  The
//! `--output-dir` flag overrides the destination.
//!
//! Stdout: per-theorem result (✓ verified / ✗ failed-reason / ⊘ skipped-unsupported).
//! Exit code: 0 if all supported theorems verified; non-zero on
//! `ElabError::KernelRejection` (the elaborator produced an
//! ill-typed term — a contract violation, not a graceful skip).

use crate::error::{CliError, Result};
use crate::ui;
use std::path::{Path, PathBuf};
use verum_ast::decl::ItemKind;
use verum_kernel::tactic_elaborator::{elaborate_theorem, ElabContext, ElabError};

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
    /// skip — Phase-3 limitations are tracked).
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
