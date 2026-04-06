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
        println!("Build information:");
        println!("  Commit: {}", option_env!("GIT_HASH").unwrap_or("unknown"));
        println!(
            "  Build date: {}",
            option_env!("BUILD_DATE").unwrap_or("unknown")
        );
        println!("  Rust version: {}", env!("CARGO_PKG_RUST_VERSION"));
        println!();
        println!("Verum Language Platform");
        println!("https://github.com/verum-lang/verum");
    }

    Ok(())
}
