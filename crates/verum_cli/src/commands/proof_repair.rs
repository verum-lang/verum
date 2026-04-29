//! `verum proof-repair` subcommand — wires
//! `verum_diagnostics::proof_repair::DefaultRepairEngine` into the CLI
//! so IDE / LSP / REPL consumers can request structured repair
//! suggestions for a typed [`ProofFailureKind`] without depending on
//! the Rust API.
//!
//! ## Why this is the integration that #74 was missing
//!
//! Prior to this command the `RepairEngine` trait surface was
//! unit-tested but had no production CLI consumer. The diagnostic
//! emission flow surfaced `KernelError` / `CheckerError` variants
//! directly, with no path to ranked structured repair suggestions.
//!
//! This command provides the **transport-layer integration**: given
//! a typed failure description (kind + structured fields) it emits
//! the ranked V0 catalogue from
//! [`DefaultRepairEngine`](verum_diagnostics::proof_repair::DefaultRepairEngine).
//! The verify-pipeline → ProofFailureKind projection (the V1 step)
//! can then call into this same engine without re-plumbing.
//!
//! Same architectural pattern as proof-draft and verify-ladder:
//! single trait boundary, reference V0 impl, future adapters
//! (LLM-repair, MSFS-corpus-aware) plug in via `CompositeRepairEngine`
//! without touching the command handler.
//!
//! ## Usage
//!
//! ```text
//! verum proof-repair --kind unbound-name --field name=foo_lemma
//! verum proof-repair --kind refine-depth \
//!     --field refined_type=CategoricalLevel \
//!     --field predicate_depth=ω·2 \
//!     --max 3 --format json
//! ```
//!
//! ## Output formats
//!
//!   * `plain` — human-readable ranked list with rationale + doc-link.
//!   * `json`  — LSP-friendly structured payload.

use crate::error::{CliError, Result};
use std::collections::HashMap;
use verum_common::Text;
use verum_diagnostics::proof_repair::{
    DefaultRepairEngine, ProofFailureKind, RepairApplicability, RepairEngine,
};

/// Run the proof-repair command.  Returns `Ok(())` on success;
/// errors propagate up the CLI dispatcher.
pub fn run_proof_repair(
    kind: &str,
    fields: &[String],
    max_results: usize,
    format: &str,
) -> Result<()> {
    if max_results == 0 {
        return Err(CliError::InvalidArgument(
            "--max must be > 0".into(),
        ));
    }
    if format != "plain" && format != "json" {
        return Err(CliError::InvalidArgument(format!(
            "--format must be 'plain' or 'json', got '{}'",
            format
        )));
    }

    let parsed_fields = parse_fields(fields)?;
    let failure = build_failure(kind, &parsed_fields)?;

    let engine = DefaultRepairEngine::new();
    let suggestions = engine.suggest(&failure, max_results);

    match format {
        "plain" => {
            println!("Failure kind: {}", kind);
            println!("Suggestions ({}):", suggestions.len());
            println!();
            if suggestions.is_empty() {
                println!("  (no repair suggestions available for this failure kind)");
            } else {
                for (i, s) in suggestions.iter().enumerate() {
                    println!(
                        "  {}. [{:.2} | {}] {}",
                        i + 1,
                        s.score,
                        applicability_name(s.applicability),
                        first_line(s.snippet.as_str())
                    );
                    println!("     ↪ {}", s.rationale.as_str());
                    if let Some(link) = &s.doc_link {
                        println!("     📖 {}", link.as_str());
                    }
                }
            }
        }
        "json" => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"kind\": \"{}\",\n", json_escape(kind)));
            out.push_str(&format!(
                "  \"suggestion_count\": {},\n",
                suggestions.len()
            ));
            out.push_str("  \"suggestions\": [\n");
            for (i, s) in suggestions.iter().enumerate() {
                let doc_link_field = match &s.doc_link {
                    Some(link) => format!("\"{}\"", json_escape(link.as_str())),
                    None => "null".to_string(),
                };
                out.push_str(&format!(
                    "    {{ \"snippet\": \"{}\", \"rationale\": \"{}\", \"applicability\": \"{}\", \"score\": {:.4}, \"doc_link\": {} }}{}\n",
                    json_escape(s.snippet.as_str()),
                    json_escape(s.rationale.as_str()),
                    applicability_name(s.applicability),
                    s.score,
                    doc_link_field,
                    if i + 1 < suggestions.len() { "," } else { "" }
                ));
            }
            out.push_str("  ]\n}");
            println!("{}", out);
        }
        _ => unreachable!(),
    }

    Ok(())
}

/// Parse `--field key=value` flags into a map.
fn parse_fields(flags: &[String]) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for raw in flags {
        let Some(eq) = raw.find('=') else {
            return Err(CliError::InvalidArgument(format!(
                "--field must be `key=value`, got `{}`",
                raw
            )));
        };
        let key = raw[..eq].trim().to_string();
        let value = raw[eq + 1..].to_string();
        if key.is_empty() {
            return Err(CliError::InvalidArgument(
                "--field key must be non-empty".into(),
            ));
        }
        map.insert(key, value);
    }
    Ok(map)
}

/// Project the kind name + field map onto a typed [`ProofFailureKind`].
/// Required fields per kind are validated up-front so the error message
/// names the missing field instead of producing an empty suggestion list.
fn build_failure(
    kind: &str,
    fields: &HashMap<String, String>,
) -> Result<ProofFailureKind> {
    fn require<'a>(
        kind: &str,
        fields: &'a HashMap<String, String>,
        key: &str,
    ) -> Result<&'a str> {
        fields.get(key).map(String::as_str).ok_or_else(|| {
            CliError::InvalidArgument(format!(
                "--kind {} requires --field {}=<value>",
                kind, key
            ))
        })
    }

    match kind {
        "refine-depth" => Ok(ProofFailureKind::RefineDepthViolation {
            refined_type: Text::from(require(kind, fields, "refined_type")?),
            predicate_depth: Text::from(require(kind, fields, "predicate_depth")?),
        }),
        "positivity" => Ok(ProofFailureKind::PositivityViolation {
            type_name: Text::from(require(kind, fields, "type_name")?),
            constructor: Text::from(require(kind, fields, "constructor")?),
            position: Text::from(require(kind, fields, "position")?),
        }),
        "universe" => Ok(ProofFailureKind::UniverseInconsistency {
            source_universe: Text::from(require(kind, fields, "source_universe")?),
            expected_universe: Text::from(require(kind, fields, "expected_universe")?),
        }),
        "fwax-not-prop" => Ok(ProofFailureKind::FrameworkAxiomNotProp {
            axiom_name: Text::from(require(kind, fields, "axiom_name")?),
            body_sort: Text::from(require(kind, fields, "body_sort")?),
        }),
        "adjunction" => {
            let side = require(kind, fields, "side")?;
            if side != "unit" && side != "counit" {
                return Err(CliError::InvalidArgument(format!(
                    "--field side must be 'unit' or 'counit', got '{}'",
                    side
                )));
            }
            Ok(ProofFailureKind::AdjunctionRoundTripFailure {
                side: Text::from(side),
            })
        }
        "type-mismatch" => Ok(ProofFailureKind::TypeMismatch {
            expected: Text::from(require(kind, fields, "expected")?),
            actual: Text::from(require(kind, fields, "actual")?),
        }),
        "unbound-name" => Ok(ProofFailureKind::UnboundName {
            name: Text::from(require(kind, fields, "name")?),
        }),
        "apply-mismatch" => Ok(ProofFailureKind::ApplyMismatch {
            lemma_name: Text::from(require(kind, fields, "lemma_name")?),
            actual_conclusion: Text::from(require(kind, fields, "actual_conclusion")?),
            goal: Text::from(require(kind, fields, "goal")?),
        }),
        "tactic-open" => Ok(ProofFailureKind::TacticOpen {
            tactic: Text::from(require(kind, fields, "tactic")?),
            reason: Text::from(require(kind, fields, "reason")?),
        }),
        other => Err(CliError::InvalidArgument(format!(
            "unknown --kind '{}' (valid: refine-depth, positivity, universe, \
             fwax-not-prop, adjunction, type-mismatch, unbound-name, \
             apply-mismatch, tactic-open)",
            other
        ))),
    }
}

fn applicability_name(a: RepairApplicability) -> &'static str {
    match a {
        RepairApplicability::MachineApplicable => "machine_applicable",
        RepairApplicability::MaybeIncorrect => "maybe_incorrect",
        RepairApplicability::HasPlaceholders => "has_placeholders",
        RepairApplicability::Speculative => "speculative",
    }
}

/// Snippets are often multi-line; the plain renderer needs a one-line
/// summary in the headline so the table stays scannable. Subsequent
/// lines drop down via the `↪ rationale` indented continuation.
fn first_line(s: &str) -> &str {
    s.split('\n').next().unwrap_or("")
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

    // ----- parse_fields -----

    #[test]
    fn parse_fields_simple_key_value() {
        let m = parse_fields(&["name=foo".to_string()]).unwrap();
        assert_eq!(m.get("name").map(String::as_str), Some("foo"));
    }

    #[test]
    fn parse_fields_value_with_equals_preserved() {
        let m = parse_fields(&["expr=a == b".to_string()]).unwrap();
        assert_eq!(m.get("expr").map(String::as_str), Some("a == b"));
    }

    #[test]
    fn parse_fields_rejects_missing_equals() {
        assert!(parse_fields(&["bad".to_string()]).is_err());
    }

    #[test]
    fn parse_fields_rejects_empty_key() {
        assert!(parse_fields(&["=value".to_string()]).is_err());
    }

    // ----- build_failure -----

    #[test]
    fn build_failure_unbound_name_ok() {
        let mut m = HashMap::new();
        m.insert("name".into(), "foo".into());
        let f = build_failure("unbound-name", &m).unwrap();
        assert!(matches!(f, ProofFailureKind::UnboundName { .. }));
    }

    #[test]
    fn build_failure_unbound_name_missing_field_errors() {
        let m = HashMap::new();
        let err = build_failure("unbound-name", &m).unwrap_err();
        assert!(matches!(err, CliError::InvalidArgument(_)));
    }

    #[test]
    fn build_failure_adjunction_validates_side() {
        let mut m = HashMap::new();
        m.insert("side".into(), "garbage".into());
        let err = build_failure("adjunction", &m).unwrap_err();
        assert!(matches!(err, CliError::InvalidArgument(_)));

        let mut ok = HashMap::new();
        ok.insert("side".into(), "unit".into());
        assert!(build_failure("adjunction", &ok).is_ok());
        let mut ok2 = HashMap::new();
        ok2.insert("side".into(), "counit".into());
        assert!(build_failure("adjunction", &ok2).is_ok());
    }

    #[test]
    fn build_failure_unknown_kind_errors() {
        let m = HashMap::new();
        let err = build_failure("nonsense", &m).unwrap_err();
        assert!(matches!(err, CliError::InvalidArgument(_)));
    }

    // ----- applicability / first_line / json_escape -----

    #[test]
    fn applicability_name_round_trip() {
        assert_eq!(
            applicability_name(RepairApplicability::MachineApplicable),
            "machine_applicable"
        );
        assert_eq!(
            applicability_name(RepairApplicability::Speculative),
            "speculative"
        );
    }

    #[test]
    fn first_line_truncates_at_newline() {
        assert_eq!(first_line("a\nb\nc"), "a");
        assert_eq!(first_line("single"), "single");
        assert_eq!(first_line(""), "");
    }

    #[test]
    fn json_escape_handles_quotes_and_newlines() {
        assert_eq!(json_escape("a\"b\nc"), "a\\\"b\\nc");
    }

    // ----- run_proof_repair -----

    #[test]
    fn run_rejects_zero_max() {
        let r = run_proof_repair("unbound-name", &["name=foo".to_string()], 0, "plain");
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    #[test]
    fn run_rejects_unknown_format() {
        let r = run_proof_repair("unbound-name", &["name=foo".to_string()], 5, "yaml");
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    #[test]
    fn run_proof_repair_unbound_name_smoke() {
        let r = run_proof_repair(
            "unbound-name",
            &["name=foo_lemma".to_string()],
            5,
            "plain",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn run_proof_repair_refine_depth_json() {
        let r = run_proof_repair(
            "refine-depth",
            &[
                "refined_type=CategoricalLevel".to_string(),
                "predicate_depth=ω·2".to_string(),
            ],
            3,
            "json",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn run_proof_repair_missing_required_field_errors() {
        // `refine-depth` requires both refined_type and predicate_depth.
        let r = run_proof_repair(
            "refine-depth",
            &["refined_type=X".to_string()],
            5,
            "plain",
        );
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    // ----- DefaultRepairEngine integration through handler -----

    #[test]
    fn engine_returns_at_least_one_for_every_kind_via_handler_path() {
        // Integration smoke: every typed kind we expose via the CLI
        // must produce ≥ 1 suggestion through the actual handler
        // path. This pins the contract that V0 catalogue gaps fail
        // fast at the handler level rather than emitting a
        // misleading "no suggestions" line to LSP consumers.
        let cases: &[(&str, &[(&str, &str)])] = &[
            ("refine-depth", &[("refined_type", "X"), ("predicate_depth", "ω")]),
            (
                "positivity",
                &[
                    ("type_name", "Bad"),
                    ("constructor", "Wrap"),
                    ("position", "left of arrow"),
                ],
            ),
            (
                "universe",
                &[("source_universe", "Type_1"), ("expected_universe", "Type_0")],
            ),
            (
                "fwax-not-prop",
                &[("axiom_name", "ax"), ("body_sort", "Type")],
            ),
            ("adjunction", &[("side", "unit")]),
            ("type-mismatch", &[("expected", "Int"), ("actual", "Bool")]),
            ("unbound-name", &[("name", "foo")]),
            (
                "apply-mismatch",
                &[
                    ("lemma_name", "f"),
                    ("actual_conclusion", "A"),
                    ("goal", "B"),
                ],
            ),
            ("tactic-open", &[("tactic", "lia"), ("reason", "non-trivial")]),
        ];

        let engine = DefaultRepairEngine::new();
        for (kind, fields) in cases {
            let mut map = HashMap::new();
            for (k, v) in *fields {
                map.insert(k.to_string(), v.to_string());
            }
            let f = build_failure(kind, &map).expect(kind);
            let s = engine.suggest(&f, 5);
            assert!(!s.is_empty(), "kind {} produced no suggestions", kind);
        }
    }
}
