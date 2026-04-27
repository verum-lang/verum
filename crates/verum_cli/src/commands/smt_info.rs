//! `verum smt-info` — diagnose the formal verification stack.
//!
//! Shows:
//! - Linked SMT backends and their versions
//! - Advanced capability matrix (interpolation, synthesis, abduction, etc.)
//! - Recommendations for enabling additional features
//!
//! This command is a pure verifier diagnostic — it does not touch user code.

use anyhow::Result;
use colored::Colorize;

/// Execute the `smt-info` command.
pub fn execute(json: bool) -> Result<()> {
    if json {
        print_json()
    } else {
        print_human_readable()
    }
}

fn print_human_readable() -> Result<()> {
    let registry = verum_smt::solver_capability::CapabilityRegistry::detect();
    let cvc5_caps = verum_smt::cvc5_advanced::detect_capabilities();

    println!();
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".cyan());
    println!("  {} Formal Verification Engine", "Verum".bold().cyan());
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".cyan());
    println!();

    // Backend status
    println!("  {}", "Backends".bold().underline());
    let primary_status = if registry.z3_available {
        format!("{} {}", "✓".green().bold(), "available".green())
    } else {
        format!("{} {}", "✗".red().bold(), "unavailable".red())
    };
    let secondary_status = if registry.cvc5_available {
        format!("{} {}", "✓".green().bold(), "available".green())
    } else {
        format!("{} {}", "✗".yellow().bold(), "stub mode".yellow())
    };
    println!(
        "    Primary solver   {}  {}",
        primary_status,
        registry.z3_version.as_deref().unwrap_or("").dimmed(),
    );
    println!(
        "    Secondary solver {}  {}",
        secondary_status,
        registry.cvc5_version.as_deref().unwrap_or("(not linked)").dimmed(),
    );
    println!();

    // Capabilities matrix — suppress backend names from output
    println!("  {}", "Capabilities".bold().underline());
    for cap in [
        verum_smt::solver_capability::SolverCapability::Interpolation,
        verum_smt::solver_capability::SolverCapability::Optimization,
        verum_smt::solver_capability::SolverCapability::HornClauses,
        verum_smt::solver_capability::SolverCapability::SygusSynthesis,
        verum_smt::solver_capability::SolverCapability::Abduction,
        verum_smt::solver_capability::SolverCapability::FiniteModelFinding,
        verum_smt::solver_capability::SolverCapability::StringsRegex,
        verum_smt::solver_capability::SolverCapability::Sequences,
        verum_smt::solver_capability::SolverCapability::CadNonlinearReal,
        verum_smt::solver_capability::SolverCapability::QuantifierElimination,
        verum_smt::solver_capability::SolverCapability::ProofProduction,
        verum_smt::solver_capability::SolverCapability::UnsatCores,
        verum_smt::solver_capability::SolverCapability::ModelExtraction,
        verum_smt::solver_capability::SolverCapability::IncrementalSolving,
        verum_smt::solver_capability::SolverCapability::BitVectors,
        verum_smt::solver_capability::SolverCapability::Arrays,
        verum_smt::solver_capability::SolverCapability::InductiveDatatypes,
    ] {
        let available = registry.supports(cap);
        let mark = if available {
            "✓".green().bold()
        } else {
            "✗".red().bold()
        };
        let status = if available {
            "available".green()
        } else {
            "unavailable".red()
        };
        println!("    {} {:<26} {}", mark, cap.name(), status);
    }
    println!();

    // Verification strategies available
    println!("  {}", "Verification strategies".bold().underline());
    let strategies = [
        ("runtime", "Runtime assertion (no formal proof)"),
        ("static", "Type-level check only"),
        ("formal", "Balanced default (recommended)"),
        ("fast", "Prefer speed over completeness"),
        ("thorough", "Maximum completeness (parallel)"),
        ("certified", "Exportable proof certificate"),
        ("synthesize", "Generate term from specification"),
    ];
    for (name, desc) in strategies {
        println!("    {:<12} {}", name.cyan(), desc.dimmed());
    }
    println!();

    // Recommendations
    if !cvc5_caps.linked {
        println!("  {}", "Notes".bold().underline());
        println!(
            "    {} Secondary verification engine is in stub mode.",
            "ℹ".blue()
        );
        println!(
            "    {} Some advanced features (synthesis, abduction, finite-model-finding)",
            "ℹ".blue()
        );
        println!(
            "    {} are unavailable. To enable them, rebuild with:",
            "ℹ".blue()
        );
        println!("      {}", "cargo build --features verum_smt/cvc5-ffi".dimmed());
        println!();
    }

    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".cyan());
    println!();

    Ok(())
}

fn print_json() -> Result<()> {
    let registry = verum_smt::solver_capability::CapabilityRegistry::detect();
    let cvc5_caps = verum_smt::cvc5_advanced::detect_capabilities();

    let out = serde_json::json!({
        "backends": {
            "primary": {
                "available": registry.z3_available,
                "version": registry.z3_version,
            },
            "secondary": {
                "available": registry.cvc5_available,
                "version": registry.cvc5_version,
            },
        },
        "capabilities": {
            "interpolation": registry.supports(verum_smt::solver_capability::SolverCapability::Interpolation),
            "optimization": registry.supports(verum_smt::solver_capability::SolverCapability::Optimization),
            "horn_clauses": registry.supports(verum_smt::solver_capability::SolverCapability::HornClauses),
            "synthesis": registry.supports(verum_smt::solver_capability::SolverCapability::SygusSynthesis),
            "abduction": registry.supports(verum_smt::solver_capability::SolverCapability::Abduction),
            "finite_model_finding": registry.supports(verum_smt::solver_capability::SolverCapability::FiniteModelFinding),
            "strings_regex": registry.supports(verum_smt::solver_capability::SolverCapability::StringsRegex),
            "sequences": registry.supports(verum_smt::solver_capability::SolverCapability::Sequences),
            "nonlinear_real": registry.supports(verum_smt::solver_capability::SolverCapability::CadNonlinearReal),
            "quantifier_elimination": registry.supports(verum_smt::solver_capability::SolverCapability::QuantifierElimination),
            "proof_production": registry.supports(verum_smt::solver_capability::SolverCapability::ProofProduction),
            "unsat_cores": registry.supports(verum_smt::solver_capability::SolverCapability::UnsatCores),
            "model_extraction": registry.supports(verum_smt::solver_capability::SolverCapability::ModelExtraction),
            "incremental": registry.supports(verum_smt::solver_capability::SolverCapability::IncrementalSolving),
            "bit_vectors": registry.supports(verum_smt::solver_capability::SolverCapability::BitVectors),
            "arrays": registry.supports(verum_smt::solver_capability::SolverCapability::Arrays),
            "inductive_datatypes": registry.supports(verum_smt::solver_capability::SolverCapability::InductiveDatatypes),
        },
        "strategies": [
            "runtime", "static", "formal", "fast", "thorough", "certified", "synthesize"
        ],
        "advanced": {
            "cpc_proofs": cvc5_caps.proofs_cpc,
        }
    });

    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
