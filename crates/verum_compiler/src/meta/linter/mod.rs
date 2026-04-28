//! Meta Linter - Static Analysis for Meta Function Safety
//!
//! This module implements the Meta Linter, which automatically detects unsafe patterns
//! in meta functions and enforces safety annotations.
//!
//! The Meta Linter provides static analysis for safety in compile-time code.
//! It detects unsafe patterns and requires explicit @safe/@unsafe annotations.
//! Meta functions marked @safe are verified to have no unsafe patterns.
//! Functions with detected unsafe patterns are auto-marked @unsafe and generate
//! warnings at usage sites.
//!
//! ## Detected Unsafe Patterns
//!
//! The linter automatically detects these unsafe patterns:
//! 1. String concatenation with external input (injection risk)
//! 2. Direct format! with user input (no escaping)
//! 3. Unchecked type casts
//! 4. panic!/unwrap() calls (compile-time failures)
//! 5. Unbounded recursion
//! 6. Unbounded loops
//! 7. Hidden I/O operations
//!
//! ## Safety Annotations
//!
//! - `@safe`: Explicitly marks meta function as safe (linter verifies)
//! - `@unsafe`: Explicitly marks meta function as potentially unsafe
//!
//! ## Module Structure
//!
//! - `patterns` - UnsafePatternKind definitions
//! - `results` - LintResult, LintWarning, LintError
//! - `config` - LinterConfig
//! - `dataflow` - External input tracking
//! - `security` - CWE-mapped security pattern detection
//! - `safety` - General safety pattern detection

pub mod config;
pub mod dataflow;
pub mod patterns;
pub mod results;
pub mod safety;
pub mod security;

use verum_ast::decl::{FunctionBody, FunctionDecl, FunctionParamKind};
use verum_ast::expr::{Expr, ExprKind, RecoverBody};
use verum_ast::stmt::{Stmt, StmtKind};
use verum_common::{List, Maybe, Text};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};

pub use config::LinterConfig;
pub use dataflow::{AnalysisContext, ExternalInputChecker};
pub use patterns::UnsafePatternKind;
pub use results::{LintError, LintResult, LintWarning, UnsafePattern};
pub use safety::SafetyDetector;
pub use security::SecurityDetector;

/// The Meta Linter analyzes meta functions for safety
pub struct MetaLinter {
    /// Configuration
    config: LinterConfig,
    /// Security pattern detector
    security: SecurityDetector,
    /// Safety pattern detector
    safety: SafetyDetector,
}

impl MetaLinter {
    /// Create a new meta linter with default configuration
    pub fn new() -> Self {
        Self {
            config: LinterConfig::default(),
            security: SecurityDetector::new(),
            safety: SafetyDetector::new(),
        }
    }

    /// Create a linter with custom configuration
    pub fn with_config(config: LinterConfig) -> Self {
        let safety = SafetyDetector::with_forbidden(config.forbidden_functions.clone());
        Self {
            config,
            security: SecurityDetector::new(),
            safety,
        }
    }

    /// Lint a meta function declaration
    ///
    /// Returns a LintResult indicating whether the function is safe
    /// and any detected issues.
    pub fn lint_function(&self, func: &FunctionDecl) -> LintResult {
        let mut result = LintResult::safe();
        let mut ctx = AnalysisContext::new();

        // Set current function name
        ctx.current_function = Some(func.name.as_str().to_string());

        // Mark all parameters as external input
        for param in func.params.iter() {
            if let FunctionParamKind::Regular { pattern, .. } = &param.kind {
                ExternalInputChecker::mark_pattern_vars_external(pattern, &mut ctx);
            }
        }

        // Check if function has @safe/@unsafe annotation
        let has_safe_attr = self.has_safe_attr(func);
        let has_unsafe_attr = self.has_unsafe_attr(func);

        // Analyze function body if present
        if let Some(body) = &func.body {
            self.analyze_body_with_context(body, &mut result, &mut ctx);
        }

        // Check for recursion after analyzing the body.  Gated on
        // `check_performance` — the flag was previously inert (set
        // but never read), so disabling it had no effect.  Now
        // `permissive()` and other configurations that opt out of
        // performance lints actually skip the recursion warning,
        // matching the documented contract.
        if self.config.check_performance {
            self.safety
                .check_unbounded_recursion(&ctx, func.span, &mut result);
        }

        // Validate annotations
        if has_safe_attr && !result.is_safe {
            result.add_error(LintError {
                message: Text::from(format!(
                    "Function '{}' is marked @safe but contains unsafe patterns",
                    func.name.as_str()
                )),
                span: func.span,
                help: Maybe::Some(Text::from(
                    "Either fix the unsafe patterns or change to @unsafe",
                )),
            });
        }

        if !has_safe_attr && !has_unsafe_attr && !result.is_safe {
            // Auto-mark as unsafe
            result.add_warning(LintWarning {
                message: Text::from(format!(
                    "Meta function '{}' automatically marked @unsafe due to detected patterns",
                    func.name.as_str()
                )),
                span: func.span,
                suggestion: Maybe::Some(Text::from(
                    "Add @unsafe annotation or fix the unsafe patterns",
                )),
            });
        }

        if self.config.require_explicit_safe && !has_safe_attr && !has_unsafe_attr {
            result.add_error(LintError {
                message: Text::from(format!(
                    "Meta function '{}' requires explicit @safe or @unsafe annotation",
                    func.name.as_str()
                )),
                span: func.span,
                help: Maybe::Some(Text::from(
                    "Add @safe if the function is verified safe, or @unsafe otherwise",
                )),
            });
        }

        result
    }

    /// Analyze a function body for unsafe patterns with context
    fn analyze_body_with_context(
        &self,
        body: &FunctionBody,
        result: &mut LintResult,
        ctx: &mut AnalysisContext,
    ) {
        match body {
            FunctionBody::Block(block) => {
                for stmt in block.stmts.iter() {
                    self.analyze_stmt_with_context(stmt, result, ctx);
                }
                if let Some(expr) = &block.expr {
                    self.analyze_expr_with_context(expr, result, ctx);
                }
            }
            FunctionBody::Expr(expr) => {
                self.analyze_expr_with_context(expr, result, ctx);
            }
        }
    }

    /// Analyze a statement for unsafe patterns with context
    fn analyze_stmt_with_context(
        &self,
        stmt: &Stmt,
        result: &mut LintResult,
        ctx: &mut AnalysisContext,
    ) {
        match &stmt.kind {
            StmtKind::Let { pattern, value, .. } => {
                // Track bound variables as potentially external if value is external
                if let Some(expr) = value {
                    let value_is_external = ExternalInputChecker::expr_uses_external(expr, ctx);
                    if value_is_external {
                        ExternalInputChecker::mark_pattern_vars_external(pattern, ctx);
                    }
                    self.analyze_expr_with_context(expr, result, ctx);
                }
            }
            StmtKind::LetElse { pattern, value, .. } => {
                let value_is_external = ExternalInputChecker::expr_uses_external(value, ctx);
                if value_is_external {
                    ExternalInputChecker::mark_pattern_vars_external(pattern, ctx);
                }
                self.analyze_expr_with_context(value, result, ctx);
            }
            StmtKind::Expr { expr, .. } => {
                self.analyze_expr_with_context(expr, result, ctx);
            }
            StmtKind::Defer(expr) => {
                self.analyze_expr_with_context(expr, result, ctx);
            }
            StmtKind::Errdefer(expr) => {
                self.analyze_expr_with_context(expr, result, ctx);
            }
            StmtKind::Provide { value, .. } => {
                self.analyze_expr_with_context(value, result, ctx);
            }
            _ => {}
        }
    }

    /// Analyze an expression for unsafe patterns with context
    fn analyze_expr_with_context(
        &self,
        expr: &Expr,
        result: &mut LintResult,
        ctx: &mut AnalysisContext,
    ) {
        match &expr.kind {
            // Check for string concatenation
            ExprKind::Binary { op, left, right } => {
                self.safety
                    .check_string_concat(op, left, right, ctx, expr.span, result);
                self.analyze_expr_with_context(left, result, ctx);
                self.analyze_expr_with_context(right, result, ctx);
            }

            // Check for potentially unsafe function calls
            ExprKind::Call { func, args, .. } => {
                if let Some(func_name) = self.extract_function_name(func) {
                    // Record function calls for recursion detection
                    if let Some(ref current_func) = ctx.current_function {
                        ctx.record_call(current_func.clone(), func_name.clone());
                    }

                    // Security checks
                    self.security
                        .check_unsafe_format(&func_name, args, ctx, expr.span, result);
                    self.security
                        .check_sql_injection(&func_name, args, ctx, expr.span, result);
                    self.security
                        .check_command_injection(&func_name, args, ctx, expr.span, result);
                    self.security
                        .check_dynamic_code_execution(&func_name, expr.span, result);
                    self.security
                        .check_hidden_io(&func_name, expr.span, result);

                    // Safety checks
                    self.safety
                        .check_forbidden_function(&func_name, expr.span, result);

                    // Check for unwrap/expect
                    if func_name.ends_with(".unwrap") || func_name.ends_with(".expect") {
                        self.safety
                            .check_unwrap_expect(&func_name, expr.span, result);
                    }
                }

                // Recurse into arguments
                for arg in args.iter() {
                    self.analyze_expr_with_context(arg, result, ctx);
                }
            }

            // Check method calls
            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                let method_name = method.as_str();

                // Check for unwrap/expect
                self.safety
                    .check_unwrap_expect(method_name, expr.span, result);

                // Check for non-deterministic methods.  Gated on
                // `check_determinism` — the flag was previously
                // inert; now `permissive()` (which sets
                // `check_determinism = false`) actually skips the
                // `now()` / `random()` warning.
                if self.config.check_determinism {
                    self.safety
                        .check_non_deterministic(method_name, expr.span, result);
                }

                // Check for I/O methods
                if self.security.is_io_method(method_name) {
                    self.security
                        .check_hidden_io(method_name, expr.span, result);
                }

                self.analyze_expr_with_context(receiver, result, ctx);
                for arg in args.iter() {
                    self.analyze_expr_with_context(arg, result, ctx);
                }
            }

            // Recurse into compound expressions
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                for cond in &condition.conditions {
                    match cond {
                        verum_ast::expr::ConditionKind::Expr(e) => {
                            self.analyze_expr_with_context(e, result, ctx)
                        }
                        verum_ast::expr::ConditionKind::Let { value, .. } => {
                            self.analyze_expr_with_context(value, result, ctx)
                        }
                    }
                }
                for stmt in &then_branch.stmts {
                    self.analyze_stmt_with_context(stmt, result, ctx);
                }
                if let Some(then_expr) = &then_branch.expr {
                    self.analyze_expr_with_context(then_expr, result, ctx);
                }
                if let Some(else_expr) = else_branch {
                    self.analyze_expr_with_context(else_expr, result, ctx);
                }
            }

            ExprKind::Match { expr, arms } => {
                self.analyze_expr_with_context(expr, result, ctx);
                for arm in arms.iter() {
                    if let Some(guard) = &arm.guard {
                        self.analyze_expr_with_context(guard, result, ctx);
                    }
                    self.analyze_expr_with_context(&arm.body, result, ctx);
                }
            }

            ExprKind::Block(block) => {
                for stmt in block.stmts.iter() {
                    self.analyze_stmt_with_context(stmt, result, ctx);
                }
                if let Some(block_expr) = &block.expr {
                    self.analyze_expr_with_context(block_expr, result, ctx);
                }
            }

            ExprKind::Loop {
                label: _,
                body: block,
                invariants: _,
            } => {
                let old_break_state = ctx.has_break_in_loop;
                ctx.has_break_in_loop = false;

                for stmt in block.stmts.iter() {
                    self.analyze_stmt_with_context(stmt, result, ctx);
                }
                if let Some(block_expr) = &block.expr {
                    self.analyze_expr_with_context(block_expr, result, ctx);
                }

                self.safety
                    .check_unbounded_loop(ctx.has_break_in_loop, expr.span, result);

                ctx.has_break_in_loop = old_break_state;
            }

            ExprKind::While {
                label: _,
                condition,
                body,
                invariants: _,
                decreases: _,
            } => {
                // Check for while true
                if let ExprKind::Literal(lit) = &condition.kind {
                    if matches!(lit.kind, verum_ast::LiteralKind::Bool(true)) {
                        let old_break_state = ctx.has_break_in_loop;
                        ctx.has_break_in_loop = false;

                        for stmt in body.stmts.iter() {
                            self.analyze_stmt_with_context(stmt, result, ctx);
                        }
                        if let Some(body_expr) = &body.expr {
                            self.analyze_expr_with_context(body_expr, result, ctx);
                        }

                        self.safety
                            .check_while_true(ctx.has_break_in_loop, condition.span, result);

                        ctx.has_break_in_loop = old_break_state;
                        return;
                    }
                }
                self.analyze_expr_with_context(condition, result, ctx);
                for stmt in body.stmts.iter() {
                    self.analyze_stmt_with_context(stmt, result, ctx);
                }
                if let Some(expr) = &body.expr {
                    self.analyze_expr_with_context(expr, result, ctx);
                }
            }

            // Other expression types - recurse as needed
            ExprKind::Unary { expr: inner, .. } => {
                self.analyze_expr_with_context(inner, result, ctx);
            }

            ExprKind::Field { expr: inner, .. } | ExprKind::TupleIndex { expr: inner, .. } => {
                self.analyze_expr_with_context(inner, result, ctx);
            }

            ExprKind::Index { expr: base, index } => {
                self.analyze_expr_with_context(base, result, ctx);
                self.analyze_expr_with_context(index, result, ctx);
            }

            ExprKind::Tuple(elements) => {
                for elem in elements.iter() {
                    self.analyze_expr_with_context(elem, result, ctx);
                }
            }

            ExprKind::Array(array_expr) => match array_expr {
                verum_ast::expr::ArrayExpr::List(elements) => {
                    for elem in elements.iter() {
                        self.analyze_expr_with_context(elem, result, ctx);
                    }
                }
                verum_ast::expr::ArrayExpr::Repeat { value, count } => {
                    self.analyze_expr_with_context(value, result, ctx);
                    self.analyze_expr_with_context(count, result, ctx);
                }
            },

            ExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.analyze_expr_with_context(s, result, ctx);
                }
                if let Some(e) = end {
                    self.analyze_expr_with_context(e, result, ctx);
                }
            }

            ExprKind::Comprehension { expr, clauses: _ }
            | ExprKind::StreamComprehension { expr, clauses: _ } => {
                self.analyze_expr_with_context(expr, result, ctx);
            }

            ExprKind::Record { fields, base, .. } => {
                for field in fields {
                    if let Some(value) = &field.value {
                        self.analyze_expr_with_context(value, result, ctx);
                    }
                }
                if let Some(base_expr) = base {
                    self.analyze_expr_with_context(base_expr, result, ctx);
                }
            }

            ExprKind::Try(inner) => {
                self.analyze_expr_with_context(inner, result, ctx);
            }

            ExprKind::TryRecover { try_block, recover } => {
                self.analyze_expr_with_context(try_block, result, ctx);
                match recover {
                    RecoverBody::MatchArms { arms, .. } => {
                        for arm in arms.iter() {
                            if let Some(guard) = &arm.guard {
                                self.analyze_expr_with_context(guard, result, ctx);
                            }
                            self.analyze_expr_with_context(&arm.body, result, ctx);
                        }
                    }
                    RecoverBody::Closure { body, .. } => {
                        self.analyze_expr_with_context(body, result, ctx);
                    }
                }
            }

            ExprKind::TryFinally {
                try_block,
                finally_block,
            } => {
                self.analyze_expr_with_context(try_block, result, ctx);
                self.analyze_expr_with_context(finally_block, result, ctx);
            }

            ExprKind::TryRecoverFinally {
                try_block,
                recover,
                finally_block,
            } => {
                self.analyze_expr_with_context(try_block, result, ctx);
                match recover {
                    RecoverBody::MatchArms { arms, .. } => {
                        for arm in arms.iter() {
                            if let Some(guard) = &arm.guard {
                                self.analyze_expr_with_context(guard, result, ctx);
                            }
                            self.analyze_expr_with_context(&arm.body, result, ctx);
                        }
                    }
                    RecoverBody::Closure { body, .. } => {
                        self.analyze_expr_with_context(body, result, ctx);
                    }
                }
                self.analyze_expr_with_context(finally_block, result, ctx);
            }

            ExprKind::Cast { expr: inner, .. } => {
                self.analyze_expr_with_context(inner, result, ctx);
            }

            ExprKind::Pipeline { left, right } | ExprKind::NullCoalesce { left, right } => {
                self.analyze_expr_with_context(left, result, ctx);
                self.analyze_expr_with_context(right, result, ctx);
            }

            ExprKind::OptionalChain { expr: inner, .. } => {
                self.analyze_expr_with_context(inner, result, ctx);
            }

            // Track break statements for loop analysis
            ExprKind::Break { value, .. } => {
                ctx.has_break_in_loop = true;
                if let Some(val) = value {
                    self.analyze_expr_with_context(val, result, ctx);
                }
            }

            ExprKind::Return(value) => {
                ctx.has_break_in_loop = true; // Return also terminates loops
                if let Some(val) = value {
                    self.analyze_expr_with_context(val, result, ctx);
                }
            }

            // Safe expressions (no recursion needed)
            ExprKind::Literal(_) | ExprKind::Path(_) => {}

            // Other expressions
            _ => {}
        }
    }

    /// Extract function name from a call expression
    fn extract_function_name(&self, func_expr: &Expr) -> Option<String> {
        match &func_expr.kind {
            ExprKind::Path(path) => {
                let segments: Vec<String> = path
                    .segments
                    .iter()
                    .filter_map(|seg| {
                        if let verum_ast::ty::PathSegment::Name(ident) = seg {
                            Some(ident.as_str().to_string())
                        } else {
                            None
                        }
                    })
                    .collect();
                Some(segments.join("."))
            }
            _ => None,
        }
    }

    /// Check if function has @safe annotation
    fn has_safe_attr(&self, func: &FunctionDecl) -> bool {
        func.attributes
            .iter()
            .any(|attr| attr.name.as_str() == "safe")
    }

    /// Check if function has @unsafe annotation
    fn has_unsafe_attr(&self, func: &FunctionDecl) -> bool {
        func.attributes
            .iter()
            .any(|attr| attr.name.as_str() == "unsafe")
    }

    /// Calculate cyclomatic complexity of a function
    pub fn calculate_complexity(&self, func: &FunctionDecl) -> usize {
        self.safety.calculate_complexity(func)
    }

    /// Convert lint result to diagnostics
    pub fn to_diagnostics(&self, result: &LintResult, _func: &FunctionDecl) -> List<Diagnostic> {
        let mut diagnostics = List::new();

        // Convert unsafe patterns to diagnostics
        for pattern in result.unsafe_patterns.iter() {
            let severity = if self.config.unsafe_as_error {
                Severity::Error
            } else {
                pattern.kind.severity()
            };

            let mut builder = DiagnosticBuilder::new(severity).message(format!(
                "[{}] {}",
                pattern.kind.name(),
                pattern.description.as_str()
            ));

            if let Maybe::Some(ref suggestion) = pattern.suggestion {
                builder = builder.help(suggestion.to_string());
            }

            diagnostics.push(builder.build());
        }

        // Convert warnings
        for warning in result.warnings.iter() {
            let mut builder =
                DiagnosticBuilder::new(Severity::Warning).message(warning.message.to_string());

            if let Maybe::Some(ref suggestion) = warning.suggestion {
                builder = builder.help(suggestion.to_string());
            }

            diagnostics.push(builder.build());
        }

        // Convert errors
        for error in result.errors.iter() {
            let mut builder =
                DiagnosticBuilder::new(Severity::Error).message(error.message.to_string());

            if let Maybe::Some(ref help) = error.help {
                builder = builder.help(help.to_string());
            }

            diagnostics.push(builder.build());
        }

        diagnostics
    }
}

impl Default for MetaLinter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linter_creation() {
        let linter = MetaLinter::new();
        assert!(linter.security.is_io_function("File.read"));
    }

    #[test]
    fn test_linter_with_config() {
        let config = LinterConfig::strict();
        let linter = MetaLinter::with_config(config);
        assert!(linter.config.unsafe_as_error);
    }
}
