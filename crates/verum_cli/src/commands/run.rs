//! Run command - builds and executes the project
//!
//! # Two-Tier Architecture
//!
//! The execution tiers are:
//! - Tier 0: VBC Interpreter (direct VBC execution, full diagnostics)
//! - Tier 1: AOT compilation via LLVM (native executable)
//!
//! Use `verum run --tier 1` or `--tier aot` for native execution.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use verum_common::{List, Text};

use crate::config::{CompilationTier, Manifest};
use crate::error::{CliError, Result};
use crate::ui;

use verum_ast::{FileId, Module};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

/// Execute the `verum run` command
pub fn execute(
    tier: Option<u8>,
    profile: Option<Text>,
    release: bool,
    example: Option<Text>,
    bin: Option<Text>,
    args: List<Text>,
) -> Result<()> {
    // Build first (always AOT)
    super::build::execute(
        profile.clone(),
        None, // refs
        None, // verify
        release,
        None,  // target
        None,  // jobs
        false, // keep_temps
        false, // all_features
        false, // no_default_features
        None,  // features
        false, // timings
        // Advanced linking options
        None,  // lto
        false, // static_link
        false, // strip
        false, // strip_debug
        false, // emit_asm
        false, // emit_llvm
        false, // emit_bc
        false, // emit_types
        false, // emit_vbc
        // Lint options (use defaults for run command)
        false,     // deny_warnings
        false,     // strict_intrinsics
        Vec::new(), // deny_lint
        Vec::new(), // warn_lint
        Vec::new(), // allow_lint
        Vec::new(), // forbid_lint
        false,      // smt_stats
    )?;

    // Determine compilation tier. Apply CLI feature overrides so
    // `--tier`, `-Z codegen.tier=...`, etc. are respected here as well.
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest_path = Manifest::manifest_path(&manifest_dir);
    let mut manifest = Manifest::from_file(&manifest_path)?;
    crate::feature_overrides::apply_global(&mut manifest)?;

    // Tier resolution priority:
    //   1. Explicit CLI tier (from --tier/--interp/--aot via run_with_tier)
    //   2. [codegen].tier from verum.toml (new unified config system)
    //   3. [profile.dev/release].tier (legacy per-profile config)
    let compilation_tier = if let Some(t) = tier {
        CompilationTier::from_u8(t)
            .ok_or_else(|| CliError::InvalidArgument(format!("Invalid tier {}. Must be 0-1", t)))?
    } else {
        // Check [codegen].tier first (unified config takes precedence).
        match manifest.codegen.tier.as_str() {
            "interpret" => CompilationTier::Interpreter,
            "aot" => CompilationTier::Aot,
            "check" => {
                return Err(CliError::InvalidArgument(
                    "[codegen].tier = \"check\" is for `verum check`, not `verum run`".into(),
                ));
            }
            _ => {
                // Fall back to legacy [profile.dev/release].tier
                let using_release =
                    profile.as_ref().map(|s| s.as_str()) == Some("release") || release;
                manifest.get_profile(using_release).tier
            }
        }
    };

    let mode = if release { "release" } else { "debug" };
    let bin_name = if let Some(b) = bin {
        b
    } else if let Some(e) = example {
        e
    } else {
        manifest.cog.name.clone()
    };

    // Execute based on compilation tier
    match compilation_tier {
        CompilationTier::Interpreter => {
            // Tier 0: VBC Interpreter
            run_vbc_interpreted(&manifest_dir, mode, &bin_name, &args)
        }
        CompilationTier::Aot => {
            // Tier 1: Run native executable (AOT)
            run_native(&manifest_dir, mode, &bin_name, &args)
        }
    }
}

/// Run VBC interpreted (Tier 0)
///
/// Finds the project entry point (src/main.vr), parses all source files,
/// compiles to VBC bytecode, and executes via the VBC interpreter.
fn run_vbc_interpreted(
    manifest_dir: &PathBuf,
    _mode: &str,
    _bin_name: &Text,
    _args: &[Text],
) -> Result<()> {
    use std::sync::Arc;
    use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
    use verum_vbc::interpreter::Interpreter;

    // Find the entry source file
    let src_dir = manifest_dir.join("src");
    let main_file = src_dir.join("main.vr");

    if !main_file.exists() {
        return Err(CliError::Custom(format!(
            "Entry point not found: {}. Create src/main.vr with a fn main() function.",
            main_file.display()
        )));
    }

    ui::status("Interpreting", &format!("{}", main_file.display()));

    // Load all source modules
    let modules = load_source_modules(&src_dir)?;
    if modules.is_empty() {
        return Err(CliError::Custom("No source files found in src/".to_string()));
    }

    // Merge all modules into a single AST for compilation
    let merged = verum_ast::Module {
        items: modules.iter().flat_map(|m| m.items.iter().cloned()).collect(),
        attributes: List::new(),
        file_id: FileId::new(0),
        span: verum_ast::Span::default(),
    };

    // Compile to VBC
    let start = Instant::now();
    let config = CodegenConfig::new("project");
    let mut codegen = VbcCodegen::with_config(config);

    let vbc_module = codegen
        .compile_module(&merged)
        .map_err(|e| CliError::Custom(format!("Compilation error: {:?}", e)))?;

    let compile_time = start.elapsed();
    ui::note(&format!("compiled in {}", ui::format_duration(compile_time)));

    // Find main function
    let vbc_module = Arc::new(vbc_module);
    let main_id = vbc_module.functions.iter()
        .find(|f| vbc_module.get_string(f.name) == Some("main"))
        .map(|f| f.id)
        .ok_or_else(|| CliError::Custom(
            "No main() function found. Add fn main() to src/main.vr.".to_string()
        ))?;

    // Execute
    let exec_start = Instant::now();
    let mut interpreter = Interpreter::new(vbc_module);

    // Pass CLI args via TLS slot 0 (convention)
    // For now, args are available but not yet wired to argv intrinsic

    let result = interpreter
        .execute_function(main_id)
        .map_err(|e| CliError::Custom(format!("Runtime error: {}", e)))?;

    let exec_time = exec_start.elapsed();

    // Print stdout if captured
    let stdout = interpreter.state.get_stdout();
    if !stdout.is_empty() {
        print!("{}", stdout);
    }

    // Show exit code
    let exit_code = if result.is_int() { result.as_i64() as i32 } else { 0 };
    if exit_code != 0 {
        ui::error(&format!("process exited with code: {}", exit_code));
    }

    ui::note(&format!("executed in {}", ui::format_duration(exec_time)));

    if exit_code != 0 {
        std::process::exit(exit_code);
    }

    Ok(())
}

/// Run native executable (Tier 1: AOT)
fn run_native(
    manifest_dir: &PathBuf,
    mode: &str,
    bin_name: &Text,
    args: &[Text],
) -> Result<()> {
    let exe_path = manifest_dir
        .join("target")
        .join(mode)
        .join(bin_name.as_str());

    #[cfg(target_os = "windows")]
    let exe_path = exe_path.with_extension("exe");

    if !exe_path.exists() {
        return Err(CliError::Custom(format!(
            "Executable not found: {}. Run 'verum build' first.",
            exe_path.display()
        )));
    }

    ui::status("Running", &format!("`{}`", exe_path.display()));

    let start = Instant::now();

    let mut cmd = Command::new(&exe_path);
    for arg in args {
        cmd.arg(arg.as_str());
    }

    let status = cmd.status().map_err(|e| {
        CliError::Custom(format!("Failed to execute {}: {}", exe_path.display(), e))
    })?;

    let elapsed = start.elapsed();

    if !status.success() {
        let code = status.code().unwrap_or(1);
        ui::error(&format!("process exited with code: {}", code));
        std::process::exit(code);
    }

    ui::note(&format!("completed in {}", ui::format_duration(elapsed)));
    Ok(())
}

/// Load and parse all source modules from a directory
/// Used for VBC codegen pipeline
fn load_source_modules(src_dir: &Path) -> Result<List<Module>> {
    let mut modules = List::new();

    if !src_dir.exists() {
        return Ok(modules);
    }

    // Walk the source directory to find .vr files
    for entry in walkdir::WalkDir::new(src_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        // Check for Verum source file extensions
        let is_verum_file = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext == "vr")
            .unwrap_or(false);

        if is_verum_file && path.is_file() {
            let module = parse_source_file(path)?;
            modules.push(module);
        }
    }

    Ok(modules)
}

/// Parse a single source file into a Module AST
fn parse_source_file(path: &Path) -> Result<Module> {
    let source = std::fs::read_to_string(path).map_err(|e| {
        CliError::Custom(format!("Failed to read {}: {}", path.display(), e))
    })?;

    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let lexer = Lexer::new(&source, file_id);

    let module = parser
        .parse_module(lexer, file_id)
        .map_err(|errors| {
            let error_msgs: Vec<String> = errors.iter().map(|e| format!("{}", e)).collect();
            CliError::Custom(format!(
                "Parse errors in {}:\n{}",
                path.display(),
                error_msgs.join("\n")
            ))
        })?;

    Ok(module)
}
