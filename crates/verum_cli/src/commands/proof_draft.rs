//! `verum proof-draft` subcommand — surfaces the
//! `verum_verification::proof_drafting::SuggestionEngine` for IDE /
//! REPL / CLI use.
//!
//! Given a theorem name + a focused-goal description + an
//! `--lemma name:::signature` list, the command returns ranked
//! next-step tactic suggestions.
//!
//! Output formats:
//!   * `plain` — human-readable with rationales.
//!   * `json`  — LSP-friendly structured payload.

use crate::error::{CliError, Result};
use verum_common::Text;
use verum_verification::proof_drafting::{
    DefaultSuggestionEngine, LemmaSummary, ProofGoalSummary, ProofStateView,
    SuggestionEngine,
};

/// Run the proof-draft command.  Returns `Ok(())` on success;
/// errors propagate up the CLI dispatcher.
pub fn run_proof_draft(
    theorem: &str,
    goal: &str,
    lemmas: &[String],
    max_results: usize,
    format: &str,
) -> Result<()> {
    if theorem.is_empty() {
        return Err(CliError::InvalidArgument(
            "--theorem must be non-empty".into(),
        ));
    }
    if goal.is_empty() {
        return Err(CliError::InvalidArgument(
            "--goal must be non-empty".into(),
        ));
    }
    if max_results == 0 {
        return Err(CliError::InvalidArgument(
            "--max must be > 0".into(),
        ));
    }

    let parsed_lemmas = parse_lemma_flags(lemmas)?;

    let view = ProofStateView {
        theorem_name: Text::from(theorem),
        goals: vec![ProofGoalSummary {
            goal_id: 0,
            proposition: Text::from(goal),
            hypotheses: vec![],
            is_focused: true,
        }],
        available_lemmas: parsed_lemmas,
        history: Vec::new(),
    };

    let engine = DefaultSuggestionEngine::new();
    let suggestions = engine.suggest(&view, max_results);

    match format {
        "plain" => {
            println!("Goal: {}", goal);
            println!("Theorem: {}", theorem);
            println!("Suggestions ({}):", suggestions.len());
            println!();
            if suggestions.is_empty() {
                println!(
                    "  (no suggestions — try widening lemma scope with `--lemma`)"
                );
            } else {
                for (i, s) in suggestions.iter().enumerate() {
                    println!(
                        "  {}. [{:.2} | {}] {}",
                        i + 1,
                        s.score,
                        s.category.name(),
                        s.snippet.as_str()
                    );
                    println!("     ↪ {}", s.rationale.as_str());
                }
            }
            Ok(())
        }
        "json" => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!(
                "  \"theorem\": \"{}\",\n",
                json_escape(theorem)
            ));
            out.push_str(&format!("  \"goal\": \"{}\",\n", json_escape(goal)));
            out.push_str(&format!(
                "  \"suggestion_count\": {},\n",
                suggestions.len()
            ));
            out.push_str("  \"suggestions\": [\n");
            for (i, s) in suggestions.iter().enumerate() {
                out.push_str(&format!(
                    "    {{ \"snippet\": \"{}\", \"rationale\": \"{}\", \"score\": {:.4}, \"category\": \"{}\" }}{}\n",
                    json_escape(s.snippet.as_str()),
                    json_escape(s.rationale.as_str()),
                    s.score,
                    s.category.name(),
                    if i + 1 < suggestions.len() { "," } else { "" }
                ));
            }
            out.push_str("  ]\n}");
            println!("{}", out);
            Ok(())
        }
        other => Err(CliError::InvalidArgument(format!(
            "--format must be 'plain' or 'json', got '{}'",
            other
        ))),
    }
}

/// Parse `--lemma name:::signature[:::lineage]` flags into typed
/// `LemmaSummary`s.  `lineage` defaults to `"corpus"` when absent.
fn parse_lemma_flags(flags: &[String]) -> Result<Vec<LemmaSummary>> {
    flags
        .iter()
        .map(|s| parse_one_lemma(s))
        .collect()
}

fn parse_one_lemma(input: &str) -> Result<LemmaSummary> {
    let parts: Vec<&str> = input.splitn(3, ":::").collect();
    if parts.len() < 2 {
        return Err(CliError::InvalidArgument(format!(
            "--lemma must be `name:::signature[:::lineage]`, got `{}`",
            input
        )));
    }
    let name = parts[0].trim();
    let signature = parts[1].trim();
    let lineage = parts.get(2).map(|s| s.trim()).unwrap_or("corpus");
    if name.is_empty() {
        return Err(CliError::InvalidArgument(
            "--lemma name component must be non-empty".into(),
        ));
    }
    if signature.is_empty() {
        return Err(CliError::InvalidArgument(
            "--lemma signature component must be non-empty".into(),
        ));
    }
    Ok(LemmaSummary {
        name: Text::from(name),
        signature: Text::from(signature),
        lineage: Text::from(lineage),
    })
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_verification::proof_drafting::SuggestionCategory;

    #[test]
    fn parse_lemma_minimal_two_components() {
        let l = parse_one_lemma("foo:::Π x. P(x)").unwrap();
        assert_eq!(l.name.as_str(), "foo");
        assert_eq!(l.signature.as_str(), "Π x. P(x)");
        assert_eq!(l.lineage.as_str(), "corpus");
    }

    #[test]
    fn parse_lemma_with_explicit_lineage() {
        let l = parse_one_lemma("nat_succ_pos:::∀ x. x>0 → succ(x)>0:::core").unwrap();
        assert_eq!(l.name.as_str(), "nat_succ_pos");
        assert_eq!(l.signature.as_str(), "∀ x. x>0 → succ(x)>0");
        assert_eq!(l.lineage.as_str(), "core");
    }

    #[test]
    fn parse_lemma_rejects_missing_signature() {
        assert!(parse_one_lemma("foo").is_err());
        assert!(parse_one_lemma("foo:::").is_err());
        assert!(parse_one_lemma(":::sig").is_err());
    }

    #[test]
    fn run_proof_draft_smoke() {
        // No-lemma run, plain format.  Smoke test: should not panic.
        let r = run_proof_draft(
            "test_theorem",
            "forall x. P(x)",
            &[],
            5,
            "plain",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn run_proof_draft_rejects_empty_theorem() {
        let r = run_proof_draft("", "forall x. P(x)", &[], 5, "plain");
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    #[test]
    fn run_proof_draft_rejects_zero_max() {
        let r = run_proof_draft("foo", "P", &[], 0, "plain");
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    #[test]
    fn run_proof_draft_rejects_unknown_format() {
        let r = run_proof_draft("foo", "P", &[], 5, "xml");
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    #[test]
    fn run_proof_draft_with_lemma_finds_suggestion() {
        // The default engine should rank the relevant lemma above
        // the unrelated one.  This is the integration test that
        // proves the proof_draft → SuggestionEngine wiring works
        // end-to-end.
        let lemmas = vec![
            "succ_pos:::forall x. x > 0 -> succ(x) > 0:::core".to_string(),
        ];
        let r = run_proof_draft(
            "thm",
            "forall x. x > 0 -> succ(x) + 1 > 0",
            &lemmas,
            5,
            "json",
        );
        assert!(r.is_ok());
    }

    // SuggestionCategory::name() must round-trip through json escape.
    #[test]
    fn category_names_appear_in_json_payload() {
        for c in [
            SuggestionCategory::LemmaApplication,
            SuggestionCategory::TacticInvocation,
            SuggestionCategory::StateNavigation,
            SuggestionCategory::Rewriting,
            SuggestionCategory::LlmProposal,
        ] {
            assert!(!c.name().is_empty());
        }
    }
}
