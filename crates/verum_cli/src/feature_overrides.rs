//! Language-feature overrides applied on top of `verum.toml`.
//!
//! CLI flags in this module let users override any subsection of the
//! manifest without editing the file. Two interfaces are supported:
//!
//! 1. **High-level flags** — the most common toggles (`--tier`, `--gpu`,
//!    `--cbgr`, `--strict`, …). Designed for everyday use.
//!
//! 2. **Generic escape hatch** — `-Z KEY=VALUE` style (mirrors `rustc`).
//!    Any dotted path into the manifest may be overridden (e.g.
//!    `-Z types.universe_polymorphism=true`,
//!    `-Z codegen.inline_depth=5`).
//!
//! Precedence (low → high): built-in defaults < `verum.toml` <
//! high-level flags < generic `-Z` overrides < environment variables.
//!
//! See `integrate-language-features.md` for the full design discussion.
//!
//! ## Example
//!
//! ```bash
//! verum build \
//!   --tier aot --gpu --cbgr checked \
//!   -Z types.universe_polymorphism=true \
//!   -Z runtime.async_worker_threads=8 \
//!   -Z safety.capability_required=true
//! ```

use std::sync::OnceLock;

use clap::Args;
use verum_common::Text;

use crate::config::Manifest;
use crate::error::{CliError, Result};

/// Process-global override set, populated by `main.rs` before the
/// chosen subcommand runs. Downstream commands (build/check/run/test)
/// read this via [`global`] and apply it to the loaded manifest.
static GLOBAL_OVERRIDES: OnceLock<LanguageFeatureOverrides> = OnceLock::new();

/// Install the process-wide override set. Safe to call once; subsequent
/// calls are ignored (the first write wins). Returns `true` on install.
pub fn install(overrides: LanguageFeatureOverrides) -> bool {
    GLOBAL_OVERRIDES.set(overrides).is_ok()
}

/// Access the installed override set, if any.
pub fn global() -> Option<&'static LanguageFeatureOverrides> {
    GLOBAL_OVERRIDES.get()
}

/// Apply globally-installed overrides to a manifest. No-op if none installed.
pub fn apply_global(manifest: &mut Manifest) -> Result<()> {
    if let Some(ov) = global() {
        ov.apply_to(manifest)?;
    }
    Ok(())
}

/// One-shot helper for single-file commands that don't have a
/// `verum.toml`.
///
/// Synthesizes a default Application-profile manifest, applies any
/// installed CLI overrides (`-Z …`, `--no-cubical`, etc.), validates,
/// and returns the resulting [`LanguageFeatures`]. This is the
/// canonical path for `verum run FILE.vr` / `verum check FILE.vr` /
/// `verum build FILE.vr` where no project manifest exists.
///
/// Use this instead of `CompilerOptions::default().language_features`
/// — the latter silently drops every CLI override.
pub fn scratch_features(
) -> Result<verum_compiler::language_features::LanguageFeatures> {
    let mut m = crate::config::create_default_manifest(
        "script",
        false,
        crate::config::LanguageProfile::Application,
    );
    apply_global(&mut m)?;
    manifest_to_features(&m)
}

/// Translate a fully-merged manifest into the compiler-facing
/// [`verum_compiler::language_features::LanguageFeatures`] value.
///
/// Called by the build/check/run/test dispatchers after
/// `apply_global(&mut manifest)` so every compilation sees the same
/// unified feature set. Validation happens inside the compiler, but we
/// surface errors from the CLI for a nicer user experience.
pub fn manifest_to_features(
    manifest: &Manifest,
) -> Result<verum_compiler::language_features::LanguageFeatures> {
    use verum_compiler::language_features as lf;

    let feats = lf::LanguageFeatures {
        types: lf::TypesFeatures {
            dependent: manifest.types.dependent,
            refinement: manifest.types.refinement,
            cubical: manifest.types.cubical,
            higher_kinded: manifest.types.higher_kinded,
            universe_polymorphism: manifest.types.universe_polymorphism,
            coinductive: manifest.types.coinductive,
            quotient: manifest.types.quotient,
            instance_search: manifest.types.instance_search,
            coherence_check_depth: manifest.types.coherence_check_depth,
        },
        runtime: lf::RuntimeFeatures {
            cbgr_mode: manifest.runtime.cbgr_mode.clone(),
            async_scheduler: manifest.runtime.async_scheduler.clone(),
            async_worker_threads: manifest.runtime.async_worker_threads,
            futures: manifest.runtime.futures,
            nurseries: manifest.runtime.nurseries,
            task_stack_size: manifest.runtime.task_stack_size,
            heap_policy: manifest.runtime.heap_policy.clone(),
            panic: manifest.runtime.panic.clone(),
        },
        codegen: lf::CodegenFeatures {
            tier: manifest.codegen.tier.clone(),
            mlir_gpu: manifest.codegen.mlir_gpu,
            gpu_backend: manifest.codegen.gpu_backend.clone(),
            monomorphization_cache: manifest.codegen.monomorphization_cache,
            proof_erasure: manifest.codegen.proof_erasure,
            debug_info: manifest.codegen.debug_info.clone(),
            tail_call_optimization: manifest.codegen.tail_call_optimization,
            vectorize: manifest.codegen.vectorize,
            inline_depth: manifest.codegen.inline_depth,
        },
        meta: lf::MetaFeatures {
            compile_time_functions: manifest.meta.compile_time_functions,
            quote_syntax: manifest.meta.quote_syntax,
            macro_recursion_limit: manifest.meta.macro_recursion_limit,
            reflection: manifest.meta.reflection,
            derive: manifest.meta.derive,
            max_stage_level: manifest.meta.max_stage_level,
        },
        protocols: lf::ProtocolsFeatures {
            coherence: manifest.protocols.coherence.clone(),
            resolution_strategy: manifest.protocols.resolution_strategy.clone(),
            blanket_impls: manifest.protocols.blanket_impls,
            higher_kinded_protocols: manifest.protocols.higher_kinded_protocols,
            associated_types: manifest.protocols.associated_types,
            generic_associated_types: manifest.protocols.generic_associated_types,
        },
        context: lf::ContextFeatures {
            enabled: manifest.context.enabled,
            unresolved_policy: manifest.context.unresolved_policy.clone(),
            negative_constraints: manifest.context.negative_constraints,
            propagation_depth: manifest.context.propagation_depth,
        },
        safety: lf::SafetyFeatures {
            unsafe_allowed: manifest.safety.unsafe_allowed,
            ffi: manifest.safety.ffi,
            ffi_boundary: manifest.safety.ffi_boundary.clone(),
            capability_required: manifest.safety.capability_required,
            mls_level: manifest.safety.mls_level.clone(),
            forbid_stdlib_extern: manifest.safety.forbid_stdlib_extern,
        },
        test: lf::TestFeatures {
            differential: manifest.test.differential,
            property_testing: manifest.test.property_testing,
            proptest_cases: manifest.test.proptest_cases,
            fuzzing: manifest.test.fuzzing,
            timeout_secs: manifest.test.timeout_secs,
            parallel: manifest.test.parallel,
            coverage: manifest.test.coverage,
            deny_warnings: manifest.test.deny_warnings,
        },
        debug: lf::DebugFeatures {
            dap_enabled: manifest.debug.dap_enabled,
            step_granularity: manifest.debug.step_granularity.clone(),
            inspect_depth: manifest.debug.inspect_depth,
            port: manifest.debug.port,
            show_erased_proofs: manifest.debug.show_erased_proofs,
        },
    };

    feats.validate().map_err(|e| {
        // Build a multi-line diagnostic that includes the source file,
        // the offending section/field, and — when available — a
        // "did you mean" suggestion computed by edit distance.
        let mut msg = format!("invalid configuration in verum.toml\n  {}", e);
        if e.provided.is_some() {
            msg.push_str(&format!(
                "\n  hint: edit `[{}]` section in verum.toml, \
                 or override at the CLI with `-Z {}.{}=...`",
                e.section, e.section, e.field
            ));
        }
        CliError::Custom(msg)
    })?;

    Ok(feats)
}

/// CLI-addressable language-feature overrides shared across commands.
///
/// Use `#[clap(flatten)]` to inject these into a subcommand's argument
/// struct. The resulting values get applied to the loaded `Manifest`
/// via [`apply_to`] before the compilation pipeline runs.
#[derive(Debug, Clone, Args, Default)]
pub struct LanguageFeatureOverrides {
    // ------------------------------------------------------------------
    // Codegen
    // ------------------------------------------------------------------
    /// Execution tier (overrides [codegen].tier). Values: interpret|aot|check.
    #[clap(long, value_name = "TIER", help_heading = "Language features")]
    pub tier: Option<Text>,

    /// Enable GPU code generation (MLIR path) for @device(GPU) code.
    #[clap(long, help_heading = "Language features")]
    pub gpu: bool,

    /// Disable GPU code generation (mutually exclusive with --gpu).
    #[clap(long = "no-gpu", help_heading = "Language features", conflicts_with = "gpu")]
    pub no_gpu: bool,

    /// GPU backend: metal|cuda|rocm|vulkan|auto.
    #[clap(long, value_name = "BACKEND", help_heading = "Language features")]
    pub gpu_backend: Option<Text>,

    // ------------------------------------------------------------------
    // Runtime / CBGR
    // ------------------------------------------------------------------
    /// CBGR reference mode: managed|checked|mixed|unsafe.
    #[clap(long, value_name = "MODE", help_heading = "Language features")]
    pub cbgr: Option<Text>,

    /// Async scheduler: single_threaded|multi_threaded|work_stealing.
    #[clap(long, value_name = "SCHED", help_heading = "Language features")]
    pub scheduler: Option<Text>,

    // ------------------------------------------------------------------
    // Types
    // ------------------------------------------------------------------
    /// Disable refinement-type checking (overrides [types].refinement).
    #[clap(long = "no-refinement", help_heading = "Language features")]
    pub no_refinement: bool,

    /// Disable cubical type theory (overrides [types].cubical).
    #[clap(long = "no-cubical", help_heading = "Language features")]
    pub no_cubical: bool,

    /// Disable dependent types entirely.
    #[clap(long = "no-dependent", help_heading = "Language features")]
    pub no_dependent: bool,

    /// Enable universe polymorphism (off by default; performance cost).
    #[clap(long = "universe-poly", help_heading = "Language features")]
    pub universe_poly: bool,

    // ------------------------------------------------------------------
    // Safety
    // ------------------------------------------------------------------
    /// Disallow `unsafe` blocks anywhere in the project.
    #[clap(long = "no-unsafe", help_heading = "Language features")]
    pub no_unsafe: bool,

    /// Require capabilities for sensitive operations (I/O, FFI, unsafe).
    #[clap(long, help_heading = "Language features")]
    pub capabilities: bool,

    /// MLS level: public|secret|top_secret.
    #[clap(long, value_name = "LEVEL", help_heading = "Language features")]
    pub mls: Option<Text>,

    // ------------------------------------------------------------------
    // Metaprogramming
    // ------------------------------------------------------------------
    /// Disable compile-time function evaluation (`meta fn`, `@const`).
    #[clap(long = "no-compile-time", help_heading = "Language features")]
    pub no_compile_time: bool,

    /// Disable `@derive(...)` codegen.
    #[clap(long = "no-derive", help_heading = "Language features")]
    pub no_derive: bool,

    // ------------------------------------------------------------------
    // Debug / DAP
    // ------------------------------------------------------------------
    /// Enable the DAP server during this build (overrides config).
    #[clap(long = "dap", help_heading = "Language features")]
    pub dap: bool,

    /// Disable the DAP server.
    #[clap(long = "no-dap", help_heading = "Language features", conflicts_with = "dap")]
    pub no_dap: bool,

    /// DAP port (0 = auto-pick).
    #[clap(long, value_name = "PORT", help_heading = "Language features")]
    pub dap_port: Option<u16>,

    // ------------------------------------------------------------------
    // Generic escape hatch
    // ------------------------------------------------------------------
    /// Set any manifest value by dotted path (repeatable).
    ///
    /// Example: `-Z types.universe_polymorphism=true`
    ///          `-Z runtime.async_worker_threads=8`
    ///          `-Z safety.mls_level=secret`
    #[clap(
        short = 'Z',
        long = "set",
        value_name = "KEY=VAL",
        help_heading = "Language features",
    )]
    pub raw_overrides: Vec<Text>,
}

impl LanguageFeatureOverrides {
    /// Apply all overrides (high-level + generic) to a loaded manifest.
    ///
    /// High-level flags are applied first, so `-Z` overrides win on
    /// conflict. This matches the documented precedence order.
    pub fn apply_to(&self, manifest: &mut Manifest) -> Result<()> {
        self.apply_high_level(manifest);
        for raw in &self.raw_overrides {
            apply_raw_override(manifest, raw.as_str())?;
        }
        Ok(())
    }

    fn apply_high_level(&self, m: &mut Manifest) {
        // Codegen
        if let Some(t) = &self.tier {
            m.codegen.tier = t.clone();
        }
        if self.gpu {
            m.codegen.mlir_gpu = true;
        }
        if self.no_gpu {
            m.codegen.mlir_gpu = false;
        }
        if let Some(b) = &self.gpu_backend {
            m.codegen.gpu_backend = b.clone();
        }

        // Runtime
        if let Some(c) = &self.cbgr {
            m.runtime.cbgr_mode = c.clone();
        }
        if let Some(s) = &self.scheduler {
            m.runtime.async_scheduler = s.clone();
        }

        // Types
        if self.no_refinement {
            m.types.refinement = false;
        }
        if self.no_cubical {
            m.types.cubical = false;
        }
        if self.no_dependent {
            m.types.dependent = false;
        }
        if self.universe_poly {
            m.types.universe_polymorphism = true;
        }

        // Safety
        if self.no_unsafe {
            m.safety.unsafe_allowed = false;
        }
        if self.capabilities {
            m.safety.capability_required = true;
        }
        if let Some(l) = &self.mls {
            m.safety.mls_level = l.clone();
        }

        // Meta
        if self.no_compile_time {
            m.meta.compile_time_functions = false;
        }
        if self.no_derive {
            m.meta.derive = false;
        }

        // Debug
        if self.dap {
            m.debug.dap_enabled = true;
        }
        if self.no_dap {
            m.debug.dap_enabled = false;
        }
        if let Some(p) = self.dap_port {
            m.debug.port = p;
        }
    }
}

/// Parse `KEY=VALUE` and apply to the manifest by dotted path.
///
/// Supported sections: `types.*`, `runtime.*`, `codegen.*`, `meta.*`,
/// `protocols.*`, `context.*`, `safety.*`, `test.*`, `debug.*`, `verify.*`.
///
/// Unknown keys return a descriptive error so typos surface immediately
/// rather than being silently ignored.
fn apply_raw_override(m: &mut Manifest, raw: &str) -> Result<()> {
    let (key, value) = raw.split_once('=').ok_or_else(|| {
        CliError::Custom(format!(
            "invalid -Z override '{}': expected KEY=VALUE",
            raw
        ))
    })?;
    let key = key.trim();
    let value = value.trim();

    match key {
        // ---------- types ----------
        "types.dependent" => m.types.dependent = parse_bool(key, value)?,
        "types.refinement" => m.types.refinement = parse_bool(key, value)?,
        "types.cubical" => m.types.cubical = parse_bool(key, value)?,
        "types.higher_kinded" => m.types.higher_kinded = parse_bool(key, value)?,
        "types.universe_polymorphism" => {
            m.types.universe_polymorphism = parse_bool(key, value)?
        }
        "types.coinductive" => m.types.coinductive = parse_bool(key, value)?,
        "types.quotient" => m.types.quotient = parse_bool(key, value)?,
        "types.instance_search" => m.types.instance_search = parse_bool(key, value)?,
        "types.coherence_check_depth" => {
            m.types.coherence_check_depth = parse_u32(key, value)?
        }

        // ---------- runtime ----------
        "runtime.cbgr_mode" => m.runtime.cbgr_mode = Text::from(value),
        "runtime.async_scheduler" => m.runtime.async_scheduler = Text::from(value),
        "runtime.async_worker_threads" => {
            m.runtime.async_worker_threads = parse_u32(key, value)?
        }
        "runtime.futures" => m.runtime.futures = parse_bool(key, value)?,
        "runtime.nurseries" => m.runtime.nurseries = parse_bool(key, value)?,
        "runtime.task_stack_size" => m.runtime.task_stack_size = parse_u64(key, value)?,
        "runtime.heap_policy" => m.runtime.heap_policy = Text::from(value),
        "runtime.panic" => m.runtime.panic = Text::from(value),

        // ---------- codegen ----------
        "codegen.tier" => m.codegen.tier = Text::from(value),
        "codegen.mlir_gpu" => m.codegen.mlir_gpu = parse_bool(key, value)?,
        "codegen.gpu_backend" => m.codegen.gpu_backend = Text::from(value),
        "codegen.monomorphization_cache" => {
            m.codegen.monomorphization_cache = parse_bool(key, value)?
        }
        "codegen.proof_erasure" => m.codegen.proof_erasure = parse_bool(key, value)?,
        "codegen.debug_info" => m.codegen.debug_info = Text::from(value),
        "codegen.tail_call_optimization" => {
            m.codegen.tail_call_optimization = parse_bool(key, value)?
        }
        "codegen.vectorize" => m.codegen.vectorize = parse_bool(key, value)?,
        "codegen.inline_depth" => m.codegen.inline_depth = parse_u32(key, value)?,

        // ---------- meta ----------
        "meta.compile_time_functions" => {
            m.meta.compile_time_functions = parse_bool(key, value)?
        }
        "meta.quote_syntax" => m.meta.quote_syntax = parse_bool(key, value)?,
        "meta.macro_recursion_limit" => {
            m.meta.macro_recursion_limit = parse_u32(key, value)?
        }
        "meta.reflection" => m.meta.reflection = parse_bool(key, value)?,
        "meta.derive" => m.meta.derive = parse_bool(key, value)?,
        "meta.max_stage_level" => m.meta.max_stage_level = parse_u32(key, value)?,

        // ---------- protocols ----------
        "protocols.coherence" => m.protocols.coherence = Text::from(value),
        "protocols.resolution_strategy" => {
            m.protocols.resolution_strategy = Text::from(value)
        }
        "protocols.blanket_impls" => m.protocols.blanket_impls = parse_bool(key, value)?,
        "protocols.higher_kinded_protocols" => {
            m.protocols.higher_kinded_protocols = parse_bool(key, value)?
        }
        "protocols.associated_types" => {
            m.protocols.associated_types = parse_bool(key, value)?
        }
        "protocols.generic_associated_types" => {
            m.protocols.generic_associated_types = parse_bool(key, value)?
        }

        // ---------- context ----------
        "context.enabled" => m.context.enabled = parse_bool(key, value)?,
        "context.unresolved_policy" => m.context.unresolved_policy = Text::from(value),
        "context.negative_constraints" => {
            m.context.negative_constraints = parse_bool(key, value)?
        }
        "context.propagation_depth" => {
            m.context.propagation_depth = parse_u32(key, value)?
        }

        // ---------- safety ----------
        "safety.unsafe_allowed" => m.safety.unsafe_allowed = parse_bool(key, value)?,
        "safety.ffi" => m.safety.ffi = parse_bool(key, value)?,
        "safety.ffi_boundary" => m.safety.ffi_boundary = Text::from(value),
        "safety.capability_required" => {
            m.safety.capability_required = parse_bool(key, value)?
        }
        "safety.mls_level" => m.safety.mls_level = Text::from(value),
        "safety.forbid_stdlib_extern" => {
            m.safety.forbid_stdlib_extern = parse_bool(key, value)?
        }

        // ---------- test ----------
        "test.differential" => m.test.differential = parse_bool(key, value)?,
        "test.property_testing" => m.test.property_testing = parse_bool(key, value)?,
        "test.proptest_cases" => m.test.proptest_cases = parse_u32(key, value)?,
        "test.fuzzing" => m.test.fuzzing = parse_bool(key, value)?,
        "test.timeout_secs" => m.test.timeout_secs = parse_u64(key, value)?,
        "test.parallel" => m.test.parallel = parse_bool(key, value)?,
        "test.coverage" => m.test.coverage = parse_bool(key, value)?,
        "test.deny_warnings" => m.test.deny_warnings = parse_bool(key, value)?,

        // ---------- debug ----------
        "debug.dap_enabled" => m.debug.dap_enabled = parse_bool(key, value)?,
        "debug.step_granularity" => m.debug.step_granularity = Text::from(value),
        "debug.inspect_depth" => m.debug.inspect_depth = parse_u32(key, value)?,
        "debug.port" => m.debug.port = parse_u16(key, value)?,
        "debug.show_erased_proofs" => {
            m.debug.show_erased_proofs = parse_bool(key, value)?
        }

        // ---------- verify ----------
        "verify.default_strategy" => {
            m.verify.default_strategy = Text::from(value)
        }
        "verify.solver_timeout_ms" => {
            m.verify.solver_timeout_ms = parse_u64(key, value)?
        }
        "verify.enable_telemetry" => {
            m.verify.enable_telemetry = parse_bool(key, value)?
        }
        "verify.persist_stats" => m.verify.persist_stats = parse_bool(key, value)?,
        "verify.fail_on_divergence" => {
            m.verify.fail_on_divergence = parse_bool(key, value)?
        }

        _ => {
            return Err(CliError::Custom(format!(
                "unknown override key '{}'. \
                 Supported prefixes: types.*, runtime.*, codegen.*, meta.*, \
                 protocols.*, context.*, safety.*, test.*, debug.*, verify.*",
                key
            )));
        }
    }

    Ok(())
}

fn parse_bool(key: &str, value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        _ => Err(CliError::Custom(format!(
            "invalid bool for '{}': '{}' (expected true|false|yes|no|on|off|1|0)",
            key, value
        ))),
    }
}

fn parse_u32(key: &str, value: &str) -> Result<u32> {
    value.parse::<u32>().map_err(|_| {
        CliError::Custom(format!("invalid u32 for '{}': '{}'", key, value))
    })
}

fn parse_u64(key: &str, value: &str) -> Result<u64> {
    value.parse::<u64>().map_err(|_| {
        CliError::Custom(format!("invalid u64 for '{}': '{}'", key, value))
    })
}

fn parse_u16(key: &str, value: &str) -> Result<u16> {
    value.parse::<u16>().map_err(|_| {
        CliError::Custom(format!("invalid u16 for '{}': '{}'", key, value))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        create_default_manifest, LanguageProfile,
    };

    fn manifest() -> Manifest {
        create_default_manifest("test-cog", false, LanguageProfile::Application)
    }

    #[test]
    fn high_level_flags_apply() {
        let ov = LanguageFeatureOverrides {
            tier: Some(Text::from("interpret")),
            gpu: true,
            no_cubical: true,
            universe_poly: true,
            no_unsafe: true,
            ..Default::default()
        };
        let mut m = manifest();
        ov.apply_to(&mut m).unwrap();
        assert_eq!(m.codegen.tier.as_str(), "interpret");
        assert!(m.codegen.mlir_gpu);
        assert!(!m.types.cubical);
        assert!(m.types.universe_polymorphism);
        assert!(!m.safety.unsafe_allowed);
    }

    #[test]
    fn raw_overrides_apply_bool() {
        let ov = LanguageFeatureOverrides {
            raw_overrides: vec![
                Text::from("types.refinement=false"),
                Text::from("safety.capability_required=yes"),
                Text::from("test.fuzzing=on"),
            ],
            ..Default::default()
        };
        let mut m = manifest();
        ov.apply_to(&mut m).unwrap();
        assert!(!m.types.refinement);
        assert!(m.safety.capability_required);
        assert!(m.test.fuzzing);
    }

    #[test]
    fn raw_overrides_apply_numeric() {
        let ov = LanguageFeatureOverrides {
            raw_overrides: vec![
                Text::from("runtime.async_worker_threads=12"),
                Text::from("codegen.inline_depth=7"),
                Text::from("verify.solver_timeout_ms=5000"),
            ],
            ..Default::default()
        };
        let mut m = manifest();
        ov.apply_to(&mut m).unwrap();
        assert_eq!(m.runtime.async_worker_threads, 12);
        assert_eq!(m.codegen.inline_depth, 7);
        assert_eq!(m.verify.solver_timeout_ms, 5000);
    }

    #[test]
    fn raw_override_text_value() {
        let ov = LanguageFeatureOverrides {
            raw_overrides: vec![Text::from("runtime.cbgr_mode=checked")],
            ..Default::default()
        };
        let mut m = manifest();
        ov.apply_to(&mut m).unwrap();
        assert_eq!(m.runtime.cbgr_mode.as_str(), "checked");
    }

    #[test]
    fn unknown_key_errors() {
        let ov = LanguageFeatureOverrides {
            raw_overrides: vec![Text::from("bogus.key=true")],
            ..Default::default()
        };
        let mut m = manifest();
        let err = ov.apply_to(&mut m).unwrap_err();
        assert!(format!("{err}").contains("unknown override key"));
    }

    #[test]
    fn malformed_raw_errors() {
        let ov = LanguageFeatureOverrides {
            raw_overrides: vec![Text::from("no_equals_sign")],
            ..Default::default()
        };
        let mut m = manifest();
        let err = ov.apply_to(&mut m).unwrap_err();
        assert!(format!("{err}").contains("KEY=VALUE"));
    }

    #[test]
    fn invalid_bool_errors() {
        let ov = LanguageFeatureOverrides {
            raw_overrides: vec![Text::from("types.cubical=maybe")],
            ..Default::default()
        };
        let mut m = manifest();
        let err = ov.apply_to(&mut m).unwrap_err();
        assert!(format!("{err}").contains("invalid bool"));
    }

    #[test]
    fn raw_wins_over_high_level() {
        // High-level sets gpu=true; raw override sets it back to false.
        let ov = LanguageFeatureOverrides {
            gpu: true,
            raw_overrides: vec![Text::from("codegen.mlir_gpu=false")],
            ..Default::default()
        };
        let mut m = manifest();
        ov.apply_to(&mut m).unwrap();
        assert!(!m.codegen.mlir_gpu);
    }
}
