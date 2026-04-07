// Initialize a new Verum project in the current directory.
// Creates verum.toml manifest, src/ directory with template files.

use colored::Colorize;
use std::env;
use std::fs;

use crate::config::{LanguageProfile, create_default_manifest};
use crate::error::{CliError, Result};
use crate::templates;
use crate::ui;

/// Execute the `verum init` command.
/// Initialize project in current directory. Requires a language profile
/// (application, systems, or research) to set default compilation settings.
pub fn execute(
    profile: &str,
    template: &str,
    force: bool,
    name_override: Option<&str>,
) -> Result<()> {
    let current_dir = env::current_dir()?;
    let dir_name = name_override.unwrap_or_else(|| {
        current_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("my_project")
    });

    // Parse language profile
    let lang_profile = parse_profile(profile)?;

    // Check if verum.toml already exists
    let verum_toml = current_dir.join("verum.toml");
    let verum_toml_alt = current_dir.join("Verum.toml");

    if (verum_toml.exists() || verum_toml_alt.exists()) && !force {
        return Err(CliError::Custom(
            "verum.toml already exists. Use --force to overwrite.".into(),
        ));
    }

    ui::step(&format!(
        "Initializing {} project in: {}",
        template.cyan(),
        current_dir.display().to_string().cyan()
    ));

    // Create directories
    fs::create_dir_all(current_dir.join("src"))?;
    fs::create_dir_all(current_dir.join("tests"))?;

    // Create verum.toml
    let is_library = template == "library";
    let manifest = create_default_manifest(dir_name, is_library, lang_profile);
    manifest.to_file(&verum_toml)?;

    // Create source files based on template
    create_template_files(&current_dir, dir_name, template, lang_profile)?;

    // Create .gitignore if it doesn't exist
    let gitignore = current_dir.join(".gitignore");
    if !gitignore.exists() {
        super::new::create_gitignore_file(&current_dir)?;
    }

    // Print success message
    print_success(dir_name, template, lang_profile);

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

fn create_template_files(
    dir: &std::path::Path,
    name: &str,
    template: &str,
    profile: LanguageProfile,
) -> Result<()> {
    match template {
        "binary" | "application" => templates::binary::create(dir, name, profile)?,
        "library" => templates::library::create(dir, name, profile)?,
        "web-api" => templates::web_api::create(dir, name, profile)?,
        "cli-app" => templates::cli_app::create(dir, name, profile)?,
        _ => {
            return Err(CliError::TemplateNotFound(template.into()));
        }
    }
    Ok(())
}

fn print_success(name: &str, template: &str, profile: LanguageProfile) {
    println!();
    ui::success(&format!("Initialized {} project", name.green().bold()));
    println!();
    println!("{}", "Project configuration:".bold());
    println!("  Language profile: {}", format!("{:?}", profile).cyan());
    println!("  Profile details:  {}", profile.description().dimmed());
    println!("  Template:         {}", template.cyan());
    println!();
    println!("{}", "Next steps:".bold());
    println!("  {} {}", "verum".cyan(), "build");
    println!("  {} {}", "verum".cyan(), "run");
    println!();
}
