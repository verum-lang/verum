// Create new Verum projects from templates.
// Generates project directory with verum.toml, src/, and template files
// based on language profile (application, systems, research).

use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{LanguageProfile, create_default_manifest, is_valid_cog_name};
use crate::error::{CliError, Result};
use crate::templates;
use crate::ui;

/// Execute the `verum new` command.
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

    // Parse language profile
    let lang_profile = if let Some(p) = profile {
        parse_profile(p)?
    } else {
        // Interactive profile selection
        let selected = ui::select(
            "Select language profile:",
            &[
                "application — Safe by default, refinement types, runtime checks (recommended)",
                "systems     — Full language including @unsafe blocks, manual memory control",
                "research    — Dependent types, formal proofs, SMT verification",
            ],
        );

        match selected {
            Some(0) => LanguageProfile::Application,
            Some(1) => LanguageProfile::Systems,
            Some(2) => LanguageProfile::Research,
            _ => {
                return Err(CliError::Custom(
                    "Language profile selection required. Use --profile <application|systems|research>".into(),
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
        "Creating {} project: {} ({})",
        template.cyan(),
        name.green().bold(),
        format!("{:?}", lang_profile).yellow()
    ));

    // Create project structure
    create_project_structure(&project_dir, name, template, lang_profile)?;

    // Initialize git repository if requested
    if git {
        ui::step("Initializing git repository");
        init_git_repo(&project_dir)?;
    }

    // Print success message
    print_success(name, template, lang_profile);

    Ok(())
}

fn parse_profile(profile: &str) -> Result<LanguageProfile> {
    match profile {
        "application" => Ok(LanguageProfile::Application),
        "systems" => Ok(LanguageProfile::Systems),
        "research" => Ok(LanguageProfile::Research),
        _ => Err(CliError::InvalidArgument(format!(
            "Invalid language profile '{}'. Valid profiles: application, systems, research",
            profile
        ))),
    }
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
            templates::binary::create(dir, name, profile)?;
        }
        "library" => {
            templates::library::create(dir, name, profile)?;
        }
        "web-api" => {
            templates::web_api::create(dir, name, profile)?;
        }
        "cli-app" => {
            templates::cli_app::create(dir, name, profile)?;
        }
        _ => {
            return Err(CliError::TemplateNotFound(template.into()));
        }
    }

    // Create .gitignore
    create_gitignore_file(dir)?;

    // Create README.md
    create_readme(dir, name, template, profile)?;

    Ok(())
}

fn init_git_repo(dir: &Path) -> Result<()> {
    use std::process::Command;

    let status = Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;

    if !status.success() {
        ui::warn("Failed to initialize git repository");
    }

    Ok(())
}

/// Create .gitignore with standard Verum patterns.
/// Shared between `new` and `init` commands.
pub fn create_gitignore_file(dir: &Path) -> Result<()> {
    let content = "\
# Build artifacts
/target/
*.o
*.so
*.dylib
*.dll

# Verum bytecode
*.vbc

# Cache
.verum_cache/

# IDE
.vscode/
.idea/
*.swp
*~

# OS
.DS_Store
Thumbs.db
";

    fs::write(dir.join(".gitignore"), content)?;
    Ok(())
}

fn create_readme(dir: &Path, name: &str, template: &str, profile: LanguageProfile) -> Result<()> {
    let profile_section = match profile {
        LanguageProfile::Application => "\
## Language Profile: Application

Safe by default. No `@unsafe` blocks allowed. Refinement types provide
compile-time guarantees via SMT verification. All references use CBGR
managed checks (~15ns per dereference) unless the compiler proves safety
via escape analysis (promoted to zero-cost `&checked` references).",
        LanguageProfile::Systems => "\
## Language Profile: Systems

Full language access including `@unsafe` blocks for manual memory control.
Three-tier reference model: `&T` (managed, ~15ns), `&checked T` (compiler-proven, 0ns),
`&unsafe T` (manual proof, 0ns). Suitable for OS kernels, drivers, embedded systems.",
        LanguageProfile::Research => "\
## Language Profile: Research

Dependent types and formal verification enabled. Write machine-checked proofs
with `theorem`, `lemma`, and `proof` blocks. SMT solver (Z3) verifies
refinement predicates at compile time. Experimental features available.",
    };

    let content = format!(
        "\
# {name}

A {template} project written in [Verum](https://github.com/verum-lang/verum).

{profile_section}

## Quick Start

```bash
verum build          # Build (dev mode — interpreter, fast compilation)
verum run            # Build and run
verum test           # Run tests
verum build --release  # Production build (AOT via LLVM)
```

## Execution Modes

| Mode | Flag | Compilation | Use Case |
|------|------|-------------|----------|
| Dev | (default) | Instant | Fast iteration, debugging |
| Release | `--release` | AOT via LLVM | Production, benchmarks |

## Project Structure

```
{name}/
  src/       — Source code
  tests/     — Tests
  benches/   — Benchmarks
  examples/  — Usage examples
  verum.toml — Project manifest
```

## License

MIT OR Apache-2.0
",
        name = name,
        template = template,
        profile_section = profile_section,
    );

    fs::write(dir.join("README.md"), content)?;
    Ok(())
}

fn print_success(name: &str, template: &str, profile: LanguageProfile) {
    println!();
    ui::success(&format!("Created {} project", name.green().bold()));
    println!();
    println!("{}", "Project configuration:".bold());
    println!("  Language profile: {}", format!("{:?}", profile).cyan());
    println!("  Profile details:  {}", profile.description().dimmed());
    println!("  Template:         {}", template.cyan());
    println!();
    println!("{}", "Next steps:".bold());
    println!("  {} {}", "cd".cyan(), name);
    println!("  {} {}", "verum".cyan(), "build");
    println!("  {} {}", "verum".cyan(), "run");
    println!();
}
