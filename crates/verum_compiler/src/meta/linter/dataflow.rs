//! Dataflow Analysis for Meta Linter
//!
//! Tracks external input propagation and function call graphs for
//! recursion detection.
//!
//! Meta linter: static analysis of meta code for unsafe patterns (unbounded
//! recursion, infinite loops, unsafe interpolation without @safe attribute).

use std::collections::{HashMap, HashSet};

use verum_ast::expr::{Expr, ExprKind};
use verum_ast::pattern::{Pattern, PatternKind};

/// Analysis context for tracking state during linting
pub struct AnalysisContext {
    /// Variables that come from external input (parameters, fields, etc.)
    external_vars: HashSet<String>,
    /// Function call graph for recursion detection
    call_graph: HashMap<String, HashSet<String>>,
    /// Current function being analyzed
    pub current_function: Option<String>,
    /// Track if we've seen a break/return in current loop
    pub has_break_in_loop: bool,
}

impl AnalysisContext {
    /// Create a new analysis context
    pub fn new() -> Self {
        Self {
            external_vars: HashSet::new(),
            call_graph: HashMap::new(),
            current_function: None,
            has_break_in_loop: false,
        }
    }

    /// Mark a variable name as coming from external input
    pub fn mark_as_external(&mut self, name: String) {
        self.external_vars.insert(name);
    }

    /// Check if a variable is marked as external
    pub fn is_external(&self, name: &str) -> bool {
        self.external_vars.contains(name)
    }

    /// Record a function call from one function to another
    pub fn record_call(&mut self, from: String, to: String) {
        self.call_graph
            .entry(from)
            .or_insert_with(HashSet::new)
            .insert(to);
    }

    /// Check if a function has potential recursive cycles
    pub fn has_recursion(&self, func_name: &str) -> bool {
        let mut visited = HashSet::new();
        let mut stack = HashSet::new();
        self.has_cycle_dfs(func_name, &mut visited, &mut stack)
    }

    /// DFS-based cycle detection in the call graph
    fn has_cycle_dfs(
        &self,
        node: &str,
        visited: &mut HashSet<String>,
        stack: &mut HashSet<String>,
    ) -> bool {
        if stack.contains(node) {
            return true; // Found cycle
        }
        if visited.contains(node) {
            return false; // Already processed
        }

        visited.insert(node.to_string());
        stack.insert(node.to_string());

        if let Some(callees) = self.call_graph.get(node) {
            for callee in callees {
                if self.has_cycle_dfs(callee, visited, stack) {
                    return true;
                }
            }
        }

        stack.remove(node);
        false
    }
}

impl Default for AnalysisContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper functions for checking if expressions use external input
pub struct ExternalInputChecker;

impl ExternalInputChecker {
    /// Check if an expression uses external (untrusted) input
    pub fn expr_uses_external(expr: &Expr, ctx: &AnalysisContext) -> bool {
        match &expr.kind {
            ExprKind::Path(path) => {
                // Check if this path references an external variable
                if let Some(ident) = path.segments.first() {
                    if let verum_ast::ty::PathSegment::Name(name) = ident {
                        return ctx.is_external(name.as_str());
                    }
                }
                false
            }
            ExprKind::Field { expr: inner, .. } | ExprKind::TupleIndex { expr: inner, .. } => {
                Self::expr_uses_external(inner, ctx)
            }
            ExprKind::Index { expr: base, .. } => Self::expr_uses_external(base, ctx),
            ExprKind::Binary { left, right, .. } => {
                Self::expr_uses_external(left, ctx) || Self::expr_uses_external(right, ctx)
            }
            ExprKind::Call { args, .. } | ExprKind::MethodCall { args, .. } => {
                // If any argument uses external input, the result might too
                args.iter().any(|arg| Self::expr_uses_external(arg, ctx))
            }
            _ => false,
        }
    }

    /// Mark all variables bound in a pattern as external
    pub fn mark_pattern_vars_external(pattern: &Pattern, ctx: &mut AnalysisContext) {
        match &pattern.kind {
            PatternKind::Ident {
                name, subpattern, ..
            } => {
                ctx.mark_as_external(name.as_str().to_string());
                if let Some(sub) = subpattern {
                    Self::mark_pattern_vars_external(sub, ctx);
                }
            }
            PatternKind::Tuple(patterns) => {
                for pat in patterns {
                    Self::mark_pattern_vars_external(pat, ctx);
                }
            }
            PatternKind::Record { fields, .. } => {
                for field in fields {
                    if let Some(pat) = &field.pattern {
                        Self::mark_pattern_vars_external(pat, ctx);
                    }
                }
            }
            PatternKind::Or(patterns) => {
                for pat in patterns {
                    Self::mark_pattern_vars_external(pat, ctx);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_external_tracking() {
        let mut ctx = AnalysisContext::new();
        assert!(!ctx.is_external("user_input"));

        ctx.mark_as_external("user_input".to_string());
        assert!(ctx.is_external("user_input"));
    }

    #[test]
    fn test_recursion_detection() {
        let mut ctx = AnalysisContext::new();
        ctx.record_call("foo".to_string(), "bar".to_string());
        ctx.record_call("bar".to_string(), "baz".to_string());

        // No cycle yet
        assert!(!ctx.has_recursion("foo"));

        // Add cycle
        ctx.record_call("baz".to_string(), "foo".to_string());
        assert!(ctx.has_recursion("foo"));
    }

    #[test]
    fn test_direct_recursion() {
        let mut ctx = AnalysisContext::new();
        ctx.record_call("factorial".to_string(), "factorial".to_string());
        assert!(ctx.has_recursion("factorial"));
    }
}
