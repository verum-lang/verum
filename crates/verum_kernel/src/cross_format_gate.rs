//! Cross-format CI hard gate — V0 algorithmic kernel rule.
//!
//! ## What this delivers
//!
//! Verum proofs can be exported to four foreign proof-assistant
//! formats: **Coq**, **Lean 4**, **Isabelle/HOL**, and **Dedukti**.
//! Each export must be re-checked by the foreign system; the
//! cross-format CI gate ensures *every* checked-in MSFS proof
//! survives a round-trip through every backend before being
//! merged.
//!
//! Pre-this-module the gate was a series of ad-hoc shell scripts in
//! `vcs/scripts/` that were neither composable nor inspectable from
//! kernel-side tooling.  V0 ships:
//!
//!   1. [`ExportFormat`] — Coq / Lean4 / Isabelle / Dedukti enumeration.
//!   2. [`FormatStatus`] — per-format CI status (`Passed` / `Failed` /
//!      `NotRun`).
//!   3. [`CrossFormatReport`] — accumulating report record covering
//!      every format.
//!   4. [`evaluate_gate`] — decision predicate: a proof passes the
//!      hard gate iff every format reports `Passed`.
//!   5. [`required_formats_for_msfs`] — returns the canonical
//!      MSFS-required format list (currently all four).
//!   6. [`format_replay_command`] — emits the deterministic shell
//!      command that reproduces a given format's check (used by
//!      `verum audit --reproducibility`).
//!
//! V1 promotion: tighten the hard gate to additionally require
//! kernel-recheck of the foreign-system's *output* (closing the loop
//! on the cross-format trust boundary).
//!
//! ## What this UNBLOCKS
//!
//!   - **`verum audit --cross-format`** CLI command: walks the gate
//!     report and surfaces the precise per-format pass/fail state.
//!   - **MSFS reproducibility chain** — every `@verify(certified)`
//!     theorem must survive the gate before merge.

use serde::{Deserialize, Serialize};
use verum_common::Text;

// =============================================================================
// Export-format enumeration
// =============================================================================

/// The foreign proof-assistant formats Verum exports to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExportFormat {
    /// **Coq** — Calculus of Inductive Constructions, via `coqc`.
    Coq,
    /// **Lean 4** — DTT + univalent foundations, via `lean`.
    Lean4,
    /// **Isabelle/HOL** — higher-order logic, via `isabelle build`.
    Isabelle,
    /// **Dedukti** — λΠ-modulo, via `kontroli` / `dkcheck`.
    Dedukti,
}

impl ExportFormat {
    /// Diagnostic name.
    pub fn name(self) -> &'static str {
        match self {
            ExportFormat::Coq => "coq",
            ExportFormat::Lean4 => "lean4",
            ExportFormat::Isabelle => "isabelle",
            ExportFormat::Dedukti => "dedukti",
        }
    }

    /// File extension produced by this format.
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Coq => "v",
            ExportFormat::Lean4 => "lean",
            ExportFormat::Isabelle => "thy",
            ExportFormat::Dedukti => "dk",
        }
    }

    /// Iterate the full list (= MSFS-required formats).
    pub fn full_list() -> [ExportFormat; 4] {
        [
            ExportFormat::Coq,
            ExportFormat::Lean4,
            ExportFormat::Isabelle,
            ExportFormat::Dedukti,
        ]
    }
}

// =============================================================================
// Per-format status
// =============================================================================

/// Per-format CI status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FormatStatus {
    /// Format export was checked successfully by the foreign tool.
    Passed {
        /// Diagnostic message (e.g. tool version, run duration).
        message: Text,
    },
    /// Format export was checked and the foreign tool reported failure.
    Failed {
        /// Failure diagnostic.
        reason: Text,
    },
    /// Format export was not run (CI not configured, or skipped).
    NotRun {
        /// Skip reason.
        reason: Text,
    },
}

impl FormatStatus {
    pub fn is_passed(&self) -> bool {
        matches!(self, FormatStatus::Passed { .. })
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, FormatStatus::Failed { .. })
    }
}

// =============================================================================
// CrossFormatReport
// =============================================================================

/// A cross-format CI report — per-format status + overall gate
/// evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrossFormatReport {
    /// Diagnostic name of the artefact under check (e.g. theorem name).
    pub artefact: Text,
    /// Per-format status entries.
    pub formats: Vec<(ExportFormat, FormatStatus)>,
}

impl CrossFormatReport {
    pub fn new(artefact: impl Into<Text>) -> Self {
        Self {
            artefact: artefact.into(),
            formats: Vec::new(),
        }
    }

    /// Record a per-format status.
    pub fn record(&mut self, fmt: ExportFormat, status: FormatStatus) {
        // Replace any existing entry for this format.
        if let Some(slot) = self.formats.iter_mut().find(|(f, _)| *f == fmt) {
            slot.1 = status;
        } else {
            self.formats.push((fmt, status));
        }
    }

    /// True iff every required format is `Passed`.
    pub fn all_passed(&self) -> bool {
        let required = required_formats_for_msfs();
        required.iter().all(|f| {
            self.formats
                .iter()
                .any(|(fmt, st)| *fmt == *f && st.is_passed())
        })
    }

    /// Return the list of formats that are not `Passed` (Failed +
    /// NotRun + missing).  Used by `verum audit` to emit the
    /// "missing-format" diagnostic.
    pub fn missing_or_failed(&self) -> Vec<ExportFormat> {
        let required = required_formats_for_msfs();
        required
            .iter()
            .filter(|f| {
                !self.formats.iter().any(|(fmt, st)| *fmt == **f && st.is_passed())
            })
            .copied()
            .collect()
    }

    /// Render a human-readable summary.
    pub fn summary(&self) -> String {
        let total = self.formats.len();
        let passed = self.formats.iter().filter(|(_, st)| st.is_passed()).count();
        format!(
            "cross-format[{}]: {}/{} passed; gate = {}",
            self.artefact.as_str(),
            passed,
            total,
            if self.all_passed() { "GREEN" } else { "RED" }
        )
    }
}

// =============================================================================
// Gate evaluation
// =============================================================================

/// MSFS-required format list — the canonical set that every theorem
/// must survive.  Currently all four.
pub fn required_formats_for_msfs() -> Vec<ExportFormat> {
    ExportFormat::full_list().to_vec()
}

/// Decide the cross-format hard gate: a report passes iff every
/// required format reports `Passed`.
pub fn evaluate_gate(report: &CrossFormatReport) -> bool {
    report.all_passed()
}

/// Emit the deterministic shell command that re-checks a given
/// format's output for an artefact.  Used by reproducibility audits.
///
/// Returns a string of the form `coqc <path>.v` (etc.).  V1
/// promotion: thread sandbox flags + version-pin metadata.
pub fn format_replay_command(fmt: ExportFormat, artefact_stem: &str) -> String {
    match fmt {
        ExportFormat::Coq => format!("coqc {}.v", artefact_stem),
        ExportFormat::Lean4 => format!("lean {}.lean", artefact_stem),
        ExportFormat::Isabelle => format!("isabelle build -d {}.thy", artefact_stem),
        ExportFormat::Dedukti => format!("kontroli check {}.dk", artefact_stem),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passed(msg: &str) -> FormatStatus {
        FormatStatus::Passed {
            message: Text::from(msg),
        }
    }

    fn failed(reason: &str) -> FormatStatus {
        FormatStatus::Failed {
            reason: Text::from(reason),
        }
    }

    fn not_run(reason: &str) -> FormatStatus {
        FormatStatus::NotRun {
            reason: Text::from(reason),
        }
    }

    // ----- ExportFormat -----

    #[test]
    fn export_format_full_list_has_four() {
        assert_eq!(ExportFormat::full_list().len(), 4);
    }

    #[test]
    fn export_format_extensions_are_distinct() {
        let exts: std::collections::HashSet<&str> = ExportFormat::full_list()
            .iter()
            .map(|f| f.extension())
            .collect();
        assert_eq!(exts.len(), 4);
    }

    // ----- FormatStatus -----

    #[test]
    fn passed_predicate_decides() {
        assert!(passed("ok").is_passed());
        assert!(!failed("nope").is_passed());
        assert!(!not_run("ci-skipped").is_passed());
    }

    // ----- CrossFormatReport -----

    #[test]
    fn empty_report_does_not_pass_gate() {
        let r = CrossFormatReport::new("theorem_X");
        assert!(!r.all_passed(),
            "Empty report must not satisfy the hard gate (every format must be checked)");
    }

    #[test]
    fn three_of_four_passed_does_not_pass_gate() {
        let mut r = CrossFormatReport::new("theorem_X");
        r.record(ExportFormat::Coq, passed("coq 8.18 ok"));
        r.record(ExportFormat::Lean4, passed("lean 4.5 ok"));
        r.record(ExportFormat::Isabelle, passed("isabelle 2024 ok"));
        // dedukti missing
        assert!(!r.all_passed());
        let missing = r.missing_or_failed();
        assert_eq!(missing, vec![ExportFormat::Dedukti]);
    }

    #[test]
    fn all_four_passed_satisfies_gate() {
        let mut r = CrossFormatReport::new("theorem_X");
        for f in ExportFormat::full_list() {
            r.record(f, passed("ok"));
        }
        assert!(r.all_passed());
        assert!(evaluate_gate(&r));
        assert_eq!(r.missing_or_failed().len(), 0);
    }

    #[test]
    fn one_failure_defeats_gate() {
        let mut r = CrossFormatReport::new("theorem_X");
        for f in ExportFormat::full_list() {
            r.record(f, passed("ok"));
        }
        r.record(ExportFormat::Lean4, failed("type error"));
        assert!(!r.all_passed());
        assert!(r.missing_or_failed().contains(&ExportFormat::Lean4));
    }

    #[test]
    fn record_replaces_existing_entry() {
        let mut r = CrossFormatReport::new("theorem_X");
        r.record(ExportFormat::Coq, failed("first"));
        r.record(ExportFormat::Coq, passed("second"));
        assert_eq!(r.formats.len(), 1);
        assert!(r.formats[0].1.is_passed());
    }

    #[test]
    fn summary_renders_progress() {
        let mut r = CrossFormatReport::new("theorem_X");
        r.record(ExportFormat::Coq, passed("ok"));
        r.record(ExportFormat::Lean4, passed("ok"));
        let s = r.summary();
        assert!(s.contains("2/2 passed"));
        assert!(s.contains("RED"), "Gate must be RED until all 4 formats pass");
    }

    // ----- Replay command -----

    #[test]
    fn replay_commands_use_correct_extensions() {
        for fmt in ExportFormat::full_list() {
            let cmd = format_replay_command(fmt, "theorem_5_1");
            assert!(cmd.contains("theorem_5_1"));
            assert!(cmd.contains(fmt.extension()),
                "Command for {:?} must reference its extension", fmt);
        }
    }

    #[test]
    fn replay_commands_invoke_correct_tool() {
        assert!(format_replay_command(ExportFormat::Coq, "x").starts_with("coqc"));
        assert!(format_replay_command(ExportFormat::Lean4, "x").starts_with("lean"));
        assert!(format_replay_command(ExportFormat::Isabelle, "x").starts_with("isabelle"));
        assert!(format_replay_command(ExportFormat::Dedukti, "x").starts_with("kontroli"));
    }

    // ----- MSFS-required-formats invariant -----

    #[test]
    fn msfs_requires_all_four_formats() {
        let req = required_formats_for_msfs();
        assert_eq!(req.len(), 4);
        for f in ExportFormat::full_list() {
            assert!(req.contains(&f));
        }
    }
}
