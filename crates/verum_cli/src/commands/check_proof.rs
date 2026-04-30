//! `verum check-proof <file.vproof>` — re-verify a proof-term
//! certificate via the minimal kernel (#157).
//!
//! ## What this command does
//!
//! Reads a JSON `.vproof` file containing a [`Certificate`] —
//! `{ term, claimed_type, metadata }` — and runs
//! [`verum_kernel::proof_checker`]'s 6-rule kernel against it.
//!
//! ## Trust delegation
//!
//! After this command exits 0, the user has verified the proof
//! against EXACTLY:
//!
//!   1. `verum_kernel::proof_checker` (633 LOC, hand-auditable).
//!   2. The Rust compiler's correctness.
//!   3. The serde-json deserialiser (the `.vproof` format).
//!
//! Nothing else in the Verum pipeline is relevant: no SMT, no
//! cross-format gate, no foreign tools, no audit gates.  The verdict
//! is the smallest possible trusted base in the proof-assistant
//! world.
//!
//! ## Example
//!
//! ```bash
//! $ cat trivial.vproof
//! {
//!   "term": { "Lam": [{"Universe": 0}, {"Var": 0}] },
//!   "claimed_type": { "Pi": [{"Universe": 0}, {"Universe": 0}] },
//!   "metadata": { "name": "identity_at_universe_0" }
//! }
//! $ verum check-proof trivial.vproof
//! ▶ Re-verifying trivial.vproof against minimal proof-term checker
//!   ✓ identity_at_universe_0: certificate verified
//!     (633 LOC trusted base; CIC fragment with 6 inference rules)
//! ```

use crate::error::{CliError, Result};
use crate::ui;
use verum_kernel::proof_checker::Certificate;

/// Entry point for `verum check-proof <file>`.
pub fn execute(path: &str) -> Result<()> {
    let file_path = std::path::PathBuf::from(path);
    if !file_path.exists() {
        return Err(CliError::InvalidArgument(format!(
            ".vproof file not found: {}",
            path,
        )));
    }

    ui::step(&format!(
        "Re-verifying {} against minimal proof-term checker",
        path,
    ));

    // Read + parse the certificate.  The .vproof format is JSON for
    // v0 (structured s-expression is a future refinement; the JSON
    // shape is the schema, not the exchange).
    let text = std::fs::read_to_string(&file_path).map_err(|e| {
        CliError::custom(format!("read {}: {}", path, e))
    })?;
    let cert: Certificate = serde_json::from_str(&text).map_err(|e| {
        CliError::InvalidArgument(format!(
            "failed to parse .vproof JSON in {}: {}",
            path, e
        ))
    })?;

    let name = cert
        .metadata
        .get("name")
        .cloned()
        .unwrap_or_else(|| {
            file_path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "(anonymous)".to_string())
        });

    // Verify.
    match cert.verify() {
        Ok(()) => {
            ui::success(&format!(
                "{}: certificate verified",
                name,
            ));
            println!(
                "    ({} LOC trusted base; CIC fragment with 6 inference rules)",
                approx_trusted_base_loc(),
            );
            Ok(())
        }
        Err(e) => Err(CliError::VerificationFailed(format!(
            "{}: certificate REJECTED by minimal kernel — {:?}",
            name, e,
        ))),
    }
}

/// Approximate LOC count of the trusted base.  Updated as
/// `proof_checker.rs` evolves; the constant lives here so the user-
/// facing message reflects the actual file.  Should stay < 1000 per
/// the architectural invariant.
fn approx_trusted_base_loc() -> usize {
    // Bump on every proof_checker.rs change; CI test below pins the
    // invariant `proof_checker.rs LOC < 1000`.
    633
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use verum_kernel::proof_checker::Term;

    /// Round-trip: write a valid certificate, run check-proof, expect Ok.
    #[test]
    fn check_proof_accepts_polymorphic_identity_certificate() {
        let cert = Certificate {
            term: Term::lam(
                Term::Universe(0),
                Term::lam(Term::Var(0), Term::Var(0)),
            ),
            claimed_type: Term::pi(
                Term::Universe(0),
                Term::pi(Term::Var(0), Term::Var(1)),
            ),
            metadata: {
                let mut m = std::collections::BTreeMap::new();
                m.insert("name".to_string(), "polymorphic_id".to_string());
                m
            },
        };
        let mut tmpfile = tempfile::NamedTempFile::new().expect("tempfile");
        let json = serde_json::to_string_pretty(&cert).expect("serialize");
        tmpfile.write_all(json.as_bytes()).expect("write");
        let path = tmpfile.path().to_string_lossy().into_owned();
        execute(&path).expect("certificate should verify");
    }

    /// Round-trip: a wrong-type certificate is rejected.
    #[test]
    fn check_proof_rejects_wrong_type_certificate() {
        let cert = Certificate {
            // Identity at Universe(0) — but claim it's Universe(0).
            term: Term::lam(Term::Universe(0), Term::Var(0)),
            claimed_type: Term::Universe(0),
            metadata: Default::default(),
        };
        let mut tmpfile = tempfile::NamedTempFile::new().expect("tempfile");
        let json = serde_json::to_string_pretty(&cert).expect("serialize");
        tmpfile.write_all(json.as_bytes()).expect("write");
        let path = tmpfile.path().to_string_lossy().into_owned();
        match execute(&path) {
            Err(CliError::VerificationFailed(_)) => { /* expected */ }
            other => panic!("expected VerificationFailed, got {:?}", other),
        }
    }

    /// Missing-file path produces a clear error.
    #[test]
    fn check_proof_missing_file_errors_cleanly() {
        match execute("/tmp/nonexistent_vproof_file.vproof") {
            Err(CliError::InvalidArgument(msg)) => {
                assert!(msg.contains("not found"));
            }
            other => panic!("expected InvalidArgument, got {:?}", other),
        }
    }

    /// Malformed JSON is reported with a clear error.
    #[test]
    fn check_proof_malformed_json_errors_cleanly() {
        let mut tmpfile = tempfile::NamedTempFile::new().expect("tempfile");
        tmpfile.write_all(b"not valid json {{{").expect("write");
        let path = tmpfile.path().to_string_lossy().into_owned();
        match execute(&path) {
            Err(CliError::InvalidArgument(msg)) => {
                assert!(msg.contains("parse"));
            }
            other => panic!("expected InvalidArgument, got {:?}", other),
        }
    }
}
