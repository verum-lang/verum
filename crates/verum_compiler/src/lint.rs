//! Lint Configuration and Intrinsic Diagnostics
//!
//! This module provides configurable lint levels for intrinsic-related diagnostics,
//! following rustc-compatible semantics for lint severity.
//!
//! ## Lint Levels
//!
//! - `Allow`: Suppress the lint entirely
//! - `Warn`: Emit as warning (default for most lints)
//! - `Deny`: Emit as error, fail compilation
//! - `Forbid`: Like `Deny`, but cannot be overridden
//!
//! ## Diagnostic Codes
//!
//! ### Errors (E09xx)
//! - E0901: Missing intrinsic (strict mode)
//! - E0902: Wrong argument count
//! - E0903: Wrong argument type
//! - E0904: Protocol bound not satisfied
//! - E0905: Platform not supported
//! - E0906: Compile-time evaluation failed
//!
//! ### Warnings (W09xx)
//! - W0901: Missing intrinsic (default mode)
//! - W0902: Deprecated intrinsic
//! - W0903: Unstable intrinsic

use std::collections::HashMap;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};

// Re-export Span from verum_ast for convenience (byte-offset based)
pub use verum_ast::Span;

/// Lint severity level (rustc-compatible semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum LintLevel {
    /// Suppress the lint entirely.
    Allow,
    /// Emit as warning (default).
    #[default]
    Warn,
    /// Emit as error, fail compilation.
    Deny,
    /// Like `Deny`, but cannot be overridden by attributes.
    Forbid,
}

impl LintLevel {
    /// Parse lint level from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "allow" | "a" => Some(LintLevel::Allow),
            "warn" | "w" => Some(LintLevel::Warn),
            "deny" | "d" => Some(LintLevel::Deny),
            "forbid" | "f" => Some(LintLevel::Forbid),
            _ => None,
        }
    }

    /// Convert to diagnostic severity.
    pub fn to_severity(self) -> Option<Severity> {
        match self {
            LintLevel::Allow => None,
            LintLevel::Warn => Some(Severity::Warning),
            LintLevel::Deny | LintLevel::Forbid => Some(Severity::Error),
        }
    }

    /// Check if this level should emit a diagnostic.
    pub fn should_emit(self) -> bool {
        !matches!(self, LintLevel::Allow)
    }

    /// Check if this level causes compilation failure.
    pub fn is_error(self) -> bool {
        matches!(self, LintLevel::Deny | LintLevel::Forbid)
    }
}

/// Intrinsic-specific lint categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntrinsicLint {
    /// Intrinsic not found in registry (W0901/E0901).
    MissingImplementation,
    /// Wrong number of arguments (E0902).
    ArgumentCount,
    /// Wrong argument type (E0903).
    ArgumentType,
    /// Type doesn't satisfy protocol bound (E0904).
    ProtocolBound,
    /// Intrinsic not available on target platform (E0905).
    PlatformNotSupported,
    /// Compile-time evaluation failed (E0906).
    ConstEvalFailed,
    /// Using deprecated intrinsic (W0902).
    Deprecated,
    /// Using unstable intrinsic (W0903).
    Unstable,
}

impl IntrinsicLint {
    /// Get the lint name for CLI/config.
    pub fn name(self) -> &'static str {
        match self {
            IntrinsicLint::MissingImplementation => "missing_intrinsic",
            IntrinsicLint::ArgumentCount => "intrinsic_arg_count",
            IntrinsicLint::ArgumentType => "intrinsic_arg_type",
            IntrinsicLint::ProtocolBound => "intrinsic_protocol_bound",
            IntrinsicLint::PlatformNotSupported => "intrinsic_platform",
            IntrinsicLint::ConstEvalFailed => "intrinsic_const_eval",
            IntrinsicLint::Deprecated => "intrinsic_deprecated",
            IntrinsicLint::Unstable => "intrinsic_unstable",
        }
    }

    /// Parse lint from string name.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "missing_intrinsic" => Some(IntrinsicLint::MissingImplementation),
            "intrinsic_arg_count" => Some(IntrinsicLint::ArgumentCount),
            "intrinsic_arg_type" => Some(IntrinsicLint::ArgumentType),
            "intrinsic_protocol_bound" => Some(IntrinsicLint::ProtocolBound),
            "intrinsic_platform" => Some(IntrinsicLint::PlatformNotSupported),
            "intrinsic_const_eval" => Some(IntrinsicLint::ConstEvalFailed),
            "intrinsic_deprecated" => Some(IntrinsicLint::Deprecated),
            "intrinsic_unstable" => Some(IntrinsicLint::Unstable),
            _ => None,
        }
    }

    /// Get all lint names for help text.
    pub fn all_names() -> &'static [&'static str] {
        &[
            "missing_intrinsic",
            "intrinsic_arg_count",
            "intrinsic_arg_type",
            "intrinsic_protocol_bound",
            "intrinsic_platform",
            "intrinsic_const_eval",
            "intrinsic_deprecated",
            "intrinsic_unstable",
        ]
    }

    /// Get the default level for this lint.
    pub fn default_level(self) -> LintLevel {
        match self {
            // Type errors are always errors
            IntrinsicLint::ArgumentCount
            | IntrinsicLint::ArgumentType
            | IntrinsicLint::ProtocolBound => LintLevel::Deny,
            // Platform and const eval are errors
            IntrinsicLint::PlatformNotSupported | IntrinsicLint::ConstEvalFailed => LintLevel::Deny,
            // Missing intrinsics are warnings by default
            IntrinsicLint::MissingImplementation => LintLevel::Warn,
            // Deprecation/unstable are warnings
            IntrinsicLint::Deprecated | IntrinsicLint::Unstable => LintLevel::Warn,
        }
    }

    /// Get the warning code for this lint.
    pub fn warning_code(self) -> &'static str {
        match self {
            IntrinsicLint::MissingImplementation => "W0901",
            IntrinsicLint::Deprecated => "W0902",
            IntrinsicLint::Unstable => "W0903",
            _ => "W0900", // Generic warning
        }
    }

    /// Get the error code for this lint.
    pub fn error_code(self) -> &'static str {
        match self {
            IntrinsicLint::MissingImplementation => "E0901",
            IntrinsicLint::ArgumentCount => "E0902",
            IntrinsicLint::ArgumentType => "E0903",
            IntrinsicLint::ProtocolBound => "E0904",
            IntrinsicLint::PlatformNotSupported => "E0905",
            IntrinsicLint::ConstEvalFailed => "E0906",
            _ => "E0900", // Generic error
        }
    }

    /// Get the appropriate code based on severity.
    pub fn code_for_level(self, level: LintLevel) -> &'static str {
        if level.is_error() {
            self.error_code()
        } else {
            self.warning_code()
        }
    }
}

/// Lint configuration for intrinsic diagnostics.
#[derive(Debug, Clone)]
pub struct LintConfig {
    /// Treat all warnings as errors.
    pub deny_warnings: bool,
    /// Missing intrinsics are errors (strict mode).
    pub strict_intrinsics: bool,
    /// Per-lint level overrides.
    pub lint_levels: HashMap<IntrinsicLint, LintLevel>,
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            deny_warnings: false,
            strict_intrinsics: false,
            lint_levels: HashMap::new(),
        }
    }
}

impl LintConfig {
    /// Create a new lint config with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder: Enable deny_warnings mode.
    pub fn with_deny_warnings(mut self, enabled: bool) -> Self {
        self.deny_warnings = enabled;
        self
    }

    /// Builder: Enable strict_intrinsics mode.
    pub fn with_strict_intrinsics(mut self, enabled: bool) -> Self {
        self.strict_intrinsics = enabled;
        self
    }

    /// Builder: Set a specific lint level.
    pub fn with_lint_level(mut self, lint: IntrinsicLint, level: LintLevel) -> Self {
        self.lint_levels.insert(lint, level);
        self
    }

    /// Set lint level (mutable).
    pub fn set_lint_level(&mut self, lint: IntrinsicLint, level: LintLevel) {
        // Forbid cannot be overridden
        if let Some(&existing) = self.lint_levels.get(&lint) {
            if existing == LintLevel::Forbid {
                return;
            }
        }
        self.lint_levels.insert(lint, level);
    }

    /// Get the effective level for a lint.
    pub fn level_for(&self, lint: IntrinsicLint) -> LintLevel {
        // Check explicit override first
        if let Some(&level) = self.lint_levels.get(&lint) {
            return self.apply_global_modifiers(level);
        }

        // Check strict_intrinsics for MissingImplementation
        if lint == IntrinsicLint::MissingImplementation && self.strict_intrinsics {
            return LintLevel::Deny;
        }

        // Apply default level with global modifiers
        self.apply_global_modifiers(lint.default_level())
    }

    /// Apply global modifiers (deny_warnings) to a level.
    fn apply_global_modifiers(&self, level: LintLevel) -> LintLevel {
        if self.deny_warnings && level == LintLevel::Warn {
            LintLevel::Deny
        } else {
            level
        }
    }

    /// Parse lint settings from CLI flags.
    ///
    /// Accepts flags like:
    /// - `-D missing_intrinsic` (deny)
    /// - `-W intrinsic_deprecated` (warn)
    /// - `-A intrinsic_unstable` (allow)
    /// - `-F intrinsic_arg_count` (forbid)
    pub fn apply_cli_flags(&mut self, deny: &[String], warn: &[String], allow: &[String], forbid: &[String]) {
        for lint_name in forbid {
            if let Some(lint) = IntrinsicLint::from_str(lint_name) {
                self.lint_levels.insert(lint, LintLevel::Forbid);
            }
        }
        for lint_name in deny {
            if let Some(lint) = IntrinsicLint::from_str(lint_name) {
                // Don't override forbid
                if self.lint_levels.get(&lint) != Some(&LintLevel::Forbid) {
                    self.lint_levels.insert(lint, LintLevel::Deny);
                }
            }
        }
        for lint_name in warn {
            if let Some(lint) = IntrinsicLint::from_str(lint_name) {
                if self.lint_levels.get(&lint) != Some(&LintLevel::Forbid) {
                    self.lint_levels.insert(lint, LintLevel::Warn);
                }
            }
        }
        for lint_name in allow {
            if let Some(lint) = IntrinsicLint::from_str(lint_name) {
                if self.lint_levels.get(&lint) != Some(&LintLevel::Forbid) {
                    self.lint_levels.insert(lint, LintLevel::Allow);
                }
            }
        }
    }
}

/// Intrinsic diagnostics generator.
///
/// Generates diagnostics for intrinsic-related errors and warnings
/// according to the configured lint levels.
pub struct IntrinsicDiagnostics<'a> {
    config: &'a LintConfig,
}

impl<'a> IntrinsicDiagnostics<'a> {
    /// Create a new diagnostics generator with the given config.
    pub fn new(config: &'a LintConfig) -> Self {
        Self { config }
    }

    /// Generate diagnostic for missing intrinsic.
    ///
    /// Returns `None` if the lint is set to `Allow`.
    pub fn missing_intrinsic(&self, name: &str, span: Option<Span>) -> Option<Diagnostic> {
        let level = self.config.level_for(IntrinsicLint::MissingImplementation);
        if !level.should_emit() {
            return None;
        }

        let code = IntrinsicLint::MissingImplementation.code_for_level(level);
        let message = format!(
            "intrinsic `{}` not found in registry. Using stub implementation.",
            name
        );

        Some(self.build_diagnostic(level, code, &message, span))
    }

    /// Generate diagnostic for wrong argument count.
    pub fn wrong_arg_count(
        &self,
        name: &str,
        expected: usize,
        actual: usize,
        span: Option<Span>,
    ) -> Diagnostic {
        let level = self.config.level_for(IntrinsicLint::ArgumentCount);
        let code = IntrinsicLint::ArgumentCount.code_for_level(level);
        let message = format!(
            "intrinsic `{}` expects {} argument(s), but {} provided",
            name, expected, actual
        );

        self.build_diagnostic(level, code, &message, span)
    }

    /// Generate diagnostic for wrong argument type.
    pub fn wrong_arg_type(
        &self,
        name: &str,
        arg_index: usize,
        expected: &str,
        actual: &str,
        span: Option<Span>,
    ) -> Diagnostic {
        let level = self.config.level_for(IntrinsicLint::ArgumentType);
        let code = IntrinsicLint::ArgumentType.code_for_level(level);
        let message = format!(
            "intrinsic `{}` argument {} expects type `{}`, but got `{}`",
            name, arg_index, expected, actual
        );

        self.build_diagnostic(level, code, &message, span)
    }

    /// Generate diagnostic for unsatisfied protocol bound.
    pub fn protocol_bound_not_satisfied(
        &self,
        name: &str,
        type_name: &str,
        protocol: &str,
        span: Option<Span>,
    ) -> Diagnostic {
        let level = self.config.level_for(IntrinsicLint::ProtocolBound);
        let code = IntrinsicLint::ProtocolBound.code_for_level(level);
        let message = format!(
            "type `{}` does not satisfy protocol `{}` required by intrinsic `{}`",
            type_name, protocol, name
        );

        self.build_diagnostic(level, code, &message, span)
    }

    /// Generate diagnostic for platform not supported.
    pub fn platform_not_supported(
        &self,
        name: &str,
        platform: &str,
        supported: &[&str],
        span: Option<Span>,
    ) -> Diagnostic {
        let level = self.config.level_for(IntrinsicLint::PlatformNotSupported);
        let code = IntrinsicLint::PlatformNotSupported.code_for_level(level);
        let supported_list = supported.join(", ");
        let message = format!(
            "intrinsic `{}` is not available on platform `{}`. Supported: {}",
            name, platform, supported_list
        );

        self.build_diagnostic(level, code, &message, span)
    }

    /// Generate diagnostic for const eval failure.
    pub fn const_eval_failed(&self, name: &str, reason: &str, span: Option<Span>) -> Diagnostic {
        let level = self.config.level_for(IntrinsicLint::ConstEvalFailed);
        let code = IntrinsicLint::ConstEvalFailed.code_for_level(level);
        let message = format!(
            "compile-time evaluation of intrinsic `{}` failed: {}",
            name, reason
        );

        self.build_diagnostic(level, code, &message, span)
    }

    /// Generate diagnostic for deprecated intrinsic.
    pub fn deprecated(
        &self,
        name: &str,
        replacement: Option<&str>,
        span: Option<Span>,
    ) -> Option<Diagnostic> {
        let level = self.config.level_for(IntrinsicLint::Deprecated);
        if !level.should_emit() {
            return None;
        }

        let code = IntrinsicLint::Deprecated.code_for_level(level);
        let message = if let Some(replacement) = replacement {
            format!(
                "intrinsic `{}` is deprecated. Use `{}` instead.",
                name, replacement
            )
        } else {
            format!("intrinsic `{}` is deprecated.", name)
        };

        Some(self.build_diagnostic(level, code, &message, span))
    }

    /// Generate diagnostic for unstable intrinsic.
    pub fn unstable(&self, name: &str, feature: &str, span: Option<Span>) -> Option<Diagnostic> {
        let level = self.config.level_for(IntrinsicLint::Unstable);
        if !level.should_emit() {
            return None;
        }

        let code = IntrinsicLint::Unstable.code_for_level(level);
        let message = format!(
            "intrinsic `{}` is unstable and requires feature `{}`",
            name, feature
        );

        Some(self.build_diagnostic(level, code, &message, span))
    }

    /// Generate diagnostic for generic VBC codegen warning.
    ///
    /// This is used for general codegen errors that don't fit specific categories.
    pub fn codegen_warning(&self, module_name: &str, error: &str, span: Option<Span>) -> Diagnostic {
        let level = self.config.level_for(IntrinsicLint::MissingImplementation);
        let code = IntrinsicLint::MissingImplementation.code_for_level(level);

        let message = if let Some(ref s) = span {
            format!(
                "VBC codegen warning in {} (byte {}-{}): {}. Using stub implementation.",
                module_name, s.start, s.end, error
            )
        } else {
            format!(
                "VBC codegen warning in {}: {}. Using stub implementation.",
                module_name, error
            )
        };

        self.build_diagnostic(level, code, &message, span)
    }

    /// Build a diagnostic with the given parameters.
    ///
    /// Note: Span information is included in the message text, not in the
    /// diagnostic metadata. This matches the original behavior in pipeline.rs
    /// and avoids the need to convert byte-offset spans to LineColSpan.
    fn build_diagnostic(
        &self,
        level: LintLevel,
        code: &str,
        message: &str,
        _span: Option<Span>,
    ) -> Diagnostic {
        let builder = match level.to_severity() {
            Some(Severity::Error) => DiagnosticBuilder::error(),
            Some(Severity::Warning) => DiagnosticBuilder::warning(),
            _ => DiagnosticBuilder::warning(),
        };

        builder.code(code).message(message.to_string()).build()
    }

    /// Check if the missing intrinsic lint is an error.
    pub fn is_missing_intrinsic_error(&self) -> bool {
        self.config.level_for(IntrinsicLint::MissingImplementation).is_error()
    }
}

// =============================================================================
// STAGED METAPROGRAMMING LINTS (E10xx, W10xx)
// =============================================================================

/// Staged metaprogramming lint categories.
///
/// These lints enforce the Stage Coherence Rule and other staged meta constraints.
///
/// ## Diagnostic Codes
///
/// ### Errors (E10xx)
/// - E1001: Stage mismatch in quote expression
/// - E1002: Cross-stage function call
/// - E1003: Stage overflow (exceeded max_stage)
/// - E1004: Cyclic stage dependency
/// - E1005: Invalid stage escape
///
/// ### Warnings (W10xx)
/// - W1001: Unused stage definition
/// - W1002: Function can be downgraded to lower stage
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StagedMetaLint {
    /// E1001: Quote generates code for wrong stage.
    ///
    /// The Stage Coherence Rule requires that a Stage N function can only
    /// directly generate Stage N-1 code via `quote { ... }`.
    StageMismatch,

    /// E1002: Cross-stage function call.
    ///
    /// Higher stage functions cannot directly call lower stage functions.
    /// They must generate code that calls them.
    CrossStageCall,

    /// E1003: Stage overflow.
    ///
    /// The stage level exceeds the configured `max_stage` in Verum.toml.
    StageOverflow,

    /// E1004: Cyclic stage dependency.
    ///
    /// A circular dependency between staged functions would create
    /// an infinite compilation loop.
    CyclicStage,

    /// E1005: Invalid stage escape.
    ///
    /// The `$(stage N) { ... }` escape specifies an invalid stage.
    InvalidStageEscape,

    /// W1001: Unused stage definition.
    ///
    /// A `meta(N)` function is defined but never invoked during compilation.
    UnusedStage,

    /// W1002: Stage can be downgraded.
    ///
    /// A `meta(N)` function only generates code for Stage N-2 or lower,
    /// so it could be simplified to `meta(N-1)`.
    StageDowngrade,
}

impl StagedMetaLint {
    /// Get the lint name for CLI/config.
    pub fn name(self) -> &'static str {
        match self {
            StagedMetaLint::StageMismatch => "stage_mismatch",
            StagedMetaLint::CrossStageCall => "cross_stage_call",
            StagedMetaLint::StageOverflow => "stage_overflow",
            StagedMetaLint::CyclicStage => "cyclic_stage",
            StagedMetaLint::InvalidStageEscape => "invalid_stage_escape",
            StagedMetaLint::UnusedStage => "unused_stage",
            StagedMetaLint::StageDowngrade => "stage_downgrade",
        }
    }

    /// Parse lint from string name.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "stage_mismatch" => Some(StagedMetaLint::StageMismatch),
            "cross_stage_call" => Some(StagedMetaLint::CrossStageCall),
            "stage_overflow" => Some(StagedMetaLint::StageOverflow),
            "cyclic_stage" => Some(StagedMetaLint::CyclicStage),
            "invalid_stage_escape" => Some(StagedMetaLint::InvalidStageEscape),
            "unused_stage" => Some(StagedMetaLint::UnusedStage),
            "stage_downgrade" => Some(StagedMetaLint::StageDowngrade),
            _ => None,
        }
    }

    /// Get all lint names for help text.
    pub fn all_names() -> &'static [&'static str] {
        &[
            "stage_mismatch",
            "cross_stage_call",
            "stage_overflow",
            "cyclic_stage",
            "invalid_stage_escape",
            "unused_stage",
            "stage_downgrade",
        ]
    }

    /// Get the default level for this lint.
    pub fn default_level(self) -> LintLevel {
        match self {
            // Stage coherence errors are always errors
            StagedMetaLint::StageMismatch
            | StagedMetaLint::CrossStageCall
            | StagedMetaLint::StageOverflow
            | StagedMetaLint::CyclicStage
            | StagedMetaLint::InvalidStageEscape => LintLevel::Deny,
            // Unused and downgrade are warnings
            StagedMetaLint::UnusedStage | StagedMetaLint::StageDowngrade => LintLevel::Warn,
        }
    }

    /// Get the warning code for this lint.
    pub fn warning_code(self) -> &'static str {
        match self {
            StagedMetaLint::UnusedStage => "W1001",
            StagedMetaLint::StageDowngrade => "W1002",
            _ => "W1000", // Generic warning
        }
    }

    /// Get the error code for this lint.
    pub fn error_code(self) -> &'static str {
        match self {
            StagedMetaLint::StageMismatch => "E1001",
            StagedMetaLint::CrossStageCall => "E1002",
            StagedMetaLint::StageOverflow => "E1003",
            StagedMetaLint::CyclicStage => "E1004",
            StagedMetaLint::InvalidStageEscape => "E1005",
            _ => "E1000", // Generic error
        }
    }

    /// Get the appropriate code based on severity.
    pub fn code_for_level(self, level: LintLevel) -> &'static str {
        if level.is_error() {
            self.error_code()
        } else {
            self.warning_code()
        }
    }
}

/// Staged meta diagnostics generator.
///
/// Generates diagnostics for staged metaprogramming errors and warnings.
/// Uses the same configuration approach as IntrinsicDiagnostics.
pub struct StagedMetaDiagnostics<'a> {
    config: &'a LintConfig,
    staged_levels: HashMap<StagedMetaLint, LintLevel>,
}

impl<'a> StagedMetaDiagnostics<'a> {
    /// Create a new diagnostics generator with the given config.
    pub fn new(config: &'a LintConfig) -> Self {
        Self {
            config,
            staged_levels: HashMap::new(),
        }
    }

    /// Create with explicit staged meta lint levels.
    pub fn with_staged_levels(
        config: &'a LintConfig,
        staged_levels: HashMap<StagedMetaLint, LintLevel>,
    ) -> Self {
        Self {
            config,
            staged_levels,
        }
    }

    /// Get the effective level for a staged meta lint.
    fn level_for(&self, lint: StagedMetaLint) -> LintLevel {
        // Check explicit override first
        if let Some(&level) = self.staged_levels.get(&lint) {
            return self.apply_global_modifiers(level);
        }

        // Use default level with global modifiers
        self.apply_global_modifiers(lint.default_level())
    }

    /// Apply global modifiers (deny_warnings) to a level.
    fn apply_global_modifiers(&self, level: LintLevel) -> LintLevel {
        if self.config.deny_warnings && level == LintLevel::Warn {
            LintLevel::Deny
        } else {
            level
        }
    }

    /// Generate diagnostic for stage mismatch (E1001).
    ///
    /// Emitted when `quote { ... }` generates code for the wrong stage.
    pub fn stage_mismatch(
        &self,
        current_stage: u32,
        target_stage: u32,
        expected_stage: u32,
        hint: &str,
        span: Option<Span>,
    ) -> Diagnostic {
        let level = self.level_for(StagedMetaLint::StageMismatch);
        let code = StagedMetaLint::StageMismatch.code_for_level(level);
        let message = format!(
            "stage mismatch in quote expression: stage {} cannot generate stage {} code \
             (expected stage {}). {}",
            current_stage, target_stage, expected_stage, hint
        );

        self.build_diagnostic(level, code, &message, span)
    }

    /// Generate diagnostic for cross-stage call (E1002).
    ///
    /// Emitted when a higher stage function tries to call a lower stage function.
    pub fn cross_stage_call(
        &self,
        caller_stage: u32,
        callee_stage: u32,
        callee_name: &str,
        hint: &str,
        span: Option<Span>,
    ) -> Diagnostic {
        let level = self.level_for(StagedMetaLint::CrossStageCall);
        let code = StagedMetaLint::CrossStageCall.code_for_level(level);
        let message = format!(
            "cross-stage function call: stage {} function cannot call stage {} function `{}`. {}",
            caller_stage, callee_stage, callee_name, hint
        );

        self.build_diagnostic(level, code, &message, span)
    }

    /// Generate diagnostic for stage overflow (E1003).
    ///
    /// Emitted when a function declares a stage higher than max_stage.
    pub fn stage_overflow(
        &self,
        used_stage: u32,
        max_stage: u32,
        function_name: &str,
        span: Option<Span>,
    ) -> Diagnostic {
        let level = self.level_for(StagedMetaLint::StageOverflow);
        let code = StagedMetaLint::StageOverflow.code_for_level(level);
        let message = format!(
            "stage overflow: function `{}` uses stage {} but max_stage is {}. \
             Increase max_stage in Verum.toml or lower the function's stage.",
            function_name, used_stage, max_stage
        );

        self.build_diagnostic(level, code, &message, span)
    }

    /// Generate diagnostic for cyclic stage dependency (E1004).
    ///
    /// Emitted when staged functions form a cycle.
    pub fn cyclic_stage(&self, cycle: &[&str], span: Option<Span>) -> Diagnostic {
        let level = self.level_for(StagedMetaLint::CyclicStage);
        let code = StagedMetaLint::CyclicStage.code_for_level(level);
        let cycle_str = cycle.join(" -> ");
        let message = format!(
            "cyclic stage dependency detected: {}. \
             Staged functions cannot form cycles as this would cause infinite compilation.",
            cycle_str
        );

        self.build_diagnostic(level, code, &message, span)
    }

    /// Generate diagnostic for invalid stage escape (E1005).
    ///
    /// Emitted when `$(stage N) { ... }` uses an invalid stage.
    pub fn invalid_stage_escape(
        &self,
        escape_stage: u32,
        current_stage: u32,
        valid_range: &str,
        span: Option<Span>,
    ) -> Diagnostic {
        let level = self.level_for(StagedMetaLint::InvalidStageEscape);
        let code = StagedMetaLint::InvalidStageEscape.code_for_level(level);
        let message = format!(
            "invalid stage escape: cannot escape to stage {} from stage {}. \
             Valid range: {}",
            escape_stage, current_stage, valid_range
        );

        self.build_diagnostic(level, code, &message, span)
    }

    /// Generate diagnostic for unused stage (W1001).
    ///
    /// Emitted when a `meta(N)` function is never invoked.
    pub fn unused_stage(
        &self,
        stage: u32,
        function_name: &str,
        span: Option<Span>,
    ) -> Option<Diagnostic> {
        let level = self.level_for(StagedMetaLint::UnusedStage);
        if !level.should_emit() {
            return None;
        }

        let code = StagedMetaLint::UnusedStage.code_for_level(level);
        let message = format!(
            "unused stage {} function `{}`: defined but never invoked during compilation. \
             Consider removing it or using it in a macro invocation.",
            stage, function_name
        );

        Some(self.build_diagnostic(level, code, &message, span))
    }

    /// Generate diagnostic for stage downgrade opportunity (W1002).
    ///
    /// Emitted when a function could use a lower stage.
    pub fn stage_downgrade(
        &self,
        current_stage: u32,
        suggested_stage: u32,
        function_name: &str,
        reason: &str,
        span: Option<Span>,
    ) -> Option<Diagnostic> {
        let level = self.level_for(StagedMetaLint::StageDowngrade);
        if !level.should_emit() {
            return None;
        }

        let code = StagedMetaLint::StageDowngrade.code_for_level(level);
        let message = format!(
            "function `{}` can be downgraded from stage {} to stage {}: {}. \
             Lower stages compile faster.",
            function_name, current_stage, suggested_stage, reason
        );

        Some(self.build_diagnostic(level, code, &message, span))
    }

    /// Build a diagnostic with the given parameters.
    fn build_diagnostic(
        &self,
        level: LintLevel,
        code: &str,
        message: &str,
        _span: Option<Span>,
    ) -> Diagnostic {
        let builder = match level.to_severity() {
            Some(Severity::Error) => DiagnosticBuilder::error(),
            Some(Severity::Warning) => DiagnosticBuilder::warning(),
            _ => DiagnosticBuilder::warning(),
        };

        builder.code(code).message(message.to_string()).build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lint_level_ordering() {
        assert!(LintLevel::Allow < LintLevel::Warn);
        assert!(LintLevel::Warn < LintLevel::Deny);
        assert!(LintLevel::Deny < LintLevel::Forbid);
    }

    #[test]
    fn test_lint_level_from_str() {
        assert_eq!(LintLevel::from_str("allow"), Some(LintLevel::Allow));
        assert_eq!(LintLevel::from_str("warn"), Some(LintLevel::Warn));
        assert_eq!(LintLevel::from_str("deny"), Some(LintLevel::Deny));
        assert_eq!(LintLevel::from_str("forbid"), Some(LintLevel::Forbid));
        assert_eq!(LintLevel::from_str("invalid"), None);
    }

    #[test]
    fn test_intrinsic_lint_names() {
        assert_eq!(IntrinsicLint::MissingImplementation.name(), "missing_intrinsic");
        assert_eq!(
            IntrinsicLint::from_str("missing_intrinsic"),
            Some(IntrinsicLint::MissingImplementation)
        );
    }

    #[test]
    fn test_lint_config_defaults() {
        let config = LintConfig::default();
        assert!(!config.deny_warnings);
        assert!(!config.strict_intrinsics);
        assert_eq!(
            config.level_for(IntrinsicLint::MissingImplementation),
            LintLevel::Warn
        );
    }

    #[test]
    fn test_lint_config_strict_mode() {
        let config = LintConfig::new().with_strict_intrinsics(true);
        assert_eq!(
            config.level_for(IntrinsicLint::MissingImplementation),
            LintLevel::Deny
        );
    }

    #[test]
    fn test_lint_config_deny_warnings() {
        let config = LintConfig::new().with_deny_warnings(true);
        // Warnings become errors
        assert_eq!(
            config.level_for(IntrinsicLint::Deprecated),
            LintLevel::Deny
        );
        // But explicit errors stay errors (not double-promoted)
        assert_eq!(
            config.level_for(IntrinsicLint::ArgumentCount),
            LintLevel::Deny
        );
    }

    #[test]
    fn test_lint_config_explicit_override() {
        let config = LintConfig::new()
            .with_lint_level(IntrinsicLint::MissingImplementation, LintLevel::Allow);
        assert_eq!(
            config.level_for(IntrinsicLint::MissingImplementation),
            LintLevel::Allow
        );
    }

    #[test]
    fn test_forbid_cannot_be_overridden() {
        let mut config = LintConfig::new();
        config.set_lint_level(IntrinsicLint::MissingImplementation, LintLevel::Forbid);
        config.set_lint_level(IntrinsicLint::MissingImplementation, LintLevel::Allow);
        // Forbid cannot be overridden
        assert_eq!(
            config.level_for(IntrinsicLint::MissingImplementation),
            LintLevel::Forbid
        );
    }

    #[test]
    fn test_intrinsic_diagnostics_missing() {
        let config = LintConfig::default();
        let diags = IntrinsicDiagnostics::new(&config);

        let diag = diags.missing_intrinsic("test_intrinsic", None);
        assert!(diag.is_some());

        let diag = diag.unwrap();
        assert_eq!(diag.code(), Some("W0901"));
    }

    #[test]
    fn test_intrinsic_diagnostics_strict_mode() {
        let config = LintConfig::new().with_strict_intrinsics(true);
        let diags = IntrinsicDiagnostics::new(&config);

        let diag = diags.missing_intrinsic("test_intrinsic", None);
        assert!(diag.is_some());

        let diag = diag.unwrap();
        assert_eq!(diag.code(), Some("E0901"));
    }

    #[test]
    fn test_intrinsic_diagnostics_allowed() {
        let config = LintConfig::new()
            .with_lint_level(IntrinsicLint::MissingImplementation, LintLevel::Allow);
        let diags = IntrinsicDiagnostics::new(&config);

        let diag = diags.missing_intrinsic("test_intrinsic", None);
        assert!(diag.is_none());
    }
}

// =============================================================================
// Stdlib hazards — separate lint family from intrinsics
// =============================================================================

/// Stdlib-specific lint categories.
///
/// Warnings in the W05xx range flag API calls that are
/// technically correct but semantically hazardous — usually
/// because they silently conflate two distinct domain cases
/// (e.g. `Map::get`'s 0-fallback conflates "key missing" with
/// "key present with zero value"). Each lint here corresponds
/// to a documented stdlib hazard with a safer alternative the
/// caller should migrate to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StdlibLint {
    /// `Map::get(key) -> V` — zero-fallback conflates missing
    /// key with zero-valued entry. Recommend `get_optional`
    /// (Maybe<V>) or `get_or(key, default)` (explicit default).
    /// Docs: `core/collections/map.vr` — the `get` doc comment
    /// carries the full hazard-and-alternatives writeup.
    MapGetHazard,
}

impl StdlibLint {
    /// The short lint name (for CLI `-A<name>` / `-W<name>`).
    pub fn name(self) -> &'static str {
        match self {
            Self::MapGetHazard => "map_get_hazard",
        }
    }

    /// Parse a lint name from the CLI.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "map_get_hazard" => Some(Self::MapGetHazard),
            _ => None,
        }
    }

    /// The W-code rendered in diagnostics.
    pub fn warning_code(self) -> &'static str {
        match self {
            Self::MapGetHazard => "W0505",
        }
    }

    /// Default severity — all stdlib hazards default to Warn
    /// so existing call sites remain buildable; users can
    /// `-Dmap_get_hazard` in CI to tighten.
    pub fn default_level(self) -> LintLevel {
        LintLevel::Warn
    }

    /// One-line hazard summary used in the diagnostic message.
    pub fn summary(self) -> &'static str {
        match self {
            Self::MapGetHazard => {
                "`Map::get(key)` returns a zero value on miss, silently \
                 conflating missing keys with zero-valued entries. \
                 Prefer `get_optional(key)` (Maybe<V>) or \
                 `get_or(key, default)` (explicit default)."
            }
        }
    }
}

/// Detect whether a call-site looks like `SOMETHING.get(KEY)`.
///
/// Takes the method name and the receiver-type name (as the
/// type-checker sees it). Returns `Some(StdlibLint::MapGetHazard)`
/// when the two combine to the flagged shape, else `None`.
///
/// We deliberately key on a string receiver-type name rather
/// than a concrete `Type` value so this helper can be invoked
/// from any AST-walker layer that has the receiver's name —
/// verum_types, the LSP, IDE adapters. The cost of a
/// stringly-typed receiver check is one `str::starts_with`;
/// fast enough to run on every call-site.
///
/// Accepts both `Map` and `Map<K, V>` forms.
pub fn detect_stdlib_hazard(
    method_name: &str,
    receiver_type_name: &str,
) -> Option<StdlibLint> {
    if method_name != "get" {
        return None;
    }
    // Accept `Map`, `Map<...>`, or any dotted path ending in
    // `Map` (e.g. `core.collections.map.Map`). We do NOT flag
    // `HashMap` / `BTreeMap` / etc — those have their own
    // presence semantics and aren't in the same hazard class.
    let ty = receiver_type_name;
    let matches_map = ty == "Map"
        || ty.starts_with("Map<")
        || ty.ends_with(".Map")
        || ty.ends_with("::Map");
    if matches_map {
        Some(StdlibLint::MapGetHazard)
    } else {
        None
    }
}

#[cfg(test)]
mod stdlib_lint_tests {
    use super::*;

    #[test]
    fn map_get_hazard_has_w0505_code() {
        assert_eq!(StdlibLint::MapGetHazard.warning_code(), "W0505");
        assert_eq!(StdlibLint::MapGetHazard.name(), "map_get_hazard");
    }

    #[test]
    fn map_get_hazard_default_is_warn() {
        assert_eq!(StdlibLint::MapGetHazard.default_level(), LintLevel::Warn);
    }

    #[test]
    fn detect_flags_simple_map_get() {
        assert_eq!(
            detect_stdlib_hazard("get", "Map"),
            Some(StdlibLint::MapGetHazard)
        );
    }

    #[test]
    fn detect_flags_generic_map_get() {
        assert_eq!(
            detect_stdlib_hazard("get", "Map<Text, Int>"),
            Some(StdlibLint::MapGetHazard)
        );
    }

    #[test]
    fn detect_flags_dotted_map_path() {
        assert_eq!(
            detect_stdlib_hazard("get", "core.collections.map.Map"),
            Some(StdlibLint::MapGetHazard)
        );
    }

    #[test]
    fn detect_does_not_flag_other_methods() {
        assert_eq!(detect_stdlib_hazard("insert", "Map"), None);
        assert_eq!(detect_stdlib_hazard("get_optional", "Map"), None);
        assert_eq!(detect_stdlib_hazard("get_or", "Map"), None);
    }

    #[test]
    fn detect_does_not_flag_non_map_types() {
        // Different presence semantics — not in the hazard
        // class this lint covers.
        assert_eq!(detect_stdlib_hazard("get", "List"), None);
        assert_eq!(detect_stdlib_hazard("get", "HashMap"), None);
        assert_eq!(detect_stdlib_hazard("get", "BTreeMap"), None);
        assert_eq!(detect_stdlib_hazard("get", "Maybe"), None);
    }

    #[test]
    fn from_str_roundtrip() {
        let l = StdlibLint::MapGetHazard;
        assert_eq!(StdlibLint::from_str(l.name()), Some(l));
        assert_eq!(StdlibLint::from_str("nonexistent"), None);
    }

    #[test]
    fn summary_mentions_both_alternatives() {
        let summary = StdlibLint::MapGetHazard.summary();
        assert!(summary.contains("get_optional"));
        assert!(summary.contains("get_or"));
    }
}

// =============================================================================
// AST walker — scans a Module for stdlib-hazard call sites (W0505 etc.)
// =============================================================================

/// A W0505 finding produced by the AST walker.
///
/// Carries the source span of the call-site so the diagnostic
/// renderer can anchor the warning. The receiver-type-name
/// information isn't available at pure AST level (only the
/// receiver *expression* is), so the walker uses the heuristic
/// "if the receiver path ends in `Map`" to trigger the warning.
/// A type-aware upgrade (taking a `TypeCheckerResult`) reduces
/// false positives; the heuristic-only walker is the
/// foundation.
#[derive(Debug, Clone)]
pub struct StdlibLintFinding {
    /// Which lint fired.
    pub lint: StdlibLint,
    /// Span of the offending method-call expression.
    pub span: Span,
    /// Receiver expression's pretty-printed form — used in the
    /// diagnostic's "you wrote `X.get(…)`" hint.
    pub receiver_repr: String,
}

/// Walk a `Module` looking for stdlib-hazard method calls.
///
/// Returns a list of `StdlibLintFinding`s — one per flagged
/// site. Uses the existing AST `Visitor` trait so this walker
/// never drifts from the AST's actual shape — a new expression
/// variant added upstream is automatically traversed.
///
/// # Coverage
///
/// Currently flags only the W0505 `map_get_hazard` family.
/// The walker descends into every expression position; the
/// `Visitor` default `walk_expr` handles every variant.
///
/// # Heuristic-only detection
///
/// Without type info, the walker uses name-based matching on
/// the receiver's Debug rendering: "contains `Map` but not
/// `HashMap`/`BTreeMap`". Matches the `detect_stdlib_hazard`
/// predicate's accepting shapes. A type-aware upgrade (fed
/// the type-checker's inferred receiver type) reduces false
/// positives; the heuristic path is the always-available
/// fallback.
pub fn walk_module_for_stdlib_hazards(
    module: &verum_ast::Module,
) -> Vec<StdlibLintFinding> {
    let mut walker = HazardCollector::default();
    // Re-use the AST's standard visitor — walk every item in
    // the module, which in turn descends into every
    // expression. The visitor's walk_* defaults handle all
    // current and future expression variants.
    use verum_ast::visitor::Visitor;
    for item in &module.items {
        walker.visit_item(item);
    }
    walker.findings
}

#[derive(Default)]
struct HazardCollector {
    findings: Vec<StdlibLintFinding>,
}

impl verum_ast::visitor::Visitor for HazardCollector {
    fn visit_expr(&mut self, expr: &verum_ast::Expr) {
        // Check this node for a map_get_hazard before
        // descending. The visitor trait's default walk_*
        // recurses into children for us after this call
        // returns.
        if let verum_ast::expr::ExprKind::MethodCall {
            receiver,
            method,
            ..
        } = &expr.kind
        {
            let receiver_repr = format!("{:?}", receiver);
            let looks_like_map = receiver_repr.contains("Map")
                && !receiver_repr.contains("HashMap")
                && !receiver_repr.contains("BTreeMap");
            if looks_like_map {
                if let Some(lint) =
                    detect_stdlib_hazard(method.name.as_str(), "Map")
                {
                    self.findings.push(StdlibLintFinding {
                        lint,
                        span: expr.span,
                        receiver_repr,
                    });
                }
            }
        }
        // Recurse through children via the default walker.
        verum_ast::visitor::walk_expr(self, expr);
    }
}

#[cfg(test)]
mod walker_tests {
    use super::*;

    #[test]
    fn walker_produces_zero_findings_on_empty_module() {
        let module = verum_ast::Module::new(
            verum_common::List::new(),
            verum_ast::FileId::new(0),
            verum_ast::Span::dummy(),
        );
        let findings = walk_module_for_stdlib_hazards(&module);
        assert!(findings.is_empty());
    }
}
