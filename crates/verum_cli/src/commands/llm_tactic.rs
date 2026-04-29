//! `verum llm-tactic` subcommand — LCF-style fail-closed LLM
//! proof-proposer.  Wires the LLM tactic adapter + kernel checker +
//! audit trail into a typed CLI surface.

use crate::error::{CliError, Result};
use std::path::PathBuf;
use verum_common::Text;
use verum_verification::llm_tactic::{
    AuditTrail, EchoLlmAdapter, FilesystemAuditTrail, KernelGate, KernelVerdict,
    LlmGoalSummary, LlmTacticAdapter, MemoryAuditTrail, MockLlmAdapter,
    PatternKernelChecker,
};

/// Default audit-trail path (relative to the current project's
/// target directory).
pub const DEFAULT_AUDIT_PATH: &str = "target/.verum_cache/llm-proofs.jsonl";

fn validate_format(s: &str) -> Result<()> {
    if s != "plain" && s != "json" {
        return Err(CliError::InvalidArgument(format!(
            "--format must be 'plain' or 'json', got '{}'",
            s
        )));
    }
    Ok(())
}

fn resolve_audit_path(override_path: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(p.clone());
    }
    let manifest_dir = crate::config::Manifest::find_manifest_dir()?;
    Ok(manifest_dir.join(DEFAULT_AUDIT_PATH))
}

fn parse_lemmas(flags: &[String]) -> Result<Vec<(Text, Text)>> {
    let mut out = Vec::new();
    for raw in flags {
        let parts: Vec<&str> = raw.splitn(2, ":::").collect();
        if parts.len() != 2 {
            return Err(CliError::InvalidArgument(format!(
                "--lemma must be `name:::signature`, got `{}`",
                raw
            )));
        }
        let name = parts[0].trim();
        let sig = parts[1].trim();
        if name.is_empty() || sig.is_empty() {
            return Err(CliError::InvalidArgument(
                "--lemma name + signature must be non-empty".into(),
            ));
        }
        out.push((Text::from(name), Text::from(sig)));
    }
    Ok(out)
}

fn parse_hyps(flags: &[String]) -> Result<Vec<(Text, Text)>> {
    let mut out = Vec::new();
    for raw in flags {
        let parts: Vec<&str> = raw.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(CliError::InvalidArgument(format!(
                "--hyp must be `name:type`, got `{}`",
                raw
            )));
        }
        let name = parts[0].trim();
        let ty = parts[1].trim();
        if name.is_empty() || ty.is_empty() {
            return Err(CliError::InvalidArgument(
                "--hyp name + type must be non-empty".into(),
            ));
        }
        out.push((Text::from(name), Text::from(ty)));
    }
    Ok(out)
}

// =============================================================================
// run_propose
// =============================================================================

#[allow(clippy::too_many_arguments)]
pub fn run_propose(
    theorem: &str,
    goal: &str,
    lemmas: &[String],
    hyps: &[String],
    history: &[String],
    model: &str,
    hint: Option<&str>,
    audit_path: Option<&PathBuf>,
    persist: bool,
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
    validate_format(format)?;

    let parsed_lemmas = parse_lemmas(lemmas)?;
    let parsed_hyps = parse_hyps(hyps)?;
    let history_steps: Vec<Text> = history.iter().map(|s| Text::from(s.as_str())).collect();

    let mut summary = LlmGoalSummary::new(theorem, goal);
    summary.lemmas_in_scope = parsed_lemmas;
    summary.hypotheses = parsed_hyps;
    summary.recent_tactic_history = history_steps;

    // Pick adapter: hint → EchoLlmAdapter; else MockLlmAdapter.
    // Production cloud / on-device adapters land via the same
    // trait without changing this dispatch.
    let result = match hint {
        Some(text) => {
            let adapter = EchoLlmAdapter::new(model, text);
            run_with(&adapter, &summary, audit_path, persist, format)
        }
        None => {
            // No hint → MockLlmAdapter with a default safe sequence.
            let adapter = MockLlmAdapter::new(model, vec!["intro", "auto"]);
            run_with(&adapter, &summary, audit_path, persist, format)
        }
    };
    result
}

fn run_with<A: LlmTacticAdapter>(
    adapter: &A,
    goal: &LlmGoalSummary,
    audit_path: Option<&PathBuf>,
    persist: bool,
    format: &str,
) -> Result<()> {
    let checker = PatternKernelChecker::new();
    let gate = KernelGate::new();

    let verdict = if persist {
        let resolved = resolve_audit_path(audit_path)?;
        let trail = FilesystemAuditTrail::new(&resolved).map_err(|e| {
            CliError::VerificationFailed(format!("audit trail open: {}", e))
        })?;
        gate.run(adapter, &checker, goal, &trail)
            .map_err(|e| CliError::VerificationFailed(format!("llm-tactic: {}", e)))?
    } else {
        let trail = MemoryAuditTrail::new();
        gate.run(adapter, &checker, goal, &trail)
            .map_err(|e| CliError::VerificationFailed(format!("llm-tactic: {}", e)))?
    };

    match format {
        "plain" => emit_verdict_plain(&verdict, adapter.model_id().as_str(), goal),
        "json" => emit_verdict_json(&verdict, adapter.model_id().as_str(), goal),
        _ => unreachable!(),
    }

    if !verdict.is_accepted() {
        return Err(CliError::VerificationFailed(
            "kernel rejected the LLM proposal".to_string(),
        ));
    }
    Ok(())
}

fn emit_verdict_plain(v: &KernelVerdict, model_id: &str, goal: &LlmGoalSummary) {
    println!("Theorem      : {}", goal.theorem_name.as_str());
    println!("Goal         : {}", goal.focused_proposition.as_str());
    println!("Model        : {}", model_id);
    println!("Prompt hash  : {}", goal.prompt_hash().as_str());
    match v {
        KernelVerdict::Accepted { steps_checked } => {
            println!();
            println!("Verdict      : ACCEPTED ({} step(s) kernel-checked)", steps_checked);
        }
        KernelVerdict::Rejected {
            failed_step_index,
            reason,
        } => {
            println!();
            println!("Verdict      : REJECTED");
            println!("  failed at step #{}", failed_step_index + 1);
            println!("  reason : {}", reason.as_str());
        }
    }
}

fn emit_verdict_json(v: &KernelVerdict, model_id: &str, goal: &LlmGoalSummary) {
    let mut out = String::from("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!(
        "  \"theorem\": \"{}\",\n",
        json_escape(goal.theorem_name.as_str())
    ));
    out.push_str(&format!(
        "  \"goal\": \"{}\",\n",
        json_escape(goal.focused_proposition.as_str())
    ));
    out.push_str(&format!(
        "  \"model_id\": \"{}\",\n",
        json_escape(model_id)
    ));
    out.push_str(&format!(
        "  \"prompt_hash\": \"{}\",\n",
        goal.prompt_hash().as_str()
    ));
    out.push_str("  \"verdict\": ");
    match v {
        KernelVerdict::Accepted { steps_checked } => {
            out.push_str(&format!(
                "{{ \"status\": \"accepted\", \"steps_checked\": {} }}\n",
                steps_checked
            ));
        }
        KernelVerdict::Rejected {
            failed_step_index,
            reason,
        } => {
            out.push_str(&format!(
                "{{ \"status\": \"rejected\", \"failed_step_index\": {}, \"reason\": \"{}\" }}\n",
                failed_step_index,
                json_escape(reason.as_str())
            ));
        }
    }
    out.push('}');
    println!("{}", out);
}

// =============================================================================
// run_audit_trail
// =============================================================================

pub fn run_audit_trail(audit_path: Option<&PathBuf>, format: &str) -> Result<()> {
    validate_format(format)?;
    let resolved = resolve_audit_path(audit_path)?;
    let trail = FilesystemAuditTrail::new(&resolved).map_err(|e| {
        CliError::VerificationFailed(format!("audit trail open: {}", e))
    })?;
    let events = trail.read_all().map_err(|e| {
        CliError::VerificationFailed(format!("audit trail read: {}", e))
    })?;

    match format {
        "plain" => {
            if events.is_empty() {
                println!(
                    "Audit trail at {} is empty (no LLM tactic invocations recorded yet).",
                    resolved.display()
                );
            } else {
                println!("Audit trail: {} ({} event(s))", resolved.display(), events.len());
                for (i, e) in events.iter().enumerate() {
                    println!("  [{:>3}] {}", i + 1, e.name());
                }
            }
        }
        "json" => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!(
                "  \"path\": \"{}\",\n",
                json_escape(&resolved.display().to_string())
            ));
            out.push_str(&format!("  \"count\": {},\n", events.len()));
            out.push_str("  \"events\": [\n");
            for (i, e) in events.iter().enumerate() {
                let body = serde_json::to_string(e).unwrap_or_default();
                out.push_str(&format!(
                    "    {}{}",
                    body,
                    if i + 1 < events.len() { ",\n" } else { "\n" }
                ));
            }
            out.push_str("  ]\n}");
            println!("{}", out);
        }
        _ => unreachable!(),
    }
    Ok(())
}

// =============================================================================
// run_models
// =============================================================================

pub fn run_models(format: &str) -> Result<()> {
    validate_format(format)?;
    // V0 ships two reference adapters; production cloud / on-device
    // adapters land via the same trait surface.
    let entries: &[(&str, &str)] = &[
        (
            "mock",
            "Deterministic mock adapter — returns a canned tactic sequence.  Used for tests + golden-CI shape pinning.",
        ),
        (
            "echo",
            "Echo adapter — emits the user-supplied --hint text as the tactic sequence.  Useful when you have a pre-computed sequence and want the LCF-style kernel re-check loop without an actual model in the loop.",
        ),
    ];
    match format {
        "plain" => {
            println!("Available LLM tactic adapters (V0):");
            println!();
            for (name, desc) in entries {
                println!("  {}", name);
                println!("    {}", desc);
            }
            println!();
            println!("Production cloud / on-device adapters plug in via the same trait without CLI changes.");
        }
        "json" => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"count\": {},\n", entries.len()));
            out.push_str("  \"adapters\": [\n");
            for (i, (name, desc)) in entries.iter().enumerate() {
                out.push_str(&format!(
                    "    {{ \"name\": \"{}\", \"description\": \"{}\" }}{}\n",
                    json_escape(name),
                    json_escape(desc),
                    if i + 1 < entries.len() { "," } else { "" }
                ));
            }
            out.push_str("  ]\n}");
            println!("{}", out);
        }
        _ => unreachable!(),
    }
    Ok(())
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

    // ----- parse_lemmas / parse_hyps -----

    #[test]
    fn parse_lemmas_simple() {
        let out = parse_lemmas(&["foo:::Π x. P(x)".into()]).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0.as_str(), "foo");
        assert_eq!(out[0].1.as_str(), "Π x. P(x)");
    }

    #[test]
    fn parse_lemmas_rejects_malformed() {
        assert!(matches!(
            parse_lemmas(&["foo".into()]),
            Err(CliError::InvalidArgument(_))
        ));
        assert!(matches!(
            parse_lemmas(&[":::sig".into()]),
            Err(CliError::InvalidArgument(_))
        ));
    }

    #[test]
    fn parse_hyps_simple() {
        let out = parse_hyps(&["h:Int".into()]).unwrap();
        assert_eq!(out[0].0.as_str(), "h");
        assert_eq!(out[0].1.as_str(), "Int");
    }

    #[test]
    fn parse_hyps_rejects_malformed() {
        assert!(matches!(
            parse_hyps(&["nojcolon".into()]),
            Err(CliError::InvalidArgument(_))
        ));
    }

    // ----- validate_format -----

    #[test]
    fn validate_format_round_trip() {
        assert!(validate_format("plain").is_ok());
        assert!(validate_format("json").is_ok());
        assert!(matches!(
            validate_format("yaml"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    // ----- json_escape -----

    #[test]
    fn json_escape_handles_quotes_and_newlines() {
        assert_eq!(json_escape("a\"b\nc"), "a\\\"b\\nc");
    }

    // ----- run_propose validation -----

    #[test]
    fn run_propose_rejects_empty_theorem() {
        let r = run_propose("", "P", &[], &[], &[], "mock", None, None, false, "plain");
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    #[test]
    fn run_propose_rejects_empty_goal() {
        let r = run_propose("foo", "", &[], &[], &[], "mock", None, None, false, "plain");
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    #[test]
    fn run_propose_rejects_unknown_format() {
        let r = run_propose("foo", "P", &[], &[], &[], "mock", None, None, false, "yaml");
        assert!(matches!(r, Err(CliError::InvalidArgument(_))));
    }

    #[test]
    fn run_propose_mock_default_sequence_passes_kernel() {
        // Mock adapter ships `intro`+`auto` — both canonical, kernel
        // accepts.  --persist false keeps it in-memory.
        let r = run_propose(
            "thm",
            "True",
            &[],
            &[],
            &[],
            "mock",
            None,
            None,
            false,
            "plain",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn run_propose_with_hint_runs_echo_adapter() {
        let r = run_propose(
            "thm",
            "True",
            &[],
            &[],
            &[],
            "echo",
            Some("intro\nauto"),
            None,
            false,
            "plain",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn run_propose_kernel_rejects_bad_hint_returns_err() {
        let r = run_propose(
            "thm",
            "True",
            &[],
            &[],
            &[],
            "echo",
            Some("xyz_garbage_step"),
            None,
            false,
            "plain",
        );
        // Kernel rejection bubbles up as VerificationFailed.
        assert!(matches!(r, Err(CliError::VerificationFailed(_))));
    }

    #[test]
    fn run_propose_apply_with_in_scope_lemma_passes() {
        let r = run_propose(
            "thm",
            "True",
            &["foo_lemma:::P".into()],
            &[],
            &[],
            "echo",
            Some("apply foo_lemma"),
            None,
            false,
            "plain",
        );
        assert!(r.is_ok());
    }

    // ----- run_audit_trail / run_models -----

    #[test]
    fn run_audit_trail_missing_path_treated_as_empty() {
        // Use a tempdir-rooted path so we don't depend on a manifest.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("missing.jsonl");
        let r = run_audit_trail(Some(&p), "plain");
        assert!(r.is_ok());
    }

    #[test]
    fn run_models_lists_two_adapters() {
        assert!(run_models("plain").is_ok());
        assert!(run_models("json").is_ok());
    }
}
