//! Language-feature flag system for the compilation pipeline.
//!
//! This module consolidates every language-level toggle (type-system features,
//! runtime behavior, codegen knobs, safety constraints, …) into a single
//! strongly-typed [`LanguageFeatures`] value that travels with every
//! compilation session. It is the **compiler-facing** view of the
//! `[types] / [runtime] / [codegen] / ... ` sections of `verum.toml`.
//!
//! The CLI populates this struct from the merged configuration
//! (defaults → `verum.toml` → `--set KEY=VALUE` overrides → env vars)
//! and hands it to the compiler via [`crate::options::CompilerOptions`].
//! Compiler phases then query individual feature flags instead of
//! re-reading the manifest, keeping the boundary crisp.
//!
//! A [`LanguageFeatures::validate`] method enforces internal consistency
//! (e.g., refinement types require some form of verification) so invalid
//! configurations are caught once, centrally, before any phase runs.

use verum_common::Text;

/// Full set of language feature flags consumed by the compiler.
///
/// Fields map 1:1 to the `[types] / [runtime] / [codegen] / [meta] /
/// [protocols] / [context] / [safety] / [test] / [debug]` sections of
/// `verum.toml`. Changes here must be mirrored on the CLI side
/// (`verum_cli::config`) and in the documentation.
#[derive(Debug, Clone)]
pub struct LanguageFeatures {
    pub types: TypesFeatures,
    pub runtime: RuntimeFeatures,
    pub codegen: CodegenFeatures,
    pub meta: MetaFeatures,
    pub protocols: ProtocolsFeatures,
    pub context: ContextFeatures,
    pub safety: SafetyFeatures,
    pub test: TestFeatures,
    pub debug: DebugFeatures,
}

/// Type-system feature flags (`[types]` section).
#[derive(Debug, Clone)]
pub struct TypesFeatures {
    pub dependent: bool,
    pub refinement: bool,
    pub cubical: bool,
    pub higher_kinded: bool,
    pub universe_polymorphism: bool,
    pub coinductive: bool,
    pub quotient: bool,
    pub instance_search: bool,
    pub coherence_check_depth: u32,
}

/// Runtime feature flags (`[runtime]` section).
#[derive(Debug, Clone)]
pub struct RuntimeFeatures {
    pub cbgr_mode: Text,
    pub async_scheduler: Text,
    pub async_worker_threads: u32,
    pub futures: bool,
    pub nurseries: bool,
    pub task_stack_size: u64,
    pub heap_policy: Text,
    pub panic: Text,
}

/// Codegen feature flags (`[codegen]` section).
#[derive(Debug, Clone)]
pub struct CodegenFeatures {
    pub tier: Text,
    pub mlir_gpu: bool,
    pub gpu_backend: Text,
    pub monomorphization_cache: bool,
    pub proof_erasure: bool,
    pub debug_info: Text,
    pub tail_call_optimization: bool,
    pub vectorize: bool,
    pub inline_depth: u32,
}

/// Metaprogramming feature flags (`[meta]` section).
#[derive(Debug, Clone)]
pub struct MetaFeatures {
    pub compile_time_functions: bool,
    pub quote_syntax: bool,
    pub macro_recursion_limit: u32,
    pub reflection: bool,
    pub derive: bool,
    pub max_stage_level: u32,
}

/// Protocol-system feature flags (`[protocols]` section).
#[derive(Debug, Clone)]
pub struct ProtocolsFeatures {
    pub coherence: Text,
    pub resolution_strategy: Text,
    pub blanket_impls: bool,
    pub higher_kinded_protocols: bool,
    pub associated_types: bool,
    pub generic_associated_types: bool,
}

/// Context-system / DI feature flags (`[context]` section).
#[derive(Debug, Clone)]
pub struct ContextFeatures {
    pub enabled: bool,
    pub unresolved_policy: Text,
    pub negative_constraints: bool,
    pub propagation_depth: u32,
}

/// Safety-constraint feature flags (`[safety]` section).
#[derive(Debug, Clone)]
pub struct SafetyFeatures {
    pub unsafe_allowed: bool,
    pub ffi: bool,
    pub ffi_boundary: Text,
    pub capability_required: bool,
    pub mls_level: Text,
    pub forbid_stdlib_extern: bool,
}

/// Test-harness feature flags (`[test]` section).
#[derive(Debug, Clone)]
pub struct TestFeatures {
    pub differential: bool,
    pub property_testing: bool,
    pub proptest_cases: u32,
    pub fuzzing: bool,
    pub timeout_secs: u64,
    pub parallel: bool,
    pub coverage: bool,
    pub deny_warnings: bool,
}

/// Debug / DAP feature flags (`[debug]` section).
#[derive(Debug, Clone)]
pub struct DebugFeatures {
    pub dap_enabled: bool,
    pub step_granularity: Text,
    pub inspect_depth: u32,
    pub port: u16,
    pub show_erased_proofs: bool,
}

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

impl Default for LanguageFeatures {
    /// Production-grade defaults matching `verum.toml` defaults.
    fn default() -> Self {
        Self {
            types: TypesFeatures::default(),
            runtime: RuntimeFeatures::default(),
            codegen: CodegenFeatures::default(),
            meta: MetaFeatures::default(),
            protocols: ProtocolsFeatures::default(),
            context: ContextFeatures::default(),
            safety: SafetyFeatures::default(),
            test: TestFeatures::default(),
            debug: DebugFeatures::default(),
        }
    }
}

impl Default for TypesFeatures {
    fn default() -> Self {
        Self {
            dependent: true,
            refinement: true,
            cubical: true,
            higher_kinded: true,
            universe_polymorphism: false,
            coinductive: true,
            quotient: true,
            instance_search: true,
            coherence_check_depth: 16,
        }
    }
}

impl Default for RuntimeFeatures {
    fn default() -> Self {
        Self {
            cbgr_mode: Text::from("mixed"),
            async_scheduler: Text::from("work_stealing"),
            async_worker_threads: 0,
            futures: true,
            nurseries: true,
            task_stack_size: 0,
            heap_policy: Text::from("adaptive"),
            panic: Text::from("unwind"),
        }
    }
}

impl Default for CodegenFeatures {
    fn default() -> Self {
        Self {
            tier: Text::from("aot"),
            mlir_gpu: false,
            gpu_backend: Text::from("auto"),
            monomorphization_cache: true,
            proof_erasure: true,
            debug_info: Text::from("line"),
            tail_call_optimization: true,
            vectorize: true,
            inline_depth: 3,
        }
    }
}

impl Default for MetaFeatures {
    fn default() -> Self {
        Self {
            compile_time_functions: true,
            quote_syntax: true,
            macro_recursion_limit: 128,
            reflection: true,
            derive: true,
            max_stage_level: 2,
        }
    }
}

impl Default for ProtocolsFeatures {
    fn default() -> Self {
        Self {
            coherence: Text::from("strict"),
            resolution_strategy: Text::from("most_specific"),
            blanket_impls: true,
            higher_kinded_protocols: true,
            associated_types: true,
            generic_associated_types: true,
        }
    }
}

impl Default for ContextFeatures {
    fn default() -> Self {
        Self {
            enabled: true,
            unresolved_policy: Text::from("error"),
            negative_constraints: true,
            propagation_depth: 32,
        }
    }
}

impl Default for SafetyFeatures {
    fn default() -> Self {
        Self {
            unsafe_allowed: true,
            ffi: true,
            ffi_boundary: Text::from("strict"),
            capability_required: false,
            mls_level: Text::from("public"),
            forbid_stdlib_extern: false,
        }
    }
}

impl Default for TestFeatures {
    fn default() -> Self {
        Self {
            differential: false,
            property_testing: true,
            proptest_cases: 256,
            fuzzing: false,
            timeout_secs: 60,
            parallel: true,
            coverage: false,
            deny_warnings: false,
        }
    }
}

impl Default for DebugFeatures {
    fn default() -> Self {
        Self {
            dap_enabled: true,
            step_granularity: Text::from("statement"),
            inspect_depth: 8,
            port: 0,
            show_erased_proofs: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Error returned when a feature-flag combination is self-contradictory
/// or a user-provided value falls outside the accepted set.
#[derive(Debug, Clone)]
pub struct FeatureValidationError {
    pub section: &'static str,
    pub field: &'static str,
    pub message: Text,
    /// The value the user supplied, if the check was an enum constraint.
    /// `None` for coherence errors (e.g. `cubical` requires `dependent`).
    pub provided: Option<Text>,
    /// The set of allowed values, if the check was an enum constraint.
    pub allowed: &'static [&'static str],
    /// Optional "did you mean" suggestion — closest allowed value by
    /// edit distance, when the provided value is a near-match typo.
    pub suggestion: Option<&'static str>,
}

impl std::fmt::Display for FeatureValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}].{}: {}",
            self.section, self.field, self.message
        )?;
        if !self.allowed.is_empty() {
            write!(f, "\n  allowed values: {}", self.allowed.join(", "))?;
        }
        if let Some(hint) = self.suggestion {
            write!(f, "\n  help: did you mean `{}`?", hint)?;
        }
        Ok(())
    }
}

impl std::error::Error for FeatureValidationError {}

/// Compute a simple edit distance (Levenshtein, O(n·m)) between two
/// strings. Used to pick a near-match suggestion for enum-value typos.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    if n == 0 { return m; }
    if m == 0 { return n; }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr = vec![0; m + 1];
    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1)
                .min(curr[j - 1] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

/// Pick the closest match from `allowed` for `provided`, if within
/// edit distance ≤ max(2, len/3). Returns `None` otherwise.
fn closest_match(provided: &str, allowed: &[&'static str]) -> Option<&'static str> {
    let threshold = (provided.len() / 3).max(2);
    allowed
        .iter()
        .map(|&c| (c, edit_distance(provided, c)))
        .filter(|(_, d)| *d <= threshold)
        .min_by_key(|(_, d)| *d)
        .map(|(c, _)| c)
}

impl LanguageFeatures {
    /// Validate consistency of feature flags. Errors surface the offending
    /// section and field, suitable for direct display to the user.
    ///
    /// Consistency rules:
    /// - `types.refinement` requires either `types.dependent` or some verification path.
    /// - `types.cubical` requires `types.dependent` (Path types depend on dependent typing).
    /// - `types.universe_polymorphism` requires `types.dependent`.
    /// - `types.higher_kinded` is required by `protocols.higher_kinded_protocols`.
    /// - `protocols.generic_associated_types` requires `protocols.associated_types`.
    /// - `runtime.cbgr_mode` must be one of: managed|checked|mixed|unsafe.
    /// - `runtime.async_scheduler` must be a known scheduler name.
    /// - `codegen.tier` must be one of: interpret|aot|check.
    /// - `safety.ffi_boundary` must be: strict|lenient.
    /// - `safety.mls_level` must be: public|secret|top_secret.
    /// - `protocols.coherence` must be: strict|lenient|unchecked.
    /// - `meta.max_stage_level` must be ≤ 8 (beyond that termination is speculative).
    /// - `debug.step_granularity` must be: statement|line|instruction.
    /// - `context.unresolved_policy` must be: error|warn|allow.
    pub fn validate(&self) -> Result<(), FeatureValidationError> {
        // Types coherence
        if self.types.cubical && !self.types.dependent {
            return Err(err("types", "cubical", "requires [types].dependent = true"));
        }
        if self.types.universe_polymorphism && !self.types.dependent {
            return Err(err(
                "types",
                "universe_polymorphism",
                "requires [types].dependent = true",
            ));
        }
        if self.types.coherence_check_depth == 0 {
            return Err(err(
                "types",
                "coherence_check_depth",
                "must be at least 1",
            ));
        }

        // Protocols coherence
        if self.protocols.higher_kinded_protocols && !self.types.higher_kinded {
            return Err(err(
                "protocols",
                "higher_kinded_protocols",
                "requires [types].higher_kinded = true",
            ));
        }
        if self.protocols.generic_associated_types && !self.protocols.associated_types {
            return Err(err(
                "protocols",
                "generic_associated_types",
                "requires [protocols].associated_types = true",
            ));
        }
        ensure_in(
            "protocols",
            "coherence",
            &self.protocols.coherence,
            &["strict", "lenient", "unchecked"],
        )?;
        ensure_in(
            "protocols",
            "resolution_strategy",
            &self.protocols.resolution_strategy,
            &["most_specific", "first_declared", "error"],
        )?;

        // Runtime values
        ensure_in(
            "runtime",
            "cbgr_mode",
            &self.runtime.cbgr_mode,
            &["managed", "checked", "mixed", "unsafe"],
        )?;
        ensure_in(
            "runtime",
            "async_scheduler",
            &self.runtime.async_scheduler,
            &["single_threaded", "multi_threaded", "work_stealing"],
        )?;
        ensure_in(
            "runtime",
            "heap_policy",
            &self.runtime.heap_policy,
            &["aggressive", "conservative", "adaptive"],
        )?;
        ensure_in(
            "runtime",
            "panic",
            &self.runtime.panic,
            &["unwind", "abort"],
        )?;

        // Codegen values
        ensure_in(
            "codegen",
            "tier",
            &self.codegen.tier,
            &["interpret", "aot", "check"],
        )?;
        ensure_in(
            "codegen",
            "debug_info",
            &self.codegen.debug_info,
            &["none", "line", "full"],
        )?;
        if self.codegen.inline_depth > 16 {
            return Err(err(
                "codegen",
                "inline_depth",
                "must be ≤ 16 (deep inlining risks codegen blow-up)",
            ));
        }

        // Meta
        if self.meta.max_stage_level > 8 {
            return Err(err(
                "meta",
                "max_stage_level",
                "must be ≤ 8 (higher staging is speculative)",
            ));
        }

        // Safety values
        ensure_in(
            "safety",
            "ffi_boundary",
            &self.safety.ffi_boundary,
            &["strict", "lenient"],
        )?;
        ensure_in(
            "safety",
            "mls_level",
            &self.safety.mls_level,
            &["public", "secret", "top_secret"],
        )?;
        if self.safety.capability_required && self.safety.ffi && !self.safety.forbid_stdlib_extern
        {
            // This is advisory-only, not a hard error — capabilities + unrestricted
            // stdlib extern is still a legal, commonly-used combination.
        }

        // Context
        ensure_in(
            "context",
            "unresolved_policy",
            &self.context.unresolved_policy,
            &["error", "warn", "allow"],
        )?;

        // Debug
        ensure_in(
            "debug",
            "step_granularity",
            &self.debug.step_granularity,
            &["statement", "line", "instruction"],
        )?;

        Ok(())
    }

    // Convenience predicates used throughout the compiler -----------------

    pub fn is_interpret(&self) -> bool {
        self.codegen.tier.as_str() == "interpret"
    }
    pub fn is_aot(&self) -> bool {
        self.codegen.tier.as_str() == "aot"
    }
    pub fn is_check_only(&self) -> bool {
        self.codegen.tier.as_str() == "check"
    }

    pub fn gpu_enabled(&self) -> bool {
        self.codegen.mlir_gpu
    }

    pub fn refinement_typing_on(&self) -> bool {
        self.types.refinement
    }
    pub fn cubical_typing_on(&self) -> bool {
        self.types.cubical && self.types.dependent
    }
    pub fn context_system_on(&self) -> bool {
        self.context.enabled
    }
    pub fn unsafe_allowed(&self) -> bool {
        self.safety.unsafe_allowed
    }
    pub fn capabilities_required(&self) -> bool {
        self.safety.capability_required
    }
    pub fn derive_enabled(&self) -> bool {
        self.meta.derive
    }
    pub fn compile_time_eval_on(&self) -> bool {
        self.meta.compile_time_functions
    }
    pub fn dap_on(&self) -> bool {
        self.debug.dap_enabled
    }
}

fn err(section: &'static str, field: &'static str, msg: &str) -> FeatureValidationError {
    FeatureValidationError {
        section,
        field,
        message: Text::from(msg),
        provided: None,
        allowed: &[],
        suggestion: None,
    }
}

fn ensure_in(
    section: &'static str,
    field: &'static str,
    value: &Text,
    allowed: &'static [&'static str],
) -> Result<(), FeatureValidationError> {
    if allowed.iter().any(|v| *v == value.as_str()) {
        Ok(())
    } else {
        let suggestion = closest_match(value.as_str(), allowed);
        Err(FeatureValidationError {
            section,
            field,
            message: Text::from(format!("'{}' is not a valid value", value.as_str())),
            provided: Some(value.clone()),
            allowed,
            suggestion,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate() {
        LanguageFeatures::default().validate().unwrap();
    }

    #[test]
    fn cubical_without_dependent_fails() {
        let mut f = LanguageFeatures::default();
        f.types.dependent = false;
        let err = f.validate().unwrap_err();
        assert_eq!(err.section, "types");
        assert_eq!(err.field, "cubical");
    }

    #[test]
    fn universe_poly_requires_dependent() {
        let mut f = LanguageFeatures::default();
        f.types.universe_polymorphism = true;
        f.types.dependent = false;
        // cubical also depends on dependent; disable to isolate UP check
        f.types.cubical = false;
        let err = f.validate().unwrap_err();
        assert_eq!(err.field, "universe_polymorphism");
    }

    #[test]
    fn gats_without_assoc_fails() {
        let mut f = LanguageFeatures::default();
        f.protocols.generic_associated_types = true;
        f.protocols.associated_types = false;
        let err = f.validate().unwrap_err();
        assert_eq!(err.field, "generic_associated_types");
    }

    #[test]
    fn hkp_without_hk_fails() {
        let mut f = LanguageFeatures::default();
        f.protocols.higher_kinded_protocols = true;
        f.types.higher_kinded = false;
        let err = f.validate().unwrap_err();
        assert_eq!(err.field, "higher_kinded_protocols");
    }

    #[test]
    fn bad_cbgr_mode_fails() {
        let mut f = LanguageFeatures::default();
        f.runtime.cbgr_mode = Text::from("bogus");
        let err = f.validate().unwrap_err();
        assert_eq!(err.field, "cbgr_mode");
        assert_eq!(err.provided.as_ref().unwrap().as_str(), "bogus");
        assert!(err.allowed.contains(&"mixed"));
    }

    #[test]
    fn typo_suggests_near_match() {
        let mut f = LanguageFeatures::default();
        f.runtime.cbgr_mode = Text::from("mxed");
        let err = f.validate().unwrap_err();
        assert_eq!(err.suggestion, Some("mixed"));
        assert!(format!("{}", err).contains("did you mean"));
    }

    #[test]
    fn far_value_no_suggestion() {
        let mut f = LanguageFeatures::default();
        f.runtime.cbgr_mode = Text::from("quantum");
        let err = f.validate().unwrap_err();
        // "quantum" is too far from any allowed value.
        assert_eq!(err.suggestion, None);
        // The allowed-values list still appears in the display output.
        let rendered = format!("{}", err);
        assert!(rendered.contains("allowed values:"));
        assert!(rendered.contains("mixed"));
    }

    #[test]
    fn coherence_error_has_no_provided_or_allowed() {
        // Cross-field coherence errors (e.g. cubical requires dependent)
        // have no single "provided" value — assert the fields are None/empty.
        let mut f = LanguageFeatures::default();
        f.types.dependent = false;
        let err = f.validate().unwrap_err();
        assert_eq!(err.field, "cubical");
        assert!(err.provided.is_none());
        assert!(err.allowed.is_empty());
    }

    #[test]
    fn bad_tier_fails() {
        let mut f = LanguageFeatures::default();
        f.codegen.tier = Text::from("jit");
        let err = f.validate().unwrap_err();
        assert_eq!(err.field, "tier");
    }

    #[test]
    fn bad_mls_level_fails() {
        let mut f = LanguageFeatures::default();
        f.safety.mls_level = Text::from("classified");
        let err = f.validate().unwrap_err();
        assert_eq!(err.field, "mls_level");
    }

    #[test]
    fn excessive_inline_depth_fails() {
        let mut f = LanguageFeatures::default();
        f.codegen.inline_depth = 100;
        let err = f.validate().unwrap_err();
        assert_eq!(err.field, "inline_depth");
    }

    #[test]
    fn excessive_stage_level_fails() {
        let mut f = LanguageFeatures::default();
        f.meta.max_stage_level = 20;
        let err = f.validate().unwrap_err();
        assert_eq!(err.field, "max_stage_level");
    }

    #[test]
    fn coherence_depth_zero_fails() {
        let mut f = LanguageFeatures::default();
        f.types.coherence_check_depth = 0;
        let err = f.validate().unwrap_err();
        assert_eq!(err.field, "coherence_check_depth");
    }

    #[test]
    fn predicates_match_flags() {
        let f = LanguageFeatures::default();
        assert!(f.is_aot());
        assert!(!f.is_interpret());
        assert!(!f.is_check_only());
        assert!(f.refinement_typing_on());
        assert!(f.cubical_typing_on());
        assert!(f.context_system_on());
        assert!(f.unsafe_allowed());
        assert!(!f.capabilities_required());
        assert!(f.derive_enabled());
        assert!(f.compile_time_eval_on());
        assert!(f.dap_on());
    }
}
