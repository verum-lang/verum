// Search the cog package registry by name, description, or tags.

use crate::error::Result;
use crate::registry::RegistryClient;
use crate::ui;
use colored::Colorize;
use verum_common::Text;

/// Search options
pub struct SearchOptions {
    pub query: Text,
    pub limit: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            query: Text::new(),
            limit: 20,
        }
    }
}

/// Search for packages in registry
pub fn search(options: SearchOptions) -> Result<()> {
    ui::step(&format!("Searching for: {}", options.query.as_str().cyan()));

    let client = RegistryClient::default()?;
    let results = client.search(options.query.as_str(), options.limit)?;

    if results.is_empty() {
        println!();
        ui::warn("No packages found");
        return Ok(());
    }

    // Print results
    println!();
    println!(
        "{:<25} {:<12} {:<50}",
        "Package".bold(),
        "Version".bold(),
        "Description".bold()
    );
    println!("{}", "─".repeat(90));

    let results_count = results.len();
    for result in results {
        let name: String = if result.verified {
            format!("{} {}", result.name, "✓".green())
        } else {
            result.name.to_string()
        };

        let version: String = if result.cbgr_optimized {
            format!("{} {}", result.version, "⚡".yellow())
        } else {
            result.version.to_string()
        };

        let description: String = result
            .description
            .as_ref()
            .map(|d| d.as_str())
            .unwrap_or("")
            .chars()
            .take(50)
            .collect();

        println!("{:<25} {:<12} {:<50}", name.cyan(), version, description);
    }

    println!();
    ui::info(&format!("Showing {} results", results_count));
    println!();
    println!("Legend:");
    println!("  {} Formally verified package", "✓".green());
    println!("  {} CBGR optimized", "⚡".yellow());

    Ok(())
}
