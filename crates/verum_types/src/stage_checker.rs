//! Stage Checker for Multi-Stage Metaprogramming
//!
//! This module implements stage-level type checking for Verum's N-level staged
//! metaprogramming system. It enforces the **Stage Coherence Rule**: a Stage N
//! function can only directly generate Stage N-1 code.
//!
//! # Staged Metaprogramming Model
//!
//! Verum supports N-level staged metaprogramming where functions execute at
//! different compilation stages:
//!
//! ```text
//! Stage N   ──►  generates  ──►  Stage N-1  ──►  ...  ──►  Stage 0 (runtime)
//! (meta(N))                       (meta(N-1))              (normal code)
//! ```
//!
//! ## Stage Semantics
//!
//! | Stage | Syntax | Execution | Description |
//! |-------|--------|-----------|-------------|
//! | 0 | `fn f()` | Runtime | Normal runtime functions |
//! | 1 | `meta fn f()` | Compile-time | Standard meta functions |
//! | 2 | `meta(2) fn f()` | Pre-compile | Generates meta functions |
//! | N | `meta(N) fn f()` | Stage N | Generates Stage N-1 code |
//!
//! ## Stage Coherence Rule
//!
//! The fundamental rule of staged metaprogramming:
//!
//! > **A Stage N function can only DIRECTLY generate Stage N-1 code.**
//!
//! This means:
//! - `meta(2)` can only directly generate `meta` (stage 1) code
//! - To generate runtime (stage 0) code from `meta(2)`, the output must contain
//!   a `meta` function that performs the final generation
//! - Each `quote { ... }` lowers the stage by 1
//!
//! ## Examples
//!
//! ```verum
//! // VALID: meta(2) generates meta(1) code
//! meta(2) fn derive_factory() -> TokenStream {
//!     quote {
//!         meta fn derive_impl<T>() -> TokenStream {
//!             quote { ... }  // This generates stage 0
//!         }
//!     }
//! }
//!
//! // INVALID: meta(2) cannot directly generate stage 0
//! meta(2) fn bad_factory() -> TokenStream {
//!     quote {
//!         fn runtime() { ... }  // Error E1001: stage mismatch
//!     }
//! }
//!
//! // INVALID: cross-stage call from higher to lower
//! meta fn helper() -> TokenStream { ... }
//!
//! meta(2) fn caller() -> TokenStream {
//!     helper()  // Error E1002: cross-stage call
//! }
//! ```
//!
//! # Diagnostic Codes
//!
//! | Code | Name | Description |
//! |------|------|-------------|
//! | E1001 | `stage_mismatch` | Quote generates wrong stage code |
//! | E1002 | `cross_stage_call` | Calling function from different stage |
//! | E1003 | `stage_overflow` | Exceeded max_stage configuration |
//! | E1004 | `cyclic_stage` | Cyclic dependency between stages |
//! | E1005 | `invalid_stage_escape` | Invalid `$(stage N)` escape |
//! | W1001 | `unused_stage` | Defined meta(N) but never invoked |
//! | W1002 | `stage_downgrade` | Function can use lower stage |
//!
//! # Integration
//!
//! The StageChecker integrates with:
//! - **verum_parser**: Receives `FunctionDecl.stage_level` from parsing
//! - **verum_types/infer**: Called during type inference for stage validation
//! - **verum_compiler/pipeline**: Used by StagedPipeline for multi-stage compilation
//! - **verum_diagnostics**: Emits E1001-E1005 and W1001-W1002 diagnostics

use std::collections::{HashMap, HashSet};
use verum_ast::Span;
use verum_common::{List, Text};

/// Errors from stage checking
#[derive(Debug, Clone, PartialEq)]
pub enum StageError {
    /// E1001: Quote generates code for wrong stage
    ///
    /// Occurs when a `quote { ... }` in a Stage N function generates code
    /// that is not Stage N-1. The Stage Coherence Rule requires that each
    /// quote lowers the stage by exactly 1.
    ///
    /// # Example
    /// ```verum
    /// // Error: meta(2) should generate meta(1), not stage 0
    /// meta(2) fn bad() -> TokenStream {
    ///     quote { fn runtime() { } }  // E1001
    /// }
    /// ```
    StageMismatch {
        /// Current stage (the stage of the function containing the quote)
        current_stage: u32,
        /// Target stage (the stage of the generated code)
        target_stage: u32,
        /// Expected target stage (should be current_stage - 1)
        expected_stage: u32,
        /// Source location of the quote expression
        span: Span,
        /// Suggested fix
        hint: Text,
    },

    /// E1002: Cross-stage function call
    ///
    /// Occurs when a Stage N function tries to directly call a function
    /// from a different stage. Higher stages cannot call lower stage functions
    /// directly (they must generate code that calls them).
    ///
    /// # Example
    /// ```verum
    /// meta fn helper() -> TokenStream { ... }
    ///
    /// meta(2) fn caller() -> TokenStream {
    ///     helper()  // E1002: cannot call stage 1 from stage 2
    /// }
    /// ```
    CrossStageCall {
        /// Stage of the caller function
        caller_stage: u32,
        /// Stage of the callee function
        callee_stage: u32,
        /// Name of the callee function
        callee_name: Text,
        /// Source location of the call
        span: Span,
        /// Suggested fix
        hint: Text,
    },

    /// E1003: Stage overflow
    ///
    /// Occurs when code uses a stage level higher than the configured
    /// `max_stage` in Verum.toml.
    ///
    /// # Configuration
    /// ```toml
    /// [meta]
    /// max_stage = 3  # Default is 2
    /// ```
    StageOverflow {
        /// Stage level that was used
        used_stage: u32,
        /// Maximum allowed stage from configuration
        max_stage: u32,
        /// Name of the function exceeding the limit
        function_name: Text,
        /// Source location
        span: Span,
    },

    /// E1004: Cyclic stage dependency
    ///
    /// Occurs when there is a circular dependency between staged functions
    /// that would create an infinite compilation loop.
    ///
    /// # Example
    /// ```verum
    /// // Hypothetical cyclic dependency
    /// meta(2) fn a() -> TokenStream {
    ///     quote { meta fn b() { @a() } }  // E1004: cycle
    /// }
    /// ```
    CyclicStage {
        /// Functions involved in the cycle
        cycle: List<Text>,
        /// Starting function
        start: Text,
        /// Source location where cycle was detected
        span: Span,
    },

    /// E1005: Invalid stage escape
    ///
    /// Occurs when `$(stage N) { ... }` escape syntax is used incorrectly.
    /// The escape stage must be between current_stage and 0.
    ///
    /// # Example
    /// ```verum
    /// meta(2) fn example() -> TokenStream {
    ///     quote {
    ///         $(stage 3) { ... }  // E1005: cannot escape to higher stage
    ///     }
    /// }
    /// ```
    InvalidStageEscape {
        /// Stage specified in the escape
        escape_stage: u32,
        /// Current stage context
        current_stage: u32,
        /// Valid range description
        valid_range: Text,
        /// Source location
        span: Span,
    },
}

/// Warnings from stage checking
#[derive(Debug, Clone, PartialEq)]
pub enum StageWarning {
    /// W1001: Unused stage definition
    ///
    /// Warns when a `meta(N)` function is defined but never invoked
    /// during compilation. This may indicate dead code.
    UnusedStage {
        /// Stage level of the unused function
        stage: u32,
        /// Name of the unused function
        function_name: Text,
        /// Source location
        span: Span,
    },

    /// W1002: Stage can be downgraded
    ///
    /// Warns when a `meta(N)` function could be lowered to `meta(N-1)`
    /// because it only generates code for stage N-2 or lower.
    /// Lower stages compile faster.
    StageDowngrade {
        /// Current stage level
        current_stage: u32,
        /// Suggested stage level
        suggested_stage: u32,
        /// Name of the function
        function_name: Text,
        /// Reason for the suggestion
        reason: Text,
        /// Source location
        span: Span,
    },
}

/// Configuration for stage checking
#[derive(Debug, Clone)]
pub struct StageConfig {
    /// Maximum allowed stage level (default: 2)
    /// Configured via `[meta] max_stage = N` in Verum.toml
    pub max_stage: u32,

    /// Enable strict mode for cross-stage calls (default: true)
    /// When true, any cross-stage call is an error.
    /// When false, downward calls (high to low) are warnings.
    pub strict_cross_stage: bool,

    /// Enable stage downgrade warnings (default: true)
    pub warn_stage_downgrade: bool,

    /// Enable unused stage warnings (default: true)
    pub warn_unused_stage: bool,
}

impl Default for StageConfig {
    fn default() -> Self {
        Self {
            max_stage: 2,
            strict_cross_stage: true,
            warn_stage_downgrade: true,
            warn_unused_stage: true,
        }
    }
}

/// Tracks stage information for a function
#[derive(Debug, Clone)]
pub struct FunctionStageInfo {
    /// Name of the function
    pub name: Text,
    /// Stage level (0 = runtime, 1+ = meta)
    pub stage: u32,
    /// Source span of the function declaration
    pub span: Span,
    /// Functions called by this function (name -> their stage)
    pub calls: HashMap<Text, u32>,
    /// Minimum stage of code generated by quote expressions
    pub min_generated_stage: Option<u32>,
    /// Maximum stage of code generated by quote expressions
    pub max_generated_stage: Option<u32>,
    /// Whether this function has been invoked during compilation
    pub invoked: bool,
}

impl FunctionStageInfo {
    /// Create a new function stage info
    pub fn new(name: Text, stage: u32, span: Span) -> Self {
        Self {
            name,
            stage,
            span,
            calls: HashMap::new(),
            min_generated_stage: None,
            max_generated_stage: None,
            invoked: false,
        }
    }

    /// Record a call to another function
    pub fn record_call(&mut self, callee: Text, callee_stage: u32) {
        self.calls.insert(callee, callee_stage);
    }

    /// Record generated code stage from a quote expression
    pub fn record_generated_stage(&mut self, stage: u32) {
        self.min_generated_stage = Some(
            self.min_generated_stage
                .map(|s| s.min(stage))
                .unwrap_or(stage),
        );
        self.max_generated_stage = Some(
            self.max_generated_stage
                .map(|s| s.max(stage))
                .unwrap_or(stage),
        );
    }

    /// Mark this function as invoked
    pub fn mark_invoked(&mut self) {
        self.invoked = true;
    }

    /// Check if this function can be downgraded to a lower stage
    pub fn can_downgrade(&self) -> Option<u32> {
        if self.stage <= 1 {
            return None; // Can't downgrade stage 0 or 1
        }

        // If we only generate code for stage N-2 or lower, we could be stage N-1
        if let Some(max_gen) = self.max_generated_stage {
            if max_gen < self.stage - 1 {
                return Some(self.stage - 1);
            }
        }

        None
    }
}

/// Stage checker for multi-stage metaprogramming
///
/// The StageChecker validates stage-related constraints during type checking:
/// - Stage coherence (quote generates correct stage)
/// - Cross-stage call restrictions
/// - Stage overflow detection
/// - Cyclic dependency detection
///
/// # Usage
///
/// ```rust,ignore
/// let config = StageConfig::default();
/// let mut checker = StageChecker::new(config);
///
/// // Enter a function context
/// checker.enter_function("my_meta_fn", 2, span);
///
/// // Check a quote expression
/// checker.check_quote(target_stage, quote_span)?;
///
/// // Check a function call
/// checker.check_call("helper", callee_stage, call_span)?;
///
/// // Exit function context
/// checker.exit_function();
///
/// // Collect warnings
/// let warnings = checker.collect_warnings();
/// ```
#[derive(Debug)]
pub struct StageChecker {
    /// Configuration
    config: StageConfig,

    /// Current function being checked (if any)
    current_function: Option<Text>,

    /// Current stage level (0 if outside meta function)
    current_stage: u32,

    /// Registered functions and their stage info
    functions: HashMap<Text, FunctionStageInfo>,

    /// Stack of function contexts for nested checking
    function_stack: Vec<(Text, u32)>,

    /// Collected errors
    errors: Vec<StageError>,

    /// Collected warnings
    warnings: Vec<StageWarning>,

    /// Functions that have been visited (for cycle detection)
    visited: HashSet<Text>,

    /// Current call path (for cycle detection)
    call_path: Vec<Text>,
}

impl StageChecker {
    /// Create a new stage checker with the given configuration
    pub fn new(config: StageConfig) -> Self {
        Self {
            config,
            current_function: None,
            current_stage: 0,
            functions: HashMap::new(),
            function_stack: Vec::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
            visited: HashSet::new(),
            call_path: Vec::new(),
        }
    }

    /// Create a stage checker with default configuration
    pub fn with_defaults() -> Self {
        Self::new(StageConfig::default())
    }

    /// Register a function with its stage information
    ///
    /// Call this for each meta function before checking its body.
    pub fn register_function(&mut self, name: Text, stage: u32, span: Span) -> Result<(), StageError> {
        // Check stage overflow
        if stage > self.config.max_stage {
            return Err(StageError::StageOverflow {
                used_stage: stage,
                max_stage: self.config.max_stage,
                function_name: name.clone(),
                span,
            });
        }

        let info = FunctionStageInfo::new(name.clone(), stage, span);
        self.functions.insert(name, info);
        Ok(())
    }

    /// Enter a function context for checking
    ///
    /// This sets the current stage and tracks nested function checking.
    pub fn enter_function(&mut self, name: &Text, stage: u32, _span: Span) {
        // Save current context
        if let Some(ref curr) = self.current_function {
            self.function_stack.push((curr.clone(), self.current_stage));
        }

        self.current_function = Some(name.clone());
        self.current_stage = stage;
        self.call_path.push(name.clone());
    }

    /// Exit the current function context
    pub fn exit_function(&mut self) {
        self.call_path.pop();

        // Restore previous context
        if let Some((name, stage)) = self.function_stack.pop() {
            self.current_function = Some(name);
            self.current_stage = stage;
        } else {
            self.current_function = None;
            self.current_stage = 0;
        }
    }

    /// Check a quote expression for stage correctness
    ///
    /// Validates that the quote generates code for the correct stage
    /// according to the Stage Coherence Rule.
    ///
    /// # Arguments
    /// - `target_stage`: The stage of the code being generated (None = current - 1)
    /// - `span`: Source location of the quote expression
    ///
    /// # Returns
    /// - `Ok(())` if the quote is valid
    /// - `Err(StageError::StageMismatch)` if the stages don't align
    pub fn check_quote(&mut self, target_stage: Option<u32>, span: Span) -> Result<(), StageError> {
        if self.current_stage == 0 {
            // Quote in runtime code is always invalid (no meta context)
            return Err(StageError::StageMismatch {
                current_stage: 0,
                target_stage: target_stage.unwrap_or(0),
                expected_stage: 0,
                span,
                hint: Text::from("quote can only be used in meta functions"),
            });
        }

        let actual_target = target_stage.unwrap_or(self.current_stage - 1);
        let expected_target = self.current_stage - 1;

        // Stage Coherence Rule: quote must generate exactly stage N-1
        if actual_target != expected_target {
            return Err(StageError::StageMismatch {
                current_stage: self.current_stage,
                target_stage: actual_target,
                expected_stage: expected_target,
                span,
                hint: if actual_target > expected_target {
                    Text::from("cannot generate higher stage code from quote")
                } else if actual_target < expected_target && self.current_stage > 1 {
                    Text::from(
                        "use nested quotes to lower stage by more than 1, or use quote(N) syntax",
                    )
                } else {
                    Text::from("quote can only generate code one stage lower")
                },
            });
        }

        // Record generated stage for downgrade analysis
        if let Some(ref name) = self.current_function {
            if let Some(info) = self.functions.get_mut(name) {
                info.record_generated_stage(actual_target);
            }
        }

        Ok(())
    }

    /// Check a function call for stage correctness
    ///
    /// Validates that the callee can be called from the current stage.
    ///
    /// # Cross-Stage Call Rules
    /// - Same stage calls are always allowed
    /// - Higher stage cannot directly call lower stage (must generate code)
    /// - Lower stage cannot call higher stage (not yet compiled)
    ///
    /// # Arguments
    /// - `callee_name`: Name of the function being called
    /// - `callee_stage`: Stage of the callee function
    /// - `span`: Source location of the call
    pub fn check_call(
        &mut self,
        callee_name: &Text,
        callee_stage: u32,
        span: Span,
    ) -> Result<(), StageError> {
        // Same stage is always OK
        if callee_stage == self.current_stage {
            self.record_call(callee_name, callee_stage);
            return Ok(());
        }

        // Check for cycles
        if self.call_path.contains(callee_name) {
            let cycle_start = self.call_path.iter().position(|n| n == callee_name).unwrap_or(0);
            let cycle: List<Text> = self.call_path[cycle_start..]
                .iter()
                .cloned()
                .chain(std::iter::once(callee_name.clone()))
                .collect::<Vec<_>>()
                .into();

            return Err(StageError::CyclicStage {
                cycle,
                start: callee_name.clone(),
                span,
            });
        }

        // Cross-stage call
        let error = StageError::CrossStageCall {
            caller_stage: self.current_stage,
            callee_stage,
            callee_name: callee_name.clone(),
            span,
            hint: if callee_stage < self.current_stage {
                Text::from(format!(
                    "stage {} cannot directly call stage {}; generate a call via quote instead",
                    self.current_stage, callee_stage
                ))
            } else {
                Text::from(format!(
                    "stage {} cannot call stage {} (not yet compiled)",
                    self.current_stage, callee_stage
                ))
            },
        };

        if self.config.strict_cross_stage {
            return Err(error);
        }

        // In non-strict mode, cross-stage calls are collected but not errors
        self.record_call(callee_name, callee_stage);
        Ok(())
    }

    /// Check a stage escape expression `$(stage N) { ... }`
    ///
    /// Validates that the escape stage is valid for the current context.
    pub fn check_stage_escape(&mut self, escape_stage: u32, span: Span) -> Result<(), StageError> {
        // Escape stage must be between 0 and current_stage (exclusive)
        if escape_stage >= self.current_stage {
            return Err(StageError::InvalidStageEscape {
                escape_stage,
                current_stage: self.current_stage,
                valid_range: Text::from(format!("0..{}", self.current_stage)),
                span,
            });
        }

        Ok(())
    }

    /// Mark a function as invoked
    pub fn mark_invoked(&mut self, name: &Text) {
        if let Some(info) = self.functions.get_mut(name) {
            info.mark_invoked();
        }
    }

    /// Record a call in the current function's info
    fn record_call(&mut self, callee: &Text, callee_stage: u32) {
        if let Some(ref name) = self.current_function {
            if let Some(info) = self.functions.get_mut(name) {
                info.record_call(callee.clone(), callee_stage);
            }
        }
    }

    /// Collect all errors
    pub fn errors(&self) -> &[StageError] {
        &self.errors
    }

    /// Collect all warnings
    pub fn collect_warnings(&mut self) -> Vec<StageWarning> {
        let mut warnings = std::mem::take(&mut self.warnings);

        // Check for unused stages
        if self.config.warn_unused_stage {
            for info in self.functions.values() {
                if info.stage > 0 && !info.invoked {
                    warnings.push(StageWarning::UnusedStage {
                        stage: info.stage,
                        function_name: info.name.clone(),
                        span: info.span,
                    });
                }
            }
        }

        // Check for potential stage downgrades
        if self.config.warn_stage_downgrade {
            for info in self.functions.values() {
                if let Some(suggested) = info.can_downgrade() {
                    warnings.push(StageWarning::StageDowngrade {
                        current_stage: info.stage,
                        suggested_stage: suggested,
                        function_name: info.name.clone(),
                        reason: Text::from(format!(
                            "function only generates stage {} code",
                            info.max_generated_stage.unwrap_or(0)
                        )),
                        span: info.span,
                    });
                }
            }
        }

        warnings
    }

    /// Check if there are any errors
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Get the current stage level
    pub fn current_stage(&self) -> u32 {
        self.current_stage
    }

    /// Get the maximum configured stage
    pub fn max_stage(&self) -> u32 {
        self.config.max_stage
    }

    /// Get information about a registered function
    pub fn get_function_info(&self, name: &Text) -> Option<&FunctionStageInfo> {
        self.functions.get(name)
    }

    /// Check a variable reference for stage correctness.
    ///
    /// A variable at `var_stage` can only be referenced from the current stage
    /// if the stages match. Cross-stage variable references are errors.
    pub fn check_variable_reference(
        &mut self,
        var_name: &Text,
        var_stage: u32,
        span: Span,
    ) -> Result<(), StageError> {
        // Same stage or lower is OK (lower stage values are always available)
        if var_stage <= self.current_stage {
            return Ok(());
        }

        // Higher stage variable referenced from lower stage
        Err(StageError::CrossStageCall {
            caller_stage: self.current_stage,
            callee_stage: var_stage,
            callee_name: var_name.clone(),
            span,
            hint: Text::from(format!(
                "stage {} cannot reference compile-time stage {} variable '{}'; it is not available at runtime",
                self.current_stage, var_stage, var_name
            )),
        })
    }

    /// Check that a type used in generated code is at the correct stage.
    ///
    /// A type at `type_stage` can be used in code generated at `target_stage`
    /// only if the type stage is at or below the target stage.
    pub fn check_quote_type(
        &mut self,
        target_stage: u32,
        type_name: &Text,
        type_stage: u32,
        span: Span,
    ) -> Result<(), StageError> {
        if type_stage <= target_stage {
            return Ok(());
        }

        Err(StageError::StageMismatch {
            current_stage: self.current_stage,
            target_stage: type_stage,
            expected_stage: target_stage,
            span,
            hint: Text::from(format!(
                "type '{}' at stage {} cannot be used in stage {} generated code",
                type_name, type_stage, target_stage
            )),
        })
    }

    /// Check a splice/unquote expression for stage correctness.
    ///
    /// A splice expression must reference the current stage.
    pub fn check_splice(&mut self, splice_stage: u32, span: Span) -> Result<(), StageError> {
        if splice_stage == self.current_stage {
            return Ok(());
        }

        Err(StageError::StageMismatch {
            current_stage: self.current_stage,
            target_stage: splice_stage,
            expected_stage: self.current_stage,
            span,
            hint: Text::from("splice expression stage mismatch"),
        })
    }

    /// Get mutable information about a registered function
    pub fn get_function_info_mut(&mut self, name: &Text) -> Option<&mut FunctionStageInfo> {
        self.functions.get_mut(name)
    }

    /// Clear all state (for reuse)
    pub fn clear(&mut self) {
        self.current_function = None;
        self.current_stage = 0;
        self.functions.clear();
        self.function_stack.clear();
        self.errors.clear();
        self.warnings.clear();
        self.visited.clear();
        self.call_path.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::FileId;

    fn test_span() -> Span {
        Span::new(0, 10, FileId::new(0))
    }

    #[test]
    fn test_register_function_valid() {
        let mut checker = StageChecker::with_defaults();
        assert!(checker
            .register_function(Text::from("my_meta"), 1, test_span())
            .is_ok());
        assert!(checker
            .register_function(Text::from("my_meta2"), 2, test_span())
            .is_ok());
    }

    #[test]
    fn test_register_function_overflow() {
        let mut checker = StageChecker::with_defaults();
        let result = checker.register_function(Text::from("bad"), 3, test_span());
        assert!(matches!(result, Err(StageError::StageOverflow { .. })));
    }

    #[test]
    fn test_quote_stage_1_valid() {
        let mut checker = StageChecker::with_defaults();
        checker
            .register_function(Text::from("meta_fn"), 1, test_span())
            .unwrap();
        checker.enter_function(&Text::from("meta_fn"), 1, test_span());

        // Stage 1 generates stage 0 (runtime) - valid
        assert!(checker.check_quote(None, test_span()).is_ok());
        assert!(checker.check_quote(Some(0), test_span()).is_ok());
    }

    #[test]
    fn test_quote_stage_2_valid() {
        let mut checker = StageChecker::with_defaults();
        checker
            .register_function(Text::from("meta2_fn"), 2, test_span())
            .unwrap();
        checker.enter_function(&Text::from("meta2_fn"), 2, test_span());

        // Stage 2 generates stage 1 - valid
        assert!(checker.check_quote(None, test_span()).is_ok());
        assert!(checker.check_quote(Some(1), test_span()).is_ok());
    }

    #[test]
    fn test_quote_stage_mismatch() {
        let mut checker = StageChecker::with_defaults();
        checker
            .register_function(Text::from("meta2_fn"), 2, test_span())
            .unwrap();
        checker.enter_function(&Text::from("meta2_fn"), 2, test_span());

        // Stage 2 cannot directly generate stage 0
        let result = checker.check_quote(Some(0), test_span());
        assert!(matches!(result, Err(StageError::StageMismatch { .. })));
    }

    #[test]
    fn test_same_stage_call_valid() {
        let mut checker = StageChecker::with_defaults();
        checker
            .register_function(Text::from("caller"), 1, test_span())
            .unwrap();
        checker
            .register_function(Text::from("callee"), 1, test_span())
            .unwrap();
        checker.enter_function(&Text::from("caller"), 1, test_span());

        // Same stage call is valid
        assert!(checker
            .check_call(&Text::from("callee"), 1, test_span())
            .is_ok());
    }

    #[test]
    fn test_cross_stage_call_error() {
        let mut checker = StageChecker::with_defaults();
        checker
            .register_function(Text::from("meta2_fn"), 2, test_span())
            .unwrap();
        checker
            .register_function(Text::from("meta1_fn"), 1, test_span())
            .unwrap();
        checker.enter_function(&Text::from("meta2_fn"), 2, test_span());

        // Stage 2 cannot call stage 1
        let result = checker.check_call(&Text::from("meta1_fn"), 1, test_span());
        assert!(matches!(result, Err(StageError::CrossStageCall { .. })));
    }

    #[test]
    fn test_stage_escape_valid() {
        let mut checker = StageChecker::with_defaults();
        checker
            .register_function(Text::from("meta2_fn"), 2, test_span())
            .unwrap();
        checker.enter_function(&Text::from("meta2_fn"), 2, test_span());

        // Escape to stage 0 or 1 from stage 2 is valid
        assert!(checker.check_stage_escape(0, test_span()).is_ok());
        assert!(checker.check_stage_escape(1, test_span()).is_ok());
    }

    #[test]
    fn test_stage_escape_invalid() {
        let mut checker = StageChecker::with_defaults();
        checker
            .register_function(Text::from("meta2_fn"), 2, test_span())
            .unwrap();
        checker.enter_function(&Text::from("meta2_fn"), 2, test_span());

        // Cannot escape to same or higher stage
        let result = checker.check_stage_escape(2, test_span());
        assert!(matches!(result, Err(StageError::InvalidStageEscape { .. })));

        let result = checker.check_stage_escape(3, test_span());
        assert!(matches!(result, Err(StageError::InvalidStageEscape { .. })));
    }

    #[test]
    fn test_unused_stage_warning() {
        let mut checker = StageChecker::with_defaults();
        checker
            .register_function(Text::from("unused"), 2, test_span())
            .unwrap();

        // Don't invoke the function
        let warnings = checker.collect_warnings();
        assert!(warnings
            .iter()
            .any(|w| matches!(w, StageWarning::UnusedStage { .. })));
    }

    #[test]
    fn test_stage_downgrade_warning() {
        let mut checker = StageChecker::with_defaults();
        checker
            .register_function(Text::from("can_downgrade"), 2, test_span())
            .unwrap();

        // Enter and record that we only generate stage 0 code
        checker.enter_function(&Text::from("can_downgrade"), 2, test_span());
        if let Some(info) = checker.functions.get_mut(&Text::from("can_downgrade")) {
            info.record_generated_stage(0);
            info.mark_invoked();
        }
        checker.exit_function();

        let warnings = checker.collect_warnings();
        assert!(warnings
            .iter()
            .any(|w| matches!(w, StageWarning::StageDowngrade { .. })));
    }

    #[test]
    fn test_nested_function_context() {
        let mut checker = StageChecker::with_defaults();
        checker
            .register_function(Text::from("outer"), 2, test_span())
            .unwrap();
        checker
            .register_function(Text::from("inner"), 2, test_span())
            .unwrap();

        checker.enter_function(&Text::from("outer"), 2, test_span());
        assert_eq!(checker.current_stage(), 2);

        checker.enter_function(&Text::from("inner"), 2, test_span());
        assert_eq!(checker.current_stage(), 2);

        checker.exit_function();
        assert_eq!(checker.current_stage(), 2);

        checker.exit_function();
        assert_eq!(checker.current_stage(), 0);
    }
}
