//! Hygiene Checker
//!
//! Verifies that expanded macro code maintains proper hygiene, detecting
//! violations such as accidental capture, shadow conflicts, and stage mismatches.
//!
//! ## Verification Algorithm
//!
//! The checker walks the expanded AST and for each identifier:
//! 1. Resolves the binding using sets-of-scopes algorithm
//! 2. Verifies mark compatibility between reference and binding
//! 3. Checks stage accessibility
//! 4. Reports any violations found
//!
//! Hygienic macro system: syntax contexts track expansion origins to prevent
//! macro-introduced bindings from capturing or shadowing user bindings.
//! Uses mark/unmark operations on syntax contexts for proper scoping.

use verum_ast::{
    expr::{Block, Expr, ExprKind},
    pattern::{Pattern, PatternKind},
    stmt::{Stmt, StmtKind},
    ty::Path,
    Span,
};
use verum_common::{List, Map, Text};

use super::expander::StageContext;
use super::scope::{BindingInfo, BindingKind, HygienicIdent, ScopeId, ScopeKind, ScopeSet};
use super::violations::{HygieneViolation, HygieneViolations};
use super::HygieneContext;

/// Configuration for hygiene checking
#[derive(Debug, Clone)]
pub struct CheckerConfig {
    /// Whether to use strict mode (fail on any violation)
    pub strict_mode: bool,
    /// Whether to allow shadow conflicts in some cases
    pub allow_shadow_recovery: bool,
    /// Maximum number of violations to collect before stopping
    pub max_violations: usize,
    /// Whether to include warning-level issues
    pub include_warnings: bool,
}

impl Default for CheckerConfig {
    fn default() -> Self {
        Self {
            strict_mode: true,
            allow_shadow_recovery: false,
            max_violations: 100,
            include_warnings: true,
        }
    }
}

/// Result of hygiene checking
#[derive(Debug, Clone)]
pub struct CheckResult {
    /// All violations found
    pub violations: HygieneViolations,
    /// Statistics about the check
    pub stats: CheckStats,
}

impl CheckResult {
    /// Create a successful result with no violations
    pub fn success() -> Self {
        Self {
            violations: HygieneViolations::new(),
            stats: CheckStats::default(),
        }
    }

    /// Check if the result is successful (no fatal violations)
    pub fn is_success(&self) -> bool {
        !self.violations.has_fatal()
    }

    /// Convert to a Result type
    pub fn into_result(self) -> Result<CheckStats, HygieneViolations> {
        if self.is_success() {
            Ok(self.stats)
        } else {
            Err(self.violations)
        }
    }
}

/// Statistics about the hygiene check
#[derive(Debug, Clone, Default)]
pub struct CheckStats {
    /// Number of expressions checked
    pub expressions_checked: usize,
    /// Number of bindings verified
    pub bindings_verified: usize,
    /// Number of references resolved
    pub references_resolved: usize,
    /// Number of warnings issued
    pub warnings: usize,
}

/// The main hygiene checker
#[derive(Debug)]
pub struct HygieneChecker {
    /// Configuration
    config: CheckerConfig,
    /// The hygiene context
    context: HygieneContext,
    /// Stage context for multi-stage checking
    stage_context: StageContext,
    /// Accumulated violations
    violations: HygieneViolations,
    /// Statistics
    stats: CheckStats,
    /// Current scope stack for tracking
    scope_stack: List<CheckerScope>,
    /// Binding table for fast lookup
    bindings: Map<Text, List<BindingEntry>>,
}

/// A scope in the checker's stack
#[derive(Debug, Clone)]
struct CheckerScope {
    /// The scope ID
    id: ScopeId,
    /// The kind of scope
    kind: ScopeKind,
    /// Bindings introduced in this scope
    bindings: Map<Text, BindingInfo>,
}

/// An entry in the binding table
#[derive(Debug, Clone)]
struct BindingEntry {
    /// The binding info
    #[allow(dead_code)]
    info: BindingInfo,
    /// The scope where it was introduced
    scope_id: ScopeId,
    /// The scopes at the point of introduction
    scopes: ScopeSet,
}

impl HygieneChecker {
    /// Create a new hygiene checker
    pub fn new(context: HygieneContext, config: CheckerConfig) -> Self {
        Self {
            config,
            context,
            stage_context: StageContext::new(0),
            violations: HygieneViolations::new(),
            stats: CheckStats::default(),
            scope_stack: List::new(),
            bindings: Map::new(),
        }
    }

    /// Create with default configuration
    pub fn with_default_config(context: HygieneContext) -> Self {
        Self::new(context, CheckerConfig::default())
    }

    /// Create for a specific stage
    pub fn for_stage(context: HygieneContext, stage: u32) -> Self {
        Self {
            config: CheckerConfig::default(),
            context,
            stage_context: StageContext::new(stage),
            violations: HygieneViolations::new(),
            stats: CheckStats::default(),
            scope_stack: List::new(),
            bindings: Map::new(),
        }
    }

    // ========================================================================
    // Main Check Entry Points
    // ========================================================================

    /// Check an expression for hygiene violations
    pub fn check_expr(&mut self, expr: &Expr) -> CheckResult {
        self.check_expr_internal(expr);
        self.build_result()
    }

    /// Check a list of statements for hygiene violations
    pub fn check_statements(&mut self, stmts: &[Stmt]) -> CheckResult {
        for stmt in stmts {
            self.check_stmt_internal(stmt);
        }
        self.build_result()
    }

    /// Check a block for hygiene violations
    pub fn check_block(&mut self, block: &Block) -> CheckResult {
        self.enter_scope(ScopeKind::Block);
        for stmt in &block.stmts {
            self.check_stmt_internal(stmt);
        }
        if let Some(expr) = &block.expr {
            self.check_expr_internal(expr);
        }
        self.exit_scope();
        self.build_result()
    }

    /// Build the final check result
    fn build_result(&self) -> CheckResult {
        CheckResult {
            violations: self.violations.clone(),
            stats: self.stats.clone(),
        }
    }

    // ========================================================================
    // Expression Checking
    // ========================================================================

    /// Check an expression
    fn check_expr_internal(&mut self, expr: &Expr) {
        self.stats.expressions_checked += 1;

        // Check for max violations
        if self.violations.len() >= self.config.max_violations {
            return;
        }

        match &expr.kind {
            ExprKind::Path(path) => {
                self.check_path_reference(path, expr.span);
            }

            ExprKind::Literal(_) => {
                // Literals don't need hygiene checking
            }

            ExprKind::Binary { left, right, .. } => {
                self.check_expr_internal(left);
                self.check_expr_internal(right);
            }

            ExprKind::Unary { expr: inner, .. } => {
                self.check_expr_internal(inner);
            }

            ExprKind::Call { func, args, .. } => {
                self.check_expr_internal(func);
                for arg in args {
                    self.check_expr_internal(arg);
                }
            }

            ExprKind::MethodCall {
                receiver, args, ..
            } => {
                self.check_expr_internal(receiver);
                for arg in args {
                    self.check_expr_internal(arg);
                }
            }

            ExprKind::Field { expr: inner, .. } => {
                self.check_expr_internal(inner);
            }

            ExprKind::Index { expr: inner, index } => {
                self.check_expr_internal(inner);
                self.check_expr_internal(index);
            }

            ExprKind::Block(block) => {
                self.enter_scope(ScopeKind::Block);
                for stmt in &block.stmts {
                    self.check_stmt_internal(stmt);
                }
                if let Some(expr) = &block.expr {
                    self.check_expr_internal(expr);
                }
                self.exit_scope();
            }

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.check_if_condition(condition);
                self.enter_scope(ScopeKind::Block);
                for stmt in &then_branch.stmts {
                    self.check_stmt_internal(stmt);
                }
                if let Some(expr) = &then_branch.expr {
                    self.check_expr_internal(expr);
                }
                self.exit_scope();
                if let verum_common::Maybe::Some(else_expr) = else_branch {
                    self.check_expr_internal(else_expr);
                }
            }

            ExprKind::Match { expr: scrutinee, arms } => {
                self.check_expr_internal(scrutinee);
                for arm in arms {
                    self.enter_scope(ScopeKind::MatchArm);
                    self.check_pattern_bindings(&arm.pattern);
                    if let Some(guard) = &arm.guard {
                        self.check_expr_internal(guard);
                    }
                    self.check_expr_internal(&arm.body);
                    self.exit_scope();
                }
            }

            ExprKind::Closure {
                params,
                body,
                ..
            } => {
                self.enter_scope(ScopeKind::Function);
                for param in params {
                    self.check_pattern_bindings(&param.pattern);
                }
                self.check_expr_internal(body);
                self.exit_scope();
            }

            ExprKind::Loop {
                body,
                ..
            } => {
                self.enter_scope(ScopeKind::Loop);
                for stmt in &body.stmts {
                    self.check_stmt_internal(stmt);
                }
                if let Some(expr) = &body.expr {
                    self.check_expr_internal(expr);
                }
                self.exit_scope();
            }

            ExprKind::For {
                pattern,
                iter,
                body,
                ..
            } => {
                self.check_expr_internal(iter);
                self.enter_scope(ScopeKind::ForLoop);
                self.check_pattern_bindings(pattern);
                for stmt in &body.stmts {
                    self.check_stmt_internal(stmt);
                }
                if let Some(expr) = &body.expr {
                    self.check_expr_internal(expr);
                }
                self.exit_scope();
            }

            ExprKind::While {
                condition, body, ..
            } => {
                self.check_expr_internal(condition);
                self.enter_scope(ScopeKind::Loop);
                for stmt in &body.stmts {
                    self.check_stmt_internal(stmt);
                }
                if let Some(expr) = &body.expr {
                    self.check_expr_internal(expr);
                }
                self.exit_scope();
            }

            ExprKind::Quote { target_stage, tokens } => {
                // For quote expressions, check hygiene of tokens
                self.enter_scope(ScopeKind::Quote);
                self.check_quote_tokens(tokens, *target_stage, expr.span);
                self.exit_scope();
            }

            ExprKind::StageEscape { stage, expr: inner } => {
                // Check stage escape validity
                if *stage > self.stage_context.current_stage() {
                    self.violations.push(HygieneViolation::StageMismatch {
                        expected_stage: self.stage_context.current_stage(),
                        actual_stage: *stage,
                        span: expr.span,
                    });
                }
                self.check_expr_internal(inner);
            }

            ExprKind::Lift { expr: inner } => {
                // Check lift expression
                if !self.in_quote() {
                    self.violations.push(HygieneViolation::UnquoteOutsideQuote {
                        span: expr.span,
                    });
                }
                self.check_expr_internal(inner);
            }

            ExprKind::Tuple(elements) => {
                for elem in elements {
                    self.check_expr_internal(elem);
                }
            }

            ExprKind::Array(array_expr) => {
                match array_expr {
                    verum_ast::expr::ArrayExpr::List(elements) => {
                        for elem in elements {
                            self.check_expr_internal(elem);
                        }
                    }
                    verum_ast::expr::ArrayExpr::Repeat { value, count } => {
                        self.check_expr_internal(value);
                        self.check_expr_internal(count);
                    }
                }
            }

            ExprKind::Record { fields, base, .. } => {
                for field in fields {
                    if let verum_common::Maybe::Some(val) = &field.value {
                        self.check_expr_internal(val);
                    }
                }
                if let verum_common::Maybe::Some(base_expr) = base {
                    self.check_expr_internal(base_expr);
                }
            }

            ExprKind::Return(value) => {
                if let verum_common::Maybe::Some(val) = value {
                    self.check_expr_internal(val);
                }
            }

            ExprKind::Break { value, .. } => {
                if let verum_common::Maybe::Some(val) = value {
                    self.check_expr_internal(val);
                }
            }

            ExprKind::Continue { .. } => {}

            ExprKind::Cast { expr: inner, .. } => {
                self.check_expr_internal(inner);
            }

            ExprKind::Try(inner) => {
                self.check_expr_internal(inner);
            }

            ExprKind::Await(inner) => {
                self.check_expr_internal(inner);
            }

            ExprKind::Async(block) => {
                self.enter_scope(ScopeKind::Block);
                for stmt in &block.stmts {
                    self.check_stmt_internal(stmt);
                }
                if let Some(expr) = &block.expr {
                    self.check_expr_internal(expr);
                }
                self.exit_scope();
            }

            ExprKind::Spawn { expr: inner, .. } => {
                self.check_expr_internal(inner);
            }

            ExprKind::Unsafe(block) | ExprKind::Meta(block) => {
                self.enter_scope(ScopeKind::Block);
                for stmt in &block.stmts {
                    self.check_stmt_internal(stmt);
                }
                if let Some(expr) = &block.expr {
                    self.check_expr_internal(expr);
                }
                self.exit_scope();
            }

            ExprKind::Throw(inner) => {
                self.check_expr_internal(inner);
            }

            ExprKind::Yield(inner) => {
                self.check_expr_internal(inner);
            }

            ExprKind::Pipeline { left, right } => {
                self.check_expr_internal(left);
                self.check_expr_internal(right);
            }

            ExprKind::NullCoalesce { left, right } => {
                self.check_expr_internal(left);
                self.check_expr_internal(right);
            }

            // Handle remaining expression kinds
            _ => {
                // Other expression kinds - traverse children as needed
            }
        }
    }

    /// Check if condition (may contain let bindings and expressions)
    fn check_if_condition(&mut self, condition: &verum_ast::expr::IfCondition) {
        for cond in condition.conditions.iter() {
            match cond {
                verum_ast::expr::ConditionKind::Expr(expr) => {
                    self.check_expr_internal(expr);
                }
                verum_ast::expr::ConditionKind::Let { pattern, value } => {
                    self.check_expr_internal(value);
                    self.check_pattern_bindings(pattern);
                }
            }
        }
    }

    /// Check a statement
    fn check_stmt_internal(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Let {
                pattern,
                value,
                ..
            } => {
                if let verum_common::Maybe::Some(val) = value {
                    self.check_expr_internal(val);
                }
                self.check_pattern_bindings(pattern);
            }

            StmtKind::LetElse {
                pattern,
                value,
                else_block,
                ..
            } => {
                self.check_expr_internal(value);
                self.check_pattern_bindings(pattern);
                self.enter_scope(ScopeKind::Block);
                for stmt in &else_block.stmts {
                    self.check_stmt_internal(stmt);
                }
                if let Some(expr) = &else_block.expr {
                    self.check_expr_internal(expr);
                }
                self.exit_scope();
            }

            StmtKind::Expr { expr, .. } => {
                self.check_expr_internal(expr);
            }

            StmtKind::Item(_item) => {
                // Items have their own scope and hygiene rules
            }

            StmtKind::Defer(expr) | StmtKind::Errdefer(expr) => {
                self.check_expr_internal(expr);
            }

            StmtKind::Provide { value, .. } => {
                self.check_expr_internal(value);
            }

            StmtKind::ProvideScope { value, block, .. } => {
                self.check_expr_internal(value);
                self.check_expr_internal(block);
            }

            StmtKind::Empty => {}
        }
    }

    // ========================================================================
    // Reference Checking
    // ========================================================================

    /// Check a path reference for hygiene
    fn check_path_reference(&mut self, path: &Path, span: Span) {
        // For single-segment paths (simple identifiers)
        if path.segments.len() == 1 {
            if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                self.check_identifier_reference(&ident.name, span);
            }
        }
    }

    /// Check an identifier reference
    fn check_identifier_reference(&mut self, name: &Text, span: Span) {
        self.stats.references_resolved += 1;

        // Create a hygienic identifier for resolution
        let ident = HygienicIdent::new(name.clone(), self.current_scopes(), span);

        // Try to resolve the binding
        if let Some(entries) = self.bindings.get(name) {
            // Find compatible bindings
            let compatible: List<_> = entries
                .iter()
                .filter(|entry| self.is_mark_compatible(&ident.scopes, &entry.scopes))
                .collect();

            if compatible.is_empty() {
                // No compatible binding found - potential accidental capture
                if let Some(_entry) = entries.first() {
                    self.violations.push(HygieneViolation::AccidentalCapture {
                        captured: ident.clone(),
                        intended_binding: Span::default(),
                        actual_binding: span,
                    });
                }
            }
        } else {
            // No binding found - might be a free variable
            // This could be an error in strict mode
            if self.config.strict_mode && self.in_quote() {
                self.violations.push(HygieneViolation::CaptureNotDeclared {
                    ident: name.clone(),
                    span,
                });
            }
        }
    }

    /// Check mark compatibility between reference and binding
    fn is_mark_compatible(&self, ref_scopes: &ScopeSet, binding_scopes: &ScopeSet) -> bool {
        // Compatible if binding scopes are a subset of reference scopes
        binding_scopes.is_subset_of(ref_scopes)
    }

    // ========================================================================
    // Pattern Checking
    // ========================================================================

    /// Check pattern bindings
    fn check_pattern_bindings(&mut self, pattern: &Pattern) {
        match &pattern.kind {
            PatternKind::Ident { name, mutable, .. } => {
                self.add_binding(name.name.clone(), *mutable, pattern.span);
            }

            PatternKind::Tuple(elements) => {
                for elem in elements {
                    self.check_pattern_bindings(elem);
                }
            }

            PatternKind::Array(elements) => {
                for elem in elements {
                    self.check_pattern_bindings(elem);
                }
            }

            PatternKind::Record { fields, .. } => {
                for field in fields {
                    // Field pattern may have an optional subpattern
                    if let verum_common::Maybe::Some(pat) = &field.pattern {
                        self.check_pattern_bindings(pat);
                    } else {
                        // Shorthand: { x } means bind x
                        self.add_binding(field.name.name.clone(), false, field.span);
                    }
                }
            }

            PatternKind::Variant { data, .. } => {
                if let verum_common::Maybe::Some(variant_data) = data {
                    match variant_data {
                        verum_ast::pattern::VariantPatternData::Tuple(patterns) => {
                            for pat in patterns {
                                self.check_pattern_bindings(pat);
                            }
                        }
                        verum_ast::pattern::VariantPatternData::Record { fields, .. } => {
                            for field in fields {
                                if let verum_common::Maybe::Some(pat) = &field.pattern {
                                    self.check_pattern_bindings(pat);
                                } else {
                                    self.add_binding(field.name.name.clone(), false, field.span);
                                }
                            }
                        }
                    }
                }
            }

            PatternKind::Or(patterns) => {
                for pat in patterns {
                    self.check_pattern_bindings(pat);
                }
            }

            PatternKind::Slice { before, rest, after } => {
                for pat in before {
                    self.check_pattern_bindings(pat);
                }
                if let verum_common::Maybe::Some(rest_pat) = rest {
                    self.check_pattern_bindings(rest_pat);
                }
                for pat in after {
                    self.check_pattern_bindings(pat);
                }
            }

            PatternKind::Reference { inner, .. } => {
                self.check_pattern_bindings(inner);
            }

            PatternKind::Paren(inner) => {
                self.check_pattern_bindings(inner);
            }

            // Literals, wildcards, and rest don't introduce bindings
            PatternKind::Literal(_) | PatternKind::Wildcard | PatternKind::Rest => {}

            // Range patterns don't introduce bindings
            PatternKind::Range { .. } => {}

            // View and Active patterns - check subpatterns
            #[allow(deprecated)]
            PatternKind::View { pattern: inner, .. } => {
                self.check_pattern_bindings(inner);
            }

            PatternKind::Active { .. } => {
                // Active patterns don't introduce bindings themselves
            }

            PatternKind::And(patterns) => {
                for pat in patterns {
                    self.check_pattern_bindings(pat);
                }
            }

            PatternKind::TypeTest { binding, .. } => {
                self.add_binding(binding.name.clone(), false, pattern.span);
            }

            PatternKind::Stream { rest, .. } => {
                // Rest binding if present
                if let verum_common::Maybe::Some(rest_ident) = rest {
                    self.add_binding(rest_ident.name.clone(), false, pattern.span);
                }
            }

            PatternKind::Guard { pattern: inner, .. } => {
                // Guard pattern: (pattern if guard_expr)
                // Spec: Rust RFC 3637 - Guard Patterns
                // Check bindings from inner pattern
                self.check_pattern_bindings(inner);
            }
            PatternKind::Cons { head, tail } => {
                self.check_pattern_bindings(head);
                self.check_pattern_bindings(tail);
            }
        }
    }

    /// Add a binding and check for shadow conflicts
    fn add_binding(&mut self, name: Text, is_mutable: bool, span: Span) {
        self.stats.bindings_verified += 1;

        let scope_id = self.current_scope_id();
        let scopes = self.current_scopes();

        // Check for shadow conflicts
        if let Some(existing) = self.bindings.get(&name) {
            for entry in existing.iter() {
                // If the existing binding is from an outer scope with compatible marks,
                // this is a shadow conflict
                if self.is_mark_compatible(&scopes, &entry.scopes) {
                    let violation = HygieneViolation::ShadowConflict {
                        shadowed: HygienicIdent::new(name.clone(), scopes.clone(), span),
                        introduced_at: span,
                    };

                    if self.config.allow_shadow_recovery {
                        // Just record it, but allow the binding
                        self.stats.warnings += 1;
                    } else {
                        self.violations.push(violation);
                    }
                }
            }
        }

        // Create the binding
        let info = BindingInfo {
            original_name: name.clone(),
            hygienic_name: self.context.gensym(name.as_str()),
            scope_id,
            is_mutable,
            kind: BindingKind::Variable,
        };

        let entry = BindingEntry {
            info: info.clone(),
            scope_id,
            scopes: scopes.clone(),
        };

        // Add to binding table
        self.bindings
            .entry(name.clone())
            .or_insert_with(List::new)
            .push(entry);

        // Add to current scope
        if let Some(scope) = self.scope_stack.last_mut() {
            scope.bindings.insert(name, info);
        }
    }

    // ========================================================================
    // Quote-Specific Checking
    // ========================================================================

    /// Check the tokens of a quote expression
    fn check_quote_tokens(
        &mut self,
        _tokens: &List<verum_ast::expr::TokenTree>,
        target_stage: Option<u32>,
        span: Span,
    ) {
        let target = target_stage.unwrap_or(0);

        // Verify stage consistency
        if target > self.stage_context.current_stage() + 1 {
            self.violations.push(HygieneViolation::StageMismatch {
                expected_stage: self.stage_context.current_stage() + 1,
                actual_stage: target,
                span,
            });
        }

        // In a full implementation, we would walk the token tree
        // and check hygiene for each interpolation
    }

    /// Check if we're currently inside a quote
    fn in_quote(&self) -> bool {
        self.scope_stack.iter().any(|s| s.kind == ScopeKind::Quote)
    }

    // ========================================================================
    // Scope Management
    // ========================================================================

    /// Enter a new scope
    fn enter_scope(&mut self, kind: ScopeKind) {
        let id = self.context.enter_scope(kind);
        self.scope_stack.push(CheckerScope {
            id,
            kind,
            bindings: Map::new(),
        });
    }

    /// Exit the current scope
    fn exit_scope(&mut self) {
        if let Some(scope) = self.scope_stack.pop() {
            // Remove bindings from the binding table
            for (name, _) in scope.bindings.iter() {
                if let Some(entries) = self.bindings.get_mut(name) {
                    entries.retain(|e| e.scope_id != scope.id);
                }
            }
            self.context.exit_scope();
        }
    }

    /// Get the current scope ID
    fn current_scope_id(&self) -> ScopeId {
        self.scope_stack
            .last()
            .map(|s| s.id)
            .unwrap_or(ScopeId::new(0))
    }

    /// Get the current scopes
    fn current_scopes(&self) -> ScopeSet {
        self.context.current_scopes()
    }

    // ========================================================================
    // Utility Methods
    // ========================================================================

    /// Get the accumulated violations
    pub fn violations(&self) -> &HygieneViolations {
        &self.violations
    }

    /// Take the violations out
    pub fn take_violations(&mut self) -> HygieneViolations {
        std::mem::take(&mut self.violations)
    }

    /// Get the statistics
    pub fn stats(&self) -> &CheckStats {
        &self.stats
    }
}

/// Visitor trait for hygiene checking
pub trait HygieneVisitor {
    /// Visit an expression
    fn visit_expr(&mut self, expr: &Expr) -> bool;

    /// Visit a statement
    fn visit_stmt(&mut self, stmt: &Stmt) -> bool;

    /// Visit a pattern
    fn visit_pattern(&mut self, pattern: &Pattern) -> bool;
}

/// Default implementation that always continues
impl<T> HygieneVisitor for T
where
    T: FnMut(&Expr) -> bool,
{
    fn visit_expr(&mut self, expr: &Expr) -> bool {
        (self)(expr)
    }

    fn visit_stmt(&mut self, _stmt: &Stmt) -> bool {
        true
    }

    fn visit_pattern(&mut self, _pattern: &Pattern) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checker_creation() {
        let context = HygieneContext::new();
        let checker = HygieneChecker::with_default_config(context);

        assert!(checker.violations.is_empty());
        assert_eq!(checker.stats.expressions_checked, 0);
    }

    #[test]
    fn test_scope_management() {
        let context = HygieneContext::new();
        let mut checker = HygieneChecker::with_default_config(context);

        checker.enter_scope(ScopeKind::Function);
        assert_eq!(checker.scope_stack.len(), 1);

        checker.enter_scope(ScopeKind::Block);
        assert_eq!(checker.scope_stack.len(), 2);

        checker.exit_scope();
        assert_eq!(checker.scope_stack.len(), 1);

        checker.exit_scope();
        assert_eq!(checker.scope_stack.len(), 0);
    }

    #[test]
    fn test_binding_tracking() {
        let context = HygieneContext::new();
        let mut checker = HygieneChecker::with_default_config(context);

        checker.enter_scope(ScopeKind::Function);
        checker.add_binding(Text::from("x"), false, Span::default());

        assert!(checker.bindings.contains_key(&Text::from("x")));
        assert_eq!(checker.stats.bindings_verified, 1);
    }

    #[test]
    fn test_in_quote_detection() {
        let context = HygieneContext::new();
        let mut checker = HygieneChecker::with_default_config(context);

        assert!(!checker.in_quote());

        checker.enter_scope(ScopeKind::Quote);
        assert!(checker.in_quote());

        checker.exit_scope();
        assert!(!checker.in_quote());
    }

    #[test]
    fn test_check_stats() {
        let context = HygieneContext::new();
        let checker = HygieneChecker::with_default_config(context);

        let stats = checker.stats();
        assert_eq!(stats.expressions_checked, 0);
        assert_eq!(stats.bindings_verified, 0);
        assert_eq!(stats.references_resolved, 0);
    }

    #[test]
    fn test_check_result_success() {
        let result = CheckResult::success();
        assert!(result.is_success());
        assert!(result.violations.is_empty());
    }

    #[test]
    fn test_checker_config() {
        let config = CheckerConfig {
            strict_mode: false,
            allow_shadow_recovery: true,
            max_violations: 50,
            include_warnings: false,
        };

        let context = HygieneContext::new();
        let checker = HygieneChecker::new(context, config.clone());

        assert!(!checker.config.strict_mode);
        assert!(checker.config.allow_shadow_recovery);
    }
}
