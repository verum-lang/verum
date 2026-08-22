//! `verum diff-tiers` — the tier-identity judge (confirmed direction
//! «идея 1», 2026-08-22).
//!
//! One bytecode, two executions is Verum's core promise; this command
//! makes the promise CONTINUOUSLY checkable instead of point-fixed:
//! run the same program under Tier 0 (interpreter) and Tier 1 (AOT),
//! compare observable behaviour, and refuse to call different answers
//! anything but a defect. The checked_add divergence (T0846-3b: one
//! program, `end=16` vs `end=-1`) is the class this institutionalises.
//!
//! Each tier runs as a SUBPROCESS of the current `verum` binary —
//! a SIGSEGV or abort in one tier is a recorded verdict, not a dead
//! judge, and the judged path is exactly the path users run.

use anyhow::{Context, Result};
use serde::Serialize;
use std::process::Command;

/// One tier's observed behaviour.
#[derive(Debug, Serialize)]
pub struct TierObservation {
    /// Exit code, or the signal rendering when killed.
    pub exit: String,
    /// Captured stdout (the comparison surface).
    pub stdout: String,
    /// Captured stderr — reported only on divergence, but always
    /// recorded (a tier that panics tells its story here).
    pub stderr: String,
}

/// The judge's report. JSON schema is append-only (agent contract).
#[derive(Debug, Serialize)]
pub struct DiffTiersReport {
    /// The program under judgment.
    pub file: String,
    /// Tier 0 (interpreter) observation.
    pub tier0: TierObservation,
    /// Tier 1 (AOT) observation.
    pub tier1: TierObservation,
    /// `identical` or `DIVERGENT`.
    pub verdict: String,
    /// First stdout line where the tiers disagree (1-based), if any.
    pub first_divergence_line: Option<usize>,
}

fn run_tier(exe: &std::path::Path, tier: &str, file: &std::path::Path) -> Result<TierObservation> {
    let out = Command::new(exe)
        .args(["run", "--tier", tier])
        .arg(file)
        .output()
        .with_context(|| format!("spawning tier {tier}"))?;
    let exit = match out.status.code() {
        Some(c) => c.to_string(),
        None => format!("signal:{:?}", out.status),
    };
    Ok(TierObservation {
        exit,
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Strip the runner's own banner lines (`Running …`, `Compiling …`,
/// `Finished …`, `Binary …`) so the comparison surface is the
/// PROGRAM's output, not the toolchain's progress narration — which
/// legitimately differs between tiers.
fn program_stdout(raw: &str) -> Vec<String> {
    raw.lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("Running ")
                || t.starts_with("Compiling ")
                || t.starts_with("Building ")
                || t.starts_with("Finished ")
                || t.starts_with("Binary "))
        })
        .map(str::to_string)
        .collect()
}

pub fn execute(file: &std::path::Path, json: bool) -> Result<()> {
    let exe = std::env::current_exe().context("locating the verum binary")?;
    let tier0 = run_tier(&exe, "interpret", file)?;
    let tier1 = run_tier(&exe, "aot", file)?;

    let t0_lines = program_stdout(&tier0.stdout);
    let t1_lines = program_stdout(&tier1.stdout);

    let mut first_divergence_line = None;
    let max = t0_lines.len().max(t1_lines.len());
    for i in 0..max {
        if t0_lines.get(i) != t1_lines.get(i) {
            first_divergence_line = Some(i + 1);
            break;
        }
    }
    let exits_agree = tier0.exit == tier1.exit;
    let identical = exits_agree && first_divergence_line.is_none();

    let report = DiffTiersReport {
        file: file.display().to_string(),
        tier0,
        tier1,
        verdict: if identical {
            "identical".to_string()
        } else {
            "DIVERGENT".to_string()
        },
        first_divergence_line,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("tier-identity judgment for {}", report.file);
        println!("  tier0 exit {}, tier1 exit {}", report.tier0.exit, report.tier1.exit);
        if identical {
            println!("  verdict: identical — one program, one answer");
        } else {
            println!("  verdict: DIVERGENT");
            if let Some(line) = report.first_divergence_line {
                let idx = line - 1;
                println!("  first stdout divergence at program line {line}:");
                println!("    tier0: {:?}", t0_lines.get(idx));
                println!("    tier1: {:?}", t1_lines.get(idx));
            }
            if !exits_agree {
                println!("  exit codes disagree");
            }
            let show = |name: &str, s: &str| {
                let tail: Vec<&str> = s.lines().rev().take(4).collect();
                if !tail.is_empty() {
                    println!("  {name} stderr tail:");
                    for l in tail.iter().rev() {
                        println!("    {l}");
                    }
                }
            };
            show("tier0", &report.tier0.stderr);
            show("tier1", &report.tier1.stderr);
        }
    }

    if identical {
        Ok(())
    } else {
        // Non-zero exit: CI treats divergence as failure by default.
        std::process::exit(3);
    }
}
