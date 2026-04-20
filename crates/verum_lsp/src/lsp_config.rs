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

use std::sync::RwLock;
use std::time::Duration;

/// Validation mode requested by the client — caps per-call SMT latency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationMode {
    /// `<100ms` — suitable for on-type validation.
    Quick,
    /// `<1s` — suitable for on-save validation.
    Thorough,
    /// Unlimited — reserved for CI/CD, not used by the LSP server directly.
    Complete,
}

impl ValidationMode {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "quick" => Some(Self::Quick),
            "thorough" => Some(Self::Thorough),
            "complete" => Some(Self::Complete),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Thorough => "thorough",
            Self::Complete => "complete",
        }
    }

    pub fn timeout(self) -> Duration {
        match self {
            Self::Quick => Duration::from_millis(100),
            Self::Thorough => Duration::from_secs(1),
            Self::Complete => Duration::from_secs(600),
        }
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
        }
    }
}

impl LspConfig {
    /// Merge values from a JSON `initializationOptions` blob. Unknown or
    /// ill-typed keys are silently ignored — we're forgiving on input so a
    /// slightly out-of-sync extension never crashes the server.
    pub fn apply_json(&mut self, opts: &serde_json::Value) {
        if let Some(v) = opts.get("enableRefinementValidation").and_then(|v| v.as_bool()) {
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
        if let Some(v) = opts
            .get("maxCounterexampleTraces")
            .and_then(|v| v.as_u64())
        {
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
