//! `verum config show` — display the resolved language-feature set.
//!
//! Loads `verum.toml`, applies any `-Z` / `--tier` / `--cbgr` / …
//! overrides supplied on the command line, translates the merged
//! manifest into the compiler-facing [`LanguageFeatures`] value, runs
//! the feature validator, and prints the result.
//!
//! This lets users verify that their config file and CLI overrides are
//! producing the effective feature set they expect — without having to
//! run a build and scrape logs.

use anyhow::Result;
use colored::Colorize;
use serde_json::json;

use crate::config::Manifest;
use crate::error::CliError;

/// Execute `verum config show`.
///
/// * `json` — emit machine-readable JSON instead of the human-friendly
///   panel.
pub fn execute(json_output: bool) -> Result<(), CliError> {
    // Locate and load verum.toml.
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest_path = Manifest::manifest_path(&manifest_dir);
    let mut manifest = Manifest::from_file(&manifest_path)?;

    // Apply CLI-installed overrides (populated in main.rs).
    crate::feature_overrides::apply_global(&mut manifest)?;

    // Translate into compiler LanguageFeatures + run the validator.
    let features = crate::feature_overrides::manifest_to_features(&manifest)?;

    if json_output {
        print_json(&manifest, &features, &manifest_path)?;
    } else {
        print_human(&manifest, &features, &manifest_path);
    }

    Ok(())
}

fn print_json(
    manifest: &Manifest,
    features: &verum_compiler::language_features::LanguageFeatures,
    manifest_path: &std::path::Path,
) -> Result<(), CliError> {
    let value = json!({
        "source": manifest_path.display().to_string(),
        "cog": {
            "name": manifest.cog.name.as_str(),
            "version": manifest.cog.version.as_str(),
        },
        "profile": format!("{:?}", manifest.language.profile),
        "features": {
            "types": {
                "dependent": features.types.dependent,
                "refinement": features.types.refinement,
                "cubical": features.types.cubical,
                "higher_kinded": features.types.higher_kinded,
                "universe_polymorphism": features.types.universe_polymorphism,
                "coinductive": features.types.coinductive,
                "quotient": features.types.quotient,
                "instance_search": features.types.instance_search,
                "coherence_check_depth": features.types.coherence_check_depth,
            },
            "runtime": {
                "cbgr_mode": features.runtime.cbgr_mode.as_str(),
                "async_scheduler": features.runtime.async_scheduler.as_str(),
                "async_worker_threads": features.runtime.async_worker_threads,
                "futures": features.runtime.futures,
                "nurseries": features.runtime.nurseries,
                "task_stack_size": features.runtime.task_stack_size,
                "heap_policy": features.runtime.heap_policy.as_str(),
                "panic": features.runtime.panic.as_str(),
            },
            "codegen": {
                "tier": features.codegen.tier.as_str(),
                "mlir_gpu": features.codegen.mlir_gpu,
                "gpu_backend": features.codegen.gpu_backend.as_str(),
                "monomorphization_cache": features.codegen.monomorphization_cache,
                "proof_erasure": features.codegen.proof_erasure,
                "debug_info": features.codegen.debug_info.as_str(),
                "tail_call_optimization": features.codegen.tail_call_optimization,
                "vectorize": features.codegen.vectorize,
                "inline_depth": features.codegen.inline_depth,
            },
            "meta": {
                "compile_time_functions": features.meta.compile_time_functions,
                "quote_syntax": features.meta.quote_syntax,
                "macro_recursion_limit": features.meta.macro_recursion_limit,
                "reflection": features.meta.reflection,
                "derive": features.meta.derive,
                "max_stage_level": features.meta.max_stage_level,
            },
            "protocols": {
                "coherence": features.protocols.coherence.as_str(),
                "resolution_strategy": features.protocols.resolution_strategy.as_str(),
                "blanket_impls": features.protocols.blanket_impls,
                "higher_kinded_protocols": features.protocols.higher_kinded_protocols,
                "associated_types": features.protocols.associated_types,
                "generic_associated_types": features.protocols.generic_associated_types,
            },
            "context": {
                "enabled": features.context.enabled,
                "unresolved_policy": features.context.unresolved_policy.as_str(),
                "negative_constraints": features.context.negative_constraints,
                "propagation_depth": features.context.propagation_depth,
            },
            "safety": {
                "unsafe_allowed": features.safety.unsafe_allowed,
                "ffi": features.safety.ffi,
                "ffi_boundary": features.safety.ffi_boundary.as_str(),
                "capability_required": features.safety.capability_required,
                "mls_level": features.safety.mls_level.as_str(),
                "forbid_stdlib_extern": features.safety.forbid_stdlib_extern,
            },
            "test": {
                "differential": features.test.differential,
                "property_testing": features.test.property_testing,
                "proptest_cases": features.test.proptest_cases,
                "fuzzing": features.test.fuzzing,
                "timeout_secs": features.test.timeout_secs,
                "parallel": features.test.parallel,
                "coverage": features.test.coverage,
                "deny_warnings": features.test.deny_warnings,
            },
            "debug": {
                "dap_enabled": features.debug.dap_enabled,
                "step_granularity": features.debug.step_granularity.as_str(),
                "inspect_depth": features.debug.inspect_depth,
                "port": features.debug.port,
                "show_erased_proofs": features.debug.show_erased_proofs,
            },
        },
    });
    println!("{}", serde_json::to_string_pretty(&value)
        .map_err(|e| CliError::Custom(format!("json serialize: {}", e)))?);
    Ok(())
}

/// Execute `verum config validate`.
///
/// Loads and validates verum.toml without printing the full feature set.
/// Exits 0 on success; exits non-zero with diagnostics on failure.
pub fn validate() -> Result<(), CliError> {
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest_path = Manifest::manifest_path(&manifest_dir);
    let mut manifest = Manifest::from_file(&manifest_path)?;
    crate::feature_overrides::apply_global(&mut manifest)?;
    let _features = crate::feature_overrides::manifest_to_features(&manifest)?;
    manifest.validate()?;

    println!(
        "  {} {} is valid.",
        "✓".green().bold(),
        manifest_path.display()
    );
    Ok(())
}

fn print_human(
    manifest: &Manifest,
    features: &verum_compiler::language_features::LanguageFeatures,
    manifest_path: &std::path::Path,
) {
    let hr = "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".cyan();
    println!();
    println!("{}", hr);
    println!("  {}", "Resolved language-feature set".bold().cyan());
    println!("{}", hr);
    println!();
    println!("  Cog:     {} v{}",
        manifest.cog.name.as_str().bold(),
        manifest.cog.version.as_str());
    println!("  Source:  {}", manifest_path.display().to_string().dimmed());
    println!("  Profile: {:?}", manifest.language.profile);
    println!();

    section("types");
    kv_bool("dependent", features.types.dependent);
    kv_bool("refinement", features.types.refinement);
    kv_bool("cubical", features.types.cubical);
    kv_bool("higher_kinded", features.types.higher_kinded);
    kv_bool("universe_polymorphism", features.types.universe_polymorphism);
    kv_bool("coinductive", features.types.coinductive);
    kv_bool("quotient", features.types.quotient);
    kv_bool("instance_search", features.types.instance_search);
    kv_num("coherence_check_depth", features.types.coherence_check_depth as u64);
    println!();

    section("runtime");
    kv_str("cbgr_mode", features.runtime.cbgr_mode.as_str());
    kv_str("async_scheduler", features.runtime.async_scheduler.as_str());
    kv_num("async_worker_threads", features.runtime.async_worker_threads as u64);
    kv_bool("futures", features.runtime.futures);
    kv_bool("nurseries", features.runtime.nurseries);
    kv_str("heap_policy", features.runtime.heap_policy.as_str());
    kv_str("panic", features.runtime.panic.as_str());
    println!();

    section("codegen");
    kv_str("tier", features.codegen.tier.as_str());
    kv_bool("mlir_gpu", features.codegen.mlir_gpu);
    kv_str("gpu_backend", features.codegen.gpu_backend.as_str());
    kv_bool("proof_erasure", features.codegen.proof_erasure);
    kv_bool("tail_call_optimization", features.codegen.tail_call_optimization);
    kv_bool("vectorize", features.codegen.vectorize);
    kv_num("inline_depth", features.codegen.inline_depth as u64);
    kv_str("debug_info", features.codegen.debug_info.as_str());
    println!();

    section("meta");
    kv_bool("compile_time_functions", features.meta.compile_time_functions);
    kv_bool("quote_syntax", features.meta.quote_syntax);
    kv_bool("reflection", features.meta.reflection);
    kv_bool("derive", features.meta.derive);
    kv_num("macro_recursion_limit", features.meta.macro_recursion_limit as u64);
    kv_num("max_stage_level", features.meta.max_stage_level as u64);
    println!();

    section("protocols");
    kv_str("coherence", features.protocols.coherence.as_str());
    kv_str("resolution_strategy", features.protocols.resolution_strategy.as_str());
    kv_bool("blanket_impls", features.protocols.blanket_impls);
    kv_bool("higher_kinded_protocols", features.protocols.higher_kinded_protocols);
    kv_bool("associated_types", features.protocols.associated_types);
    kv_bool("generic_associated_types", features.protocols.generic_associated_types);
    println!();

    section("context");
    kv_bool("enabled", features.context.enabled);
    kv_str("unresolved_policy", features.context.unresolved_policy.as_str());
    kv_bool("negative_constraints", features.context.negative_constraints);
    kv_num("propagation_depth", features.context.propagation_depth as u64);
    println!();

    section("safety");
    kv_bool("unsafe_allowed", features.safety.unsafe_allowed);
    kv_bool("ffi", features.safety.ffi);
    kv_str("ffi_boundary", features.safety.ffi_boundary.as_str());
    kv_bool("capability_required", features.safety.capability_required);
    kv_str("mls_level", features.safety.mls_level.as_str());
    kv_bool("forbid_stdlib_extern", features.safety.forbid_stdlib_extern);
    println!();

    section("test");
    kv_bool("differential", features.test.differential);
    kv_bool("property_testing", features.test.property_testing);
    kv_num("proptest_cases", features.test.proptest_cases as u64);
    kv_bool("fuzzing", features.test.fuzzing);
    kv_num("timeout_secs", features.test.timeout_secs);
    kv_bool("parallel", features.test.parallel);
    kv_bool("coverage", features.test.coverage);
    println!();

    section("debug");
    kv_bool("dap_enabled", features.debug.dap_enabled);
    kv_str("step_granularity", features.debug.step_granularity.as_str());
    kv_num("inspect_depth", features.debug.inspect_depth as u64);
    kv_num("port", features.debug.port as u64);
    println!();

    println!("{}", hr);
    println!("  {} Configuration validated.", "✓".green().bold());
    println!("{}", hr);
    println!();
}

fn section(name: &str) {
    println!("  {} {}", "▸".cyan(), name.bold().underline());
}

fn kv_bool(key: &str, value: bool) {
    let marker = if value { "✓".green() } else { "·".dimmed() };
    let val = if value {
        "enabled".green()
    } else {
        "disabled".red().dimmed()
    };
    println!("    {} {:<28}  {}", marker, key, val);
}

fn kv_str(key: &str, value: &str) {
    println!("    {} {:<28}  {}", "·".dimmed(), key, value.cyan());
}

fn kv_num(key: &str, value: u64) {
    println!("    {} {:<28}  {}", "·".dimmed(), key, value.to_string().cyan());
}
