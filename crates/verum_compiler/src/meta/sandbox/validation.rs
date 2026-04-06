//! Expression Validation for Meta Sandbox
//!
//! Validates expressions and function declarations for sandbox compliance.
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

use verum_ast::decl::{FunctionBody, FunctionDecl};
use verum_ast::expr::{ArrayExpr, ConditionKind, Expr, ExprKind};
use verum_ast::stmt::StmtKind;
use verum_ast::PathSegment;
use verum_common::{List, Text};

use super::allowlist::AllowlistRegistry;
use super::errors::SandboxError;

/// Expression validator for sandbox compliance
#[derive(Debug, Clone)]
pub struct ExpressionValidator {
    /// Allowlist registry for function checks
    allowlist: AllowlistRegistry,
}

impl ExpressionValidator {
    /// Create a new expression validator
    pub fn new() -> Self {
        Self {
            allowlist: AllowlistRegistry::new(),
        }
    }

    /// Create a validator with a custom allowlist
    pub fn with_allowlist(allowlist: AllowlistRegistry) -> Self {
        Self { allowlist }
    }

    /// Get the underlying allowlist registry
    pub fn allowlist(&self) -> &AllowlistRegistry {
        &self.allowlist
    }

    /// Validate a function call for sandbox compliance
    pub fn validate_function_call(&self, call: &Expr) -> Result<(), SandboxError> {
        if let ExprKind::Call { func, .. } = &call.kind {
            if let ExprKind::Path(path) = &func.kind {
                let path_str = format!("{:?}", path);
                let func_name = Text::from(path_str.as_str());

                // Check filesystem operations
                if self.allowlist.is_filesystem_function(&func_name) {
                    return Err(SandboxError::FileSystemViolation {
                        operation: func_name.clone(),
                        function: func_name.clone(),
                        context: Text::from("meta"),
                    });
                }

                // Check network operations
                if self.allowlist.is_network_function(&func_name) {
                    return Err(SandboxError::NetworkViolation {
                        operation: func_name.clone(),
                        function: func_name.clone(),
                        context: Text::from("meta"),
                    });
                }

                // Check process operations
                if self.allowlist.is_process_function(&func_name) {
                    return Err(SandboxError::ProcessViolation {
                        operation: func_name.clone(),
                        function: func_name,
                    });
                }

                // Check time operations
                if self.allowlist.is_time_function(&func_name) {
                    return Err(SandboxError::TimeViolation {
                        operation: func_name.clone(),
                        function: func_name,
                    });
                }

                // Check environment operations
                if self.allowlist.is_env_function(&func_name) {
                    return Err(SandboxError::EnvViolation {
                        operation: func_name.clone(),
                        function: func_name,
                    });
                }

                // Check random operations
                if self.allowlist.is_random_function(&func_name) {
                    return Err(SandboxError::RandomViolation {
                        operation: func_name.clone(),
                        function: func_name,
                    });
                }

                // Check unsafe operations
                if self.allowlist.is_unsafe_function(&func_name) {
                    return Err(SandboxError::UnsafeViolation {
                        operation: func_name.clone(),
                        function: func_name,
                    });
                }

                // Check FFI operations
                if self.allowlist.is_ffi_function(&func_name) {
                    return Err(SandboxError::FFIViolation {
                        operation: func_name.clone(),
                        function: func_name,
                    });
                }
            }
        }

        Ok(())
    }

    /// Validate an expression for sandbox compliance (recursive)
    pub fn validate_expr(&self, expr: &Expr) -> Result<(), SandboxError> {
        match &expr.kind {
            ExprKind::Call { func, args, .. } => {
                // Validate the call itself
                self.validate_function_call(expr)?;

                // Recursively validate function expression and arguments
                self.validate_expr(func)?;
                for arg in args.iter() {
                    self.validate_expr(arg)?;
                }
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                // Validate receiver and arguments
                self.validate_expr(receiver)?;
                for arg in args.iter() {
                    self.validate_expr(arg)?;
                }
            }
            ExprKind::Binary { left, right, .. } => {
                self.validate_expr(left)?;
                self.validate_expr(right)?;
            }
            ExprKind::Unary { expr: operand, .. } => {
                self.validate_expr(operand)?;
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Validate condition expressions
                for cond in &condition.conditions {
                    match cond {
                        ConditionKind::Expr(e) => self.validate_expr(e)?,
                        ConditionKind::Let { value, .. } => {
                            self.validate_expr(value)?
                        }
                    }
                }

                // Validate branches
                if let Some(final_expr) = &then_branch.expr {
                    self.validate_expr(final_expr)?;
                }
                if let Some(else_expr) = else_branch {
                    self.validate_expr(else_expr)?;
                }
            }
            ExprKind::Match { expr, arms } => {
                self.validate_expr(expr)?;
                for arm in arms.iter() {
                    if let Some(guard) = &arm.guard {
                        self.validate_expr(guard)?;
                    }
                    self.validate_expr(&arm.body)?;
                }
            }
            ExprKind::Block(block) => {
                for stmt in block.stmts.iter() {
                    if let StmtKind::Expr { expr: e, .. } = &stmt.kind {
                        self.validate_expr(e)?;
                    }
                }
                if let Some(final_expr) = &block.expr {
                    self.validate_expr(final_expr)?;
                }
            }
            ExprKind::Array(array_expr) => {
                match array_expr {
                    ArrayExpr::List(elements) => {
                        for elem in elements.iter() {
                            self.validate_expr(elem)?;
                        }
                    }
                    ArrayExpr::Repeat { value, count } => {
                        self.validate_expr(value)?;
                        self.validate_expr(count)?;
                    }
                }
            }
            ExprKind::Tuple(elements) => {
                for elem in elements.iter() {
                    self.validate_expr(elem)?;
                }
            }
            ExprKind::Index { expr, index } => {
                self.validate_expr(expr)?;
                self.validate_expr(index)?;
            }
            ExprKind::Field { expr, .. } => {
                self.validate_expr(expr)?;
            }
            ExprKind::Closure { body, .. } => {
                self.validate_expr(body)?;
            }
            // Literals, paths, and other leaf nodes are always safe
            _ => {}
        }

        Ok(())
    }

    /// Validate an async meta function for I/O operations
    ///
    /// CRITICAL: meta async fn is allowed for parallelism but FORBIDS all I/O
    pub fn validate_async_meta_fn(&self, func: &FunctionDecl) -> Result<(), SandboxError> {
        if !func.is_async || !func.is_meta {
            return Ok(());
        }

        // Collect all I/O operations in the function body
        let mut io_operations = List::new();

        if let Some(body) = &func.body {
            match body {
                FunctionBody::Block(block) => {
                    if let Some(final_expr) = &block.expr {
                        if let Err(e) = self.validate_expr(final_expr) {
                            match e {
                                SandboxError::FileSystemViolation { function, .. }
                                | SandboxError::NetworkViolation { function, .. }
                                | SandboxError::ProcessViolation { function, .. }
                                | SandboxError::TimeViolation { function, .. } => {
                                    io_operations.push(function);
                                }
                                _ => return Err(e),
                            }
                        }
                    }
                }
                FunctionBody::Expr(expr) => {
                    if let Err(e) = self.validate_expr(expr) {
                        match e {
                            SandboxError::FileSystemViolation { function, .. }
                            | SandboxError::NetworkViolation { function, .. }
                            | SandboxError::ProcessViolation { function, .. }
                            | SandboxError::TimeViolation { function, .. } => {
                                io_operations.push(function);
                            }
                            _ => return Err(e),
                        }
                    }
                }
            }
        }

        if !io_operations.is_empty() {
            return Err(SandboxError::IoInAsyncMeta {
                function: func.name.name.clone(),
                io_operations,
                message: Text::from(
                    "meta async fn cannot perform I/O operations. \
                     Async is allowed for parallelism only, not for I/O.",
                ),
            });
        }

        Ok(())
    }

    /// Check if a function declaration uses the BuildAssets context
    pub fn has_build_assets_context(func: &FunctionDecl) -> bool {
        for ctx in func.contexts.iter() {
            // Check the path for "BuildAssets"
            if let Some(last_segment) = ctx.path.segments.last() {
                if let PathSegment::Name(ident) = last_segment {
                    if ident.as_str() == "BuildAssets" {
                        return true;
                    }
                }
            }
        }
        false
    }
}

impl Default for ExpressionValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validator_creation() {
        let validator = ExpressionValidator::new();
        assert!(validator.allowlist().is_operation_allowed(super::super::errors::Operation::Arithmetic));
    }
}
