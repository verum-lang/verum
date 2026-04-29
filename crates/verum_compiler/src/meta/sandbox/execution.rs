//! Sandboxed Execution for Meta Functions
//!
//! Executes meta expressions within the sandbox constraints.
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

use std::time::Instant;

use verum_ast::expr::{ConditionKind, Expr, ExprKind};
use verum_ast::{BinOp, UnOp};
use verum_common::{List, Text};

use crate::meta::context::{ConstValue, MetaContext};
use crate::meta::value_ops::MetaValueOps;

use super::allowlist::AllowlistRegistry;
use super::errors::SandboxError;
use super::resource_limits::{RecursionGuard, ResourceLimiter};
use super::validation::ExpressionValidator;

/// Sandboxed executor for meta expressions
#[derive(Debug)]
pub struct SandboxedExecutor {
    /// Resource limiter for execution bounds
    limiter: ResourceLimiter,
    /// Expression validator
    validator: ExpressionValidator,
    /// Allowlist registry
    allowlist: AllowlistRegistry,
}

impl SandboxedExecutor {
    /// Create a new sandboxed executor
    pub fn new() -> Self {
        Self {
            limiter: ResourceLimiter::new(),
            validator: ExpressionValidator::new(),
            allowlist: AllowlistRegistry::new(),
        }
    }

    /// Create an executor with custom components
    pub fn with_components(
        limiter: ResourceLimiter,
        validator: ExpressionValidator,
        allowlist: AllowlistRegistry,
    ) -> Self {
        Self {
            limiter,
            validator,
            allowlist,
        }
    }

    /// Get the resource limiter
    pub fn limiter(&self) -> &ResourceLimiter {
        &self.limiter
    }

    /// Get the validator
    pub fn validator(&self) -> &ExpressionValidator {
        &self.validator
    }

    /// Get the allowlist
    pub fn allowlist(&self) -> &AllowlistRegistry {
        &self.allowlist
    }

    /// Estimate the memory cost of a ConstValue
    ///
    /// This is an approximation for memory limit enforcement.
    /// Returns size in bytes.
    fn estimate_value_memory(&self, value: &ConstValue) -> usize {
        match value {
            ConstValue::Unit => 0,
            ConstValue::Bool(_) => 1,
            ConstValue::Char(_) => 4,
            ConstValue::Int(_) => 16, // i128
            ConstValue::UInt(_) => 16, // u128
            ConstValue::Float(_) => 8, // f64
            ConstValue::Text(s) => s.len() + 24, // String overhead + content
            ConstValue::Bytes(b) => b.len() + 24, // Vec overhead + content
            ConstValue::Array(arr) => {
                arr.iter().map(|v| self.estimate_value_memory(v)).sum::<usize>() + 24
            }
            ConstValue::Tuple(items) => {
                items.iter().map(|v| self.estimate_value_memory(v)).sum::<usize>() + 24
            }
            ConstValue::Maybe(maybe) => {
                match maybe.as_ref() {
                    verum_common::Maybe::Some(v) => self.estimate_value_memory(v) + 8,
                    verum_common::Maybe::None => 8,
                }
            }
            ConstValue::Map(map) => {
                map.iter()
                    .map(|(k, v)| k.len() + 24 + self.estimate_value_memory(v))
                    .sum::<usize>()
                    + 24
            }
            ConstValue::Set(set) => {
                set.iter().map(|s| s.len() + 24).sum::<usize>() + 24
            }
            ConstValue::Expr(_) => 64, // AST node approximation
            ConstValue::Type(_) => 32, // Type approximation
            ConstValue::Pattern(_) => 64, // Pattern approximation
            ConstValue::Item(_) => 128, // Item approximation
            ConstValue::Items(items) => items.len() * 128 + 24, // Items approximation
        }
    }

    /// Track memory allocation for a value and check limits
    fn track_value_memory(&self, value: &ConstValue) -> Result<(), SandboxError> {
        let bytes = self.estimate_value_memory(value);
        self.limiter.check_memory_limit(bytes)
    }

    /// Execute an expression in the sandbox
    ///
    /// This is the main entry point for executing meta functions at compile-time.
    /// It recursively evaluates the expression AST while enforcing I/O restrictions.
    pub fn execute(
        &self,
        ctx: &MetaContext,
        expr: &Expr,
    ) -> Result<ConstValue, SandboxError> {
        // Reset state before execution
        self.limiter.reset_execution_state();

        // Validate expression before execution
        self.validator.validate_expr(expr)?;

        // Execute with timeout check
        let start = Instant::now();
        let result = self.execute_expr(ctx, expr, start)?;

        Ok(result)
    }

    /// Execute an expression node with timeout tracking
    fn execute_expr(
        &self,
        ctx: &MetaContext,
        expr: &Expr,
        start: Instant,
    ) -> Result<ConstValue, SandboxError> {
        // Check timeout
        self.limiter.check_timeout(start)?;

        // Check iteration limit
        self.limiter.check_iteration_limit()?;

        let result = match &expr.kind {
            ExprKind::Literal(lit) => {
                // Literals are always safe
                ConstValue::from_literal(lit)
            }

            ExprKind::Binary { op, left, right } => {
                // Check if binary operation is allowed
                if !self.is_allowed_binary_op(op) {
                    return Err(SandboxError::IoViolation {
                        operation: Text::from(format!("{:?}", op)),
                        reason: Text::from("Binary operation not allowed in meta context"),
                    });
                }

                let left_val = self.execute_expr(ctx, left, start)?;
                let right_val = self.execute_expr(ctx, right, start)?;
                self.execute_binary_op(op, left_val, right_val)?
            }

            ExprKind::Unary { op, expr: operand } => {
                let operand_val = self.execute_expr(ctx, operand, start)?;
                self.execute_unary_op(op, operand_val)?
            }

            ExprKind::Call { func, args, .. } => {
                // Validation already done in validate_expr
                // Execute allowed function calls
                self.execute_call(ctx, func, args, start)?
            }

            ExprKind::Path(path) => {
                // Look up variable in context (for single-segment paths)
                if let Some(name) = path.as_ident() {
                    ctx.get(&Text::from(name.as_str())).ok_or_else(|| {
                        SandboxError::UnsafeOperation {
                            operation: Text::from(name.as_str()),
                            reason: Text::from("Variable not found in meta context"),
                        }
                    })?
                } else {
                    return Err(SandboxError::UnsafeOperation {
                        operation: Text::from(format!("{:?}", path)),
                        reason: Text::from("Multi-segment paths not supported in meta context"),
                    });
                }
            }

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Execute the first condition in the IfCondition
                let cond_val = if let Some(cond) = condition.conditions.first() {
                    match cond {
                        ConditionKind::Expr(expr) => self.execute_expr(ctx, expr, start)?,
                        ConditionKind::Let { .. } => {
                            return Err(SandboxError::UnsafeOperation {
                                operation: Text::from("let condition"),
                                reason: Text::from("Let conditions not supported in meta context"),
                            });
                        }
                    }
                } else {
                    ConstValue::Bool(true)
                };

                if cond_val.as_bool() {
                    if let Some(expr) = &then_branch.expr {
                        self.execute_expr(ctx, expr, start)?
                    } else {
                        ConstValue::Unit
                    }
                } else if let Some(else_expr) = else_branch.as_ref() {
                    self.execute_expr(ctx, else_expr, start)?
                } else {
                    ConstValue::Unit
                }
            }

            ExprKind::Block(block) => {
                // Execute the block's final expression
                if let Some(final_expr) = &block.expr {
                    self.execute_expr(ctx, final_expr, start)?
                } else {
                    ConstValue::Unit
                }
            }

            // Other expression types that are not yet supported in meta context
            _ => {
                return Err(SandboxError::UnsafeOperation {
                    operation: Text::from(format!("{:?}", expr.kind)),
                    reason: Text::from("Expression type not supported in meta context"),
                });
            }
        };

        // Track memory usage for the result value
        self.track_value_memory(&result)?;

        Ok(result)
    }

    /// Check if a binary operator is allowed
    fn is_allowed_binary_op(&self, op: &BinOp) -> bool {
        matches!(
            op,
            BinOp::Add
                | BinOp::Sub
                | BinOp::Mul
                | BinOp::Div
                | BinOp::Rem
                | BinOp::Eq
                | BinOp::Ne
                | BinOp::Lt
                | BinOp::Le
                | BinOp::Gt
                | BinOp::Ge
                | BinOp::And
                | BinOp::Or
        )
    }

    /// Execute a binary operation
    fn execute_binary_op(
        &self,
        op: &BinOp,
        left: ConstValue,
        right: ConstValue,
    ) -> Result<ConstValue, SandboxError> {
        match op {
            BinOp::Add => left.add(right),
            BinOp::Sub => left.sub(right),
            BinOp::Mul => left.mul(right),
            BinOp::Div => left.div(right),
            BinOp::Rem => left.modulo(right),
            BinOp::Eq => Ok(ConstValue::Bool(left.eq(&right))),
            BinOp::Ne => Ok(ConstValue::Bool(!left.eq(&right))),
            BinOp::Lt => left.lt(right),
            BinOp::Le => left.le(right),
            BinOp::Gt => left.gt(right),
            BinOp::Ge => left.ge(right),
            BinOp::And => left.and(right),
            BinOp::Or => left.or(right),
            _ => Err(SandboxError::UnsafeOperation {
                operation: Text::from(format!("{:?}", op)),
                reason: Text::from("Binary operation not implemented"),
            }),
        }
    }

    /// Execute a unary operation
    fn execute_unary_op(
        &self,
        op: &UnOp,
        operand: ConstValue,
    ) -> Result<ConstValue, SandboxError> {
        match op {
            UnOp::Neg => operand.neg(),
            UnOp::Not => operand.not(),
            _ => Err(SandboxError::UnsafeOperation {
                operation: Text::from(format!("{:?}", op)),
                reason: Text::from("Unary operation not implemented"),
            }),
        }
    }

    /// Execute a function call in the sandbox
    fn execute_call(
        &self,
        ctx: &MetaContext,
        func: &Expr,
        args: &[Expr],
        start: Instant,
    ) -> Result<ConstValue, SandboxError> {
        // Check recursion limit using RAII guard
        let _guard = RecursionGuard::new(&self.limiter)?;

        // Extract function name from path
        let func_name = if let ExprKind::Path(path) = &func.kind {
            path.segments
                .iter()
                .map(|seg| match seg {
                    verum_ast::PathSegment::Name(ident) => ident.name.as_str(),
                    verum_ast::PathSegment::SelfValue => "self",
                    verum_ast::PathSegment::Super => "super",
                    verum_ast::PathSegment::Cog => "cog",
                    verum_ast::PathSegment::Relative => ".",
                })
                .collect::<Vec<_>>()
                .join(".")
        } else {
            return Err(SandboxError::UnsafeOperation {
                operation: Text::from("complex_call"),
                reason: Text::from("Only path-based function calls are supported in meta context"),
            });
        };

        // Evaluate arguments
        let mut arg_values = Vec::new();
        for arg in args {
            arg_values.push(self.execute_expr(ctx, arg, start)?);
        }

        // Delegate to builtin function executor
        self.execute_builtin_function(&func_name, arg_values, ctx)
    }

    /// Execute a builtin function
    fn execute_builtin_function(
        &self,
        func_name: &str,
        arg_values: Vec<ConstValue>,
        ctx: &MetaContext,
    ) -> Result<ConstValue, SandboxError> {
        match func_name {
            // ========== Collection Constructors ==========
            "List.new" | "list" | "List" => Ok(ConstValue::Array(arg_values.into_iter().collect())),
            "Map.new" | "map" | "Map" => {
                Ok(ConstValue::Tuple(List::new())) // Empty map as tuple for now
            }
            "Set.new" | "set" | "Set" => Ok(ConstValue::Array(List::new())),

            // ========== Option/Result Constructors ==========
            "Maybe.Some" | "Some" => {
                if arg_values.len() != 1 {
                    return Err(SandboxError::UnsafeOperation {
                        operation: Text::from("Some"),
                        reason: Text::from("Some expects exactly 1 argument"),
                    });
                }
                let mut result = List::new();
                result.push(ConstValue::Text(Text::from("Some")));
                result.push(arg_values.into_iter().next().unwrap());
                Ok(ConstValue::Tuple(result))
            }
            "Maybe.None" | "None" => {
                let mut result = List::new();
                result.push(ConstValue::Text(Text::from("None")));
                Ok(ConstValue::Tuple(result))
            }
            // Result.Ok / Result.Err — symmetric to Maybe.Some /
            // Maybe.None above. Pre-fix the meta sandbox handled
            // only Maybe constructors; meta code constructing
            // `Ok(x)` or `Err(e)` errored out as "unknown function".
            // Both forms (qualified `Result.Ok` and bare `Ok`)
            // accepted to mirror the Maybe arms — `Ok` may be
            // glob-imported via `mount core.base.{Ok}` in the meta
            // function's source, so the bare form must work.
            "Result.Ok" | "Ok" => {
                if arg_values.len() != 1 {
                    return Err(SandboxError::UnsafeOperation {
                        operation: Text::from("Ok"),
                        reason: Text::from("Ok expects exactly 1 argument"),
                    });
                }
                let mut result = List::new();
                result.push(ConstValue::Text(Text::from("Ok")));
                result.push(arg_values.into_iter().next().unwrap());
                Ok(ConstValue::Tuple(result))
            }
            "Result.Err" | "Err" => {
                if arg_values.len() != 1 {
                    return Err(SandboxError::UnsafeOperation {
                        operation: Text::from("Err"),
                        reason: Text::from("Err expects exactly 1 argument"),
                    });
                }
                let mut result = List::new();
                result.push(ConstValue::Text(Text::from("Err")));
                result.push(arg_values.into_iter().next().unwrap());
                Ok(ConstValue::Tuple(result))
            }

            // ========== Collection Operations ==========
            "len" => {
                if arg_values.len() != 1 {
                    return Err(SandboxError::UnsafeOperation {
                        operation: Text::from("len"),
                        reason: Text::from("len expects exactly 1 argument"),
                    });
                }
                let len = match &arg_values[0] {
                    ConstValue::Array(arr) => arr.len() as i128,
                    ConstValue::Text(t) => t.len() as i128,
                    ConstValue::Tuple(t) => t.len() as i128,
                    _ => {
                        return Err(SandboxError::UnsafeOperation {
                            operation: Text::from("len"),
                            reason: Text::from("len requires a collection or text"),
                        });
                    }
                };
                Ok(ConstValue::Int(len))
            }

            // ========== Type Introspection ==========
            "typeof" | "type_of" => {
                if arg_values.len() != 1 {
                    return Err(SandboxError::UnsafeOperation {
                        operation: Text::from("typeof"),
                        reason: Text::from("typeof expects exactly 1 argument"),
                    });
                }
                let type_name = match &arg_values[0] {
                    ConstValue::Unit => "Unit",
                    ConstValue::Bool(_) => "Bool",
                    ConstValue::Int(_) => "Int",
                    ConstValue::UInt(_) => "UInt",
                    ConstValue::Float(_) => "Float",
                    ConstValue::Char(_) => "Char",
                    ConstValue::Text(_) => "Text",
                    ConstValue::Array(_) => "List",
                    ConstValue::Tuple(_) => "Tuple",
                    ConstValue::Map(_) => "Map",
                    ConstValue::Set(_) => "Set",
                    ConstValue::Expr(_) => "Expr",
                    ConstValue::Type(_) => "Type",
                    ConstValue::Pattern(_) => "Pattern",
                    ConstValue::Item(_) => "Item",
                    ConstValue::Items(_) => "Items",
                    ConstValue::Bytes(_) => "Bytes",
                    ConstValue::Maybe(_) => "Maybe",
                };
                Ok(ConstValue::Text(Text::from(type_name)))
            }

            // ========== Reflection API ==========
            "fields_of" => {
                if arg_values.len() != 1 {
                    return Err(SandboxError::UnsafeOperation {
                        operation: Text::from("fields_of"),
                        reason: Text::from("fields_of expects exactly 1 argument"),
                    });
                }
                let type_name = match &arg_values[0] {
                    ConstValue::Text(t) => t.clone(),
                    ConstValue::Type(ty) => Text::from(format!("{:?}", ty)),
                    _ => {
                        return Err(SandboxError::UnsafeOperation {
                            operation: Text::from("fields_of"),
                            reason: Text::from("fields_of expects a type name (Text) or Type"),
                        });
                    }
                };

                if let Some(fields) = ctx.get_struct_fields(&type_name) {
                    let field_info: List<ConstValue> = fields
                        .iter()
                        .map(|(name, ty)| {
                            ConstValue::Tuple(
                                vec![
                                    ConstValue::Text(name.clone()),
                                    ConstValue::Text(Text::from(format!("{:?}", ty))),
                                ]
                                .into_iter()
                                .collect(),
                            )
                        })
                        .collect();
                    Ok(ConstValue::Array(field_info))
                } else {
                    Ok(ConstValue::Array(List::new()))
                }
            }

            // ========== Numeric Operations ==========
            "abs" => {
                if arg_values.len() != 1 {
                    return Err(SandboxError::UnsafeOperation {
                        operation: Text::from("abs"),
                        reason: Text::from("abs expects exactly 1 argument"),
                    });
                }
                match &arg_values[0] {
                    ConstValue::Int(i) => Ok(ConstValue::Int(i.abs())),
                    ConstValue::Float(f) => Ok(ConstValue::Float(f.abs())),
                    _ => Err(SandboxError::UnsafeOperation {
                        operation: Text::from("abs"),
                        reason: Text::from("abs requires a numeric argument"),
                    }),
                }
            }

            // ========== Code Generation Helpers ==========
            "stringify" => {
                if arg_values.len() != 1 {
                    return Err(SandboxError::UnsafeOperation {
                        operation: Text::from("stringify"),
                        reason: Text::from("stringify expects exactly 1 argument"),
                    });
                }
                let text = match &arg_values[0] {
                    ConstValue::Text(t) => t.clone(),
                    ConstValue::Int(i) => Text::from(i.to_string()),
                    ConstValue::Float(f) => Text::from(f.to_string()),
                    ConstValue::Bool(b) => Text::from(b.to_string()),
                    ConstValue::Expr(_) => Text::from("<expr>"),
                    ConstValue::Type(_) => Text::from("<type>"),
                    _ => Text::from("<value>"),
                };
                Ok(ConstValue::Text(text))
            }

            // ========== Asset Loading Operations ==========
            "include_str" | "include_bytes" | "include_file" | "load_build_asset" => {
                if !self.limiter.is_asset_loading_allowed() {
                    return Err(SandboxError::AssetLoadingNotAllowed {
                        function: Text::from(func_name),
                        message: Text::from(format!(
                            "{} can only be called from functions with `using BuildAssets` context",
                            func_name
                        )),
                    });
                }
                // Pre-fix this branch returned a hardcoded "Asset loading not
                // implemented" error EVEN WHEN the BuildAssets gate above
                // passed — making the gate a misleading no-op. Now actually
                // delegates to the BuildAssets subsystem's path-validated
                // file readers.
                //
                // We use the `_uncached` variants because `ctx` arrives as
                // `&MetaContext` here (not `&mut`) — threading `&mut`
                // through `execute_expr`'s recursive callgraph would
                // balloon blast radius. Cache miss cost is dominated by
                // the file-read cost itself, so the architectural
                // tradeoff is favourable.
                if arg_values.len() != 1 {
                    return Err(SandboxError::UnsafeOperation {
                        operation: Text::from(func_name),
                        reason: Text::from(format!(
                            "{} expects exactly 1 argument (the path)",
                            func_name
                        )),
                    });
                }
                let path = match &arg_values[0] {
                    ConstValue::Text(t) => t.clone(),
                    _ => {
                        return Err(SandboxError::UnsafeOperation {
                            operation: Text::from(func_name),
                            reason: Text::from(format!(
                                "{} requires a Text argument (the path)",
                                func_name
                            )),
                        });
                    }
                };
                match func_name {
                    "include_str" | "include_file" => {
                        ctx.build_assets
                            .read_text_uncached(path.as_str())
                            .map(ConstValue::Text)
                            .map_err(|e| SandboxError::UnsafeOperation {
                                operation: Text::from(func_name),
                                reason: Text::from(format!("{:?}", e)),
                            })
                    }
                    "include_bytes" | "load_build_asset" => {
                        ctx.build_assets
                            .read_bytes_uncached(path.as_str())
                            .map(|bytes| {
                                ConstValue::Array(
                                    bytes
                                        .into_iter()
                                        .map(|b| ConstValue::Int(b as i128))
                                        .collect(),
                                )
                            })
                            .map_err(|e| SandboxError::UnsafeOperation {
                                operation: Text::from(func_name),
                                reason: Text::from(format!("{:?}", e)),
                            })
                    }
                    // Unreachable — outer match guard already filtered to
                    // these four names. The catch-all is defensive in
                    // case the outer pattern is later widened.
                    _ => unreachable!("outer match guard restricts func_name"),
                }
            }

            // ========== Unknown function ==========
            _ => {
                Err(SandboxError::ForbiddenFunction {
                    function: Text::from(func_name),
                    reason: Text::from(
                        "Unknown function in meta context. Only built-in meta functions are allowed.",
                    ),
                })
            }
        }
    }
}

impl Default for SandboxedExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for SandboxedExecutor {
    fn clone(&self) -> Self {
        Self {
            limiter: self.limiter.clone(),
            validator: self.validator.clone(),
            allowlist: self.allowlist.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_executor_creation() {
        let executor = SandboxedExecutor::new();
        assert_eq!(executor.limiter().current_iterations(), 0);
    }

    #[test]
    fn execute_builtin_ok_emits_tagged_tuple() {
        // Pin the Ok constructor: meta code calling `Ok(42)` must
        // produce a tagged tuple `["Ok", 42]` matching the existing
        // Some pattern. Pre-fix Result.Ok / Ok hit the catch-all
        // ForbiddenFunction error.
        let exec = SandboxedExecutor::new();
        let ctx = MetaContext::new();
        let result = exec
            .execute_builtin_function("Ok", vec![ConstValue::Int(42)], &ctx)
            .expect("Ok must succeed when sandbox supports Result");
        match result {
            ConstValue::Tuple(parts) => {
                assert_eq!(parts.len(), 2);
                assert!(matches!(&parts[0], ConstValue::Text(t) if t.as_str() == "Ok"));
                assert!(matches!(&parts[1], ConstValue::Int(42)));
            }
            other => panic!("Ok must produce a Tuple, got {:?}", other),
        }
    }

    #[test]
    fn execute_builtin_err_emits_tagged_tuple() {
        // Symmetric pin for Err. A regression that drops the Err
        // arm but keeps Ok would silently break Result-returning
        // meta functions.
        let exec = SandboxedExecutor::new();
        let ctx = MetaContext::new();
        let result = exec
            .execute_builtin_function(
                "Err",
                vec![ConstValue::Text(Text::from("oops"))],
                &ctx,
            )
            .expect("Err must succeed when sandbox supports Result");
        match result {
            ConstValue::Tuple(parts) => {
                assert_eq!(parts.len(), 2);
                assert!(matches!(&parts[0], ConstValue::Text(t) if t.as_str() == "Err"));
                assert!(matches!(&parts[1], ConstValue::Text(t) if t.as_str() == "oops"));
            }
            other => panic!("Err must produce a Tuple, got {:?}", other),
        }
    }

    #[test]
    fn execute_builtin_qualified_result_paths_resolve() {
        // Pin: both bare (`Ok(x)`) and qualified (`Result.Ok(x)`)
        // forms must work, mirroring the Some/None convention.
        // Glob-imported `mount core.base.{Ok}` produces the bare
        // form; explicit qualification produces the qualified form.
        // A regression that drops one path silently breaks one
        // import style.
        let exec = SandboxedExecutor::new();
        let ctx = MetaContext::new();
        let qualified = exec
            .execute_builtin_function("Result.Ok", vec![ConstValue::Int(7)], &ctx)
            .expect("Result.Ok must resolve");
        let bare = exec
            .execute_builtin_function("Ok", vec![ConstValue::Int(7)], &ctx)
            .expect("Ok must resolve");
        // Both paths produce identical tagged-tuple shapes.
        assert!(matches!(&qualified, ConstValue::Tuple(p) if p.len() == 2));
        assert!(matches!(&bare, ConstValue::Tuple(p) if p.len() == 2));
    }

    #[test]
    fn include_str_blocked_when_asset_gate_closed() {
        // Pin the security gate: include_str / include_bytes /
        // include_file / load_build_asset MUST refuse with
        // AssetLoadingNotAllowed when the BuildAssets context isn't
        // active. Pre-fix the gate fired correctly but then a
        // hardcoded "not implemented" error masked the wired path —
        // this test pins both the gate and the new wiring at the
        // same boundary.
        let exec = SandboxedExecutor::new();
        let ctx = MetaContext::new();
        // Gate is closed by default (no BuildAssets context).
        let err = exec
            .execute_builtin_function(
                "include_str",
                vec![ConstValue::Text(Text::from("anything.txt"))],
                &ctx,
            )
            .unwrap_err();
        assert!(
            matches!(err, SandboxError::AssetLoadingNotAllowed { .. }),
            "expected AssetLoadingNotAllowed, got {:?}",
            err
        );
    }

    #[test]
    fn include_str_reads_file_when_gate_open() {
        // End-to-end pin: with the asset-loading gate open AND a
        // configured project root, include_str reads the file's
        // text content via the BuildAssets read_text_uncached path.
        // Pre-fix this returned a hardcoded "not implemented"
        // SandboxError::UnsafeOperation regardless of gate state.
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let asset_path = dir.path().join("hello.txt");
        let mut f = std::fs::File::create(&asset_path).unwrap();
        f.write_all(b"hello world").unwrap();

        let exec = SandboxedExecutor::new();
        let mut ctx = MetaContext::new();
        ctx.build_assets = ctx
            .build_assets
            .clone()
            .with_project_root(dir.path().to_string_lossy().to_string());

        let result = exec.limiter().with_asset_loading(|| {
            exec.execute_builtin_function(
                "include_str",
                vec![ConstValue::Text(Text::from("hello.txt"))],
                &ctx,
            )
        });

        let value = result.expect("include_str must succeed when gate is open + path resolves");
        assert!(
            matches!(&value, ConstValue::Text(t) if t.as_str() == "hello world"),
            "expected ConstValue::Text(\"hello world\"), got {:?}",
            value
        );
    }
}
