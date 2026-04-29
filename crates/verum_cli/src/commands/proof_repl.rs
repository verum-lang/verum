//! `verum proof-repl` subcommand — non-interactive batch driver
//! for the proof REPL state machine.  Interactive TUI is a future
//! v1; this surface is what tests / IDE integrations / CI scripts
//! consume today.

use crate::error::{CliError, Result};
use std::path::PathBuf;
use verum_common::Text;
use verum_verification::proof_drafting::LemmaSummary;
use verum_verification::proof_repl::{
    run_batch, summarise, DefaultReplSession, ReplCommand, ReplResponse, ReplSession,
};

fn parse_lemmas(flags: &[String]) -> Result<Vec<LemmaSummary>> {
    let mut out = Vec::new();
    for raw in flags {
        let parts: Vec<&str> = raw.splitn(3, ":::").collect();
        if parts.len() < 2 {
            return Err(CliError::InvalidArgument(format!(
                "--lemma must be `name:::signature[:::lineage]`, got `{}`",
                raw
            )));
        }
        let name = parts[0].trim();
        let sig = parts[1].trim();
        let lineage = parts.get(2).map(|s| s.trim()).unwrap_or("repl");
        if name.is_empty() || sig.is_empty() {
            return Err(CliError::InvalidArgument(
                "--lemma name + signature must be non-empty".into(),
            ));
        }
        out.push(LemmaSummary {
            name: Text::from(name),
            signature: Text::from(sig),
            lineage: Text::from(lineage),
        });
    }
    Ok(out)
}

/// Parse a single command line from the batch script.  One command
/// per line; blank lines and `#`-comments are skipped.
fn parse_command_line(line: &str) -> Result<Option<ReplCommand>> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    if line == "undo" {
        return Ok(Some(ReplCommand::Undo));
    }
    if line == "redo" {
        return Ok(Some(ReplCommand::Redo));
    }
    if line == "show-goals" {
        return Ok(Some(ReplCommand::ShowGoals));
    }
    if line == "show-context" {
        return Ok(Some(ReplCommand::ShowContext));
    }
    if line == "visualise" || line == "visualize" {
        return Ok(Some(ReplCommand::Visualise));
    }
    if line == "status" {
        return Ok(Some(ReplCommand::Status));
    }
    if let Some(rest) = line.strip_prefix("hint") {
        let rest = rest.trim();
        let max: usize = if rest.is_empty() {
            5
        } else {
            rest.parse().map_err(|_| {
                CliError::InvalidArgument(format!(
                    "hint must be `hint [N]`, got `{}`",
                    line
                ))
            })?
        };
        return Ok(Some(ReplCommand::Hint { max }));
    }
    // Both `apply <tactic>` and a bare `<tactic>` line route to
    // `ReplCommand::Apply { tactic }`.  The tactic string is
    // preserved verbatim — the kernel checker expects e.g.
    // "apply foo_lemma" (with prefix) for lemma application, or
    // bare keywords like "intro" / "auto" for canonical tactics.
    if line == "apply" {
        return Err(CliError::InvalidArgument(
            "apply: tactic must be non-empty".into(),
        ));
    }
    Ok(Some(ReplCommand::Apply {
        tactic: Text::from(line),
    }))
}

fn parse_command_script(content: &str) -> Result<Vec<ReplCommand>> {
    let mut out = Vec::new();
    for (i, line) in content.lines().enumerate() {
        match parse_command_line(line) {
            Ok(Some(cmd)) => out.push(cmd),
            Ok(None) => {}
            Err(CliError::InvalidArgument(msg)) => {
                return Err(CliError::InvalidArgument(format!(
                    "line {}: {}",
                    i + 1,
                    msg
                )));
            }
            Err(other) => return Err(other),
        }
    }
    Ok(out)
}

fn validate_format(s: &str) -> Result<()> {
    if s != "plain" && s != "json" {
        return Err(CliError::InvalidArgument(format!(
            "--format must be 'plain' or 'json', got '{}'",
            s
        )));
    }
    Ok(())
}

// =============================================================================
// run_batch_cli
// =============================================================================

#[allow(clippy::too_many_arguments)]
pub fn run_batch_cli(
    theorem: &str,
    goal: &str,
    lemmas: &[String],
    commands_file: Option<&PathBuf>,
    inline_commands: &[String],
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
    let mut session = DefaultReplSession::new(theorem, goal, parsed_lemmas);

    // Compose the command sequence: file content first, then
    // inline `--cmd` repeats (ordered as on the CLI).
    let mut script = String::new();
    if let Some(path) = commands_file {
        let body = std::fs::read_to_string(path).map_err(|e| {
            CliError::VerificationFailed(format!(
                "reading {}: {}",
                path.display(),
                e
            ))
        })?;
        script.push_str(&body);
        script.push('\n');
    }
    for line in inline_commands {
        script.push_str(line);
        script.push('\n');
    }
    let commands = parse_command_script(&script)?;
    let responses = run_batch(&mut session, commands);

    match format {
        "plain" => emit_responses_plain(&responses, &session),
        "json" => emit_responses_json(&responses, &session),
        _ => unreachable!(),
    }

    // Non-zero exit on any rejection.
    for r in &responses {
        if let ReplResponse::Rejected { reason, tactic, .. } = r {
            return Err(CliError::VerificationFailed(format!(
                "REPL rejected `{}`: {}",
                tactic.as_str(),
                reason.as_str()
            )));
        }
    }
    Ok(())
}

fn emit_responses_plain(responses: &[ReplResponse], session: &DefaultReplSession) {
    println!(
        "REPL transcript ({} command(s) executed):",
        responses.len()
    );
    println!();
    for (i, r) in responses.iter().enumerate() {
        match r {
            ReplResponse::Accepted {
                tactic,
                elapsed_ms,
                snapshot,
            } => {
                println!(
                    "  [{:>3}] ✓ apply  {}  ({}ms)  history={}",
                    i + 1,
                    tactic.as_str(),
                    elapsed_ms,
                    snapshot.history_depth
                );
            }
            ReplResponse::Rejected {
                tactic, reason, ..
            } => {
                println!(
                    "  [{:>3}] ✗ apply  {}  → {}",
                    i + 1,
                    tactic.as_str(),
                    reason.as_str()
                );
            }
            ReplResponse::Undone { popped, snapshot } => {
                println!(
                    "  [{:>3}] ↶ undo  (popped: {})  history={}  redo={}",
                    i + 1,
                    popped.as_str(),
                    snapshot.history_depth,
                    snapshot.redo_depth
                );
            }
            ReplResponse::Redone {
                reapplied,
                snapshot,
            } => {
                println!(
                    "  [{:>3}] ↷ redo  (re-applied: {})  history={}",
                    i + 1,
                    reapplied.as_str(),
                    snapshot.history_depth
                );
            }
            ReplResponse::Status { snapshot } => {
                println!(
                    "  [{:>3}] status  history={}  redo={}  open_goals={}",
                    i + 1,
                    snapshot.history_depth,
                    snapshot.redo_depth,
                    snapshot.open_goals.len()
                );
            }
            ReplResponse::Hints { suggestions } => {
                println!(
                    "  [{:>3}] hint   {} suggestion(s):",
                    i + 1,
                    suggestions.len()
                );
                for (j, s) in suggestions.iter().enumerate() {
                    println!(
                        "         {}. [{:.2}|{}] {}",
                        j + 1,
                        s.score,
                        s.category.as_str(),
                        s.snippet.as_str()
                    );
                }
            }
            ReplResponse::Tree { dot } => {
                println!("  [{:>3}] visualise:", i + 1);
                for line in dot.as_str().lines() {
                    println!("         {}", line);
                }
            }
            ReplResponse::NoOp { reason } => {
                println!("  [{:>3}] noop   {}", i + 1, reason.as_str());
            }
            ReplResponse::Error { message } => {
                println!("  [{:>3}] error  {}", i + 1, message.as_str());
            }
        }
    }
    println!();
    let summary = summarise(responses);
    println!("Summary:");
    for (k, v) in &summary {
        println!("  {:<10} {:>4}", k, v);
    }
    let final_snap = session.snapshot();
    println!();
    println!("Final state:");
    println!("  history_depth : {}", final_snap.history_depth);
    println!("  redo_depth    : {}", final_snap.redo_depth);
    println!("  applied_steps : {} step(s)", final_snap.applied_steps.len());
}

fn emit_responses_json(responses: &[ReplResponse], session: &DefaultReplSession) {
    let mut out = String::from("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!("  \"count\": {},\n", responses.len()));
    out.push_str("  \"responses\": [\n");
    for (i, r) in responses.iter().enumerate() {
        let body = serde_json::to_string(r).unwrap_or_default();
        out.push_str(&format!(
            "    {}{}",
            body,
            if i + 1 < responses.len() { ",\n" } else { "\n" }
        ));
    }
    out.push_str("  ],\n");
    let summary = summarise(responses);
    out.push_str("  \"summary\": {");
    let entries: Vec<String> = summary
        .iter()
        .map(|(k, v)| format!("\"{}\": {}", k, v))
        .collect();
    out.push_str(&entries.join(", "));
    out.push_str("},\n");
    let snap = session.snapshot();
    let snap_json = serde_json::to_string(&snap).unwrap_or_default();
    out.push_str(&format!("  \"final_state\": {}\n", snap_json));
    out.push('}');
    println!("{}", out);
}

// =============================================================================
// run_tree_cli — emit proof-tree DOT after applying steps
// =============================================================================

pub fn run_tree_cli(
    theorem: &str,
    goal: &str,
    lemmas: &[String],
    apply: &[String],
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
    let parsed_lemmas = parse_lemmas(lemmas)?;
    let mut session = DefaultReplSession::new(theorem, goal, parsed_lemmas);
    for tactic in apply {
        let r = session.step(ReplCommand::Apply {
            tactic: Text::from(tactic.as_str()),
        });
        if let ReplResponse::Rejected { reason, .. } = r {
            return Err(CliError::VerificationFailed(format!(
                "tree: kernel rejected `{}`: {}",
                tactic,
                reason.as_str()
            )));
        }
    }
    let dot = session.proof_tree().to_dot();
    println!("{}", dot.as_str());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp(content: &str) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    // ----- parse_lemmas / parse_command_line -----

    #[test]
    fn parse_lemmas_minimal() {
        let l = parse_lemmas(&["foo:::P".into()]).unwrap();
        assert_eq!(l[0].name.as_str(), "foo");
        assert_eq!(l[0].signature.as_str(), "P");
    }

    #[test]
    fn parse_lemmas_with_lineage() {
        let l = parse_lemmas(&["foo:::P:::corpus".into()]).unwrap();
        assert_eq!(l[0].lineage.as_str(), "corpus");
    }

    #[test]
    fn parse_lemmas_rejects_malformed() {
        assert!(parse_lemmas(&["bare".into()]).is_err());
        assert!(parse_lemmas(&[":::sig".into()]).is_err());
    }

    #[test]
    fn parse_command_line_keywords() {
        for (input, expected) in &[
            ("undo", ReplCommand::Undo),
            ("redo", ReplCommand::Redo),
            ("show-goals", ReplCommand::ShowGoals),
            ("show-context", ReplCommand::ShowContext),
            ("visualise", ReplCommand::Visualise),
            ("status", ReplCommand::Status),
        ] {
            assert_eq!(parse_command_line(input).unwrap().unwrap(), *expected);
        }
    }

    #[test]
    fn parse_command_line_apply_explicit() {
        // `apply X` is preserved verbatim so the kernel checker
        // recognises it as the `apply NAME` shape (which validates
        // `X` against in-scope lemmas).
        let cmd = parse_command_line("apply foo_lemma").unwrap().unwrap();
        match cmd {
            ReplCommand::Apply { tactic } => {
                assert_eq!(tactic.as_str(), "apply foo_lemma")
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_command_line_apply_implicit() {
        // Bare `intro` should be treated as `apply intro`.
        let cmd = parse_command_line("intro").unwrap().unwrap();
        match cmd {
            ReplCommand::Apply { tactic } => assert_eq!(tactic.as_str(), "intro"),
            _ => panic!(),
        }
    }

    #[test]
    fn parse_command_line_hint_optional_count() {
        let cmd = parse_command_line("hint").unwrap().unwrap();
        match cmd {
            ReplCommand::Hint { max } => assert_eq!(max, 5),
            _ => panic!(),
        }
        let cmd = parse_command_line("hint 3").unwrap().unwrap();
        match cmd {
            ReplCommand::Hint { max } => assert_eq!(max, 3),
            _ => panic!(),
        }
    }

    #[test]
    fn parse_command_line_skips_blank_and_comments() {
        assert_eq!(parse_command_line("").unwrap(), None);
        assert_eq!(parse_command_line("   ").unwrap(), None);
        assert_eq!(parse_command_line("# comment").unwrap(), None);
    }

    #[test]
    fn parse_command_script_carries_line_number_in_error() {
        let script = "intro\nhint not-a-number\nauto\n";
        let err = parse_command_script(script).unwrap_err();
        match err {
            CliError::InvalidArgument(msg) => {
                assert!(msg.contains("line 2"), "msg={}", msg);
            }
            _ => panic!(),
        }
    }

    // ----- run_batch_cli -----

    #[test]
    fn run_batch_cli_validates_inputs() {
        assert!(matches!(
            run_batch_cli("", "P", &[], None, &[], "plain"),
            Err(CliError::InvalidArgument(_))
        ));
        assert!(matches!(
            run_batch_cli("t", "", &[], None, &[], "plain"),
            Err(CliError::InvalidArgument(_))
        ));
        assert!(matches!(
            run_batch_cli("t", "P", &[], None, &[], "yaml"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    #[test]
    fn run_batch_cli_inline_commands_smoke() {
        let r = run_batch_cli(
            "thm",
            "P",
            &[],
            None,
            &["intro".into(), "auto".into()],
            "plain",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn run_batch_cli_rejection_returns_err() {
        let r = run_batch_cli(
            "thm",
            "P",
            &[],
            None,
            &["xyz_garbage".into()],
            "plain",
        );
        assert!(matches!(r, Err(CliError::VerificationFailed(_))));
    }

    #[test]
    fn run_batch_cli_reads_file() {
        let f = write_temp("intro\nauto\nstatus\n");
        let r = run_batch_cli(
            "thm",
            "P",
            &[],
            Some(&f.path().to_path_buf()),
            &[],
            "plain",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn run_batch_cli_combines_file_and_inline() {
        let f = write_temp("intro\n");
        let r = run_batch_cli(
            "thm",
            "P",
            &[],
            Some(&f.path().to_path_buf()),
            &["status".into()],
            "json",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn run_batch_cli_apply_lemma_with_in_scope_target() {
        let r = run_batch_cli(
            "thm",
            "P",
            &["foo_lemma:::P".into()],
            None,
            &["apply foo_lemma".into()],
            "plain",
        );
        assert!(r.is_ok());
    }

    // ----- run_tree_cli -----

    #[test]
    fn run_tree_cli_emits_dot() {
        // Sink stdout via capturing isn't trivial in unit tests;
        // we just verify the function returns Ok on a happy path.
        let r = run_tree_cli(
            "thm",
            "P",
            &[],
            &["intro".into(), "auto".into()],
        );
        assert!(r.is_ok());
    }

    #[test]
    fn run_tree_cli_rejection_returns_err() {
        let r = run_tree_cli("thm", "P", &[], &["xyz_garbage".into()]);
        assert!(matches!(r, Err(CliError::VerificationFailed(_))));
    }

    #[test]
    fn run_tree_cli_validates_inputs() {
        assert!(matches!(
            run_tree_cli("", "P", &[], &[]),
            Err(CliError::InvalidArgument(_))
        ));
        assert!(matches!(
            run_tree_cli("t", "", &[], &[]),
            Err(CliError::InvalidArgument(_))
        ));
    }
}
