//! `verum smt-stats` — show verification routing telemetry.
//!
//! Reads statistics from the session-local stats cache (written during the
//! most recent `verum build`, `verum check`, or `verum verify` run) and
//! prints either a human-readable report or machine-readable JSON.
//!
//! The stats cache lives at `$VERUM_STATE_DIR/smt-stats.json` (defaults to
//! `~/.verum/state/smt-stats.json` or the project's `.verum/state/` directory).

use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};

/// Name of the on-disk stats file.
const STATS_FILE_NAME: &str = "smt-stats.json";

/// Execute the `smt-stats` command.
pub fn execute(json: bool, reset: bool) -> Result<()> {
    let stats_path = find_stats_file();

    if !stats_path.exists() {
        if json {
            println!("{}", serde_json::json!({
                "status": "no_data",
                "message": "no verification statistics found. Run `verum build` or `verum verify` first.",
            }));
        } else {
            println!();
            println!(
                "  {} No verification statistics available yet.",
                "ℹ".blue().bold()
            );
            println!(
                "  {} Run a build with verification enabled: {}",
                "→".dimmed(),
                "verum build --verify formal --smt-stats".cyan()
            );
            println!();
        }
        return Ok(());
    }

    let raw = fs::read_to_string(&stats_path)
        .with_context(|| format!("failed to read {}", stats_path.display()))?;

    if json {
        // Pretty-print the JSON directly.
        let value: serde_json::Value = serde_json::from_str(&raw)
            .context("stats file is not valid JSON")?;
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        print_human_report(&raw, &stats_path)?;
    }

    if reset {
        fs::remove_file(&stats_path).ok();
        if !json {
            println!();
            println!("  {} Statistics reset.", "✓".green().bold());
        }
    }

    Ok(())
}

fn print_human_report(raw: &str, path: &Path) -> Result<()> {
    let json: serde_json::Value = serde_json::from_str(raw).context("invalid stats JSON")?;

    let total = json["total_queries"].as_u64().unwrap_or(0);
    let z3_only = json["routing"]["z3_only"].as_u64().unwrap_or(0);
    let cvc5_only = json["routing"]["cvc5_only"].as_u64().unwrap_or(0);
    let portfolio = json["routing"]["portfolio"].as_u64().unwrap_or(0);
    let cross = json["routing"]["cross_validate"].as_u64().unwrap_or(0);

    let sat = json["outcomes"]["sat"].as_u64().unwrap_or(0);
    let unsat = json["outcomes"]["unsat"].as_u64().unwrap_or(0);
    let unknown = json["outcomes"]["unknown"].as_u64().unwrap_or(0);
    let errors = json["outcomes"]["errors"].as_u64().unwrap_or(0);

    let cv_agreed = json["cross_validate"]["agreed"].as_u64().unwrap_or(0);
    let cv_diverged = json["cross_validate"]["diverged"].as_u64().unwrap_or(0);
    let cv_incomplete = json["cross_validate"]["incomplete"].as_u64().unwrap_or(0);

    let total_nanos = json["total_nanos"].as_u64().unwrap_or(0);
    let total_ms = total_nanos as f64 / 1_000_000.0;
    let avg_ms = if total > 0 { total_ms / total as f64 } else { 0.0 };

    println!();
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".cyan());
    println!("  {}", "Verum Formal Verification — Routing Statistics".bold().cyan());
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".cyan());
    println!();
    println!("  Source: {}", path.display().to_string().dimmed());
    println!();

    // Overview
    println!("  {}", "Overview".bold().underline());
    println!("    Total goals checked:     {}", total.to_string().bold());
    if total > 0 {
        println!("    Average time per goal:   {:.2} ms", avg_ms);
        println!("    Total verification time: {:.2} s", total_ms / 1000.0);
    }
    println!();

    // Outcomes
    println!("  {}", "Outcomes".bold().underline());
    println!("    {} {:<14} {}", "✓".green(), "SAT", sat.to_string().green());
    println!("    {} {:<14} {}", "✓".green(), "UNSAT", unsat.to_string().green());
    println!("    {} {:<14} {}", "?".yellow(), "Unknown", unknown.to_string().yellow());
    if errors > 0 {
        println!("    {} {:<14} {}", "✗".red(), "Errors", errors.to_string().red());
    }
    println!();

    // Strategy dispatch (anonymized — don't name specific solvers)
    if total > 0 {
        println!("  {}", "Strategy dispatch".bold().underline());
        let primary_count = z3_only + cvc5_only;
        let pct = |n: u64| format!("({:>5.1}%)", 100.0 * n as f64 / total as f64);
        println!("    Single-engine routing:   {:>6}  {}", primary_count, pct(primary_count));
        println!("    Parallel (thorough):     {:>6}  {}", portfolio, pct(portfolio));
        println!("    Cross-validated:         {:>6}  {}", cross, pct(cross));
        println!();
    }

    // Cross-validation health
    if cv_agreed + cv_diverged + cv_incomplete > 0 {
        println!("  {}", "Cross-validation health".bold().underline());
        println!("    {} Agreements:     {}", "✓".green(), cv_agreed);
        if cv_diverged > 0 {
            println!(
                "    {} Divergences:    {}  {}",
                "⚠".red().bold(),
                cv_diverged.to_string().red().bold(),
                "INVESTIGATE — indicates potential bug".red()
            );
        } else {
            println!("    ✓ Divergences:    0  {}", "(healthy)".green().dimmed());
        }
        if cv_incomplete > 0 {
            println!("    ? Incomplete:     {}  {}", cv_incomplete, "(timeouts)".yellow().dimmed());
        }
        println!();
    }

    // Per-theory breakdown
    if let Some(theories) = json["per_theory"].as_object() {
        if !theories.is_empty() {
            println!("  {}", "Per-theory breakdown".bold().underline());
            let mut rows: Vec<_> = theories.iter().collect();
            rows.sort_by_key(|(_, v)| {
                let total = v["z3_only"].as_u64().unwrap_or(0)
                    + v["cvc5_only"].as_u64().unwrap_or(0)
                    + v["portfolio"].as_u64().unwrap_or(0)
                    + v["cross_validate"].as_u64().unwrap_or(0);
                std::cmp::Reverse(total)
            });

            for (theory, stats) in rows.iter().take(10) {
                let total = stats["z3_only"].as_u64().unwrap_or(0)
                    + stats["cvc5_only"].as_u64().unwrap_or(0)
                    + stats["portfolio"].as_u64().unwrap_or(0)
                    + stats["cross_validate"].as_u64().unwrap_or(0);
                let definitive = stats["definitive"].as_u64().unwrap_or(0);
                let failed = stats["failed"].as_u64().unwrap_or(0);
                let success_rate = if definitive + failed > 0 {
                    100.0 * definitive as f64 / (definitive + failed) as f64
                } else {
                    0.0
                };
                let total_nanos = stats["total_nanos"].as_u64().unwrap_or(0);
                let avg_ms = if total > 0 {
                    total_nanos as f64 / 1_000_000.0 / total as f64
                } else {
                    0.0
                };
                println!(
                    "    {:<8} {:>6} goals,  {:>5.1}% solved,  {:>7.2} ms avg",
                    theory.cyan(),
                    total,
                    success_rate,
                    avg_ms,
                );
            }
            println!();
        }
    }

    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".cyan());
    println!();

    Ok(())
}

/// Find the stats file using this search order:
///
/// 1. `VERUM_STATS_FILE` environment variable (explicit override)
/// 2. `$VERUM_STATE_DIR/smt-stats.json`
/// 3. `<project-root>/.verum/state/smt-stats.json` (if in a Verum project)
/// 4. `~/.verum/state/smt-stats.json` (user-global fallback)
fn find_stats_file() -> PathBuf {
    if let Ok(explicit) = std::env::var("VERUM_STATS_FILE") {
        return PathBuf::from(explicit);
    }
    if let Ok(state_dir) = std::env::var("VERUM_STATE_DIR") {
        return PathBuf::from(state_dir).join(STATS_FILE_NAME);
    }
    // Project-local state.
    if let Ok(cwd) = std::env::current_dir() {
        let project_state = cwd.join(".verum").join("state").join(STATS_FILE_NAME);
        if project_state.exists() {
            return project_state;
        }
    }
    // User-global fallback.
    if let Some(home) = dirs::home_dir() {
        return home.join(".verum").join("state").join(STATS_FILE_NAME);
    }
    // Last resort.
    PathBuf::from(STATS_FILE_NAME)
}

/// Write `RoutingStats` to the on-disk state file.
///
/// Called by the compiler at the end of a verification session so that
/// `verum smt-stats` can retrieve the data in a later CLI invocation.
pub fn persist_stats(stats_json: &serde_json::Value) -> Result<()> {
    let path = find_stats_file();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create stats dir {}", parent.display()))?;
    }
    let contents = serde_json::to_string_pretty(stats_json)?;
    fs::write(&path, contents)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}
