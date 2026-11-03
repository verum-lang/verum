//! `verum arch` — the architecture query surface (T0848/T0849).
//!
//! The agent-facing half of inference-first ATS-V-2: one stable
//! question protocol over the SAME row inference the compiler phase
//! runs. `--json` is the machine contract (append-only fields);
//! the default rendering is the human contract.

use anyhow::{Context, Result};

/// `verum arch query` body — the subcommand enum lives in `main.rs`
/// alongside the pre-existing Explain/Catalog arms.
pub fn query(path: &std::path::Path, json: bool) -> Result<()> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let report = verum_compiler::arch_query::arch_query_source(&source)
        .with_context(|| format!("querying {}", path.display()))?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    // Human rendering: the four questions in reading order.
    match &report.module {
        Some(m) => println!("module {m}"),
        None => println!("module <unnamed>"),
    }
    println!("\ninferred surface ({}):", report.inferred.len());
    if report.inferred.is_empty() {
        println!("  (empty — no capability-relevant calls found)");
    }
    for a in &report.inferred {
        println!("  {}  [{}]", a.atom, a.evidence);
    }
    match &report.pinned {
        Some(p) => {
            println!("\npinned by @arch_module ({}):", p.len());
            for a in p {
                println!("  {a}");
            }
            let esc = report.escalations.as_deref().unwrap_or(&[]);
            let dead = report.dead_rights.as_deref().unwrap_or(&[]);
            use verum_compiler::arch_query::PinComparison;
            match report.comparison {
                PinComparison::Partial => println!("\ncomparison incomplete: the inferred surface has unresolved dependencies"),
                PinComparison::GroundMatch => println!("\nextracted capability facts match the pin"),
                _ => {}
            }
            if !esc.is_empty() {
                println!("\nESCALATIONS (inferred beyond the pin):");
                for a in esc {
                    println!("  {}  [{}]", a.atom, a.evidence);
                }
            }
            if !dead.is_empty() {
                println!("\nPINNED RIGHTS ABSENT FROM THE EXTRACTED FACTS:");
                for a in dead {
                    println!("  {a}");
                }
            }
        }
        None => println!("\npinned: (no @arch_module — surface is derived only)"),
    }
    if !report.unresolved_calls.is_empty() {
        println!(
            "\nunresolved local calls ({}) — surfaced, never guessed:",
            report.unresolved_calls.len()
        );
        for e in &report.unresolved_calls {
            println!("  {e}");
        }
    }
    Ok(())
}
