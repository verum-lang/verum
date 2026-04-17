// Version command

use crate::error::Result;
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
        println!("  Commit:       {}", option_env!("GIT_HASH").unwrap_or("unknown"));
        println!("  Build date:   {}", option_env!("BUILD_DATE").unwrap_or("unknown"));
        println!("  Rust version: {}", env!("CARGO_PKG_RUST_VERSION"));
        println!("  LLVM version: {}", verum_codegen::llvm::LLVM_VERSION);
        println!("  Host target:  {}-{}", std::env::consts::ARCH, std::env::consts::OS);
        println!();
        println!("{}", "Capabilities:".bold());
        println!("  AOT backend:    {}", "LLVM".green());
        println!("  Interpreter:    {}", "VBC Tier 0".green());
        println!("  GPU backend:    {}", "MLIR (optional)".yellow());
        println!("  SMT solver:     {}", "Z3".green());
        println!("  Verification:   {}", "refinement + dependent types".green());
        println!();
        println!("{}", "Verum Language Platform".bold());
    }

    Ok(())
}
