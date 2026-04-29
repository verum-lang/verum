//! `verum verify --ladder` — wires
//! `verum_verification::ladder_dispatch::DefaultLadderDispatcher` into the
//! CLI verify command path.
//!
//! Walks every `.vr` file in the project, projects every
//! `@verify(<strategy>)` annotation onto a typed
//! [`LadderObligation`](verum_verification::ladder_dispatch::LadderObligation),
//! routes through [`LadderDispatcher::dispatch`](verum_verification::ladder_dispatch::LadderDispatcher::dispatch),
//! and emits the per-theorem [`LadderVerdict`](verum_verification::ladder_dispatch::LadderVerdict).
//!
//! ## Why this is the integration that #71 was missing
//!
//! Before this command, the dispatcher trait surface was unit-tested but
//! never invoked from the CLI — meaning the typed-strategy contract was
//! a closed module with no production consumer.  This wires it through
//! the same architectural pattern as the proof-draft integration:
//!
//!   * single trait boundary (`LadderDispatcher`)
//!   * reference V0 impl (`DefaultLadderDispatcher`)
//!   * future LLM / portfolio / cross-format adapters slot in without
//!     touching the command handler
//!
//! ## Exit status
//!
//!   * `0`  — every dispatched obligation is Closed or DispatchPending.
//!   * `non-zero` — at least one obligation is Open or Timeout (a *real*
//!     verification failure, distinct from "not yet implemented").
//!     DispatchPending is treated as advisory rather than failure
//!     because the V0 ladder has 11 strategy slots whose backends ship
//!     in V1+; failing the build on every annotated `@verify(formal)`
//!     would be louder than useful at this stage.
//!
//! ## Output formats
//!
//!   * `plain` — human-readable verdict table + summary.
//!   * `json`  — LSP / CI-friendly structured payload.

use crate::error::{CliError, Result};
use crate::ui;
use verum_ast::decl::ItemKind;
use verum_common::Text;
use verum_verification::kernel_recheck::KernelRecheck;
use verum_verification::ladder_dispatch::{
    DefaultLadderDispatcher, KernelRecheckOutcome, LadderDispatcher, LadderObligation,
    LadderStrategy, LadderVerdict,
};

use super::audit::{discover_vr_files, parse_file_for_audit, strictest_verify_strategy};

/// True iff `strategy` is on the kernel-attestation tier of the
/// backbone (Proof or stricter).  Only these strata benefit from a
/// pre-computed `KernelRecheck::recheck_theorem` outcome — the
/// lower strata (Runtime/Static/Fast/CT/Formal) admit through
/// SMT/dataflow paths that don't consult the kernel attestation.
fn requires_kernel_recheck(strategy: LadderStrategy) -> bool {
    matches!(
        strategy,
        LadderStrategy::Proof
            | LadderStrategy::Thorough
            | LadderStrategy::Reliable
            | LadderStrategy::Certified
    )
}

/// Run `KernelRecheck::recheck_theorem` against a theorem-shaped
/// item and project the result list to a single dispatcher-facing
/// `KernelRecheckOutcome`.
///
/// Recognises:
///   * Theorem / Lemma / Corollary → `recheck_theorem` (#118)
///   * Axiom                        → `recheck_axiom`   (#119)
///
/// Returns `None` for kinds that don't carry refinement-type
/// leakage at this layer (definitions / functions / types are
/// covered by other verification phases).
fn run_kernel_recheck(
    item_kind: &ItemKind,
    item_name: &Text,
) -> Option<KernelRecheckOutcome> {
    let results = match item_kind {
        ItemKind::Theorem(t) | ItemKind::Lemma(t) | ItemKind::Corollary(t) => {
            KernelRecheck::recheck_theorem(t)
        }
        ItemKind::Axiom(a) => KernelRecheck::recheck_axiom(a),
        _ => return None,
    };
    let errors: Vec<&verum_verification::kernel_recheck::KernelRecheckError> = results
        .iter()
        .filter_map(|(_, r)| r.as_ref().err())
        .collect();
    if errors.is_empty() {
        Some(KernelRecheckOutcome::Admitted {
            context: item_name.clone(),
        })
    } else {
        // Pick the first rejection as the representative reason.
        let reason = Text::from(format!("{}", errors[0]));
        Some(KernelRecheckOutcome::Rejected {
            reason,
            error_count: errors.len(),
        })
    }
}

/// One per-theorem verdict line.
struct VerdictRecord {
    item_kind: &'static str,
    item_name: Text,
    file: std::path::PathBuf,
    declared_strategy: Text,
    verdict: LadderVerdict,
}

/// Aggregate counters across the dispatcher run.
#[derive(Debug, Default, Clone, Copy)]
struct VerdictTotals {
    closed: usize,
    open: usize,
    pending: usize,
    timeout: usize,
}

impl VerdictTotals {
    fn record(&mut self, v: &LadderVerdict) {
        match v {
            LadderVerdict::Closed { .. } => self.closed += 1,
            LadderVerdict::Open { .. } => self.open += 1,
            LadderVerdict::DispatchPending { .. } => self.pending += 1,
            LadderVerdict::Timeout { .. } => self.timeout += 1,
        }
    }

    fn total(&self) -> usize {
        self.closed + self.open + self.pending + self.timeout
    }

    /// Hard failure ⇒ Open or Timeout.  DispatchPending is advisory.
    fn has_hard_failure(&self) -> bool {
        self.open > 0 || self.timeout > 0
    }
}

/// Run the ladder-verify command on the project rooted at the current
/// manifest.  Format must be `"plain"` or `"json"`.
pub fn run_verify_ladder(format: &str) -> Result<()> {
    if format != "plain" && format != "json" {
        return Err(CliError::InvalidArgument(format!(
            "--format must be 'plain' or 'json', got '{}'",
            format
        )));
    }

    let manifest_dir = crate::config::Manifest::find_manifest_dir()?;
    let vr_files = discover_vr_files(&manifest_dir);

    let dispatcher = DefaultLadderDispatcher::new();
    let mut records: Vec<VerdictRecord> = Vec::new();
    let mut totals = VerdictTotals::default();
    let mut parsed_files = 0usize;
    let mut skipped_files = 0usize;

    for abs_path in &vr_files {
        let rel_path = abs_path
            .strip_prefix(&manifest_dir)
            .unwrap_or(abs_path)
            .to_path_buf();
        let module = match parse_file_for_audit(abs_path) {
            Ok(m) => m,
            Err(_) => {
                skipped_files += 1;
                continue;
            }
        };
        parsed_files += 1;

        for item in &module.items {
            let (kind_label, item_name, decl_attrs): (
                &'static str,
                Text,
                &verum_common::List<verum_ast::attr::Attribute>,
            ) = match &item.kind {
                ItemKind::Theorem(decl) => ("theorem", decl.name.name.clone(), &decl.attributes),
                ItemKind::Lemma(decl) => ("lemma", decl.name.name.clone(), &decl.attributes),
                ItemKind::Corollary(decl) => {
                    ("corollary", decl.name.name.clone(), &decl.attributes)
                }
                // #119 — axioms carry refinement-type leakage in their
                // params + return type + proposition; surface them to
                // the dispatcher so the kernel-recheck bridge fires
                // on `@verify(proof)` axiom declarations too.
                ItemKind::Axiom(decl) => ("axiom", decl.name.name.clone(), &decl.attributes),
                _ => continue,
            };

            let Some(strategy_label) =
                strictest_verify_strategy(&item.attributes, decl_attrs)
            else {
                continue;
            };

            let typed_strategy = match LadderStrategy::from_name(strategy_label.as_str()) {
                Some(s) => s,
                None => {
                    // Unknown strategy label — record an explicit Open
                    // verdict so the user sees the label that didn't
                    // project to any of the 13 ladder slots, rather
                    // than silently skipping.
                    let verdict = LadderVerdict::Open {
                        strategy: LadderStrategy::Runtime,
                        reason: Text::from(format!(
                            "unknown verify strategy '{}' — not one of the 13 ladder slots",
                            strategy_label.as_str()
                        )),
                    };
                    totals.record(&verdict);
                    records.push(VerdictRecord {
                        item_kind: kind_label,
                        item_name,
                        file: rel_path.clone(),
                        declared_strategy: strategy_label,
                        verdict,
                    });
                    continue;
                }
            };

            // #118 — when the strategy is `Proof` (or stricter on
            // the backbone) AND the item is theorem-shaped, run
            // `KernelRecheck::recheck_theorem` and stash the
            // outcome so the dispatcher can admit / reject on
            // kernel attestation directly.  The other strategies
            // continue through the trivial-decider for V0 surface.
            let mut obligation = LadderObligation::text(
                item_name.clone(),
                typed_strategy,
                "(elaborated obligation)",
            );
            if requires_kernel_recheck(typed_strategy) {
                if let Some(outcome) = run_kernel_recheck(&item.kind, &item_name) {
                    obligation = obligation.with_kernel_recheck_outcome(outcome);
                }
            }
            let verdict = dispatcher.dispatch(&obligation);
            totals.record(&verdict);
            records.push(VerdictRecord {
                item_kind: kind_label,
                item_name,
                file: rel_path.clone(),
                declared_strategy: strategy_label,
                verdict,
            });
        }
    }

    match format {
        "plain" => emit_plain(&records, &totals, vr_files.len(), parsed_files, skipped_files),
        "json" => emit_json(&records, &totals, vr_files.len(), parsed_files, skipped_files),
        _ => unreachable!(),
    }

    if totals.has_hard_failure() {
        return Err(CliError::VerificationFailed(format!(
            "ladder dispatch produced {} hard-failure verdict(s) (open={}, timeout={})",
            totals.open + totals.timeout,
            totals.open,
            totals.timeout,
        )));
    }
    Ok(())
}

fn emit_plain(
    records: &[VerdictRecord],
    totals: &VerdictTotals,
    files_scanned: usize,
    files_parsed: usize,
    files_skipped: usize,
) {
    ui::step("Ladder dispatch — per-theorem @verify(strategy) verdicts");
    println!();
    println!(
        "  {:<48}  {:<18}  {:<18}  {}",
        "Theorem / lemma / corollary", "Strategy", "Verdict", "Detail"
    );
    println!(
        "  {}  {}  {}  {}",
        "─".repeat(48),
        "─".repeat(18),
        "─".repeat(18),
        "─".repeat(20)
    );
    for r in records {
        let (verdict_label, detail) = verdict_summary(&r.verdict);
        println!(
            "  {:<48}  {:<18}  {:<18}  {}",
            r.item_name.as_str(),
            r.declared_strategy.as_str(),
            verdict_label,
            detail
        );
    }
    println!();
    println!("  Verdict totals:");
    println!("    closed             {:>4}", totals.closed);
    println!("    open               {:>4}", totals.open);
    println!("    dispatch_pending   {:>4}", totals.pending);
    println!("    timeout            {:>4}", totals.timeout);
    println!("    total              {:>4}", totals.total());
    println!();
    println!(
        "  Files: {} scanned, {} parsed, {} skipped",
        files_scanned, files_parsed, files_skipped
    );
}

fn emit_json(
    records: &[VerdictRecord],
    totals: &VerdictTotals,
    files_scanned: usize,
    files_parsed: usize,
    files_skipped: usize,
) {
    let mut out = String::from("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!("  \"theorem_count\": {},\n", records.len()));
    out.push_str("  \"totals\": {\n");
    out.push_str(&format!("    \"closed\": {},\n", totals.closed));
    out.push_str(&format!("    \"open\": {},\n", totals.open));
    out.push_str(&format!(
        "    \"dispatch_pending\": {},\n",
        totals.pending
    ));
    out.push_str(&format!("    \"timeout\": {}\n", totals.timeout));
    out.push_str("  },\n");
    out.push_str("  \"files\": {\n");
    out.push_str(&format!("    \"scanned\": {},\n", files_scanned));
    out.push_str(&format!("    \"parsed\": {},\n", files_parsed));
    out.push_str(&format!("    \"skipped\": {}\n", files_skipped));
    out.push_str("  },\n");
    out.push_str("  \"theorems\": [\n");
    for (i, r) in records.iter().enumerate() {
        let (label, detail) = verdict_summary(&r.verdict);
        out.push_str(&format!(
            "    {{ \"kind\": \"{}\", \"name\": \"{}\", \"file\": \"{}\", \"strategy\": \"{}\", \"verdict\": \"{}\", \"detail\": \"{}\" }}{}\n",
            r.item_kind,
            json_escape(r.item_name.as_str()),
            json_escape(&r.file.display().to_string()),
            json_escape(r.declared_strategy.as_str()),
            label,
            json_escape(&detail),
            if i + 1 < records.len() { "," } else { "" }
        ));
    }
    out.push_str("  ]\n}");
    println!("{}", out);
}

fn verdict_summary(v: &LadderVerdict) -> (&'static str, String) {
    match v {
        LadderVerdict::Closed {
            witness,
            elapsed_ms,
            ..
        } => (
            "closed",
            format!("{} ({}ms)", truncate(witness.as_str(), 60), elapsed_ms),
        ),
        LadderVerdict::Open { reason, .. } => ("open", truncate(reason.as_str(), 80)),
        LadderVerdict::DispatchPending { note, .. } => {
            ("dispatch_pending", truncate(note.as_str(), 80))
        }
        LadderVerdict::Timeout { budget_ms, .. } => {
            ("timeout", format!("budget={}ms", budget_ms))
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obligation(strategy: LadderStrategy, name: &str) -> LadderObligation {
        LadderObligation::text(name, strategy, "trivial")
    }

    // ----- VerdictTotals -----

    #[test]
    fn totals_record_each_variant_increments_one_counter() {
        let mut t = VerdictTotals::default();
        t.record(&LadderVerdict::Closed {
            strategy: LadderStrategy::Runtime,
            witness: Text::from("w"),
            elapsed_ms: 0,
        });
        t.record(&LadderVerdict::Open {
            strategy: LadderStrategy::Runtime,
            reason: Text::from("r"),
        });
        t.record(&LadderVerdict::DispatchPending {
            strategy: LadderStrategy::Formal,
            note: Text::from("V1"),
        });
        t.record(&LadderVerdict::Timeout {
            strategy: LadderStrategy::Fast,
            budget_ms: 100,
        });
        assert_eq!(t.closed, 1);
        assert_eq!(t.open, 1);
        assert_eq!(t.pending, 1);
        assert_eq!(t.timeout, 1);
        assert_eq!(t.total(), 4);
    }

    #[test]
    fn has_hard_failure_only_open_or_timeout() {
        let mut t = VerdictTotals::default();
        t.closed = 100;
        t.pending = 10;
        assert!(!t.has_hard_failure(), "Closed + Pending only → not hard failure");
        t.open = 1;
        assert!(t.has_hard_failure(), "Open is a hard failure");
        t.open = 0;
        t.timeout = 1;
        assert!(t.has_hard_failure(), "Timeout is a hard failure");
    }

    // ----- verdict_summary -----

    #[test]
    fn verdict_summary_labels_match_variants() {
        assert_eq!(
            verdict_summary(&LadderVerdict::Closed {
                strategy: LadderStrategy::Runtime,
                witness: Text::from("w"),
                elapsed_ms: 0,
            })
            .0,
            "closed"
        );
        assert_eq!(
            verdict_summary(&LadderVerdict::Open {
                strategy: LadderStrategy::Runtime,
                reason: Text::from("r"),
            })
            .0,
            "open"
        );
        assert_eq!(
            verdict_summary(&LadderVerdict::DispatchPending {
                strategy: LadderStrategy::Formal,
                note: Text::from("V1"),
            })
            .0,
            "dispatch_pending"
        );
        assert_eq!(
            verdict_summary(&LadderVerdict::Timeout {
                strategy: LadderStrategy::Fast,
                budget_ms: 100,
            })
            .0,
            "timeout"
        );
    }

    // ----- truncate -----

    #[test]
    fn truncate_short_pass_through() {
        assert_eq!(truncate("hello", 80), "hello");
    }

    #[test]
    fn truncate_long_appends_ellipsis() {
        let long = "a".repeat(200);
        let t = truncate(&long, 50);
        assert_eq!(t.chars().count(), 50);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn truncate_handles_unicode_no_panic() {
        let s = "αβγδ".repeat(50);
        let t = truncate(&s, 10);
        assert_eq!(t.chars().count(), 10);
    }

    // ----- json_escape -----

    #[test]
    fn json_escape_quotes_and_backslashes() {
        assert_eq!(json_escape("a\"b\\c"), "a\\\"b\\\\c");
    }

    #[test]
    fn json_escape_control_chars() {
        assert_eq!(json_escape("a\nb"), "a\\nb");
    }

    // ----- DefaultLadderDispatcher integration -----

    #[test]
    fn dispatcher_runtime_to_closed_via_handler_path() {
        let d = DefaultLadderDispatcher::new();
        let v = d.dispatch(&obligation(LadderStrategy::Runtime, "thm_r"));
        let (label, _) = verdict_summary(&v);
        assert_eq!(label, "closed");
    }

    #[test]
    fn dispatcher_formal_to_pending_via_handler_path() {
        let d = DefaultLadderDispatcher::new();
        let v = d.dispatch(&obligation(LadderStrategy::Formal, "thm_f"));
        let (label, _) = verdict_summary(&v);
        assert_eq!(label, "dispatch_pending");
    }

    // ----- format validation -----

    #[test]
    fn run_verify_ladder_rejects_unknown_format() {
        // Doesn't need a manifest — fails at format validation.
        let r = run_verify_ladder("yaml");
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }
}
