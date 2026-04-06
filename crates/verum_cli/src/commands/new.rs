// Create new Verum projects from templates.
// Generates project directory with verum.toml, src/, and template files
// based on language profile (systems, application, scripting).

use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{LanguageProfile, create_default_manifest, is_valid_cog_name};
use crate::error::{CliError, Result};
use crate::templates;
use crate::ui;

/// Execute the `verum new` command
/// Create a new Verum project in a new directory with the given name.
pub fn execute(
    name: &str,
    profile: Option<&str>,
    template: &str,
    git: bool,
    path: Option<&str>,
) -> Result<()> {
    // Validate project name
    if !is_valid_cog_name(name) {
        return Err(CliError::InvalidProjectName(name.into()));
    }

    // Parse language profile (REQUIRED by spec)
    let lang_profile = if let Some(p) = profile {
        match p {
            "application" => LanguageProfile::Application,
            "systems" => LanguageProfile::Systems,
            "research" => LanguageProfile::Research,
            _ => {
                return Err(CliError::InvalidArgument(format!(
                    "Invalid language profile '{}'. Must be: application, systems, or research",
                    p
                )));
            }
        }
    } else {
        // Interactive profile selection
        let selected = ui::select(
            "Select language profile:",
            &[
                "application - No unsafe, refinements + runtime checks (recommended for 80% of users)",
                "systems - Full language including unsafe (for systems programming)",
                "research - Dependent types, formal proofs (experimental)",
            ],
        );

        match selected {
            Some(0) => LanguageProfile::Application,
            Some(1) => LanguageProfile::Systems,
            Some(2) => LanguageProfile::Research,
            _ => {
                return Err(CliError::Custom(
                    "Language profile selection required".into(),
                ));
            }
        }
    };

    // Determine project directory
    let project_dir = if let Some(p) = path {
        PathBuf::from(p)
    } else {
        PathBuf::from(name)
    };

    // Check if directory already exists
    if project_dir.exists() {
        return Err(CliError::ProjectExists(project_dir));
    }

    // Display what we're creating
    ui::step(&format!(
        "Creating new {} project: {} ({})",
        template.cyan(),
        name.green().bold(),
        lang_profile.description().yellow()
    ));

    // Create project structure
    create_project_structure(&project_dir, name, template, lang_profile)?;

    // Initialize git repository if requested
    if git {
        ui::step("Initializing git repository");
        init_git_repo(&project_dir)?;
    }

    // Print success message with semantic honesty
    println!();
    ui::success(&format!("Created {} project", name.green().bold()));
    println!();
    println!("{}", "Project Configuration:".bold());
    println!(
        "  Language profile: {}",
        format!("{:?}", lang_profile).cyan()
    );
    println!("  Template: {}", template.cyan());
    println!("  Default tier: {} (fast iteration)", "Tier 0".cyan());
    println!("  Verification: {} (safe by default)", "Runtime".cyan());
    println!();
    println!("{}", "Next steps:".bold());
    println!("  {} {}", "cd".cyan(), name);
    println!("  {} {}", "verum".cyan(), "build".cyan());
    println!("  {} {}", "verum".cyan(), "run".cyan());
    println!();
    println!("{}", "Performance notes:".dimmed());
    println!("  • Tier 0 (dev): Instant compilation, interpreted execution");
    println!("  • CBGR checks: ~15ns overhead per reference operation");
    println!("  • Run 'verum build --release' for Tier 2 (production) build");
    println!();

    Ok(())
}

fn create_project_structure(
    dir: &Path,
    name: &str,
    template: &str,
    profile: LanguageProfile,
) -> Result<()> {
    // Create directories
    fs::create_dir_all(dir)?;
    fs::create_dir_all(dir.join("src"))?;
    fs::create_dir_all(dir.join("tests"))?;
    fs::create_dir_all(dir.join("benches"))?;
    fs::create_dir_all(dir.join("examples"))?;

    // Create verum.toml
    let is_library = template == "library";
    let manifest = create_default_manifest(name, is_library, profile);
    manifest.to_file(&dir.join("verum.toml"))?;

    // Create source files based on template
    match template {
        "binary" | "application" => {
            templates::binary::create(dir, name)?;
        }
        "library" => {
            templates::library::create(dir, name)?;
        }
        "web-api" => {
            templates::web_api::create(dir, name)?;
        }
        "cli-app" => {
            templates::cli_app::create(dir, name)?;
        }
        _ => {
            return Err(CliError::TemplateNotFound(template.into()));
        }
    }

    // Create .gitignore
    create_gitignore(dir)?;

    // Create README.md
    create_readme(dir, name, template, profile)?;

    Ok(())
}

fn init_git_repo(dir: &Path) -> Result<()> {
    use std::process::Command;

    let status = Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .status()?;

    if !status.success() {
        ui::warn("Failed to initialize git repository");
    }

    Ok(())
}

fn create_gitignore(dir: &Path) -> Result<()> {
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

fn create_readme(dir: &Path, name: &str, template: &str, profile: LanguageProfile) -> Result<()> {
    let content = format!(
        r#"# {}

A {} project written in Verum.

**Language Profile:** {:?}
**Description:** {}

## Quick Start

```bash
# Build (Tier 0 - instant compilation)
verum build

# Run
verum run

# Run tests
verum test

# Build for production (Tier 2 - AOT compilation)
verum build --release
```

## Performance Profiles

| Tier | Compilation | Execution | Use Case |
|------|-------------|-----------|----------|
| 0 | Instant (<100ms) | Interpreted | Fast iteration |
| 1 | Fast (~1s) | JIT | Development |
| 2 | Moderate (~10s) | Native | Production |
| 3 | Slow (~30s) | Optimized | Maximum performance |

## Project Structure

- `src/` - Source code
- `tests/` - Integration tests
- `benches/` - Performance benchmarks
- `examples/` - Usage examples
- `verum.toml` - Project manifest

## Semantic Honesty

Verum shows real costs transparently:
- CBGR overhead: ~15ns per reference check
- Verification: Runtime by default, compile-time optional
- Memory safety: Zero-cost where proven, minimal cost otherwise

Run `verum profile --memory` to analyze CBGR overhead in your code.

## License

MIT OR Apache-2.0
"#,
        name,
        template,
        profile,
        profile.description()
    );

    fs::write(dir.join("README.md"), content)?;
    Ok(())
}
