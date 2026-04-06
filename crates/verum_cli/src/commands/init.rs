// Initialize a new Verum project in the current directory.
// Creates verum.toml manifest, src/ directory with main.vr, and optional git init.

use colored::Colorize;
use std::env;
use std::fs;
use std::path::PathBuf;

use crate::config::{LanguageProfile, create_default_manifest};
use crate::error::{CliError, Result};
use crate::templates;
use crate::ui;

/// Execute the `verum init` command
/// Initialize project in current directory. Requires a language profile
/// (systems, application, or scripting) to set default compilation settings.
pub fn execute(profile: &str, lib: bool, force: bool) -> Result<()> {
    let current_dir = env::current_dir()?;
    let dir_name = current_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("my_project");

    ui::step(&format!(
        "Initializing Verum project in current directory: {}",
        current_dir.display().to_string().cyan()
    ));

    // Parse language profile (REQUIRED by spec)
    let lang_profile = match profile {
        "application" => LanguageProfile::Application,
        "systems" => LanguageProfile::Systems,
        "research" => LanguageProfile::Research,
        _ => {
            return Err(CliError::InvalidArgument(format!(
                "Invalid language profile '{}'. Must be: application, systems, or research",
                profile
            )));
        }
    };

    // Check if verum.toml already exists
    let verum_toml = current_dir.join("verum.toml");
    let verum_toml_alt = current_dir.join("Verum.toml");

    if (verum_toml.exists() || verum_toml_alt.exists()) && !force {
        return Err(CliError::Custom(
            "verum.toml already exists. Use --force to overwrite.".into(),
        ));
    }

    // Create directories if they don't exist
    fs::create_dir_all(current_dir.join("src"))?;
    fs::create_dir_all(current_dir.join("tests"))?;

    // Create verum.toml
    let manifest = create_default_manifest(dir_name, lib, lang_profile);
    manifest.to_file(&verum_toml)?;

    // Create source files based on template
    let template = if lib { "library" } else { "binary" };
    match template {
        "binary" => {
            templates::binary::create(&current_dir, dir_name)?;
        }
        "library" => {
            templates::library::create(&current_dir, dir_name)?;
        }
        _ => unreachable!(),
    }

    // Create .gitignore if it doesn't exist
    let gitignore = current_dir.join(".gitignore");
    if !gitignore.exists() {
        create_gitignore(&current_dir)?;
    }

    // Print success message
    println!();
    ui::success("Initialized Verum project");
    println!();
    println!("{}", "Project Configuration:".bold());
    println!(
        "  Language profile: {}",
        format!("{:?}", lang_profile).cyan()
    );
    println!("  Project type: {}", template.cyan());
    println!("  Default tier: {} (fast iteration)", "Tier 0".cyan());
    println!("  Verification: {} (safe by default)", "Runtime".cyan());
    println!();
    println!("{}", "Next steps:".bold());
    println!("  {} {}", "verum".cyan(), "build".cyan());
    println!("  {} {}", "verum".cyan(), "run".cyan());
    println!();

    Ok(())
}

fn create_gitignore(dir: &PathBuf) -> Result<()> {
    let content = r#"# Verum Build artifacts
/target/
*.ll
*.bc
*.o
*.so
*.dylib
*.dll
*.exe

# Tier-specific outputs
/target/tier0/
/target/tier1/
/target/tier2/
/target/tier3/
/target/cbgr-profile/
/target/verify-cache/

# Cache
.verum_cache/

# IDE
.vscode/
.idea/
*.swp
*.swo
*~

# OS
.DS_Store
Thumbs.db

# Lock file (commit for libraries, ignore for binaries)
# verum.lock
"#;

    fs::write(dir.join(".gitignore"), content)?;
    Ok(())
}
