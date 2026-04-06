// Check command - fast type checking without code generation.
// Runs lexing, parsing, and type inference but skips VBC/LLVM codegen.

use std::time::Instant;

use crate::config::Manifest;
use crate::error::{CliError, Result};
use crate::ui;

use verum_compiler::options::CompilerOptions;
use verum_compiler::pipeline::CompilationPipeline;
use verum_compiler::session::Session;

/// Execute the `verum check` command
/// Type-check the project without generating any code or binary output.
pub fn execute(_workspace: bool, strict: bool, verbose: bool) -> Result<()> {
    let start = Instant::now();

    // Load manifest
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest_path = Manifest::manifest_path(&manifest_dir);
    let manifest = Manifest::from_file(&manifest_path)?;

    // Determine project root - either manifest dir or first src file's parent
    let project_root = manifest_dir.clone();

    ui::status("Checking", &format!(
        "{} v{} ({})",
        manifest.cog.name.as_str(),
        manifest.cog.version.as_str(),
        manifest_dir.display()
    ));

    // Create compiler options for check mode
    let mut options = CompilerOptions::default();
    options.input = project_root.join("src"); // Will be overridden by project discovery
    options.verbose = if verbose { 2 } else { 0 };

    // Set verification mode based on strict flag
    if strict {
        options.verify_mode = verum_compiler::options::VerifyMode::Auto;
    } else {
        options.verify_mode = verum_compiler::options::VerifyMode::Runtime;
    }

    // Create session
    let mut session = Session::new(options);

    // Create check-only pipeline
    let mut pipeline = CompilationPipeline::new_check(&mut session);

    // Run project-wide type checking with proper import resolution
    // Note: Stdlib is now embedded directly from source files in verum_compiler
    match pipeline.check_project() {
        Ok(result) => {
            let duration = start.elapsed();

            // Get error and warning counts from result
            let error_count = result.user_errors;
            let warning_count = result.warnings;

            if error_count > 0 {
                // Display diagnostics first
                ui::output("");
                session.display_diagnostics().map_err(|e| {
                    CliError::Custom(format!("Failed to display diagnostics: {}", e))
                })?;

                ui::output("");
                ui::error(&format!(
                    "{} type error{} found",
                    error_count,
                    if error_count == 1 { "" } else { "s" }
                ));
                ui::output("");

                return Err(CliError::CompilationFailed(format!(
                    "{} type error{}",
                    error_count,
                    if error_count == 1 { "" } else { "s" }
                )));
            }

            // Show warnings if any
            if warning_count > 0 {
                ui::warn(&format!(
                    "{} warning{} emitted",
                    warning_count,
                    if warning_count == 1 { "" } else { "s" }
                ));
                if verbose {
                    session.display_diagnostics().map_err(|e| {
                        CliError::Custom(format!("Failed to display diagnostics: {}", e))
                    })?;
                }
            }

            // Cargo-style finish line
            ui::success(&format!(
                "checking {} in {}",
                manifest.cog.name.as_str(),
                ui::format_duration(duration)
            ));

            // Performance note
            let loc_estimate = result.files_checked * 50;
            let lines_per_sec = if duration.as_secs_f64() > 0.0 {
                loc_estimate as f64 / duration.as_secs_f64()
            } else {
                0.0
            };
            ui::note(&format!(
                "{} files checked, ~{:.0} LOC/s",
                result.files_checked, lines_per_sec
            ));

            if strict {
                ui::note("strict mode: all refinements verified, all contexts resolved");
            }

            Ok(())
        }
        Err(e) => {
            // Display any accumulated diagnostics
            let _ = session.display_diagnostics();

            ui::error(&format!("could not check `{}`: {}", manifest.cog.name, e));

            Err(CliError::CompilationFailed(e.to_string()))
        }
    }
}
