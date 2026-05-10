//! LSP-wide configuration driven by the client's `initializationOptions`.
//!

//! Every knob surfaced in the VS Code extension's `package.json` (and the
//! settings snippet in `docs/detailed/25-developer-tooling.md §3.12`) lands
//! here. Components that need the value (refinement validator, CBGR hints,
//! diagnostics) read from a shared `LspConfig` via interior mutability, so
//! we can update it from the `initialize` handler without `&mut self` on
//! the `LanguageServer` trait.
//!

//! All fields have sensible defaults: a freshly constructed `LspConfig`
//! reproduces the old hard-coded behaviour, so existing code paths continue
//! to work even when the client sends no init options.

use serde::{Deserialize, Serialize};
use std::sync::RwLock;
use std::time::Duration;

/// Validation mode requested by the client — caps per-call SMT latency.
///
/// Canonical home for the validator/initialize-handler/JSON-wire
/// surface.  `refinement_validation::ValidationMode` re-exports
/// this type rather than defining its own — pre-collapse the two
/// were structural duplicates that an identity-shape mapper at the
/// `apply_config` boundary kept in sync, with a per-mode
/// `cfg.smt_timeout` fallback baked into the `Complete` arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValidationMode {
    /// `<100ms` — suitable for on-type validation.
    Quick,
    /// `<1s` — suitable for on-save validation.
    Thorough,
    /// Unlimited — reserved for CI/CD invocations that don't need
    /// to fit inside a typical editor budget.  Per-call timeout
    /// falls back to the configured `cfg.smt_timeout` (typically
    /// tens of seconds) instead of a hardcoded short bound.  Pre-
    /// fix the validator's `mode_to_timeout` silently normalised
    /// `Complete` to `Thorough`, so clients that set
    /// `verum.lsp.validationMode = "complete"` got the 1 s
    /// Thorough timeout instead of the documented unlimited
    /// latency.
    Complete,
}

/// Static fact-pack for a [`ValidationMode`] — pinned via the
/// `meta_pin_validation_mode` drift test so the
/// editor-budget / on-save-budget / CI-budget classification is
/// kept in lock-step with `mode_to_timeout` consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidationModeMeta {
    /// Canonical lowercase wire form — matches the JSON
    /// `verum.lsp.validationMode` value.  Same surface as
    /// `as_str()`.
    pub name: &'static str,
    /// Hard upper bound on per-call validator latency.  `None`
    /// for unlimited (Complete falls back to the runtime-
    /// configured `cfg.smt_timeout`).
    pub fixed_timeout: Option<Duration>,
    /// Whether this mode is suitable for the *on-type*
    /// validation path (the keystroke-budget surface).
    pub fits_on_type_budget: bool,
    /// Whether this mode is suitable for the *on-save* path.
    pub fits_on_save_budget: bool,
    /// Whether this mode unlocks CI-grade verification —
    /// unlimited timeout, intended for batch invocations.
    pub is_ci_grade: bool,
}

impl ValidationMode {
    /// All variants in declaration order — drives drift-pin
    /// tests and any downstream surface that enumerates the
    /// full mode set.
    pub const ALL: &'static [ValidationMode] = &[
        ValidationMode::Quick,
        ValidationMode::Thorough,
        ValidationMode::Complete,
    ];

    /// Static fact-pack — the partition table behind
    /// classification consumers (timeout selector, IDE warning
    /// gate, CI ramp).
    pub const fn meta(self) -> ValidationModeMeta {
        match self {
            ValidationMode::Quick => ValidationModeMeta {
                name: "quick",
                fixed_timeout: Some(Duration::from_millis(100)),
                fits_on_type_budget: true,
                fits_on_save_budget: true,
                is_ci_grade: false,
            },
            ValidationMode::Thorough => ValidationModeMeta {
                name: "thorough",
                fixed_timeout: Some(Duration::from_secs(1)),
                fits_on_type_budget: false,
                fits_on_save_budget: true,
                is_ci_grade: false,
            },
            ValidationMode::Complete => ValidationModeMeta {
                name: "complete",
                fixed_timeout: None,
                fits_on_type_budget: false,
                fits_on_save_budget: false,
                is_ci_grade: true,
            },
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        ValidationMode::ALL
            .iter()
            .copied()
            .find(|m| m.meta().name == s)
    }

    pub fn as_str(self) -> &'static str {
        self.meta().name
    }

    /// Returns the hard upper bound on per-call validator
    /// latency, or a 10-minute placeholder for `Complete` so the
    /// legacy `Duration`-returning surface still works for
    /// callers that don't have access to a `cfg.smt_timeout`
    /// fallback.  Prefer reading `meta().fixed_timeout` directly
    /// when the caller can do its own None-handling.
    pub fn timeout(self) -> Duration {
        self.meta()
            .fixed_timeout
            .unwrap_or(Duration::from_secs(600))
    }
}

/// SMT solver selection driven by `verum.lsp.smtSolver`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtSolverChoice {
    Z3,
    Cvc5,
    Auto,
}

impl SmtSolverChoice {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "z3" => Some(Self::Z3),
            "cvc5" => Some(Self::Cvc5),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Z3 => "Z3",
            Self::Cvc5 => "CVC5",
            Self::Auto => "Auto",
        }
    }
}

/// All LSP-side knobs exposed by the client.
///

/// Field names mirror the `initializationOptions` keys the VS Code extension
/// sends in `extension.ts::startLanguageClient`. Unknown / missing keys fall
/// back to `Default` values, so older clients remain forward-compatible.
#[derive(Debug, Clone)]
pub struct LspConfig {
    // ── Refinement validation ───────────────────────────────────────────
    pub enable_refinement_validation: bool,
    pub validation_mode: ValidationMode,
    pub show_counterexamples: bool,
    pub max_counterexample_traces: u32,

    // ── SMT solver ───────────────────────────────────────────────────────
    pub smt_solver: SmtSolverChoice,
    pub smt_timeout: Duration,

    // ── Cache ────────────────────────────────────────────────────────────
    pub cache_validation_results: bool,
    pub cache_ttl: Duration,
    pub cache_max_entries: usize,

    // ── CBGR ─────────────────────────────────────────────────────────────
    pub cbgr_enable_profiling: bool,
    pub cbgr_show_optimization_hints: bool,

    // ── Verification cost feedback ───────────────────────────────────────
    pub verification_show_cost_warnings: bool,
    pub verification_slow_threshold: Duration,

    // ── Lint integration ────────────────────────────────────────────────
    /// Whether `verum lint` runs on diagnostic publication. Default
    /// is `true`; flip via `verum.lint.enabled = false` in the
    /// editor settings.
    pub lint_enabled: bool,
    /// Optional `--profile NAME` override forwarded to the linter.
    pub lint_profile: Option<String>,
    /// Path to the `verum` binary. `None` resolves to PATH lookup.
    pub lint_binary: Option<std::path::PathBuf>,

    // ── Format integration ──────────────────────────────────────────────
    /// Whether `verum fmt` is the source of truth for
    /// `textDocument/formatting`. Default is `true`; flip to
    /// `false` to fall back to the in-LSP formatter.
    pub fmt_enabled: bool,
    /// Path to the `verum` binary used for formatting. `None`
    /// resolves to PATH lookup; a separate setting from
    /// `lint_binary` lets one editor experiment with a custom
    /// formatter while keeping the canonical linter.
    pub fmt_binary: Option<std::path::PathBuf>,
}

impl Default for LspConfig {
    fn default() -> Self {
        Self {
            enable_refinement_validation: true,
            validation_mode: ValidationMode::Quick,
            show_counterexamples: true,
            max_counterexample_traces: 5,

            smt_solver: SmtSolverChoice::Z3,
            smt_timeout: Duration::from_millis(50),

            cache_validation_results: true,
            cache_ttl: Duration::from_secs(300),
            cache_max_entries: 1000,

            cbgr_enable_profiling: false,
            cbgr_show_optimization_hints: false,

            verification_show_cost_warnings: true,
            verification_slow_threshold: Duration::from_millis(5_000),

            lint_enabled: true,
            lint_profile: None,
            lint_binary: None,

            fmt_enabled: true,
            fmt_binary: None,
        }
    }
}

impl LspConfig {
    /// Merge values from a JSON `initializationOptions` blob. Unknown or
    /// ill-typed keys are silently ignored — we're forgiving on input so a
    /// slightly out-of-sync extension never crashes the server.
    pub fn apply_json(&mut self, opts: &serde_json::Value) {
        if let Some(v) = opts
            .get("enableRefinementValidation")
            .and_then(|v| v.as_bool())
        {
            self.enable_refinement_validation = v;
        }
        if let Some(v) = opts.get("validationMode").and_then(|v| v.as_str()) {
            if let Some(mode) = ValidationMode::from_str(v) {
                self.validation_mode = mode;
            }
        }
        if let Some(v) = opts.get("showCounterexamples").and_then(|v| v.as_bool()) {
            self.show_counterexamples = v;
        }
        if let Some(v) = opts.get("maxCounterexampleTraces").and_then(|v| v.as_u64()) {
            self.max_counterexample_traces = v.min(u32::MAX as u64) as u32;
        }

        if let Some(v) = opts.get("smtSolver").and_then(|v| v.as_str()) {
            if let Some(solver) = SmtSolverChoice::from_str(v) {
                self.smt_solver = solver;
            }
        }
        if let Some(v) = opts.get("smtTimeout").and_then(|v| v.as_u64()) {
            self.smt_timeout = Duration::from_millis(v);
        }

        if let Some(v) = opts.get("cacheValidationResults").and_then(|v| v.as_bool()) {
            self.cache_validation_results = v;
        }
        if let Some(v) = opts.get("cacheTtlSeconds").and_then(|v| v.as_u64()) {
            self.cache_ttl = Duration::from_secs(v);
        }
        if let Some(v) = opts.get("cacheMaxEntries").and_then(|v| v.as_u64()) {
            self.cache_max_entries = v as usize;
        }

        if let Some(v) = opts.get("cbgrEnableProfiling").and_then(|v| v.as_bool()) {
            self.cbgr_enable_profiling = v;
        }
        if let Some(v) = opts
            .get("cbgrShowOptimizationHints")
            .and_then(|v| v.as_bool())
        {
            self.cbgr_show_optimization_hints = v;
        }

        if let Some(v) = opts
            .get("verificationShowCostWarnings")
            .and_then(|v| v.as_bool())
        {
            self.verification_show_cost_warnings = v;
        }
        if let Some(v) = opts
            .get("verificationSlowThresholdMs")
            .and_then(|v| v.as_u64())
        {
            self.verification_slow_threshold = Duration::from_millis(v);
        }

        // Lint integration knobs. Both `lint.<key>` (preferred) and
        // the flat `lint<Key>` form are accepted so older clients
        // don't need to migrate.
        let lint_section = opts.get("lint");
        if let Some(v) = lint_section
            .and_then(|s| s.get("enabled"))
            .and_then(|v| v.as_bool())
            .or_else(|| opts.get("lintEnabled").and_then(|v| v.as_bool()))
        {
            self.lint_enabled = v;
        }
        if let Some(v) = lint_section
            .and_then(|s| s.get("profile"))
            .and_then(|v| v.as_str())
            .or_else(|| opts.get("lintProfile").and_then(|v| v.as_str()))
        {
            self.lint_profile = Some(v.to_string());
        }
        if let Some(v) = lint_section
            .and_then(|s| s.get("binary"))
            .and_then(|v| v.as_str())
            .or_else(|| opts.get("lintBinary").and_then(|v| v.as_str()))
        {
            self.lint_binary = Some(std::path::PathBuf::from(v));
        }

        // Format integration knobs — same pattern, separate
        // namespace so the user can flip the formatter independently
        // of the linter.
        let fmt_section = opts.get("fmt").or_else(|| opts.get("format"));
        if let Some(v) = fmt_section
            .and_then(|s| s.get("enabled"))
            .and_then(|v| v.as_bool())
            .or_else(|| opts.get("fmtEnabled").and_then(|v| v.as_bool()))
        {
            self.fmt_enabled = v;
        }
        if let Some(v) = fmt_section
            .and_then(|s| s.get("binary"))
            .and_then(|v| v.as_str())
            .or_else(|| opts.get("fmtBinary").and_then(|v| v.as_str()))
        {
            self.fmt_binary = Some(std::path::PathBuf::from(v));
        }
    }
}

/// Thread-safe shared view for components that read the config.
///

/// Uses `RwLock` so concurrent reads (every hover / validate call) are
/// lock-free after initialization; writes happen only from `initialize` and
/// `workspace/didChangeConfiguration`.
#[derive(Debug, Default)]
pub struct SharedLspConfig {
    inner: RwLock<LspConfig>,
}

impl SharedLspConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot the current config — cheap; clones once.
    pub fn snapshot(&self) -> LspConfig {
        self.inner.read().expect("lsp config poisoned").clone()
    }

    /// Apply JSON overrides to the shared config.
    pub fn apply_json(&self, opts: &serde_json::Value) {
        let mut guard = self.inner.write().expect("lsp config poisoned");
        guard.apply_json(opts);
    }

    /// Replace the entire config (used by tests).
    pub fn set(&self, cfg: LspConfig) {
        *self.inner.write().expect("lsp config poisoned") = cfg;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Drift-pin: `ValidationMode` is the canonical home for the
    /// validator/initialize-handler/JSON-wire surface — pre-
    /// collapse the same 3-variant enum lived in
    /// `refinement_validation` too, kept in sync via an
    /// identity-shape mapper.  This test pins the partition table
    /// + wire-form round-trip + alias contract.
    #[test]
    fn meta_pin_validation_mode_round_trip_and_partitions() {
        // 1. Variant count + names + as_str round-trip.
        assert_eq!(ValidationMode::ALL.len(), 3);
        let names: Vec<_> =
            ValidationMode::ALL.iter().map(|m| m.meta().name).collect();
        assert_eq!(names, vec!["quick", "thorough", "complete"]);

        for m in ValidationMode::ALL {
            assert_eq!(ValidationMode::from_str(m.as_str()), Some(*m));
            assert_eq!(m.as_str(), m.meta().name);
        }
        assert_eq!(ValidationMode::from_str("nope"), None);

        // 2. Editor-budget partition — `Quick` is the singleton
        //    that fits the on-type budget; `Thorough` and
        //    `Quick` both fit on-save; `Complete` is the
        //    singleton CI-grade band.
        let on_type: Vec<_> = ValidationMode::ALL
            .iter()
            .filter(|m| m.meta().fits_on_type_budget)
            .copied()
            .collect();
        assert_eq!(on_type, vec![ValidationMode::Quick]);

        let on_save: Vec<_> = ValidationMode::ALL
            .iter()
            .filter(|m| m.meta().fits_on_save_budget)
            .copied()
            .collect();
        assert_eq!(
            on_save,
            vec![ValidationMode::Quick, ValidationMode::Thorough],
        );

        let ci: Vec<_> = ValidationMode::ALL
            .iter()
            .filter(|m| m.meta().is_ci_grade)
            .copied()
            .collect();
        assert_eq!(ci, vec![ValidationMode::Complete]);

        // 3. Cross-cutting: only `Complete` has unlimited
        //    timeout (None).  The other two carry concrete
        //    Duration bounds.
        for m in ValidationMode::ALL {
            let meta = m.meta();
            assert_eq!(
                meta.fixed_timeout.is_none(),
                meta.is_ci_grade,
                "{:?}: unlimited-timeout flag must equal CI-grade flag",
                m,
            );
        }

        // 4. JSON wire form is lowercase per
        //    `serde(rename_all = "lowercase")`.
        let q: ValidationMode = serde_json::from_str("\"quick\"").unwrap();
        assert_eq!(q, ValidationMode::Quick);
        let c: ValidationMode = serde_json::from_str("\"complete\"").unwrap();
        assert_eq!(c, ValidationMode::Complete);

        // 5. timeout() agrees with meta().fixed_timeout for the
        //    bounded modes.
        assert_eq!(
            ValidationMode::Quick.timeout(),
            ValidationMode::Quick.meta().fixed_timeout.unwrap(),
        );
        assert_eq!(
            ValidationMode::Thorough.timeout(),
            ValidationMode::Thorough.meta().fixed_timeout.unwrap(),
        );
    }

    /// Alias contract: `refinement_validation::ValidationMode` is
    /// a `pub use` re-export of `lsp_config::ValidationMode` —
    /// asserts the two paths refer to the same type so consumers
    /// can pass values across without an identity-shape mapper.
    #[test]
    fn validation_mode_alias_contract() {
        // Compile-time alias check via assignment.
        let m: crate::refinement_validation::ValidationMode =
            ValidationMode::Quick;
        assert_eq!(m, ValidationMode::Quick);

        // Round-trip through both names.
        let canonical: ValidationMode = ValidationMode::Complete;
        let aliased: crate::refinement_validation::ValidationMode = canonical;
        assert_eq!(aliased, ValidationMode::Complete);
    }

    #[test]
    fn defaults_are_sane() {
        let cfg = LspConfig::default();
        assert!(cfg.enable_refinement_validation);
        assert_eq!(cfg.validation_mode, ValidationMode::Quick);
        assert_eq!(cfg.cache_max_entries, 1000);
    }

    #[test]
    fn apply_json_overrides_known_keys() {
        let mut cfg = LspConfig::default();
        cfg.apply_json(&json!({
            "enableRefinementValidation": false,
            "validationMode": "thorough",
            "smtSolver": "cvc5",
            "smtTimeout": 200,
            "cacheTtlSeconds": 600,
            "cacheMaxEntries": 2048,
            "cbgrShowOptimizationHints": true,
            "maxCounterexampleTraces": 10,
            "verificationSlowThresholdMs": 7500
        }));
        assert!(!cfg.enable_refinement_validation);
        assert_eq!(cfg.validation_mode, ValidationMode::Thorough);
        assert_eq!(cfg.smt_solver, SmtSolverChoice::Cvc5);
        assert_eq!(cfg.smt_timeout, Duration::from_millis(200));
        assert_eq!(cfg.cache_ttl, Duration::from_secs(600));
        assert_eq!(cfg.cache_max_entries, 2048);
        assert!(cfg.cbgr_show_optimization_hints);
        assert_eq!(cfg.max_counterexample_traces, 10);
        assert_eq!(cfg.verification_slow_threshold, Duration::from_millis(7500));
    }

    #[test]
    fn apply_json_ignores_unknown_and_mistyped_keys() {
        let mut cfg = LspConfig::default();
        cfg.apply_json(&json!({
            "enableRefinementValidation": "not-a-bool",
            "validationMode": "bananas",
            "smtSolver": 42,
            "nonsenseKey": true,
        }));
        // All overrides were invalid — defaults must survive unchanged.
        assert!(cfg.enable_refinement_validation);
        assert_eq!(cfg.validation_mode, ValidationMode::Quick);
        assert_eq!(cfg.smt_solver, SmtSolverChoice::Z3);
    }

    #[test]
    fn shared_config_round_trip() {
        let shared = SharedLspConfig::new();
        shared.apply_json(&json!({"cacheMaxEntries": 123}));
        assert_eq!(shared.snapshot().cache_max_entries, 123);
    }
}
