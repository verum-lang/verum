// Version command

use crate::{build_identity::BUILD_IDENTITY, error::Result};
use colored::Colorize;

pub fn execute(verbose: bool) -> Result<()> {
    println!(
        "{} {}",
        "verum".cyan().bold(),
        env!("CARGO_PKG_VERSION").green()
    );

    if verbose {
        println!();
        println!("{}", "Build information:".bold());
        println!("  Commit:       {}", BUILD_IDENTITY.git_sha);
        println!("  Built at (Unix seconds): {}", BUILD_IDENTITY.timestamp);
        println!("  Worktree:     {}", BUILD_IDENTITY.git_dirty);
        println!("  Rust version: {}", BUILD_IDENTITY.rustc);
        println!("  LLVM version: {}", verum_codegen::llvm::LLVM_VERSION);
        println!("  Target:       {}", BUILD_IDENTITY.target);
        println!();
        println!("{}", "Capabilities:".bold());
        println!("  AOT backend:    {}", "LLVM".green());
        println!("  Interpreter:    {}", "VBC Tier 0".green());
        println!("  GPU backend:    {}", "MLIR (optional)".yellow());
        println!("  SMT solver:     {}", "Z3".green());
        println!(
            "  Verification:   {}",
            "refinement + dependent types".green()
        );
        println!();
        println!("{}", "Verum Language Platform".bold());
    }

    Ok(())
}
