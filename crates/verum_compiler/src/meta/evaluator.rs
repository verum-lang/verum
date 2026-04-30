//! Meta Expression Evaluator
//!
//! This module provides expression evaluation logic for meta-programming,
//! including AST-to-MetaExpr conversion and MetaExpr evaluation.
//!
//! ## Responsibility
//!
//! The evaluator handles:
//! - Converting AST expressions to MetaExpr (meta IR)
//! - Evaluating MetaExpr to produce ConstValue
//! - Pattern matching for meta match expressions
//! - Type inference for meta values
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

use verum_ast::{
    Span,
    expr::{BinOp, Expr, ExprKind, TypeProperty, UnOp},
    pattern::{Pattern, PatternKind},
    stmt::StmtKind,
    ty::{GenericArg, Ident, Path, Type, TypeKind},
};
use verum_common::well_known_types::variant_tags;
use verum_common::{Heap, List, Maybe, Text};

use super::{MetaError, MetaExpr, MetaPattern, MetaStmt};
use super::builtins::EnabledContexts;
use super::context::{ConstValue, MetaContext};
use super::ir::expr::MetaArm;
use super::registry::MetaFunction;

/// Extract a qualified path from an expression chain.
///
/// For example, given `std.env` (parsed as Field { expr: Path("std"), field: "env" }),
/// this returns Some("std.env"). For expressions that aren't simple namespace paths,
/// returns None.
fn extract_qualified_path(expr: &Expr) -> Option<String> {
    match &expr.kind {
        // Base case: a single-segment path like "std"
        ExprKind::Path(path) => {
            if let Some(ident) = path.as_ident() {
                Some(ident.as_str().to_string())
            } else {
                // Multi-segment path like std::collections
                // Join segments with dots for Verum syntax
                let segments: Vec<&str> = path.segments.iter()
                    .filter_map(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(ident) => Some(ident.as_str()),
                        _ => None,
                    })
                    .collect();
                if !segments.is_empty() {
                    Some(segments.join("."))
                } else {
                    None
                }
            }
        }
        // Recursive case: field access like std.env
        ExprKind::Field { expr: inner, field } => {
            extract_qualified_path(inner).map(|prefix| {
                format!("{}.{}", prefix, field.as_str())
            })
        }
        _ => None,
    }
}

/// Convert an AST Pattern to a MetaPattern
///
/// MetaPattern is a comprehensive pattern representation for compile-time evaluation.
/// Supports most pattern kinds for industrial-grade meta-programming.
fn ast_pattern_to_meta_pattern(pattern: &Pattern) -> Result<MetaPattern, MetaError> {
    match &pattern.kind {
        PatternKind::Wildcard => Ok(MetaPattern::Wildcard),

        PatternKind::Ident { name, subpattern, .. } => {
            if let Maybe::Some(sub) = subpattern {
                // x @ pattern - supported
                let sub_meta = ast_pattern_to_meta_pattern(sub)?;
                Ok(MetaPattern::ident_at(Text::from(name.as_str()), sub_meta))
            } else {
                Ok(MetaPattern::Ident(Text::from(name.as_str())))
            }
        }

        PatternKind::Literal(lit) => {
            let val = ConstValue::from_literal(lit);
            Ok(MetaPattern::Literal(val))
        }

        PatternKind::Tuple(patterns) => {
            let meta_patterns = patterns
                .iter()
                .map(ast_pattern_to_meta_pattern)
                .collect::<Result<List<_>, _>>()?;
            Ok(MetaPattern::Tuple(meta_patterns))
        }

        PatternKind::Or(patterns) => {
            let meta_patterns = patterns
                .iter()
                .map(ast_pattern_to_meta_pattern)
                .collect::<Result<List<_>, _>>()?;
            Ok(MetaPattern::Or(meta_patterns))
        }

        PatternKind::Paren(inner) => {
            // Parenthesized pattern - just unwrap
            ast_pattern_to_meta_pattern(inner)
        }

        PatternKind::Rest => Ok(MetaPattern::Rest(Maybe::None)),

        PatternKind::Array(patterns) => {
            let meta_patterns = patterns
                .iter()
                .map(ast_pattern_to_meta_pattern)
                .collect::<Result<List<_>, _>>()?;
            Ok(MetaPattern::Array(meta_patterns))
        }

        PatternKind::Slice { before, rest, after } => {
            let before_meta = before
                .iter()
                .map(ast_pattern_to_meta_pattern)
                .collect::<Result<List<_>, _>>()?;
            let after_meta = after
                .iter()
                .map(ast_pattern_to_meta_pattern)
                .collect::<Result<List<_>, _>>()?;
            // Extract rest binding name if it's an identifier pattern
            let rest_name = if let Maybe::Some(rest_pat) = rest {
                match &rest_pat.kind {
                    PatternKind::Ident { name, .. } => Maybe::Some(Text::from(name.as_str())),
                    PatternKind::Wildcard | PatternKind::Rest => Maybe::None,
                    _ => Maybe::None,
                }
            } else {
                Maybe::None
            };
            Ok(MetaPattern::Slice {
                before: before_meta,
                rest: rest_name,
                after: after_meta,
            })
        }

        PatternKind::Record { path, fields, rest } => {
            let name = path
                .as_ident()
                .map(|i| Text::from(i.as_str()))
                .unwrap_or_else(|| Text::from(""));
            let meta_fields = fields
                .iter()
                .map(|f| {
                    let field_name = Text::from(f.name.as_str());
                    let field_pat = if let Maybe::Some(ref pat) = f.pattern {
                        ast_pattern_to_meta_pattern(pat)?
                    } else {
                        // Shorthand: { x } means { x: x }
                        MetaPattern::Ident(field_name.clone())
                    };
                    Ok((field_name, field_pat))
                })
                .collect::<Result<List<_>, _>>()?;
            Ok(MetaPattern::Record {
                name,
                fields: meta_fields,
                rest: *rest,
            })
        }

        PatternKind::Variant { path, data } => {
            let name = path
                .as_ident()
                .map(|i| Text::from(i.as_str()))
                .unwrap_or_else(|| {
                    // For paths like Option::Some, use the last segment
                    path.segments.last()
                        .and_then(|s| match s {
                            verum_ast::ty::PathSegment::Name(ident) => Some(Text::from(ident.as_str())),
                            _ => None,
                        })
                        .unwrap_or_else(|| Text::from(""))
                });
            let data_meta = if let Maybe::Some(variant_data) = data {
                use verum_ast::pattern::VariantPatternData;
                match variant_data {
                    VariantPatternData::Tuple(pats) => {
                        if pats.len() == 1 {
                            Maybe::Some(ast_pattern_to_meta_pattern(&pats[0])?)
                        } else {
                            let tuple_pats = pats
                                .iter()
                                .map(ast_pattern_to_meta_pattern)
                                .collect::<Result<List<_>, _>>()?;
                            Maybe::Some(MetaPattern::Tuple(tuple_pats))
                        }
                    }
                    VariantPatternData::Record { fields, rest } => {
                        let meta_fields = fields
                            .iter()
                            .map(|f| {
                                let field_name = Text::from(f.name.as_str());
                                let field_pat = if let Maybe::Some(ref pat) = f.pattern {
                                    ast_pattern_to_meta_pattern(pat)?
                                } else {
                                    MetaPattern::Ident(field_name.clone())
                                };
                                Ok((field_name, field_pat))
                            })
                            .collect::<Result<List<_>, _>>()?;
                        Maybe::Some(MetaPattern::Record {
                            name: name.clone(),
                            fields: meta_fields,
                            rest: *rest,
                        })
                    }
                }
            } else {
                Maybe::None
            };
            Ok(MetaPattern::Variant { name, data: data_meta.map(Heap::new) })
        }

        PatternKind::Reference { mutable, inner } => {
            let inner_meta = ast_pattern_to_meta_pattern(inner)?;
            Ok(MetaPattern::Reference {
                mutable: *mutable,
                inner: Heap::new(inner_meta),
            })
        }

        PatternKind::Range { start, end, inclusive } => {
            let start_val = if let Maybe::Some(lit) = start {
                Maybe::Some(ConstValue::from_literal(lit))
            } else {
                Maybe::None
            };
            let end_val = if let Maybe::Some(lit) = end {
                Maybe::Some(ConstValue::from_literal(lit))
            } else {
                Maybe::None
            };
            Ok(MetaPattern::Range {
                start: start_val,
                end: end_val,
                inclusive: *inclusive,
            })
        }

        PatternKind::And(patterns) => {
            let meta_patterns = patterns
                .iter()
                .map(ast_pattern_to_meta_pattern)
                .collect::<Result<List<_>, _>>()?;
            Ok(MetaPattern::And(meta_patterns))
        }

        PatternKind::TypeTest { binding, test_type } => {
            let type_name = match &test_type.kind {
                TypeKind::Path(path) => path
                    .as_ident()
                    .map(|i| Text::from(i.as_str()))
                    .unwrap_or_else(|| Text::from("")),
                _ => Text::from(""),
            };
            Ok(MetaPattern::TypeTest {
                name: Text::from(binding.as_str()),
                type_name,
            })
        }

        // Patterns that require runtime execution
        PatternKind::View { .. } => Err(MetaError::Other(Text::from(
            "View patterns require runtime execution and are not supported in meta patterns"
        ))),
        PatternKind::Active { .. } => Err(MetaError::Other(Text::from(
            "Active patterns require runtime execution and are not supported in meta patterns"
        ))),
        PatternKind::Stream { .. } => Err(MetaError::Other(Text::from(
            "Stream patterns require async execution and are not supported in meta patterns"
        ))),
        PatternKind::Guard { .. } => Err(MetaError::Other(Text::from(
            "Guard patterns require runtime evaluation and are not supported in meta patterns"
        ))),
        PatternKind::Cons { .. } => Err(MetaError::Other(Text::from(
            "Cons patterns are not supported in meta patterns"
        ))),
    }
}

/// Check if a name is a primitive type name
fn is_primitive_type_name(name: &str) -> bool {
    verum_common::well_known_types::type_names::is_primitive_value_type(name)
        || matches!(name, "Text" | "Never")
}

/// Check if a name looks like a type name (PascalCase convention)
/// In Verum, types use PascalCase (Point, Color) while variables use snake_case (my_var)
fn is_likely_type_name(name: &str) -> bool {
    // Must start with uppercase letter and not be all uppercase (which might be a constant)
    if let Some(first_char) = name.chars().next() {
        first_char.is_ascii_uppercase()
    } else {
        false
    }
}

/// Convert a primitive type name to a Type
fn primitive_type_from_name(name: &str) -> Type {
    use verum_ast::ty::TypeKind;
    let kind = match name {
        "Int" => TypeKind::Int,
        "Float" => TypeKind::Float,
        "Bool" => TypeKind::Bool,
        "Text" => TypeKind::Text,
        "Char" => TypeKind::Char,
        "Unit" => TypeKind::Unit,
        "Never" => TypeKind::Never,
        _ => TypeKind::Unknown, // Fallback
    };
    Type::new(kind, Span::default())
}

/// Try to convert a Path to a Type (for generic types like List<Int>)
fn path_to_type(path: &Path) -> Option<Type> {
    use verum_ast::ty::TypeKind;

    // For complex paths, wrap them in TypeKind::Path
    Some(Type::new(TypeKind::Path(path.clone()), path.span))
}

impl MetaContext {
    /// Convert AST statement to MetaStmt
    fn ast_stmt_to_meta_stmt(&self, stmt: &verum_ast::stmt::Stmt) -> Result<MetaStmt, MetaError> {
        match &stmt.kind {
            StmtKind::Expr { expr, .. } => {
                let meta_expr = self.ast_expr_to_meta_expr(expr)?;
                Ok(MetaStmt::Expr(meta_expr))
            }
            StmtKind::Let { pattern, value, .. } => {
                let meta_value = if let Maybe::Some(val_expr) = value {
                    self.ast_expr_to_meta_expr(val_expr)?
                } else {
                    MetaExpr::Literal(ConstValue::Unit)
                };

                // Handle different pattern kinds
                match &pattern.kind {
                    PatternKind::Ident { name, .. } => {
                        Ok(MetaStmt::Let {
                            name: Text::from(name.as_str()),
                            value: meta_value,
                        })
                    }
                    PatternKind::Tuple(patterns) => {
                        // Convert tuple pattern to LetTuple
                        let names = patterns
                            .iter()
                            .map(|p| self.extract_pattern_name(p))
                            .collect::<Result<Vec<_>, _>>()?;
                        Ok(MetaStmt::LetTuple {
                            names,
                            value: meta_value,
                        })
                    }
                    PatternKind::Wildcard => {
                        // Wildcard pattern - just evaluate the value for side effects
                        Ok(MetaStmt::Expr(meta_value))
                    }
                    _ => Err(MetaError::Other(Text::from(
                        "Complex patterns not yet supported in meta let bindings",
                    ))),
                }
            }
            StmtKind::LetElse { .. } => {
                Err(MetaError::Other(Text::from(
                    "let-else not yet supported in meta functions"
                )))
            }
            StmtKind::Item(_) => {
                Err(MetaError::Other(Text::from(
                    "Item declarations not yet supported in meta blocks"
                )))
            }
            StmtKind::Defer(_) => {
                Err(MetaError::Other(Text::from(
                    "defer not yet supported in meta functions"
                )))
            }
            StmtKind::Errdefer { .. } => {
                Err(MetaError::Other(Text::from(
                    "errdefer not yet supported in meta functions"
                )))
            }
            StmtKind::Provide { .. } => {
                Err(MetaError::Other(Text::from(
                    "provide not yet supported in meta functions"
                )))
            }
            StmtKind::ProvideScope { .. } => {
                Err(MetaError::Other(Text::from(
                    "provide scope not yet supported in meta functions"
                )))
            }
            StmtKind::Empty => {
                // Empty statement - just return Unit
                Ok(MetaStmt::Expr(MetaExpr::Literal(ConstValue::Unit)))
            }
        }
    }

    /// Extract a binding name from a pattern for tuple destructuring
    /// Returns Some(name) for identifier patterns, None for wildcard
    fn extract_pattern_name(&self, pattern: &Pattern) -> Result<Option<Text>, MetaError> {
        match &pattern.kind {
            PatternKind::Ident { name, .. } => Ok(Some(Text::from(name.as_str()))),
            PatternKind::Wildcard => Ok(None),
            _ => Err(MetaError::Other(Text::from(
                "Only simple identifiers and wildcards supported in tuple patterns",
            ))),
        }
    }

    /// Convert AST Expr to MetaExpr
    pub fn ast_expr_to_meta_expr(&self, expr: &Expr) -> Result<MetaExpr, MetaError> {
        match &expr.kind {
            ExprKind::Literal(lit) => {
                let val = ConstValue::from_literal(lit);
                Ok(MetaExpr::Literal(val))
            }
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    let name = ident.as_str();
                    // Check if this is a primitive type name - treat as type literal
                    if is_primitive_type_name(name) {
                        let ty = primitive_type_from_name(name);
                        Ok(MetaExpr::Literal(ConstValue::Type(ty)))
                    } else if is_likely_type_name(name) {
                        // PascalCase name - likely a user-defined type (Point, Color, etc.)
                        // Treat as a type literal using Path TypeKind
                        let ty = path_to_type(path).unwrap_or_else(|| {
                            Type::new(TypeKind::Unknown, Span::default())
                        });
                        Ok(MetaExpr::Literal(ConstValue::Type(ty)))
                    } else {
                        // snake_case name - treat as a variable
                        Ok(MetaExpr::Variable(Text::from(name)))
                    }
                } else {
                    // Could be a complex path like List<Int> - try to interpret as type
                    match path_to_type(path) {
                        Some(ty) => Ok(MetaExpr::Literal(ConstValue::Type(ty))),
                        None => Err(MetaError::Other(Text::from(
                            "Complex paths not yet supported in meta functions",
                        ))),
                    }
                }
            }
            // TypeExpr: generic type expressions like List<Int>, Maybe<Text>, etc.
            // These are parsed as ExprKind::TypeExpr(Type) and should produce a Type literal.
            ExprKind::TypeExpr(ty) => {
                Ok(MetaExpr::Literal(ConstValue::Type(ty.clone())))
            }
            ExprKind::Binary { op, left, right } => {
                let left_meta = self.ast_expr_to_meta_expr(left)?;
                let right_meta = self.ast_expr_to_meta_expr(right)?;
                Ok(MetaExpr::Binary {
                    op: *op,
                    left: Heap::new(left_meta),
                    right: Heap::new(right_meta),
                })
            }
            ExprKind::Call { func, args, .. } => {
                if let ExprKind::Path(path) = &func.kind {
                    if let Some(ident) = path.as_ident() {
                        let func_name = Text::from(ident.as_str());
                        let meta_args = args
                            .iter()
                            .map(|arg| self.ast_expr_to_meta_expr(arg))
                            .collect::<Result<List<_>, _>>()?;
                        return Ok(MetaExpr::Call(func_name, meta_args));
                    }
                }
                Err(MetaError::Other(Text::from(
                    "Complex function calls not yet supported",
                )))
            }
            ExprKind::Block(block) => {
                // Convert all statements first
                let mut stmts: List<MetaStmt> = block
                    .stmts
                    .iter()
                    .map(|stmt| self.ast_stmt_to_meta_stmt(stmt))
                    .collect::<Result<List<_>, _>>()?;

                // If there's a tail expression, add it as a final expression statement
                if let Maybe::Some(tail_expr) = &block.expr {
                    let meta_tail = self.ast_expr_to_meta_expr(tail_expr)?;
                    stmts.push(MetaStmt::Expr(meta_tail));
                }

                Ok(MetaExpr::Block(stmts))
            }
            ExprKind::If { condition, then_branch, else_branch } => {
                // Extract the first condition expression from IfCondition
                let cond_expr = condition.conditions.first()
                    .ok_or_else(|| MetaError::Other(Text::from("Empty if condition")))?;
                let cond = match cond_expr {
                    verum_ast::ConditionKind::Expr(e) => self.ast_expr_to_meta_expr(e)?,
                    verum_ast::ConditionKind::Let { .. } => {
                        return Err(MetaError::Other(Text::from("Let bindings in if condition not supported in meta")))
                    }
                };
                // Convert block to expression (use block result)
                let then_stmts: List<MetaStmt> = then_branch.stmts.iter()
                    .map(|s| self.ast_stmt_to_meta_stmt(s))
                    .collect::<Result<List<_>, _>>()?;
                let mut then_all_stmts = then_stmts;
                if let Maybe::Some(tail) = &then_branch.expr {
                    let tail_meta = self.ast_expr_to_meta_expr(tail)?;
                    then_all_stmts.push(MetaStmt::Expr(tail_meta));
                }
                let then_expr = MetaExpr::Block(then_all_stmts);
                let else_expr = if let Maybe::Some(else_e) = else_branch {
                    Maybe::Some(Heap::new(self.ast_expr_to_meta_expr(else_e)?))
                } else {
                    Maybe::None
                };
                Ok(MetaExpr::If {
                    condition: Heap::new(cond),
                    then_branch: Heap::new(then_expr),
                    else_branch: else_expr,
                })
            }
            ExprKind::Unary { op, expr: inner } => {
                let inner_meta = self.ast_expr_to_meta_expr(inner)?;
                Ok(MetaExpr::Unary {
                    op: *op,
                    expr: Heap::new(inner_meta),
                })
            }
            ExprKind::Tuple(elements) => {
                let meta_elements = elements
                    .iter()
                    .map(|e| self.ast_expr_to_meta_expr(e))
                    .collect::<Result<List<_>, _>>()?;
                // Convert to tuple literal
                Ok(MetaExpr::Literal(ConstValue::Tuple(
                    meta_elements
                        .into_iter()
                        .map(|e| match e {
                            MetaExpr::Literal(v) => v,
                            _ => ConstValue::Unit,
                        })
                        .collect(),
                )))
            }
            ExprKind::Array(array_expr) => {
                match array_expr {
                    verum_ast::ArrayExpr::List(elements) => {
                        let meta_elements = elements
                            .iter()
                            .map(|e| self.ast_expr_to_meta_expr(e))
                            .collect::<Result<List<_>, _>>()?;
                        Ok(MetaExpr::Literal(ConstValue::Array(
                            meta_elements
                                .into_iter()
                                .map(|e| match e {
                                    MetaExpr::Literal(v) => v,
                                    _ => ConstValue::Unit,
                                })
                                .collect(),
                        )))
                    }
                    verum_ast::ArrayExpr::Repeat { value, count } => {
                        // For repeat syntax [value; count], evaluate count
                        let val = self.ast_expr_to_meta_expr(value)?;
                        let count_val = self.ast_expr_to_meta_expr(count)?;
                        // For now, just quote the repeat expression
                        if let (MetaExpr::Literal(v), MetaExpr::Literal(ConstValue::UInt(n))) = (&val, &count_val) {
                            let arr: List<ConstValue> = (0..*n).map(|_| v.clone()).collect();
                            Ok(MetaExpr::Literal(ConstValue::Array(arr)))
                        } else {
                            // Can't evaluate at compile time - quote it
                            Ok(MetaExpr::Quote(expr.clone()))
                        }
                    }
                }
            }
            // Priority 1: MethodCall
            ExprKind::MethodCall { receiver, method, args, .. } => {
                // First, check if this is a qualified function call like std.env.var(...)
                // Extract the full path from the receiver
                if let Some(receiver_path) = extract_qualified_path(receiver) {
                    // Combine receiver path with method to get full qualified name
                    let qualified_name = Text::from(format!("{}.{}", receiver_path, method.as_str()));

                    // Check if this is a forbidden operation
                    if self.is_forbidden_function(&qualified_name) {
                        let category = self.get_forbidden_category(&qualified_name)
                            .unwrap_or("I/O");
                        return Err(MetaError::ForbiddenOperation {
                            operation: qualified_name,
                            reason: Text::from(format!(
                                "{} operations are forbidden in meta functions",
                                category
                            )),
                        });
                    }

                    // Convert to regular Call with qualified name
                    let meta_args = args
                        .iter()
                        .map(|arg| self.ast_expr_to_meta_expr(arg))
                        .collect::<Result<List<_>, _>>()?;
                    return Ok(MetaExpr::Call(qualified_name, meta_args));
                }

                // Normal method call
                let recv = self.ast_expr_to_meta_expr(receiver)?;
                let meta_args = args
                    .iter()
                    .map(|arg| self.ast_expr_to_meta_expr(arg))
                    .collect::<Result<List<_>, _>>()?;
                Ok(MetaExpr::MethodCall {
                    receiver: Heap::new(recv),
                    method: Text::from(method.as_str()),
                    args: meta_args,
                })
            }
            // Priority 1: Field access
            ExprKind::Field { expr: inner, field } => {
                let inner_meta = self.ast_expr_to_meta_expr(inner)?;
                Ok(MetaExpr::FieldAccess {
                    expr: Heap::new(inner_meta),
                    field: Text::from(field.as_str()),
                })
            }
            // Priority 1: Index
            ExprKind::Index { expr: inner, index } => {
                let inner_meta = self.ast_expr_to_meta_expr(inner)?;
                let index_meta = self.ast_expr_to_meta_expr(index)?;
                Ok(MetaExpr::Index {
                    expr: Heap::new(inner_meta),
                    index: Heap::new(index_meta),
                })
            }
            // Priority 1: Match
            ExprKind::Match { expr: scrutinee, arms } => {
                let scrutinee_meta = self.ast_expr_to_meta_expr(scrutinee)?;
                let meta_arms = arms
                    .iter()
                    .map(|arm| {
                        let pattern = ast_pattern_to_meta_pattern(&arm.pattern)?;
                        let guard = if let Maybe::Some(ref g) = arm.guard {
                            Maybe::Some(self.ast_expr_to_meta_expr(g)?)
                        } else {
                            Maybe::None
                        };
                        let body = self.ast_expr_to_meta_expr(&arm.body)?;
                        Ok(MetaArm { pattern, guard, body })
                    })
                    .collect::<Result<List<_>, _>>()?;
                Ok(MetaExpr::Match {
                    scrutinee: Heap::new(scrutinee_meta),
                    arms: meta_arms,
                })
            }
            // Priority 1: Closure
            ExprKind::Closure { params, body, return_type, .. } => {
                let meta_params = params
                    .iter()
                    .filter_map(|p| {
                        // Extract the identifier from the pattern
                        if let verum_ast::PatternKind::Ident { name, .. } = &p.pattern.kind {
                            Some(Text::from(name.as_str()))
                        } else {
                            None
                        }
                    })
                    .collect();
                let meta_body = self.ast_expr_to_meta_expr(body)?;
                Ok(MetaExpr::Closure {
                    params: meta_params,
                    body: Heap::new(meta_body),
                    return_type: return_type.clone(),
                })
            }
            // Priority 1: For loop
            ExprKind::For { pattern, iter, body, .. } => {
                let pattern_meta = ast_pattern_to_meta_pattern(pattern)?;
                let iter_meta = self.ast_expr_to_meta_expr(iter)?;
                let body_stmts = body
                    .stmts
                    .iter()
                    .map(|stmt| self.ast_stmt_to_meta_stmt(stmt))
                    .collect::<Result<List<_>, _>>()?;
                Ok(MetaExpr::For {
                    pattern: pattern_meta,
                    iter: Heap::new(iter_meta),
                    body: body_stmts,
                })
            }
            // Priority 1: While loop
            ExprKind::While { condition, body, .. } => {
                let cond_meta = self.ast_expr_to_meta_expr(condition)?;
                let body_stmts = body
                    .stmts
                    .iter()
                    .map(|stmt| self.ast_stmt_to_meta_stmt(stmt))
                    .collect::<Result<List<_>, _>>()?;
                Ok(MetaExpr::While {
                    condition: Heap::new(cond_meta),
                    body: body_stmts,
                })
            }
            // Priority 2: TupleIndex
            ExprKind::TupleIndex { expr: inner, index } => {
                let inner_meta = self.ast_expr_to_meta_expr(inner)?;
                Ok(MetaExpr::TupleIndex {
                    expr: Heap::new(inner_meta),
                    index: *index,
                })
            }
            // Priority 2: Record
            ExprKind::Record { path, fields, base } => {
                let name = path
                    .as_ident()
                    .map(|i| Text::from(i.as_str()))
                    .unwrap_or_else(|| Text::from(""));
                let meta_fields = fields
                    .iter()
                    .map(|f| {
                        // Handle shorthand syntax: { x } means { x: x }
                        let value = match &f.value {
                            Maybe::Some(val_expr) => self.ast_expr_to_meta_expr(val_expr)?,
                            Maybe::None => {
                                // Shorthand - use field name as variable reference
                                MetaExpr::Variable(Text::from(f.name.as_str()))
                            }
                        };
                        Ok((Text::from(f.name.as_str()), value))
                    })
                    .collect::<Result<List<_>, _>>()?;
                let meta_base = if let Maybe::Some(b) = base {
                    Maybe::Some(Heap::new(self.ast_expr_to_meta_expr(b)?))
                } else {
                    Maybe::None
                };
                Ok(MetaExpr::Record {
                    name,
                    fields: meta_fields,
                    base: meta_base,
                })
            }
            // Priority 2: Return
            ExprKind::Return(value) => {
                let meta_value = if let Maybe::Some(v) = value {
                    Maybe::Some(Heap::new(self.ast_expr_to_meta_expr(v)?))
                } else {
                    Maybe::None
                };
                Ok(MetaExpr::Return(meta_value))
            }
            // Priority 2: Break
            ExprKind::Break { label, value } => {
                let meta_value = if let Maybe::Some(v) = value {
                    Maybe::Some(Heap::new(self.ast_expr_to_meta_expr(v)?))
                } else {
                    Maybe::None
                };
                Ok(MetaExpr::Break {
                    label: label.clone(),
                    value: meta_value,
                })
            }
            // Priority 2: Continue
            ExprKind::Continue { label } => {
                Ok(MetaExpr::Continue { label: label.clone() })
            }
            // Priority 2: Cast
            ExprKind::Cast { expr: inner, ty } => {
                let inner_meta = self.ast_expr_to_meta_expr(inner)?;
                Ok(MetaExpr::Cast {
                    expr: Heap::new(inner_meta),
                    ty: ty.clone(),
                })
            }
            // Assignment: name = value or name[idx] = value
            ExprKind::DestructuringAssign { pattern, op, value } => {
                use verum_ast::expr::BinOp;
                use verum_ast::pattern::PatternKind;
                let val_meta = self.ast_expr_to_meta_expr(value)?;

                if matches!(op, BinOp::Assign) {
                    match &pattern.kind {
                        // Simple variable assignment: ident = expr
                        PatternKind::Ident { name, .. } => {
                            return Ok(MetaExpr::Assign {
                                target: Text::from(name.as_str()),
                                value: Heap::new(val_meta),
                            });
                        }
                        _ => {}
                    }
                }
                // Fall through to quote for complex patterns or compound ops
                Ok(MetaExpr::Quote(expr.clone()))
            }

            // Priority 3: Lift - M401 error when used outside quote
            ExprKind::Lift { expr: _ } => {
                // lift() can only be used inside a quote expression.
                // When we encounter it at the meta-expression level (outside quote),
                // we emit M401: UnquoteOutsideQuote error.
                #[cfg(debug_assertions)]
                {
                    // eprintln!("[DEBUG] M401: lift() used outside of quote expression");
                }
                Err(MetaError::UnquoteOutsideQuote)
            }
            // Priority 4: StageEscape - M401 error when used outside quote
            ExprKind::StageEscape { stage: _, expr: _ } => {
                // $(stage N){expr} can only be used inside a quote expression.
                #[cfg(debug_assertions)]
                {
                    // eprintln!("[DEBUG] M401: stage escape used outside of quote expression");
                }
                Err(MetaError::UnquoteOutsideQuote)
            }
            _ => {
                // For unsupported expressions, return them as quoted AST
                #[cfg(debug_assertions)]
                {
                    // eprintln!("[DEBUG] ast_expr_to_meta_expr fallback: expr.kind = {:?}", std::mem::discriminant(&expr.kind));
                }
                Ok(MetaExpr::Quote(expr.clone()))
            }
        }
    }

    /// Evaluate a meta expression
    pub fn eval_meta_expr(&mut self, expr: &MetaExpr) -> Result<ConstValue, MetaError> {
        match expr {
            MetaExpr::Literal(val) => Ok(val.clone()),

            MetaExpr::Variable(name) => self
                .get(name)
                .ok_or_else(|| MetaError::MetaEvaluationFailed {
                    message: format!("Undefined variable: {}", name).into(),
                }),

            MetaExpr::Call(func_name, args) => {
                // Check for forbidden operations FIRST (before evaluating args)
                if self.is_forbidden_function(func_name) {
                    let category = self.get_forbidden_category(func_name)
                        .unwrap_or("I/O");
                    return Err(MetaError::ForbiddenOperation {
                        operation: func_name.clone(),
                        reason: Text::from(format!(
                            "{} operations are forbidden in meta functions",
                            category
                        )),
                    });
                }

                // Evaluate arguments
                let arg_values = args
                    .iter()
                    .map(|arg| self.eval_meta_expr(arg))
                    .collect::<Result<List<_>, _>>()?;

                // First, try to look up a built-in function
                match self.get_builtin(func_name) {
                    Ok(builtin_info) => {
                        // Call the builtin function
                        (builtin_info.function)(self, arg_values)
                    }
                    Err(MetaError::MetaFunctionNotFound(_)) => {
                        // Builtin not found - try user-defined meta function
                        if let Some(ref registry) = self.registry {
                            if let Maybe::Some(user_fn) = registry.get_user_meta_fn(&self.current_module, func_name) {
                                // Convert List to Vec for execute_user_meta_fn
                                let args_vec: Vec<ConstValue> = arg_values.iter().cloned().collect();
                                return self.execute_user_meta_fn(&user_fn, args_vec);
                            }

                            // Check if this is an extern (FFI) function - forbidden in meta context
                            if registry.is_any_extern_function(func_name) {
                                return Err(MetaError::ForbiddenOperation {
                                    operation: func_name.clone(),
                                    reason: Text::from("FFI/extern functions cannot be called in meta context"),
                                });
                            }
                        }
                        // No builtin and no user function found
                        Err(MetaError::MetaFunctionNotFound(func_name.clone()))
                    }
                    Err(other_error) => {
                        // Other errors (like MissingContext) should propagate
                        Err(other_error)
                    }
                }
            }

            MetaExpr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond_value = self.eval_meta_expr(condition)?;

                if cond_value.as_bool() {
                    self.eval_meta_expr(then_branch)
                } else if let Maybe::Some(else_expr) = else_branch {
                    self.eval_meta_expr(else_expr)
                } else {
                    Ok(ConstValue::Unit)
                }
            }

            MetaExpr::Match { scrutinee, arms } => {
                let scrutinee_value = self.eval_meta_expr(scrutinee)?;

                for arm in arms {
                    if self.matches_pattern(&scrutinee_value, &arm.pattern)? {
                        // Check guard if present
                        if let Maybe::Some(guard) = &arm.guard {
                            let guard_val = self.eval_meta_expr(guard)?;
                            if !guard_val.as_bool() {
                                continue;
                            }
                        }
                        return self.eval_meta_expr(&arm.body);
                    }
                }

                Err(MetaError::NoMatchingArm {
                    value: format!("{:?}", scrutinee_value).into(),
                })
            }

            MetaExpr::Let { name, value, body } => {
                let value_result = self.eval_meta_expr(value)?;
                let saved = self.get(name).clone();
                self.bind(name.clone(), value_result);
                let result = self.eval_meta_expr(body);
                // Restore
                if let Some(old_val) = saved {
                    self.bind(name.clone(), old_val);
                } else {
                    self.unbind(name);
                }
                result
            }

            MetaExpr::Block(stmts) => {
                let mut last = ConstValue::Unit;
                for stmt in stmts {
                    last = self.eval_meta_stmt(stmt)?;
                }
                Ok(last)
            }

            MetaExpr::Quote(expr) => {
                // Perform hygiene checking on the quote expression
                #[cfg(debug_assertions)]
                {
                    // eprintln!("[DEBUG] MetaExpr::Quote evaluation - expr.kind = {:?}", std::mem::discriminant(&expr.kind));
                }
                self.check_quote_hygiene(expr)?;

                // Expand splices in the quote expression
                // This substitutes $var and ${expr} with their evaluated values
                let expanded_expr = self.expand_quote_splices(expr)?;
                Ok(ConstValue::Expr(expanded_expr))
            }

            MetaExpr::Unquote(expr) => {
                let value = self.eval_meta_expr(expr)?;
                match value {
                    ConstValue::Expr(ast) => Ok(ConstValue::Expr(ast)),
                    _ => Err(MetaError::UnquoteOutsideQuote),
                }
            }

            MetaExpr::TypeOf(expr) => {
                // Infer type from the AST expression structure
                let ty = self.infer_type_from_expr(expr);
                Ok(ConstValue::Type(ty))
            }

            MetaExpr::Binary { op, left, right } => {
                let left_val = self.eval_meta_expr(left)?;
                let right_val = self.eval_meta_expr(right)?;
                self.eval_binary_op(*op, left_val, right_val)
            }

            MetaExpr::Unary { op, expr: inner } => {
                let val = self.eval_meta_expr(inner)?;
                self.eval_unary_op(*op, val)
            }

            MetaExpr::ListComp { expr, var, iter, filter } => {
                let iter_val = self.eval_meta_expr(iter)?;
                let elements = match iter_val {
                    ConstValue::Array(arr) => arr,
                    ConstValue::Tuple(tup) => tup,
                    _ => return Err(MetaError::Other(Text::from("Expected iterable"))),
                };

                let mut result = List::new();
                for elem in elements {
                    self.bind(var.clone(), elem.clone());

                    // Check filter if present
                    if let Maybe::Some(filter_expr) = filter {
                        let filter_val = self.eval_meta_expr(filter_expr)?;
                        if !filter_val.as_bool() {
                            continue;
                        }
                    }

                    let mapped = self.eval_meta_expr(expr)?;
                    result.push(mapped);
                }

                Ok(ConstValue::Array(result))
            }

            // === New expression kinds (Phase 6) ===

            MetaExpr::MethodCall { receiver, method, args } => {
                let receiver_val = self.eval_meta_expr(receiver)?;
                let arg_values = args
                    .iter()
                    .map(|arg| self.eval_meta_expr(arg))
                    .collect::<Result<List<_>, _>>()?;

                // Dispatch method call based on receiver type
                self.eval_method_call(receiver_val, method, arg_values)
            }

            MetaExpr::FieldAccess { expr: inner, field } => {
                let val = self.eval_meta_expr(inner)?;
                self.eval_field_access(val, field)
            }

            MetaExpr::Index { expr: inner, index } => {
                let val = self.eval_meta_expr(inner)?;
                let idx = self.eval_meta_expr(index)?;
                self.eval_index_access(val, idx)
            }

            MetaExpr::Closure { params, .. } => {
                // Closures become ConstValue::Closure which can be called later
                // For now, we represent closures as quoted expressions
                // A full implementation would store the closure environment
                Ok(ConstValue::Expr(Expr::new(
                    ExprKind::Closure {
                        params: params.iter().map(|p| {
                            verum_ast::ClosureParam {
                                pattern: Pattern::ident(
                                    Ident::new(p.as_str(), Span::dummy()),
                                    false,
                                    Span::dummy(),
                                ),
                                ty: Maybe::None,
                                span: Span::dummy(),
                            }
                        }).collect(),
                        body: Heap::new(Expr::literal(verum_ast::Literal::int(0, Span::dummy()))),
                        return_type: Maybe::None,
                        move_: false,
                        async_: false,
                        contexts: List::new(),
                    },
                    Span::dummy(),
                )))
            }

            MetaExpr::For { pattern, iter, body } => {
                let iter_val = self.eval_meta_expr(iter)?;
                let elements = match iter_val {
                    ConstValue::Array(arr) => arr,
                    ConstValue::Tuple(tup) => tup,
                    _ => return Err(MetaError::Other(Text::from("Expected iterable in for loop"))),
                };

                let mut last = ConstValue::Unit;
                for elem in elements {
                    // Bind the loop variable
                    if !self.matches_pattern(&elem, pattern)? {
                        continue;
                    }
                    // Evaluate body
                    for stmt in body {
                        last = self.eval_meta_stmt(stmt)?;
                    }
                }
                Ok(last)
            }

            MetaExpr::While { condition, body } => {
                let mut last = ConstValue::Unit;
                let mut iterations = 0;
                const MAX_ITERATIONS: usize = 10_000;

                loop {
                    let cond_val = self.eval_meta_expr(condition)?;
                    if !cond_val.as_bool() {
                        break;
                    }

                    iterations += 1;
                    if iterations > MAX_ITERATIONS {
                        return Err(MetaError::IterationLimitExceeded {
                            count: iterations,
                            limit: MAX_ITERATIONS,
                        });
                    }

                    for stmt in body {
                        last = self.eval_meta_stmt(stmt)?;
                    }
                }
                Ok(last)
            }

            MetaExpr::TupleIndex { expr: inner, index } => {
                let val = self.eval_meta_expr(inner)?;
                match val {
                    ConstValue::Tuple(elems) => {
                        let idx = *index as usize;
                        if idx < elems.len() {
                            Ok(elems[idx].clone())
                        } else {
                            Err(MetaError::Other(Text::from(format!(
                                "Tuple index {} out of bounds (tuple has {} elements)",
                                idx, elems.len()
                            ))))
                        }
                    }
                    _ => Err(MetaError::TypeMismatch {
                        expected: Text::from("Tuple"),
                        found: val.type_name(),
                    }),
                }
            }

            MetaExpr::Record { name: _, fields, base } => {
                // Evaluate all field values
                let mut field_values = List::new();
                for (field_name, field_expr) in fields {
                    let val = self.eval_meta_expr(field_expr)?;
                    field_values.push((field_name.clone(), val));
                }

                // If there's a base, merge with it
                if let Maybe::Some(base_expr) = base {
                    let _base_val = self.eval_meta_expr(base_expr)?;
                    // For now, just use the specified fields
                    // A full implementation would merge with base
                }

                // Records become tuples of field values in meta evaluation
                // A full implementation would preserve record structure
                let tuple_vals: List<ConstValue> = field_values.iter().map(|(_, v)| v.clone()).collect();
                Ok(ConstValue::Tuple(tuple_vals))
            }

            MetaExpr::Return(value) => {
                if let Maybe::Some(val_expr) = value {
                    self.eval_meta_expr(val_expr)
                } else {
                    Ok(ConstValue::Unit)
                }
            }

            MetaExpr::Break { value, .. } => {
                // Break in meta context just evaluates the value
                // A full implementation would handle loop labels
                if let Maybe::Some(val_expr) = value {
                    self.eval_meta_expr(val_expr)
                } else {
                    Ok(ConstValue::Unit)
                }
            }

            MetaExpr::Continue { .. } => {
                // Continue in meta context is a no-op
                // A full implementation would handle loop labels
                Ok(ConstValue::Unit)
            }

            MetaExpr::Cast { expr: inner, ty } => {
                let val = self.eval_meta_expr(inner)?;
                self.eval_cast(val, ty)
            }

            MetaExpr::Assign { target, value } => {
                // Evaluate the value and update the binding (mutable assignment)
                let val = self.eval_meta_expr(value)?;
                self.bind(target.clone(), val.clone());
                Ok(ConstValue::Unit)
            }

            MetaExpr::AssignIndex { target, index, value } => {
                // Evaluate index and value, update the array element
                let idx_val = self.eval_meta_expr(index)?;
                let new_val = self.eval_meta_expr(value)?;
                let idx = match &idx_val {
                    ConstValue::Int(n) => *n as usize,
                    ConstValue::UInt(n) => *n as usize,
                    _ => return Err(MetaError::TypeMismatch {
                        expected: Text::from("integer"),
                        found: idx_val.type_name(),
                    }),
                };
                // Get the existing array, update it, and re-bind
                if let Some(arr_val) = self.get(target) {
                    if let ConstValue::Array(mut arr) = arr_val {
                        if idx < arr.len() {
                            arr[idx] = new_val;
                            self.bind(target.clone(), ConstValue::Array(arr));
                            Ok(ConstValue::Unit)
                        } else {
                            Err(MetaError::IndexOutOfBounds { index: idx as i128, length: arr.len() })
                        }
                    } else {
                        Err(MetaError::TypeMismatch {
                            expected: Text::from("array"),
                            found: arr_val.type_name(),
                        })
                    }
                } else {
                    Err(MetaError::MetaEvaluationFailed {
                        message: format!("Undefined variable in index assignment: {}", target).into(),
                    })
                }
            }
        }
    }

    /// Evaluate a method call on a value
    fn eval_method_call(
        &mut self,
        receiver: ConstValue,
        method: &Text,
        args: List<ConstValue>,
    ) -> Result<ConstValue, MetaError> {
        match (&receiver, method.as_str()) {
            // Array/List methods
            (ConstValue::Array(arr), "len") => {
                Ok(ConstValue::UInt(arr.len() as u128))
            }
            (ConstValue::Array(arr), "is_empty") => {
                Ok(ConstValue::Bool(arr.is_empty()))
            }
            (ConstValue::Array(arr), "first") => {
                match arr.first() {
                    Some(v) => Ok(ConstValue::Maybe(Maybe::Some(Heap::new(v.clone())))),
                    None => Ok(ConstValue::Maybe(Maybe::None)),
                }
            }
            (ConstValue::Array(arr), "last") => {
                match arr.last() {
                    Some(v) => Ok(ConstValue::Maybe(Maybe::Some(Heap::new(v.clone())))),
                    None => Ok(ConstValue::Maybe(Maybe::None)),
                }
            }
            (ConstValue::Array(arr), "get") => {
                if args.len() != 1 {
                    return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
                }
                let idx = match &args[0] {
                    ConstValue::Int(i) => *i as usize,
                    ConstValue::UInt(u) => *u as usize,
                    _ => return Err(MetaError::TypeMismatch {
                        expected: Text::from("Int or UInt"),
                        found: args[0].type_name(),
                    }),
                };
                match arr.get(idx) {
                    Some(v) => Ok(ConstValue::Maybe(Maybe::Some(Heap::new(v.clone())))),
                    None => Ok(ConstValue::Maybe(Maybe::None)),
                }
            }

            // Text/String methods
            (ConstValue::Text(s), "len") => {
                Ok(ConstValue::UInt(s.len() as u128))
            }
            (ConstValue::Text(s), "is_empty") => {
                Ok(ConstValue::Bool(s.is_empty()))
            }
            (ConstValue::Text(s), "to_uppercase") => {
                Ok(ConstValue::Text(Text::from(s.to_uppercase())))
            }
            (ConstValue::Text(s), "to_lowercase") => {
                Ok(ConstValue::Text(Text::from(s.to_lowercase())))
            }
            (ConstValue::Text(s), "trim") => {
                Ok(ConstValue::Text(Text::from(s.trim())))
            }
            (ConstValue::Text(s), "contains") => {
                if args.len() != 1 {
                    return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
                }
                match &args[0] {
                    ConstValue::Text(needle) => Ok(ConstValue::Bool(s.contains(needle.as_str()))),
                    _ => Err(MetaError::TypeMismatch {
                        expected: Text::from("Text"),
                        found: args[0].type_name(),
                    }),
                }
            }
            (ConstValue::Text(s), "starts_with") => {
                if args.len() != 1 {
                    return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
                }
                match &args[0] {
                    ConstValue::Text(prefix) => Ok(ConstValue::Bool(s.starts_with(prefix.as_str()))),
                    _ => Err(MetaError::TypeMismatch {
                        expected: Text::from("Text"),
                        found: args[0].type_name(),
                    }),
                }
            }
            (ConstValue::Text(s), "ends_with") => {
                if args.len() != 1 {
                    return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
                }
                match &args[0] {
                    ConstValue::Text(suffix) => Ok(ConstValue::Bool(s.ends_with(suffix.as_str()))),
                    _ => Err(MetaError::TypeMismatch {
                        expected: Text::from("Text"),
                        found: args[0].type_name(),
                    }),
                }
            }

            // Tuple methods
            (ConstValue::Tuple(elems), "len") => {
                Ok(ConstValue::UInt(elems.len() as u128))
            }

            _ => Err(MetaError::Other(Text::from(format!(
                "Unknown method '{}' for type {}",
                method, receiver.type_name()
            )))),
        }
    }

    /// Evaluate field access on a value
    fn eval_field_access(
        &self,
        value: ConstValue,
        field: &Text,
    ) -> Result<ConstValue, MetaError> {
        // In meta context, we don't have actual struct definitions,
        // so field access is limited
        Err(MetaError::Other(Text::from(format!(
            "Field access '.{}' not supported on {} in meta context",
            field, value.type_name()
        ))))
    }

    /// Evaluate index access
    fn eval_index_access(
        &self,
        value: ConstValue,
        index: ConstValue,
    ) -> Result<ConstValue, MetaError> {
        match (&value, &index) {
            (ConstValue::Array(arr), ConstValue::Int(i)) => {
                let idx = *i as usize;
                arr.get(idx).cloned().ok_or_else(|| {
                    MetaError::Other(Text::from(format!(
                        "Array index {} out of bounds (length {})",
                        i, arr.len()
                    )))
                })
            }
            (ConstValue::Array(arr), ConstValue::UInt(u)) => {
                let idx = *u as usize;
                arr.get(idx).cloned().ok_or_else(|| {
                    MetaError::Other(Text::from(format!(
                        "Array index {} out of bounds (length {})",
                        u, arr.len()
                    )))
                })
            }
            (ConstValue::Tuple(elems), ConstValue::Int(i)) => {
                let idx = *i as usize;
                elems.get(idx).cloned().ok_or_else(|| {
                    MetaError::Other(Text::from(format!(
                        "Tuple index {} out of bounds (length {})",
                        i, elems.len()
                    )))
                })
            }
            (ConstValue::Tuple(elems), ConstValue::UInt(u)) => {
                let idx = *u as usize;
                elems.get(idx).cloned().ok_or_else(|| {
                    MetaError::Other(Text::from(format!(
                        "Tuple index {} out of bounds (length {})",
                        u, elems.len()
                    )))
                })
            }
            (ConstValue::Text(s), ConstValue::Int(i)) => {
                let idx = *i as usize;
                s.chars().nth(idx)
                    .map(|c| ConstValue::Char(c))
                    .ok_or_else(|| {
                        MetaError::Other(Text::from(format!(
                            "String index {} out of bounds (length {})",
                            i, s.len()
                        )))
                    })
            }
            (ConstValue::Text(s), ConstValue::UInt(u)) => {
                let idx = *u as usize;
                s.chars().nth(idx)
                    .map(|c| ConstValue::Char(c))
                    .ok_or_else(|| {
                        MetaError::Other(Text::from(format!(
                            "String index {} out of bounds (length {})",
                            u, s.len()
                        )))
                    })
            }
            _ => Err(MetaError::TypeMismatch {
                expected: Text::from("Array, Tuple, or Text with Int/UInt index"),
                found: Text::from(format!("{}[{}]", value.type_name(), index.type_name())),
            }),
        }
    }

    /// Evaluate a type cast
    fn eval_cast(&self, value: ConstValue, ty: &Type) -> Result<ConstValue, MetaError> {
        use verum_ast::ty::PathSegment;

        // Simple type casts in meta context
        match &ty.kind {
            TypeKind::Path(path) => {
                let type_name = path.segments.last()
                    .and_then(|s| {
                        match s {
                            PathSegment::Name(ident) => Some(ident.as_str()),
                            _ => None,
                        }
                    })
                    .unwrap_or("");

                match type_name {
                    "Int" | "i64" | "i32" | "i16" | "i8" => {
                        match value {
                            ConstValue::Int(i) => Ok(ConstValue::Int(i)),
                            ConstValue::UInt(u) => Ok(ConstValue::Int(u as i128)),
                            ConstValue::Float(f) => Ok(ConstValue::Int(f as i128)),
                            ConstValue::Bool(b) => Ok(ConstValue::Int(if b { 1 } else { 0 })),
                            _ => Err(MetaError::Other(Text::from(format!(
                                "Cannot cast {} to Int", value.type_name()
                            )))),
                        }
                    }
                    "UInt" | "u64" | "u32" | "u16" | "u8" => {
                        match value {
                            ConstValue::Int(i) => Ok(ConstValue::UInt(i as u128)),
                            ConstValue::UInt(u) => Ok(ConstValue::UInt(u)),
                            ConstValue::Float(f) => Ok(ConstValue::UInt(f as u128)),
                            ConstValue::Bool(b) => Ok(ConstValue::UInt(if b { 1 } else { 0 })),
                            _ => Err(MetaError::Other(Text::from(format!(
                                "Cannot cast {} to UInt", value.type_name()
                            )))),
                        }
                    }
                    "Float" | "f64" | "f32" => {
                        match value {
                            ConstValue::Int(i) => Ok(ConstValue::Float(i as f64)),
                            ConstValue::UInt(u) => Ok(ConstValue::Float(u as f64)),
                            ConstValue::Float(f) => Ok(ConstValue::Float(f)),
                            _ => Err(MetaError::Other(Text::from(format!(
                                "Cannot cast {} to Float", value.type_name()
                            )))),
                        }
                    }
                    "Text" | "String" => {
                        Ok(ConstValue::Text(Text::from(format!("{}", value))))
                    }
                    "Bool" => {
                        Ok(ConstValue::Bool(value.as_bool()))
                    }
                    _ => {
                        // Unknown type - return value unchanged
                        Ok(value)
                    }
                }
            }
            _ => {
                // Complex type cast - return value unchanged
                Ok(value)
            }
        }
    }

    /// Evaluate a meta statement
    pub(crate) fn eval_meta_stmt(&mut self, stmt: &MetaStmt) -> Result<ConstValue, MetaError> {
        match stmt {
            MetaStmt::Expr(expr) => self.eval_meta_expr(expr),
            MetaStmt::Let { name, value } => {
                let val = self.eval_meta_expr(value)?;
                self.bind(name.clone(), val);
                Ok(ConstValue::Unit)
            }
            MetaStmt::LetTuple { names, value } => {
                let val = self.eval_meta_expr(value)?;
                // Destructure tuple value into individual bindings
                match val {
                    ConstValue::Tuple(elements) => {
                        if elements.len() != names.len() {
                            return Err(MetaError::TypeMismatch {
                                expected: Text::from(format!("tuple with {} elements", names.len())),
                                found: Text::from(format!("tuple with {} elements", elements.len())),
                            });
                        }
                        for (i, name) in names.iter().enumerate() {
                            if let Some(n) = name {
                                self.bind(n.clone(), elements[i].clone());
                            }
                            // Wildcard (None) - skip binding
                        }
                        Ok(ConstValue::Unit)
                    }
                    _ => Err(MetaError::TypeMismatch {
                        expected: Text::from("tuple"),
                        found: val.type_name(),
                    }),
                }
            }
            MetaStmt::Return(e) => {
                if let Maybe::Some(expr) = e {
                    self.eval_meta_expr(expr)
                } else {
                    Ok(ConstValue::Unit)
                }
            }
        }
    }

    /// Evaluate a type property (T.size, T.alignment, etc.)
    ///
    /// # Example
    /// ```ignore
    /// let size = ctx.eval_type_property(&ty, TypeProperty::Size)?;
    /// ```
    pub fn eval_type_property(
        &self,
        ty: &Type,
        property: TypeProperty,
    ) -> Result<ConstValue, MetaError> {
        use super::builtins::type_props::{
            compute_type_alignment, compute_type_id, compute_type_max, compute_type_min,
            compute_type_name, compute_type_size, compute_type_stride,
        };

        match property {
            TypeProperty::Size => {
                let size = compute_type_size(&ty.kind)?;
                Ok(ConstValue::Int(size.into()))
            }
            TypeProperty::Alignment => {
                let align = compute_type_alignment(&ty.kind)?;
                Ok(ConstValue::Int(align.into()))
            }
            TypeProperty::Stride => {
                let stride = compute_type_stride(&ty.kind)?;
                Ok(ConstValue::Int(stride.into()))
            }
            TypeProperty::Bits => {
                let size = compute_type_size(&ty.kind)?;
                Ok(ConstValue::Int((size * 8).into()))
            }
            TypeProperty::Min => compute_type_min(&ty.kind),
            TypeProperty::Max => compute_type_max(&ty.kind),
            TypeProperty::Name => {
                let name = compute_type_name(&ty.kind);
                Ok(ConstValue::Text(name))
            }
            TypeProperty::Id => {
                let id = compute_type_id(&ty.kind);
                Ok(ConstValue::UInt(id.into()))
            }
        }
    }

    /// Check if a value matches a pattern
    ///
    /// This is a comprehensive pattern matcher supporting all MetaPattern variants.
    /// Bindings are added to the context as patterns match.
    pub fn matches_pattern(
        &mut self,
        value: &ConstValue,
        pattern: &MetaPattern,
    ) -> Result<bool, MetaError> {
        match pattern {
            MetaPattern::Wildcard => Ok(true),

            MetaPattern::Literal(lit) => Ok(value == lit),

            MetaPattern::Ident(name) => {
                self.bind(name.clone(), value.clone());
                Ok(true)
            }

            MetaPattern::IdentAt { name, subpattern } => {
                // Bind the name first, then match the subpattern
                self.bind(name.clone(), value.clone());
                self.matches_pattern(value, subpattern)
            }

            MetaPattern::Tuple(patterns) => {
                if let ConstValue::Tuple(values) = value {
                    if values.len() != patterns.len() {
                        return Ok(false);
                    }
                    for (v, p) in values.iter().zip(patterns.iter()) {
                        if !self.matches_pattern(v, p)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                } else {
                    Ok(false)
                }
            }

            MetaPattern::Array(patterns) => {
                if let ConstValue::Array(values) = value {
                    if values.len() != patterns.len() {
                        return Ok(false);
                    }
                    for (v, p) in values.iter().zip(patterns.iter()) {
                        if !self.matches_pattern(v, p)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                } else {
                    Ok(false)
                }
            }

            MetaPattern::Slice { before, rest, after } => {
                if let ConstValue::Array(values) = value {
                    let total_fixed = before.len() + after.len();
                    if values.len() < total_fixed {
                        return Ok(false);
                    }

                    // Match patterns before the rest
                    for (i, p) in before.iter().enumerate() {
                        if let Some(v) = values.get(i) {
                            if !self.matches_pattern(v, p)? {
                                return Ok(false);
                            }
                        } else {
                            return Ok(false);
                        }
                    }

                    // Match patterns after the rest (from the end)
                    let after_start = values.len() - after.len();
                    for (i, p) in after.iter().enumerate() {
                        if let Some(v) = values.get(after_start + i) {
                            if !self.matches_pattern(v, p)? {
                                return Ok(false);
                            }
                        } else {
                            return Ok(false);
                        }
                    }

                    // Bind rest if named
                    if let Maybe::Some(rest_name) = rest {
                        let rest_values: List<ConstValue> = values
                            .iter()
                            .skip(before.len())
                            .take(after_start - before.len())
                            .cloned()
                            .collect();
                        self.bind(rest_name.clone(), ConstValue::Array(rest_values));
                    }

                    Ok(true)
                } else {
                    Ok(false)
                }
            }

            MetaPattern::Record { name, fields, rest } => {
                // For now, records are represented as tuples or as special values
                // Check if value is a tuple with matching field count
                if let ConstValue::Tuple(values) = value {
                    if !*rest && values.len() != fields.len() {
                        return Ok(false);
                    }
                    // Match fields by position (assuming ordered fields)
                    for (i, (_, p)) in fields.iter().enumerate() {
                        if i >= values.len() {
                            return Ok(false);
                        }
                        if !self.matches_pattern(&values[i], p)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                } else {
                    // Check if it's a named record type
                    // For now, we don't have first-class record support in ConstValue
                    // so we treat the name as informational
                    let _ = name;
                    Ok(false)
                }
            }

            MetaPattern::Variant { name, data } => {
                // Handle common variant types: Maybe (Some/None), Result (Ok/Err)
                match (name.as_str(), value) {
                    (variant_tags::SOME, ConstValue::Maybe(Maybe::Some(inner))) => {
                        if let Maybe::Some(pat) = data {
                            self.matches_pattern(inner, pat)
                        } else {
                            Ok(true)
                        }
                    }
                    (variant_tags::NONE, ConstValue::Maybe(Maybe::None)) => {
                        Ok(data.is_none())
                    }
                    ("true", ConstValue::Bool(true)) | ("True", ConstValue::Bool(true)) => {
                        Ok(data.is_none())
                    }
                    ("false", ConstValue::Bool(false)) | ("False", ConstValue::Bool(false)) => {
                        Ok(data.is_none())
                    }
                    // For other variants, check if value type matches
                    _ => {
                        // Could be a user-defined variant - for now, return false
                        // A full implementation would check the type registry
                        Ok(false)
                    }
                }
            }

            MetaPattern::Range { start, end, inclusive } => {
                // Range patterns work with Int, UInt, Char
                match value {
                    ConstValue::Int(n) => {
                        let in_start = match start {
                            Maybe::Some(ConstValue::Int(s)) => *n >= *s,
                            Maybe::None => true,
                            _ => false,
                        };
                        let in_end = match end {
                            Maybe::Some(ConstValue::Int(e)) => {
                                if *inclusive { *n <= *e } else { *n < *e }
                            }
                            Maybe::None => true,
                            _ => false,
                        };
                        Ok(in_start && in_end)
                    }
                    ConstValue::UInt(n) => {
                        let in_start = match start {
                            Maybe::Some(ConstValue::UInt(s)) => *n >= *s,
                            Maybe::Some(ConstValue::Int(s)) if *s >= 0 => *n >= (*s as u128),
                            Maybe::None => true,
                            _ => false,
                        };
                        let in_end = match end {
                            Maybe::Some(ConstValue::UInt(e)) => {
                                if *inclusive { *n <= *e } else { *n < *e }
                            }
                            Maybe::Some(ConstValue::Int(e)) if *e >= 0 => {
                                if *inclusive { *n <= (*e as u128) } else { *n < (*e as u128) }
                            }
                            Maybe::None => true,
                            _ => false,
                        };
                        Ok(in_start && in_end)
                    }
                    ConstValue::Char(c) => {
                        let in_start = match start {
                            Maybe::Some(ConstValue::Char(s)) => *c >= *s,
                            Maybe::None => true,
                            _ => false,
                        };
                        let in_end = match end {
                            Maybe::Some(ConstValue::Char(e)) => {
                                if *inclusive { *c <= *e } else { *c < *e }
                            }
                            Maybe::None => true,
                            _ => false,
                        };
                        Ok(in_start && in_end)
                    }
                    _ => Ok(false),
                }
            }

            MetaPattern::Rest(binding) => {
                // Rest pattern always matches, optionally binding the value
                if let Maybe::Some(name) = binding {
                    self.bind(name.clone(), value.clone());
                }
                Ok(true)
            }

            MetaPattern::Or(patterns) => {
                // Save bindings before trying each branch
                for p in patterns {
                    // Clone the current bindings to allow rollback
                    let saved = self.bindings.clone();
                    if self.matches_pattern(value, p)? {
                        return Ok(true);
                    }
                    // Rollback bindings if this branch didn't match
                    self.bindings = saved;
                }
                Ok(false)
            }

            MetaPattern::And(patterns) => {
                // All patterns must match
                for p in patterns {
                    if !self.matches_pattern(value, p)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }

            MetaPattern::Reference { inner, .. } => {
                // In meta context, we don't have real references
                // Just match the inner pattern against the value
                self.matches_pattern(value, inner)
            }

            MetaPattern::TypeTest { name, type_name } => {
                // Check if the value's type matches
                let value_type = value.type_name();
                if value_type.as_str() == type_name.as_str() {
                    self.bind(name.clone(), value.clone());
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    /// Evaluate a binary operation
    fn eval_binary_op(
        &self,
        op: BinOp,
        left: ConstValue,
        right: ConstValue,
    ) -> Result<ConstValue, MetaError> {
        use super::value_ops::MetaValueOps;

        // Helper to convert SandboxError to MetaError with proper categorization
        let map_err = |e: super::sandbox::SandboxError| {
            Self::sandbox_error_to_meta_error(e)
        };

        match op {
            BinOp::Add => left.add(right).map_err(map_err),
            BinOp::Sub => left.sub(right).map_err(map_err),
            BinOp::Mul => left.mul(right).map_err(map_err),
            BinOp::Div => left.div(right).map_err(map_err),
            BinOp::Rem => left.modulo(right).map_err(map_err),
            BinOp::Eq => Ok(ConstValue::Bool(left == right)),
            BinOp::Ne => Ok(ConstValue::Bool(left != right)),
            BinOp::Lt => left.lt(right).map_err(map_err),
            BinOp::Le => left.le(right).map_err(map_err),
            BinOp::Gt => left.gt(right).map_err(map_err),
            BinOp::Ge => left.ge(right).map_err(map_err),
            BinOp::And => Ok(ConstValue::Bool(left.as_bool() && right.as_bool())),
            BinOp::Or => Ok(ConstValue::Bool(left.as_bool() || right.as_bool())),
            BinOp::BitAnd => self.eval_bitwise_and(left, right),
            BinOp::BitOr => self.eval_bitwise_or(left, right),
            BinOp::BitXor => self.eval_bitwise_xor(left, right),
            BinOp::Shl => self.eval_shift_left(left, right),
            BinOp::Shr => self.eval_shift_right(left, right),
            _ => Err(MetaError::Other(Text::from(format!(
                "Unsupported binary operator: {:?}",
                op
            )))),
        }
    }

    /// Evaluate a unary operation
    fn eval_unary_op(&self, op: UnOp, val: ConstValue) -> Result<ConstValue, MetaError> {
        use super::value_ops::MetaValueOps;

        // Helper to convert SandboxError to MetaError with proper categorization
        let map_err = |e: super::sandbox::SandboxError| {
            Self::sandbox_error_to_meta_error(e)
        };

        match op {
            UnOp::Not => val.not().map_err(map_err),
            UnOp::Neg => val.neg().map_err(map_err),
            UnOp::BitNot => self.eval_bitwise_not(val),
            _ => Err(MetaError::Other(Text::from(format!(
                "Unsupported unary operator: {:?}",
                op
            )))),
        }
    }

    /// Convert SandboxError to properly categorized MetaError.
    fn sandbox_error_to_meta_error(e: super::sandbox::SandboxError) -> MetaError {
        use super::sandbox::SandboxError;
        match &e {
            SandboxError::UnsafeOperation { operation, reason } => {
                let reason_lower = reason.as_str().to_lowercase();
                if reason_lower.contains("division by zero") || reason_lower.contains("modulo by zero") {
                    MetaError::DivisionByZero
                } else if reason_lower.contains("overflow") || reason_lower.contains("shift amount") {
                    MetaError::ConstOverflow {
                        operation: operation.clone(),
                        value: reason.clone(),
                    }
                } else {
                    MetaError::Other(Text::from(format!("{}", e)))
                }
            }
            SandboxError::IterationLimitExceeded { iterations, limit } => {
                MetaError::IterationLimitExceeded { count: *iterations, limit: *limit }
            }
            SandboxError::StackOverflow { depth, limit } => {
                MetaError::RecursionLimitExceeded { depth: *depth, limit: *limit }
            }
            SandboxError::Timeout { elapsed_ms, limit_ms } => {
                MetaError::TimeoutExceeded { elapsed_ms: *elapsed_ms, limit_ms: *limit_ms }
            }
            _ => MetaError::Other(Text::from(format!("{}", e))),
        }
    }

    /// Bitwise AND operation
    fn eval_bitwise_and(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue, MetaError> {
        match (&left, &right) {
            (ConstValue::Int(a), ConstValue::Int(b)) => Ok(ConstValue::Int(a & b)),
            (ConstValue::UInt(a), ConstValue::UInt(b)) => Ok(ConstValue::UInt(a & b)),
            _ => Err(MetaError::TypeMismatch {
                expected: Text::from("Int or UInt"),
                found: Text::from(format!("({}, {})", left.type_name(), right.type_name())),
            }),
        }
    }

    /// Bitwise OR operation
    fn eval_bitwise_or(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue, MetaError> {
        match (&left, &right) {
            (ConstValue::Int(a), ConstValue::Int(b)) => Ok(ConstValue::Int(a | b)),
            (ConstValue::UInt(a), ConstValue::UInt(b)) => Ok(ConstValue::UInt(a | b)),
            _ => Err(MetaError::TypeMismatch {
                expected: Text::from("Int or UInt"),
                found: Text::from(format!("({}, {})", left.type_name(), right.type_name())),
            }),
        }
    }

    /// Bitwise XOR operation
    fn eval_bitwise_xor(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue, MetaError> {
        match (&left, &right) {
            (ConstValue::Int(a), ConstValue::Int(b)) => Ok(ConstValue::Int(a ^ b)),
            (ConstValue::UInt(a), ConstValue::UInt(b)) => Ok(ConstValue::UInt(a ^ b)),
            _ => Err(MetaError::TypeMismatch {
                expected: Text::from("Int or UInt"),
                found: Text::from(format!("({}, {})", left.type_name(), right.type_name())),
            }),
        }
    }

    /// Shift left operation
    fn eval_shift_left(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue, MetaError> {
        match (&left, &right) {
            (ConstValue::Int(a), ConstValue::Int(b)) => {
                if *b < 0 || *b > 127 {
                    return Err(MetaError::Other(Text::from("Shift amount out of range")));
                }
                Ok(ConstValue::Int(a << (*b as u8)))
            }
            (ConstValue::UInt(a), ConstValue::UInt(b)) => {
                if *b > 127 {
                    return Err(MetaError::Other(Text::from("Shift amount out of range")));
                }
                Ok(ConstValue::UInt(a << (*b as u8)))
            }
            (ConstValue::UInt(a), ConstValue::Int(b)) => {
                if *b < 0 || *b > 127 {
                    return Err(MetaError::Other(Text::from("Shift amount out of range")));
                }
                Ok(ConstValue::UInt(a << (*b as u8)))
            }
            _ => Err(MetaError::TypeMismatch {
                expected: Text::from("Int or UInt"),
                found: Text::from(format!("({}, {})", left.type_name(), right.type_name())),
            }),
        }
    }

    /// Shift right operation
    fn eval_shift_right(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue, MetaError> {
        match (&left, &right) {
            (ConstValue::Int(a), ConstValue::Int(b)) => {
                if *b < 0 || *b > 127 {
                    return Err(MetaError::Other(Text::from("Shift amount out of range")));
                }
                Ok(ConstValue::Int(a >> (*b as u8)))
            }
            (ConstValue::UInt(a), ConstValue::UInt(b)) => {
                if *b > 127 {
                    return Err(MetaError::Other(Text::from("Shift amount out of range")));
                }
                Ok(ConstValue::UInt(a >> (*b as u8)))
            }
            (ConstValue::UInt(a), ConstValue::Int(b)) => {
                if *b < 0 || *b > 127 {
                    return Err(MetaError::Other(Text::from("Shift amount out of range")));
                }
                Ok(ConstValue::UInt(a >> (*b as u8)))
            }
            _ => Err(MetaError::TypeMismatch {
                expected: Text::from("Int or UInt"),
                found: Text::from(format!("({}, {})", left.type_name(), right.type_name())),
            }),
        }
    }

    /// Bitwise NOT operation
    fn eval_bitwise_not(&self, val: ConstValue) -> Result<ConstValue, MetaError> {
        match val {
            ConstValue::Int(a) => Ok(ConstValue::Int(!a)),
            ConstValue::UInt(a) => Ok(ConstValue::UInt(!a)),
            _ => Err(MetaError::TypeMismatch {
                expected: Text::from("Int or UInt"),
                found: val.type_name(),
            }),
        }
    }

    /// Infer type from a constant value
    pub(crate) fn infer_type_from_value(&self, value: &ConstValue) -> Type {
        let span = Span::dummy();

        match value {
            ConstValue::Unit => Type::unit(span),
            ConstValue::Bool(_) => Type::bool(span),
            ConstValue::Int(_) => Type::int(span),
            ConstValue::UInt(_) => Type::int(span), // Use Int for UInt for simplicity
            ConstValue::Float(_) => Type::float(span),
            ConstValue::Char(_) => Type::new(TypeKind::Char, span),
            ConstValue::Text(_) => Type::text(span),
            ConstValue::Bytes(_) => self.make_path_type("Bytes"),
            ConstValue::Array(arr) => {
                if arr.is_empty() {
                    Type::new(
                        TypeKind::Array {
                            element: Heap::new(Type::inferred(span)),
                            size: Maybe::None,
                        },
                        span,
                    )
                } else {
                    let elem_ty = self.infer_type_from_value(&arr[0]);
                    Type::new(
                        TypeKind::Array {
                            element: Heap::new(elem_ty),
                            size: Maybe::None, // Size is dynamic at meta level
                        },
                        span,
                    )
                }
            }
            ConstValue::Tuple(elems) => {
                let types: List<Type> = elems.iter().map(|e| self.infer_type_from_value(e)).collect();
                Type::new(TypeKind::Tuple(types), span)
            }
            ConstValue::Maybe(inner) => {
                let inner_ty = match inner {
                    Maybe::Some(v) => self.infer_type_from_value(v),
                    Maybe::None => Type::inferred(span),
                };
                self.make_generic_type("Maybe", vec![inner_ty])
            }
            ConstValue::Map(map) => {
                // Infer value type from first entry if present
                let value_ty = if let Some((_, v)) = map.iter().next() {
                    self.infer_type_from_value(v)
                } else {
                    Type::inferred(span)
                };
                self.make_generic_type("Map", vec![Type::text(span), value_ty])
            }
            ConstValue::Set(_) => {
                self.make_generic_type("Set", vec![Type::text(span)])
            }
            ConstValue::Type(_) => self.make_path_type("Type"),
            ConstValue::Expr(_) => self.make_path_type("Expr"),
            ConstValue::Pattern(_) => self.make_path_type("Pattern"),
            ConstValue::Item(_) => self.make_path_type("Item"),
            ConstValue::Items(_) => self.make_path_type("Items"),
        }
    }

    /// Helper to create a simple path type like `Type`, `Expr`, etc.
    fn make_path_type(&self, name: &str) -> Type {
        let span = Span::dummy();
        let ident = Ident::new(name, span);
        let path = Path::single(ident);
        Type::new(TypeKind::Path(path), span)
    }

    /// Helper to create a generic type like `Maybe<T>`, `List<T>`, etc.
    fn make_generic_type(&self, name: &str, args: Vec<Type>) -> Type {
        let span = Span::dummy();
        let base = self.make_path_type(name);
        let generic_args: List<GenericArg> = args
            .into_iter()
            .map(GenericArg::Type)
            .collect();
        Type::new(
            TypeKind::Generic {
                base: Heap::new(base),
                args: generic_args,
            },
            span,
        )
    }

    /// Infer type from an AST expression
    pub(crate) fn infer_type_from_expr(&self, expr: &Expr) -> Type {
        let span = Span::dummy();

        match &expr.kind {
            ExprKind::Literal(lit) => {
                use verum_ast::LiteralKind;
                match &lit.kind {
                    LiteralKind::Bool(_) => Type::bool(span),
                    LiteralKind::Int(_) => Type::int(span),
                    LiteralKind::Float(_) => Type::float(span),
                    LiteralKind::Char(_) => Type::new(TypeKind::Char, span),
                    LiteralKind::Text(_) => Type::text(span),
                    LiteralKind::ByteString(_) => self.make_path_type("Bytes"),
                    _ => Type::inferred(span),
                }
            }
            ExprKind::Path(path) => {
                // Look up type from context if possible
                if let Some(ident) = path.as_ident() {
                    if let Some(val) = self.get(&Text::from(ident.as_str())) {
                        return self.infer_type_from_value(&val);
                    }
                }
                Type::inferred(span)
            }
            ExprKind::Tuple(elems) => {
                let types: List<Type> = elems.iter().map(|e| self.infer_type_from_expr(e)).collect();
                Type::new(TypeKind::Tuple(types), span)
            }
            ExprKind::Array(array_expr) => {
                use verum_ast::expr::ArrayExpr;
                match array_expr {
                    ArrayExpr::List(elems) => {
                        if elems.is_empty() {
                            Type::new(
                                TypeKind::Array {
                                    element: Heap::new(Type::inferred(span)),
                                    size: Maybe::None,
                                },
                                span,
                            )
                        } else {
                            let elem_ty = self.infer_type_from_expr(&elems[0]);
                            Type::new(
                                TypeKind::Array {
                                    element: Heap::new(elem_ty),
                                    size: Maybe::None,
                                },
                                span,
                            )
                        }
                    }
                    ArrayExpr::Repeat { value, .. } => {
                        let elem_ty = self.infer_type_from_expr(value);
                        Type::new(
                            TypeKind::Array {
                                element: Heap::new(elem_ty),
                                size: Maybe::None,
                            },
                            span,
                        )
                    }
                }
            }
            _ => Type::inferred(span),
        }
    }

    // ======== User Meta Function Execution ========

    /// Execute a user-defined meta function
    ///
    /// This method binds the provided arguments to the function parameters,
    /// converts the function body to MetaExpr, evaluates it, and returns the result.
    ///
    /// # Arguments
    /// * `func` - The meta function to execute
    /// * `args` - The arguments to pass to the function (Vec for compatibility with callers)
    ///
    /// # Returns
    /// The result of evaluating the function body
    pub fn execute_user_meta_fn(&mut self, func: &MetaFunction, args: Vec<ConstValue>) -> Result<ConstValue, MetaError> {
        // Check recursion limit FIRST to prevent stack overflow
        if self.current_recursion_depth >= self.recursion_limit {
            return Err(MetaError::RecursionLimitExceeded {
                depth: self.current_recursion_depth as usize,
                limit: self.recursion_limit as usize,
            });
        }

        // Increment recursion depth
        self.current_recursion_depth += 1;

        // Execute the function (with cleanup on any exit path)
        let result = self.execute_user_meta_fn_inner(func, args);

        // Decrement recursion depth
        self.current_recursion_depth -= 1;

        result
    }

    /// Internal implementation of user meta function execution.
    /// Called by execute_user_meta_fn after recursion limit check.
    fn execute_user_meta_fn_inner(&mut self, func: &MetaFunction, args: Vec<ConstValue>) -> Result<ConstValue, MetaError> {
        #[cfg(debug_assertions)]
        {
            // eprintln!("[DEBUG] execute_user_meta_fn_inner: func.name = {:?}", func.name);
            // eprintln!("[DEBUG] execute_user_meta_fn_inner: func.body.kind = {:?}", std::mem::discriminant(&func.body.kind));
        }

        // Validate argument count
        if args.len() != func.params.len() {
            return Err(MetaError::ArityMismatch {
                expected: func.params.len(),
                got: args.len(),
            });
        }

        // Save current enabled_contexts and apply function's contexts
        let saved_contexts = self.enabled_contexts.clone();

        // Save and set is_transparent flag for hygiene checking
        let saved_is_transparent = self.is_transparent;
        self.is_transparent = func.is_transparent;

        // Extract context names from the function's using clause and enable them
        if !func.contexts.is_empty() {
            let context_names: Vec<Text> = func.contexts.iter()
                .filter_map(|ctx| {
                    // Extract the context name from the path
                    // For simple contexts like `MetaTypes`, this is just the path's string representation
                    ctx.path.as_ident().map(|ident| Text::from(ident.as_str()))
                })
                .collect();

            // Parse the contexts from the function's using clause with duplicate detection
            let parsed = EnabledContexts::parse_using_clause(&context_names);

            // Check for duplicate context declarations
            if !parsed.duplicates.is_empty() {
                return Err(MetaError::DuplicateContext(parsed.duplicates[0].name.clone()));
            }

            // Apply the parsed contexts
            self.enabled_contexts = parsed.enabled_contexts;
        }

        // Save current bindings that will be shadowed
        let mut saved_bindings: List<(Text, Option<ConstValue>)> = List::new();
        for param in &func.params {
            saved_bindings.push((param.name.clone(), self.get(&param.name)));
        }

        // Set execution stage from the meta function's declared stage level.
        // meta fn = stage 1, meta(2) fn = stage 2, etc.
        // This ensures quote hygiene checks use the correct stage context.
        let saved_stage = self.current_stage;
        self.current_stage = func.stage_level.max(1); // At least stage 1 for meta functions

        // Bind arguments to parameters
        for (param, arg) in func.params.iter().zip(args.iter()) {
            self.bind(param.name.clone(), arg.clone());
        }

        // Convert body to MetaExpr and evaluate
        let result = self.ast_expr_to_meta_expr(&func.body)
            .and_then(|meta_expr| self.eval_meta_expr(&meta_expr));

        // Restore original bindings
        for (name, saved_value) in saved_bindings {
            match saved_value {
                Some(val) => self.bind(name, val),
                None => { self.unbind(&name); }
            }
        }

        // Restore original contexts
        self.enabled_contexts = saved_contexts;

        // Restore execution stage
        self.current_stage = saved_stage;

        // Restore is_transparent flag
        self.is_transparent = saved_is_transparent;

        result
    }

    // ======== Quote Hygiene Checking ========

    /// Check quote expression for hygiene violations
    ///
    /// This method analyzes a quote expression to detect:
    /// - M400/M408: Unbound splice variables (${undefined_var})
    /// - M402: Accidental variable capture
    /// - M404: Scope resolution failures
    /// - M405: Stage mismatches
    ///
    /// Quote hygiene ensures that quoted code (quote! { ... }) does not accidentally
    /// capture variables from the expansion site. Splice expressions (#expr) are checked
    /// for proper scoping. This prevents the classic macro hygiene problem where generated
    /// identifiers collide with user-defined names.
    fn check_quote_hygiene(&self, expr: &Expr) -> Result<(), MetaError> {
        // Only check Quote expressions
        if let ExprKind::Quote { tokens, .. } = &expr.kind {
            // Collect all splice references and regular identifiers from the token tree
            let mut violations = Vec::new();
            self.analyze_token_tree_hygiene(tokens, &mut violations);

            #[cfg(debug_assertions)]
            {
                // eprintln!("[DEBUG] check_quote_hygiene: found {} violations", violations.len());
            }

            // Return the first violation if any
            if let Some(violation) = violations.into_iter().next() {
                return Err(violation);
            }
        }

        Ok(())
    }

    // ======== Quote Splice Expansion ========

    /// Expand splices in a quote expression
    ///
    /// This method walks the token tree of a quote expression and substitutes
    /// splice patterns (`$var` and `${expr}`) with their evaluated values from
    /// the meta scope.
    ///
    /// # Splice Patterns
    ///
    /// - `$ident`: Substitutes the value of `ident` from the meta scope
    /// - `${expr}`: Evaluates `expr` and substitutes the result
    /// - `$[for pattern in iter { body }]`: Repetition (handled separately)
    ///
    /// # Example
    ///
    /// ```verum
    /// meta fn generate_getter(name: Text, ty: Type) -> TokenStream {
    ///     quote {
    ///         fn get_$name() -> $ty { self.$name }
    ///     }
    /// }
    /// ```
    ///
    /// Here, `$name` and `$ty` are substituted with the actual values.
    ///
    /// Splice interpolation substitutes $name and #expr placeholders in quote blocks
    /// with actual values from the meta evaluation context. $name splices identifiers,
    /// #expr splices arbitrary expressions, and #(#items),* splices repeated sequences
    /// with separators. This is the core mechanism for procedural code generation.
    fn expand_quote_splices(&mut self, expr: &Expr) -> Result<Expr, MetaError> {
        use verum_ast::expr::ExprKind;

        // Only process Quote expressions
        if let ExprKind::Quote { target_stage, tokens } = &expr.kind {
            // Expand splices in the token tree
            let expanded_tokens = self.expand_token_tree_splices(tokens)?;

            // Hygiene re-check after splice substitution.
            //
            // A `${expr}` splice may have brought in identifiers from
            // the splice site that the quote-site author never saw.
            // Re-walk the expanded tokens through the hygiene checker
            // so accidental capture is caught at expansion time rather
            // than surfacing as a mysterious type error later. Closes
            // master-audit finding F-1 (P0).
            //
            // The check is non-fatal — violations are accumulated on
            // the checker. Hard-fail is the embedder's choice via
            // `CheckerConfig::strict_mode`.
            self.recheck_post_splice_hygiene(&expanded_tokens, expr.span);

            // Return the new Quote expression with expanded tokens
            Ok(Expr::new(
                ExprKind::Quote {
                    target_stage: *target_stage,
                    tokens: expanded_tokens,
                },
                expr.span,
            ))
        } else {
            // Not a quote expression, return as-is
            Ok(expr.clone())
        }
    }

    /// Run the hygiene checker over post-splice tokens to detect
    /// identifiers that would shadow bindings the quote-site author
    /// did not anticipate.
    ///
    /// Each violation is converted to a `verum_diagnostics::Diagnostic`
    /// (severity `Warning`, code from `violation.error_code()` —
    /// the M4xx range) and pushed onto `MetaContext.diagnostics`.
    /// The macro-expansion phase drains this list into
    /// `PhaseOutput.warnings`, which `api.rs::run_pipeline` extends
    /// onto the session's diagnostic stream. Pre-fix violations only
    /// reached `tracing::warn!` — they didn't surface in `cargo build`
    /// output, IDE diagnostics, or compilation failure decisions, so
    /// macros with capture issues silently produced wrong code.
    ///
    /// `&mut self` so the diagnostics list can be appended to. The
    /// caller still sees a `tracing::warn!` summary at the original
    /// site (one log line per quote with violations) for log-tailing
    /// observability.
    fn recheck_post_splice_hygiene(
        &mut self,
        tokens: &List<verum_ast::expr::TokenTree>,
        span: verum_ast::span::Span,
    ) {
        use crate::hygiene::{HygieneChecker, CheckerConfig, HygieneContext};
        let mut checker = HygieneChecker::new(
            HygieneContext::new(),
            CheckerConfig::default(),
        );
        checker.check_post_splice_tokens(tokens);
        let violations = checker.take_violations();
        if violations.is_empty() {
            return;
        }
        tracing::warn!(
            "post-splice hygiene check found {} potential capture violation(s) at {:?}",
            violations.len(),
            span,
        );
        // Materialise each violation as a user-facing diagnostic so
        // the session diagnostic stream catches it. The conversion
        // is in `hygiene_violation_to_diagnostic` so unit tests can
        // pin the M4xx-code shape and span resolution without
        // building a full checker fixture.
        for v in violations.iter() {
            let diag = hygiene_violation_to_diagnostic(v, span);
            self.diagnostics.push(diag);
        }
    }

    /// Expand splices in a token tree
    ///
    /// Walks the token tree and processes:
    /// - `$ident` patterns: Look up ident in bindings, substitute value
    /// - `${...}` patterns: Parse inner tokens as expr, evaluate, substitute
    /// - `$[...]` patterns: Repetition blocks (iterate and expand)
    fn expand_token_tree_splices(
        &self,
        tokens: &List<verum_ast::expr::TokenTree>,
    ) -> Result<List<verum_ast::expr::TokenTree>, MetaError> {
        use verum_ast::expr::{TokenTree, TokenTreeKind, MacroDelimiter};

        let mut result = List::new();
        let mut i = 0;

        while i < tokens.len() {
            match &tokens[i] {
                TokenTree::Token(tok) => {
                    // Check for $ splice operator
                    if tok.kind == TokenTreeKind::Punct && tok.text.as_str() == "$" {
                        // Look ahead to determine splice type
                        if i + 1 < tokens.len() {
                            match &tokens[i + 1] {
                                // $ident - simple identifier splice
                                TokenTree::Token(next_tok)
                                    if next_tok.kind == TokenTreeKind::Ident =>
                                {
                                    let var_name = Text::from(next_tok.text.as_str());
                                    let expanded = self.expand_ident_splice(&var_name, next_tok.span)?;
                                    for t in expanded {
                                        result.push(t);
                                    }
                                    i += 2; // Skip $ and ident
                                    continue;
                                }

                                // ${expr} - expression splice
                                TokenTree::Group {
                                    delimiter: MacroDelimiter::Brace,
                                    tokens: inner_tokens,
                                    span,
                                } => {
                                    let expanded = self.expand_expr_splice(inner_tokens, *span)?;
                                    for t in expanded {
                                        result.push(t);
                                    }
                                    i += 2; // Skip $ and {...}
                                    continue;
                                }

                                // $[for ...] - repetition block
                                TokenTree::Group {
                                    delimiter: MacroDelimiter::Bracket,
                                    tokens: inner_tokens,
                                    span,
                                } => {
                                    let expanded = self.expand_repetition_splice(inner_tokens, *span)?;
                                    for t in expanded {
                                        result.push(t);
                                    }
                                    i += 2; // Skip $ and [...]
                                    continue;
                                }

                                _ => {
                                    // Not a recognized splice pattern, keep the $
                                    result.push(tokens[i].clone());
                                }
                            }
                        } else {
                            // $ at end of tokens, keep it
                            result.push(tokens[i].clone());
                        }
                    } else {
                        // Regular token, keep it
                        result.push(tokens[i].clone());
                    }
                }

                TokenTree::Group { delimiter, tokens: inner, span } => {
                    // Recursively expand splices in nested groups
                    let expanded_inner = self.expand_token_tree_splices(inner)?;
                    result.push(TokenTree::Group {
                        delimiter: *delimiter,
                        tokens: expanded_inner,
                        span: *span,
                    });
                }
            }
            i += 1;
        }

        Ok(result)
    }

    /// Expand a simple identifier splice ($ident)
    ///
    /// Looks up the identifier in the meta scope and converts the value
    /// to tokens for substitution.
    fn expand_ident_splice(
        &self,
        name: &Text,
        span: Span,
    ) -> Result<List<verum_ast::expr::TokenTree>, MetaError> {
        // Look up the variable in the meta scope
        match self.get(name) {
            Some(value) => {
                // Convert ConstValue to tokens
                self.const_value_to_tokens(&value, span)
            }
            None => {
                // M400: Unbound splice variable
                Err(MetaError::InvalidQuoteSyntax {
                    message: Text::from(format!(
                        "unbound splice variable '{}' is not defined in meta scope",
                        name.as_str()
                    )),
                })
            }
        }
    }

    /// Expand an expression splice (${expr})
    ///
    /// Parses the inner tokens as an expression, evaluates it in the meta
    /// context, and converts the result to tokens.
    fn expand_expr_splice(
        &self,
        inner_tokens: &List<verum_ast::expr::TokenTree>,
        span: Span,
    ) -> Result<List<verum_ast::expr::TokenTree>, MetaError> {
        // Convert tokens to source text for parsing
        let source_text = self.tokens_to_source_text(inner_tokens);

        // Parse as expression using a dummy file ID
        let parser = verum_fast_parser::FastParser::new();
        let file_id = verum_common::FileId::dummy();
        let expr = parser.parse_expr_str(&source_text, file_id)
            .map_err(|_| MetaError::InvalidQuoteSyntax {
                message: Text::from(format!(
                    "failed to parse splice expression: {}",
                    &source_text
                )),
            })?;

        // Convert AST expression to meta expression
        let meta_expr = self.ast_expr_to_meta_expr(&expr)?;

        // Evaluate the meta expression
        // Note: We need a mutable self for evaluation, but we only have &self here.
        // This is a design constraint - for now, we'll use a simple approach that
        // handles common cases like variable references.
        let value = self.evaluate_simple_meta_expr(&meta_expr)?;

        // Convert the result to tokens
        self.const_value_to_tokens(&value, span)
    }

    /// Evaluate a simple meta expression (for splice context)
    ///
    /// This is an extended evaluation that handles common expression patterns
    /// without requiring mutable self. Supports:
    /// - Literals and variables
    /// - Binary and unary operations
    /// - Field access, indexing, and tuple indexing
    /// - Simple method calls on known types (len, is_empty, etc.)
    fn evaluate_simple_meta_expr(&self, expr: &MetaExpr) -> Result<ConstValue, MetaError> {
        match expr {
            MetaExpr::Variable(name) => {
                self.get(name).ok_or_else(|| MetaError::Other(
                    Text::from(format!("undefined variable '{}' in splice expression", name.as_str()))
                ))
            }
            MetaExpr::Literal(val) => Ok(val.clone()),

            // Binary operations - evaluate both sides and apply operator
            MetaExpr::Binary { op, left, right } => {
                let left_val = self.evaluate_simple_meta_expr(left)?;
                let right_val = self.evaluate_simple_meta_expr(right)?;
                self.eval_binary_op(*op, left_val, right_val)
            }

            // Unary operations - evaluate inner and apply operator
            MetaExpr::Unary { op, expr: inner } => {
                let val = self.evaluate_simple_meta_expr(inner)?;
                self.eval_unary_op(*op, val)
            }

            // Tuple index - access tuple element by numeric index
            MetaExpr::TupleIndex { expr: inner, index } => {
                let val = self.evaluate_simple_meta_expr(inner)?;
                match &val {
                    ConstValue::Tuple(tup) => {
                        let i = *index as usize;
                        tup.get(i).cloned().ok_or_else(|| MetaError::Other(
                            Text::from(format!("tuple index {} out of bounds (len: {})", i, tup.len()))
                        ))
                    }
                    _ => Err(MetaError::TypeMismatch {
                        expected: Text::from("Tuple"),
                        found: Text::from(val.type_name()),
                    })
                }
            }

            // Method calls - support common methods on known types
            MetaExpr::MethodCall { receiver, method, args } => {
                let recv_val = self.evaluate_simple_meta_expr(receiver)?;
                let method_name = method.as_str();

                match (&recv_val, method_name) {
                    // Array/List methods
                    (ConstValue::Array(arr), "len") if args.is_empty() => {
                        Ok(ConstValue::Int(arr.len() as i128))
                    }
                    (ConstValue::Array(arr), "is_empty") if args.is_empty() => {
                        Ok(ConstValue::Bool(arr.is_empty()))
                    }
                    (ConstValue::Array(arr), "first") if args.is_empty() => {
                        arr.first().cloned().ok_or_else(|| MetaError::Other(
                            Text::from("first() called on empty array")
                        ))
                    }
                    (ConstValue::Array(arr), "last") if args.is_empty() => {
                        arr.last().cloned().ok_or_else(|| MetaError::Other(
                            Text::from("last() called on empty array")
                        ))
                    }
                    (ConstValue::Array(arr), "get") if args.len() == 1 => {
                        let idx = self.evaluate_simple_meta_expr(&args[0])?;
                        if let ConstValue::Int(i) = idx {
                            arr.get(i as usize).cloned().ok_or_else(|| MetaError::Other(
                                Text::from(format!("index {} out of bounds", i))
                            ))
                        } else {
                            Err(MetaError::TypeMismatch {
                                expected: Text::from("Int"),
                                found: Text::from(idx.type_name()),
                            })
                        }
                    }

                    // String/Text methods
                    (ConstValue::Text(s), "len") if args.is_empty() => {
                        Ok(ConstValue::Int(s.len() as i128))
                    }
                    (ConstValue::Text(s), "is_empty") if args.is_empty() => {
                        Ok(ConstValue::Bool(s.is_empty()))
                    }
                    (ConstValue::Text(s), "to_uppercase") if args.is_empty() => {
                        Ok(ConstValue::Text(Text::from(s.as_str().to_uppercase())))
                    }
                    (ConstValue::Text(s), "to_lowercase") if args.is_empty() => {
                        Ok(ConstValue::Text(Text::from(s.as_str().to_lowercase())))
                    }
                    (ConstValue::Text(s), "trim") if args.is_empty() => {
                        Ok(ConstValue::Text(Text::from(s.as_str().trim())))
                    }
                    (ConstValue::Text(s), "starts_with") if args.len() == 1 => {
                        let prefix = self.evaluate_simple_meta_expr(&args[0])?;
                        if let ConstValue::Text(p) = prefix {
                            Ok(ConstValue::Bool(s.as_str().starts_with(p.as_str())))
                        } else {
                            Err(MetaError::TypeMismatch {
                                expected: Text::from("Text"),
                                found: Text::from(prefix.type_name()),
                            })
                        }
                    }
                    (ConstValue::Text(s), "ends_with") if args.len() == 1 => {
                        let suffix = self.evaluate_simple_meta_expr(&args[0])?;
                        if let ConstValue::Text(p) = suffix {
                            Ok(ConstValue::Bool(s.as_str().ends_with(p.as_str())))
                        } else {
                            Err(MetaError::TypeMismatch {
                                expected: Text::from("Text"),
                                found: Text::from(suffix.type_name()),
                            })
                        }
                    }
                    (ConstValue::Text(s), "contains") if args.len() == 1 => {
                        let substr = self.evaluate_simple_meta_expr(&args[0])?;
                        if let ConstValue::Text(p) = substr {
                            Ok(ConstValue::Bool(s.as_str().contains(p.as_str())))
                        } else {
                            Err(MetaError::TypeMismatch {
                                expected: Text::from("Text"),
                                found: Text::from(substr.type_name()),
                            })
                        }
                    }

                    // Tuple methods
                    (ConstValue::Tuple(tup), "len") if args.is_empty() => {
                        Ok(ConstValue::Int(tup.len() as i128))
                    }

                    // Map methods
                    (ConstValue::Map(map), "len") if args.is_empty() => {
                        Ok(ConstValue::Int(map.len() as i128))
                    }
                    (ConstValue::Map(map), "is_empty") if args.is_empty() => {
                        Ok(ConstValue::Bool(map.is_empty()))
                    }
                    (ConstValue::Map(map), "contains_key") if args.len() == 1 => {
                        let key = self.evaluate_simple_meta_expr(&args[0])?;
                        if let ConstValue::Text(k) = key {
                            Ok(ConstValue::Bool(map.contains_key(&k)))
                        } else {
                            Err(MetaError::TypeMismatch {
                                expected: Text::from("Text"),
                                found: Text::from(key.type_name()),
                            })
                        }
                    }
                    (ConstValue::Map(map), "get") if args.len() == 1 => {
                        let key = self.evaluate_simple_meta_expr(&args[0])?;
                        if let ConstValue::Text(k) = key {
                            map.get(&k).cloned().ok_or_else(|| MetaError::Other(
                                Text::from(format!("key '{}' not found in map", k.as_str()))
                            ))
                        } else {
                            Err(MetaError::TypeMismatch {
                                expected: Text::from("Text"),
                                found: Text::from(key.type_name()),
                            })
                        }
                    }

                    // Numeric methods
                    (ConstValue::Int(n), "abs") if args.is_empty() => {
                        Ok(ConstValue::Int(n.abs()))
                    }
                    (ConstValue::Float(n), "abs") if args.is_empty() => {
                        Ok(ConstValue::Float(n.abs()))
                    }
                    (ConstValue::Float(n), "floor") if args.is_empty() => {
                        Ok(ConstValue::Float(n.floor()))
                    }
                    (ConstValue::Float(n), "ceil") if args.is_empty() => {
                        Ok(ConstValue::Float(n.ceil()))
                    }
                    (ConstValue::Float(n), "round") if args.is_empty() => {
                        Ok(ConstValue::Float(n.round()))
                    }
                    (ConstValue::Float(n), "sqrt") if args.is_empty() => {
                        Ok(ConstValue::Float(n.sqrt()))
                    }

                    _ => Err(MetaError::Other(
                        Text::from(format!(
                            "unknown method '{}' on type '{}' in splice context",
                            method_name,
                            recv_val.type_name()
                        ))
                    ))
                }
            }

            // If expression - evaluate condition and choose branch
            MetaExpr::If { condition, then_branch, else_branch } => {
                let cond_val = self.evaluate_simple_meta_expr(condition)?;
                if cond_val.as_bool() {
                    self.evaluate_simple_meta_expr(then_branch)
                } else if let Maybe::Some(else_br) = else_branch {
                    self.evaluate_simple_meta_expr(else_br)
                } else {
                    Ok(ConstValue::Unit)
                }
            }

            MetaExpr::FieldAccess { expr, field } => {
                let val = self.evaluate_simple_meta_expr(expr)?;
                match &val {
                    ConstValue::Map(map) => {
                        map.get(field).cloned().ok_or_else(|| MetaError::Other(
                            Text::from(format!("field '{}' not found", field.as_str()))
                        ))
                    }
                    _ => Err(MetaError::Other(
                        Text::from("field access on non-map value")
                    ))
                }
            }
            MetaExpr::Index { expr, index } => {
                let val = self.evaluate_simple_meta_expr(expr)?;
                let idx = self.evaluate_simple_meta_expr(index)?;
                match (&val, &idx) {
                    (ConstValue::Array(arr), ConstValue::Int(i)) => {
                        let i = *i as usize;
                        arr.get(i).cloned().ok_or_else(|| MetaError::Other(
                            Text::from(format!("index {} out of bounds", i))
                        ))
                    }
                    (ConstValue::Tuple(tup), ConstValue::Int(i)) => {
                        let i = *i as usize;
                        tup.get(i).cloned().ok_or_else(|| MetaError::Other(
                            Text::from(format!("tuple index {} out of bounds", i))
                        ))
                    }
                    (ConstValue::Text(s), ConstValue::Int(i)) => {
                        let i = *i as usize;
                        s.as_str().chars().nth(i)
                            .map(|c| ConstValue::Text(Text::from(c.to_string())))
                            .ok_or_else(|| MetaError::Other(
                                Text::from(format!("string index {} out of bounds", i))
                            ))
                    }
                    _ => Err(MetaError::Other(
                        Text::from("invalid index operation")
                    ))
                }
            }

            // Call expression - support simple built-in functions
            MetaExpr::Call(name, args) => {
                let func_name = name.as_str();
                match func_name {
                    "len" if args.len() == 1 => {
                        let val = self.evaluate_simple_meta_expr(&args[0])?;
                        match &val {
                            ConstValue::Array(arr) => Ok(ConstValue::Int(arr.len() as i128)),
                            ConstValue::Text(s) => Ok(ConstValue::Int(s.len() as i128)),
                            ConstValue::Tuple(tup) => Ok(ConstValue::Int(tup.len() as i128)),
                            ConstValue::Map(map) => Ok(ConstValue::Int(map.len() as i128)),
                            _ => Err(MetaError::TypeMismatch {
                                expected: Text::from("Array, Text, Tuple, or Map"),
                                found: Text::from(val.type_name()),
                            })
                        }
                    }
                    "min" if args.len() == 2 => {
                        let a = self.evaluate_simple_meta_expr(&args[0])?;
                        let b = self.evaluate_simple_meta_expr(&args[1])?;
                        match (&a, &b) {
                            (ConstValue::Int(x), ConstValue::Int(y)) => Ok(ConstValue::Int((*x).min(*y))),
                            (ConstValue::Float(x), ConstValue::Float(y)) => Ok(ConstValue::Float(x.min(*y))),
                            _ => Err(MetaError::TypeMismatch {
                                expected: Text::from("numeric types"),
                                found: Text::from(format!("({}, {})", a.type_name(), b.type_name())),
                            })
                        }
                    }
                    "max" if args.len() == 2 => {
                        let a = self.evaluate_simple_meta_expr(&args[0])?;
                        let b = self.evaluate_simple_meta_expr(&args[1])?;
                        match (&a, &b) {
                            (ConstValue::Int(x), ConstValue::Int(y)) => Ok(ConstValue::Int((*x).max(*y))),
                            (ConstValue::Float(x), ConstValue::Float(y)) => Ok(ConstValue::Float(x.max(*y))),
                            _ => Err(MetaError::TypeMismatch {
                                expected: Text::from("numeric types"),
                                found: Text::from(format!("({}, {})", a.type_name(), b.type_name())),
                            })
                        }
                    }
                    "abs" if args.len() == 1 => {
                        let val = self.evaluate_simple_meta_expr(&args[0])?;
                        match &val {
                            ConstValue::Int(n) => Ok(ConstValue::Int(n.abs())),
                            ConstValue::Float(n) => Ok(ConstValue::Float(n.abs())),
                            _ => Err(MetaError::TypeMismatch {
                                expected: Text::from("Int or Float"),
                                found: Text::from(val.type_name()),
                            })
                        }
                    }
                    "sqrt" if args.len() == 1 => {
                        let val = self.evaluate_simple_meta_expr(&args[0])?;
                        match &val {
                            ConstValue::Float(n) => Ok(ConstValue::Float(n.sqrt())),
                            ConstValue::Int(n) => Ok(ConstValue::Float((*n as f64).sqrt())),
                            _ => Err(MetaError::TypeMismatch {
                                expected: Text::from("Float"),
                                found: Text::from(val.type_name()),
                            })
                        }
                    }
                    "to_string" if args.len() == 1 => {
                        let val = self.evaluate_simple_meta_expr(&args[0])?;
                        Ok(ConstValue::Text(Text::from(format!("{}", val))))
                    }
                    "type_of" if args.len() == 1 => {
                        let val = self.evaluate_simple_meta_expr(&args[0])?;
                        Ok(ConstValue::Text(Text::from(val.type_name())))
                    }
                    _ => Err(MetaError::Other(
                        Text::from(format!(
                            "unknown function '{}' in splice context (use $var for complex expressions)",
                            func_name
                        ))
                    ))
                }
            }

            _ => Err(MetaError::Other(
                Text::from("expression type not supported in splice context - use $var for complex expressions")
            ))
        }
    }

    /// Expand a repetition splice ($[for pattern in iter { body }])
    ///
    /// Iterates over the collection and expands the body for each element.
    fn expand_repetition_splice(
        &self,
        inner_tokens: &List<verum_ast::expr::TokenTree>,
        _span: Span,
    ) -> Result<List<verum_ast::expr::TokenTree>, MetaError> {
        use verum_ast::expr::{TokenTree, TokenTreeKind, MacroDelimiter};

        // Parse the repetition structure: for pattern in iter { body }
        // Expected tokens: for, pattern, in, iter_name, { body_tokens }

        let mut i = 0;

        // Check for 'for' keyword
        if i >= inner_tokens.len() {
            return Err(MetaError::InvalidQuoteSyntax {
                message: Text::from("expected 'for' in repetition block"),
            });
        }

        if let TokenTree::Token(tok) = &inner_tokens[i] {
            if tok.text.as_str() != "for" {
                return Err(MetaError::InvalidQuoteSyntax {
                    message: Text::from("expected 'for' at start of repetition block"),
                });
            }
        }
        i += 1;

        // Get pattern variable name(s)
        // Supports both simple identifier: `for x in xs`
        // And tuple patterns: `for (a, b) in pairs`
        if i >= inner_tokens.len() {
            return Err(MetaError::InvalidQuoteSyntax {
                message: Text::from("expected pattern variable after 'for'"),
            });
        }

        let pattern_names: List<Text> = match &inner_tokens[i] {
            TokenTree::Token(tok) => {
                // Also accept keywords used as identifiers (like 'field', 'type', etc.)
                if tok.kind == TokenTreeKind::Ident || tok.kind == TokenTreeKind::Keyword {
                    let mut names = List::new();
                    names.push(Text::from(tok.text.as_str()));
                    names
                } else {
                    return Err(MetaError::InvalidQuoteSyntax {
                        message: Text::from("expected identifier for pattern"),
                    });
                }
            }
            TokenTree::Group { delimiter: MacroDelimiter::Paren, tokens: pattern_tokens, .. } => {
                // Tuple pattern: extract identifiers
                let mut names = List::new();
                for ptok in pattern_tokens.iter() {
                    if let TokenTree::Token(t) = ptok {
                        if t.kind == TokenTreeKind::Ident {
                            names.push(Text::from(t.text.as_str()));
                        }
                        // Skip commas and other punctuation
                    }
                }
                if names.is_empty() {
                    return Err(MetaError::InvalidQuoteSyntax {
                        message: Text::from("tuple pattern must have at least one identifier"),
                    });
                }
                names
            }
            _ => {
                return Err(MetaError::InvalidQuoteSyntax {
                    message: Text::from("expected identifier or tuple pattern"),
                });
            }
        };
        let is_tuple_pattern = pattern_names.len() > 1;
        i += 1;

        // Check for 'in' keyword
        if i >= inner_tokens.len() {
            return Err(MetaError::InvalidQuoteSyntax {
                message: Text::from("expected 'in' after pattern"),
            });
        }

        if let TokenTree::Token(tok) = &inner_tokens[i] {
            if tok.text.as_str() != "in" {
                return Err(MetaError::InvalidQuoteSyntax {
                    message: Text::from("expected 'in' keyword"),
                });
            }
        }
        i += 1;

        // Get iterator variable name
        if i >= inner_tokens.len() {
            return Err(MetaError::InvalidQuoteSyntax {
                message: Text::from("expected iterator expression after 'in'"),
            });
        }

        let iter_name = if let TokenTree::Token(tok) = &inner_tokens[i] {
            if tok.kind == TokenTreeKind::Ident {
                Text::from(tok.text.as_str())
            } else {
                return Err(MetaError::InvalidQuoteSyntax {
                    message: Text::from("expected identifier for iterator"),
                });
            }
        } else {
            return Err(MetaError::InvalidQuoteSyntax {
                message: Text::from("expected identifier for iterator"),
            });
        };
        i += 1;

        // Get body block
        if i >= inner_tokens.len() {
            return Err(MetaError::InvalidQuoteSyntax {
                message: Text::from("expected body block after iterator"),
            });
        }

        let body_tokens = if let TokenTree::Group {
            delimiter: MacroDelimiter::Brace,
            tokens,
            ..
        } = &inner_tokens[i]
        {
            tokens.clone()
        } else {
            return Err(MetaError::InvalidQuoteSyntax {
                message: Text::from("expected { body } for repetition"),
            });
        };

        // Look up the iterator value
        let iter_value = self.get(&iter_name).ok_or_else(|| MetaError::InvalidQuoteSyntax {
            message: Text::from(format!(
                "unbound variable '{}' in repetition",
                iter_name.as_str()
            )),
        })?;

        // Get elements to iterate over
        let elements = match iter_value {
            ConstValue::Array(arr) => arr.clone(),
            ConstValue::Tuple(tup) => tup.clone(),
            _ => {
                return Err(MetaError::InvalidQuoteSyntax {
                    message: Text::from(format!(
                        "'{}' is not iterable (expected array or tuple)",
                        iter_name.as_str()
                    )),
                });
            }
        };

        // Expand body for each element
        let mut result = List::new();
        for elem in elements {
            // Build bindings: either single var or tuple destructuring
            let bindings: List<(Text, ConstValue)> = if is_tuple_pattern {
                // Destructure the element as a tuple
                match &elem {
                    ConstValue::Tuple(tuple_elems) => {
                        if tuple_elems.len() != pattern_names.len() {
                            return Err(MetaError::InvalidQuoteSyntax {
                                message: Text::from(format!(
                                    "tuple pattern has {} elements, but value has {}",
                                    pattern_names.len(),
                                    tuple_elems.len()
                                )),
                            });
                        }
                        pattern_names.iter()
                            .zip(tuple_elems.iter())
                            .map(|(name, val)| (name.clone(), val.clone()))
                            .collect()
                    }
                    _ => {
                        return Err(MetaError::InvalidQuoteSyntax {
                            message: Text::from("expected tuple value for tuple pattern"),
                        });
                    }
                }
            } else {
                // Single pattern variable
                let mut b = List::new();
                b.push((pattern_names[0].clone(), elem.clone()));
                b
            };

            // Create a child context with the pattern variables bound
            let expanded_body = self.expand_repetition_body_multi(
                &body_tokens,
                &bindings,
            )?;
            for t in expanded_body {
                result.push(t);
            }
        }

        Ok(result)
    }

    /// Expand a repetition body with bound variables
    ///
    /// This is the unified implementation that handles both single variable patterns
    /// (`for x in xs`) and tuple patterns (`for (a, b) in pairs`). The bindings list
    /// contains all the variables that should be substituted in the body.
    fn expand_repetition_body_multi(
        &self,
        body_tokens: &List<verum_ast::expr::TokenTree>,
        bindings: &List<(Text, ConstValue)>,
    ) -> Result<List<verum_ast::expr::TokenTree>, MetaError> {
        use verum_ast::expr::{TokenTree, TokenTreeKind};

        let mut result = List::new();
        let mut i = 0;

        while i < body_tokens.len() {
            match &body_tokens[i] {
                TokenTree::Token(tok) => {
                    // Check for $ splice operator
                    if tok.kind == TokenTreeKind::Punct && tok.text.as_str() == "$" {
                        if i + 1 < body_tokens.len() {
                            if let TokenTree::Token(next_tok) = &body_tokens[i + 1] {
                                // Accept both Ident and Keyword as splice names
                                // (keywords like 'field' can be used as variable names)
                                if next_tok.kind == TokenTreeKind::Ident
                                    || next_tok.kind == TokenTreeKind::Keyword
                                {
                                    let splice_name = Text::from(next_tok.text.as_str());

                                    // Check if it matches any binding
                                    let mut found = false;
                                    for (var_name, var_value) in bindings.iter() {
                                        if splice_name == *var_name {
                                            let tokens = self.const_value_to_tokens(var_value, next_tok.span)?;
                                            for t in tokens {
                                                result.push(t);
                                            }
                                            found = true;
                                            break;
                                        }
                                    }
                                    if found {
                                        i += 2;
                                        continue;
                                    }

                                    // Otherwise, look up in the meta scope
                                    if let Some(value) = self.get(&splice_name) {
                                        let tokens = self.const_value_to_tokens(&value, next_tok.span)?;
                                        for t in tokens {
                                            result.push(t);
                                        }
                                        i += 2;
                                        continue;
                                    }

                                    // Unknown variable - will be caught by hygiene check
                                    result.push(body_tokens[i].clone());
                                }
                            }
                        }
                        // Not a recognized splice, keep the $
                        result.push(body_tokens[i].clone());
                    } else {
                        // Regular token
                        result.push(body_tokens[i].clone());
                    }
                }

                TokenTree::Group { delimiter, tokens: inner, span } => {
                    // Recursively expand in nested groups
                    let expanded_inner = self.expand_repetition_body_multi(inner, bindings)?;
                    result.push(TokenTree::Group {
                        delimiter: *delimiter,
                        tokens: expanded_inner,
                        span: *span,
                    });
                }
            }
            i += 1;
        }

        Ok(result)
    }

    /// Convert a ConstValue to token tree tokens
    ///
    /// This converts meta values back to tokens for splice substitution.
    fn const_value_to_tokens(
        &self,
        value: &ConstValue,
        span: Span,
    ) -> Result<List<verum_ast::expr::TokenTree>, MetaError> {
        use verum_ast::expr::{TokenTree, TokenTreeToken, TokenTreeKind};

        let mut result = List::new();

        match value {
            ConstValue::Int(n) => {
                result.push(TokenTree::Token(TokenTreeToken::new(
                    TokenTreeKind::IntLiteral,
                    Text::from(n.to_string()),
                    span,
                )));
            }

            ConstValue::Float(f) => {
                result.push(TokenTree::Token(TokenTreeToken::new(
                    TokenTreeKind::FloatLiteral,
                    Text::from(f.to_string()),
                    span,
                )));
            }

            ConstValue::Bool(b) => {
                result.push(TokenTree::Token(TokenTreeToken::new(
                    TokenTreeKind::BoolLiteral,
                    Text::from(if *b { "true" } else { "false" }),
                    span,
                )));
            }

            ConstValue::Text(s) => {
                // Output as string literal with quotes
                result.push(TokenTree::Token(TokenTreeToken::new(
                    TokenTreeKind::StringLiteral,
                    Text::from(format!("\"{}\"", s.as_str())),
                    span,
                )));
            }

            ConstValue::Char(c) => {
                result.push(TokenTree::Token(TokenTreeToken::new(
                    TokenTreeKind::CharLiteral,
                    Text::from(format!("'{}'", c)),
                    span,
                )));
            }

            ConstValue::Type(ty) => {
                // Convert type to tokens - simplified version
                let type_text = format!("{:?}", ty);
                result.push(TokenTree::Token(TokenTreeToken::new(
                    TokenTreeKind::Ident,
                    Text::from(type_text),
                    span,
                )));
            }

            ConstValue::Array(arr) => {
                // Output as array literal: [elem1, elem2, ...]
                result.push(TokenTree::Token(TokenTreeToken::new(
                    TokenTreeKind::Punct,
                    Text::from("["),
                    span,
                )));

                for (i, elem) in arr.iter().enumerate() {
                    if i > 0 {
                        result.push(TokenTree::Token(TokenTreeToken::new(
                            TokenTreeKind::Punct,
                            Text::from(","),
                            span,
                        )));
                    }
                    let elem_tokens = self.const_value_to_tokens(elem, span)?;
                    for t in elem_tokens {
                        result.push(t);
                    }
                }

                result.push(TokenTree::Token(TokenTreeToken::new(
                    TokenTreeKind::Punct,
                    Text::from("]"),
                    span,
                )));
            }

            ConstValue::Tuple(tup) => {
                // Output as tuple literal: (elem1, elem2, ...)
                result.push(TokenTree::Token(TokenTreeToken::new(
                    TokenTreeKind::Punct,
                    Text::from("("),
                    span,
                )));

                for (i, elem) in tup.iter().enumerate() {
                    if i > 0 {
                        result.push(TokenTree::Token(TokenTreeToken::new(
                            TokenTreeKind::Punct,
                            Text::from(","),
                            span,
                        )));
                    }
                    let elem_tokens = self.const_value_to_tokens(elem, span)?;
                    for t in elem_tokens {
                        result.push(t);
                    }
                }

                result.push(TokenTree::Token(TokenTreeToken::new(
                    TokenTreeKind::Punct,
                    Text::from(")"),
                    span,
                )));
            }

            ConstValue::Unit => {
                result.push(TokenTree::Token(TokenTreeToken::new(
                    TokenTreeKind::Punct,
                    Text::from("("),
                    span,
                )));
                result.push(TokenTree::Token(TokenTreeToken::new(
                    TokenTreeKind::Punct,
                    Text::from(")"),
                    span,
                )));
            }

            ConstValue::Expr(expr) => {
                // For AST expressions, we need to convert back to tokens
                // This is complex - for now, output as a placeholder
                // The proper implementation would use the quote.rs TokenStream
                let expr_text = format!("{:?}", expr.kind);
                result.push(TokenTree::Token(TokenTreeToken::new(
                    TokenTreeKind::Ident,
                    Text::from(expr_text),
                    span,
                )));
            }

            // For other value types, output as-is (may need expansion)
            _ => {
                let val_text = format!("{:?}", value);
                result.push(TokenTree::Token(TokenTreeToken::new(
                    TokenTreeKind::Ident,
                    Text::from(val_text),
                    span,
                )));
            }
        }

        Ok(result)
    }

    /// Convert tokens to source text for parsing
    fn tokens_to_source_text(&self, tokens: &List<verum_ast::expr::TokenTree>) -> String {
        use verum_ast::expr::{TokenTree, MacroDelimiter};

        let mut result = String::new();
        let mut prev_needs_space = false;

        for token in tokens.iter() {
            match token {
                TokenTree::Token(tok) => {
                    // Add space between tokens if needed
                    if prev_needs_space && !result.is_empty() {
                        let text = tok.text.as_str();
                        if !text.starts_with(',') && !text.starts_with(';')
                            && !text.starts_with(')') && !text.starts_with(']')
                            && !text.starts_with('}')
                        {
                            result.push(' ');
                        }
                    }
                    result.push_str(tok.text.as_str());
                    prev_needs_space = true;
                }
                TokenTree::Group { delimiter, tokens: inner, .. } => {
                    let (open, close) = match delimiter {
                        MacroDelimiter::Paren => ('(', ')'),
                        MacroDelimiter::Brace => ('{', '}'),
                        MacroDelimiter::Bracket => ('[', ']'),
                    };
                    result.push(open);
                    result.push_str(&self.tokens_to_source_text(inner));
                    result.push(close);
                    prev_needs_space = true;
                }
            }
        }

        result
    }

    /// Analyze token tree for hygiene violations
    ///
    /// Walks the token tree looking for:
    /// - `$ident` patterns where ident is not in scope
    /// - `${expr}` patterns with undefined variables
    /// - Regular identifiers that might accidentally capture caller's scope
    /// - M402: Shadow conflicts in @transparent macros
    fn analyze_token_tree_hygiene(
        &self,
        tokens: &List<verum_ast::expr::TokenTree>,
        violations: &mut Vec<MetaError>,
    ) {
        // First, collect all identifiers that are declared locally within this quote
        // These include `let x = ...`, function parameters, pattern bindings, etc.
        let local_bindings = self.collect_quote_local_bindings(tokens);

        // M402: In @transparent macros, new bindings created in the quote can shadow
        // the caller's bindings, leading to accidental capture. Report the first binding.
        if self.is_transparent && !local_bindings.is_empty() {
            // Find the first binding name for the error message
            if let Some(binding_name) = local_bindings.iter().next() {
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG] M402 triggered for shadow conflict: '{}' in @transparent macro",
//                    binding_name.as_str());
                violations.push(MetaError::HygieneViolation {
                    identifier: binding_name.clone(),
                    message: Text::from(format!(
                        "@transparent macro creates binding '{}' which could shadow caller's binding",
                        binding_name.as_str()
                    )),
                });
                return; // Report first violation only
            }
        }

        // Now analyze with knowledge of local bindings
        // Pass check_double_splice=true for the outermost quote
        // Pass inside_meta_fn=false because we're at the outermost level
        self.analyze_token_tree_hygiene_with_locals(tokens, &local_bindings, violations, true, false);
    }

    /// Collect all identifiers that are declared within the quote's token tree
    ///
    /// This includes:
    /// - `let x = ...` bindings
    /// - Function parameters in `fn name(x: T, y: U)`
    /// - Closure parameters in `|x, y| ...`
    /// - For loop variables in `for x in ...`
    /// - Match pattern bindings
    fn collect_quote_local_bindings(
        &self,
        tokens: &List<verum_ast::expr::TokenTree>,
    ) -> std::collections::HashSet<Text> {
        use verum_ast::expr::{TokenTree, TokenTreeKind, MacroDelimiter};

        let mut bindings = std::collections::HashSet::new();
        let mut i = 0;

        while i < tokens.len() {
            match &tokens[i] {
                TokenTree::Token(token) => {
                    // Helper: check if token is a keyword (can be Ident or Keyword kind)
                    let is_keyword = |t: &verum_ast::expr::TokenTreeToken, kw: &str| {
                        (t.kind == TokenTreeKind::Ident || t.kind == TokenTreeKind::Keyword)
                        && t.text.as_str() == kw
                    };

                    // Check for `let` keyword followed by identifier
                    if is_keyword(token, "let") {
                        // Look for the identifier being bound
                        // Note: Some keywords (like `result`) can be used as variable names
                        if i + 1 < tokens.len() {
                            match &tokens[i + 1] {
                                TokenTree::Token(next) if next.kind == TokenTreeKind::Ident
                                    || next.kind == TokenTreeKind::Keyword => {
                                    let name = Text::from(next.text.as_str());
                                    // Don't treat actual keywords like let, fn, etc. as bindings
                                    // but contextual keywords like 'result' can be variable names
                                    if !self.is_reserved_keyword(&name) {
                                        bindings.insert(name);
                                    }
                                }
                                // Tuple pattern: let (a, b) = ...
                                TokenTree::Group { delimiter: MacroDelimiter::Paren, tokens: inner, .. } => {
                                    self.collect_pattern_bindings(inner, &mut bindings);
                                }
                                _ => {}
                            }
                        }
                    }
                    // Check for `for` loop: for x in ...
                    else if is_keyword(token, "for") {
                        if i + 1 < tokens.len() {
                            if let TokenTree::Token(next) = &tokens[i + 1] {
                                if next.kind == TokenTreeKind::Ident {
                                    let name = Text::from(next.text.as_str());
                                    if !self.is_keyword_or_builtin(&name) {
                                        bindings.insert(name);
                                    }
                                }
                            }
                        }
                    }
                    // Check for `fn` declaration: fn name(params...)
                    else if is_keyword(token, "fn") {
                        // Skip function name, look for parameter list
                        let mut j = i + 1;
                        while j < tokens.len() {
                            if let TokenTree::Group { delimiter: MacroDelimiter::Paren, tokens: params, .. } = &tokens[j] {
                                self.collect_fn_param_bindings(params, &mut bindings);
                                break;
                            }
                            j += 1;
                            if j > i + 3 { break; } // Don't look too far
                        }
                    }
                    // Check for closure: |params| ...
                    else if token.kind == TokenTreeKind::Punct && token.text.as_str() == "|" {
                        // Collect identifiers until next |
                        let mut j = i + 1;
                        while j < tokens.len() {
                            if let TokenTree::Token(t) = &tokens[j] {
                                if t.kind == TokenTreeKind::Punct && t.text.as_str() == "|" {
                                    break;
                                }
                                if t.kind == TokenTreeKind::Ident {
                                    let name = Text::from(t.text.as_str());
                                    if !self.is_keyword_or_builtin(&name) {
                                        bindings.insert(name);
                                    }
                                }
                            }
                            j += 1;
                        }
                    }
                }
                TokenTree::Group { tokens: inner, .. } => {
                    // Recursively collect from nested groups
                    let inner_bindings = self.collect_quote_local_bindings(inner);
                    bindings.extend(inner_bindings);
                }
            }
            i += 1;
        }

        bindings
    }

    /// Collect identifiers from a pattern (used in let, match, etc.)
    fn collect_pattern_bindings(
        &self,
        tokens: &List<verum_ast::expr::TokenTree>,
        bindings: &mut std::collections::HashSet<Text>,
    ) {
        use verum_ast::expr::{TokenTree, TokenTreeKind};

        for token in tokens.iter() {
            match token {
                TokenTree::Token(t) if t.kind == TokenTreeKind::Ident => {
                    let name = Text::from(t.text.as_str());
                    if !self.is_keyword_or_builtin(&name) {
                        bindings.insert(name);
                    }
                }
                TokenTree::Group { tokens: inner, .. } => {
                    self.collect_pattern_bindings(inner, bindings);
                }
                _ => {}
            }
        }
    }

    /// Collect function parameter identifiers
    fn collect_fn_param_bindings(
        &self,
        tokens: &List<verum_ast::expr::TokenTree>,
        bindings: &mut std::collections::HashSet<Text>,
    ) {
        use verum_ast::expr::{TokenTree, TokenTreeKind};

        let mut i = 0;
        while i < tokens.len() {
            if let TokenTree::Token(t) = &tokens[i] {
                // Look for identifier followed by : (parameter name)
                if t.kind == TokenTreeKind::Ident {
                    if i + 1 < tokens.len() {
                        if let TokenTree::Token(next) = &tokens[i + 1] {
                            if next.kind == TokenTreeKind::Punct && next.text.as_str() == ":" {
                                let name = Text::from(t.text.as_str());
                                if !self.is_keyword_or_builtin(&name) {
                                    bindings.insert(name);
                                }
                            }
                        }
                    }
                }
            }
            i += 1;
        }
    }

    /// Analyze token tree for hygiene violations with knowledge of local bindings
    ///
    /// `check_double_splice` - if true, check for M407 double-splice `$$` errors.
    /// This should be true for the outermost quote but false for nested quotes,
    /// since `$$x` is valid in `quote { quote { $$x } }` (inner accesses outer).
    /// HOWEVER, if we've passed through a meta function boundary, it should be true
    /// because `$$` would be escaping past the stage boundary.
    ///
    /// `inside_meta_fn` - if true, we're inside a meta function body within the quote.
    /// Nested quotes inside a meta fn should still check for double-splice because
    /// `$$` would be escaping past the stage boundary.
    fn analyze_token_tree_hygiene_with_locals(
        &self,
        tokens: &List<verum_ast::expr::TokenTree>,
        local_bindings: &std::collections::HashSet<Text>,
        violations: &mut Vec<MetaError>,
        check_double_splice: bool,
        inside_meta_fn: bool,
    ) {
        use verum_ast::expr::{TokenTree, TokenTreeKind, MacroDelimiter};

        #[cfg(debug_assertions)]
        {
            // eprintln!("[DEBUG] analyze_token_tree_hygiene: {} tokens, {} local bindings",
//                tokens.len(), local_bindings.len());
            for binding in local_bindings.iter() {
                eprintln!("  [LOCAL] {}", binding.as_str());
            }
            // Print first 20 tokens for debugging
            for (idx, tok) in tokens.iter().enumerate().take(20) {
                match tok {
                    verum_ast::expr::TokenTree::Token(t) => {
                        eprintln!("  Token[{}]: kind={:?}, text='{}'", idx, t.kind, t.text.as_str());
                    }
                    verum_ast::expr::TokenTree::Group { delimiter, tokens: inner, .. } => {
                        eprintln!("  Group[{}]: {:?} with {} inner tokens", idx, delimiter, inner.len());
                    }
                }
            }
            if tokens.len() > 20 {
                eprintln!("  ... and {} more tokens", tokens.len() - 20);
            }
        }

        // Helper to check if a token is a specific keyword
        let is_keyword_token = |t: &verum_ast::expr::TokenTreeToken, kw: &str| -> bool {
            (t.kind == TokenTreeKind::Ident || t.kind == TokenTreeKind::Keyword)
                && t.text.as_str() == kw
        };

        // Track indices of identifiers that are being defined (not referenced)
        // These follow: fn, meta fn, type, implement, context, module, protocol
        // Also track type references (after -> or :) which should not trigger hygiene checks
        let mut definition_idents: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut type_reference_idents: std::collections::HashSet<usize> = std::collections::HashSet::new();

        // First pass: find all identifiers that are being defined or are type references
        let mut i = 0;
        while i < tokens.len() {
            if let TokenTree::Token(token) = &tokens[i] {
                // `fn name(...)` or `meta fn name(...)`
                if is_keyword_token(token, "fn") {
                    if i + 1 < tokens.len() {
                        if let TokenTree::Token(next) = &tokens[i + 1] {
                            if next.kind == TokenTreeKind::Ident {
                                definition_idents.insert(i + 1);
                            }
                        }
                    }
                }
                // `type Name is ...` or `type Name<...>`
                else if is_keyword_token(token, "type") {
                    if i + 1 < tokens.len() {
                        if let TokenTree::Token(next) = &tokens[i + 1] {
                            if next.kind == TokenTreeKind::Ident {
                                definition_idents.insert(i + 1);
                            }
                        }
                    }
                }
                // `implement Name { ... }` or `implement Trait for Type`
                else if is_keyword_token(token, "implement") {
                    if i + 1 < tokens.len() {
                        if let TokenTree::Token(next) = &tokens[i + 1] {
                            if next.kind == TokenTreeKind::Ident {
                                definition_idents.insert(i + 1);
                            }
                        }
                    }
                }
                // `context Name { ... }`
                else if is_keyword_token(token, "context") {
                    if i + 1 < tokens.len() {
                        if let TokenTree::Token(next) = &tokens[i + 1] {
                            if next.kind == TokenTreeKind::Ident {
                                definition_idents.insert(i + 1);
                            }
                        }
                    }
                }
                // `module Name { ... }`
                else if is_keyword_token(token, "module") {
                    if i + 1 < tokens.len() {
                        if let TokenTree::Token(next) = &tokens[i + 1] {
                            if next.kind == TokenTreeKind::Ident {
                                definition_idents.insert(i + 1);
                            }
                        }
                    }
                }
                // `-> Type` (return type) - mark as type reference
                else if token.kind == TokenTreeKind::Punct && token.text.as_str() == "-" {
                    // Check for -> (two character arrow)
                    if i + 1 < tokens.len() {
                        if let TokenTree::Token(next) = &tokens[i + 1] {
                            if next.kind == TokenTreeKind::Punct && next.text.as_str() == ">" {
                                // Found ->, next identifier is return type
                                if i + 2 < tokens.len() {
                                    if let TokenTree::Token(type_tok) = &tokens[i + 2] {
                                        if type_tok.kind == TokenTreeKind::Ident {
                                            type_reference_idents.insert(i + 2);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // `: Type` (type annotation) - mark as type reference
                else if token.kind == TokenTreeKind::Punct && token.text.as_str() == ":" {
                    if i + 1 < tokens.len() {
                        if let TokenTree::Token(next) = &tokens[i + 1] {
                            if next.kind == TokenTreeKind::Ident {
                                type_reference_idents.insert(i + 1);
                            }
                        }
                    }
                }
            }
            i += 1;
        }

        // Second pass: analyze for hygiene violations
        let mut i = 0;
        while i < tokens.len() {
            match &tokens[i] {
                TokenTree::Token(token) => {
                    // M403: Check for gensym collision - identifiers matching __verum_gensym_*
                    // User code should never use these names as they're reserved for hygiene
                    if token.kind == TokenTreeKind::Ident
                        && token.text.as_str().starts_with("__verum_gensym_")
                    {
                        violations.push(MetaError::GensymCollision {
                            symbol: Text::from(token.text.as_str()),
                        });
                    }

                    // M407: Check for double-splice $$ (stage escape attempt)
                    // $$ tries to escape the current stage, which is not allowed
                    // EXCEPT in nested quotes: `quote { quote { $$x } }` is valid
                    // (inner quote accesses outer quote's scope)
                    // Note: The lexer may tokenize $$ as a single token with text="$$"
                    // or as two separate $ tokens. Check both cases.
                    if check_double_splice && token.kind == TokenTreeKind::Punct && token.text.as_str() == "$$" {
                        // Single $$ token case
                        // Look for the identifier after $$
                        let var_hint = if i + 1 < tokens.len() {
                            if let TokenTree::Token(ident_tok) = &tokens[i + 1] {
                                if ident_tok.kind == TokenTreeKind::Ident {
                                    format!("'{}'", ident_tok.text.as_str())
                                } else {
                                    "expression".to_string()
                                }
                            } else {
                                "expression".to_string()
                            }
                        } else {
                            "expression".to_string()
                        };

                        // #[cfg(debug_assertions)]
                        // eprintln!("[DEBUG] M407 triggered: double-splice $$ on {}", var_hint);

                        violations.push(MetaError::InvalidTokenTree {
                            message: Text::from(format!(
                                "double-splice $${} attempts to escape the current stage; use a single $ to splice meta-level bindings",
                                var_hint
                            )),
                        });
                        return; // Report first violation
                    }
                    // Also check for two separate $ tokens (alternative tokenization)
                    if check_double_splice && token.kind == TokenTreeKind::Punct && token.text.as_str() == "$" {
                        if i + 1 < tokens.len() {
                            if let TokenTree::Token(next) = &tokens[i + 1] {
                                if next.kind == TokenTreeKind::Punct && next.text.as_str() == "$" {
                                    // Found two consecutive $, this is a stage escape attempt (only at outermost quote)
                                    // Look for the identifier after $$
                                    let var_hint = if i + 2 < tokens.len() {
                                        if let TokenTree::Token(ident_tok) = &tokens[i + 2] {
                                            if ident_tok.kind == TokenTreeKind::Ident {
                                                format!("'{}'", ident_tok.text.as_str())
                                            } else {
                                                "expression".to_string()
                                            }
                                        } else {
                                            "expression".to_string()
                                        }
                                    } else {
                                        "expression".to_string()
                                    };

                                    // #[cfg(debug_assertions)]
                                    // eprintln!("[DEBUG] M407 triggered: double-splice $$ on {}", var_hint);

                                    violations.push(MetaError::InvalidTokenTree {
                                        message: Text::from(format!(
                                            "double-splice $${} attempts to escape the current stage; use a single $ to splice meta-level bindings",
                                            var_hint
                                        )),
                                    });
                                    return; // Report first violation
                                }
                            }
                        }
                    }

                    // Check for $ splice operator (but not $$, handled above)
                    if token.kind == TokenTreeKind::Punct && token.text.as_str() == "$" {
                        // Look for the next token to get the spliced variable
                        if i + 1 < tokens.len() {
                            match &tokens[i + 1] {
                                // $ident pattern
                                TokenTree::Token(next_token) if next_token.kind == TokenTreeKind::Ident => {
                                    let var_name = Text::from(next_token.text.as_str());
                                    // Check if this variable is in the current meta scope
                                    if self.get(&var_name).is_none() {
                                        // M400: Invalid quote syntax - unbound splice
                                        violations.push(MetaError::InvalidQuoteSyntax {
                                            message: Text::from(format!(
                                                "unbound splice variable '{}' is not defined in meta scope",
                                                var_name.as_str()
                                            )),
                                        });
                                    }
                                    i += 1; // Skip the ident token
                                }
                                // ${expr} pattern
                                TokenTree::Group { delimiter: MacroDelimiter::Brace, tokens: inner_tokens, span } => {
                                    // Analyze the expression inside the splice
                                    self.analyze_splice_expr(inner_tokens, *span, violations);
                                    i += 1; // Skip the group
                                }
                                // $[...] pattern - check for repetition syntax
                                TokenTree::Group { delimiter: MacroDelimiter::Bracket, tokens: inner_tokens, .. } => {
                                    // $[...] must be a repetition: $[for ... in ... { ... }]
                                    // Check if the first token is 'for' (can be Ident or Keyword kind)
                                    let is_valid_repetition = inner_tokens.first().map_or(false, |t| {
                                        matches!(t, TokenTree::Token(tok)
                                            if (tok.kind == TokenTreeKind::Ident || tok.kind == TokenTreeKind::Keyword)
                                            && tok.text.as_str() == "for")
                                    });
                                    if !is_valid_repetition {
                                        // M400: Invalid quote syntax - $[...] requires 'for' for repetition
                                        violations.push(MetaError::InvalidQuoteSyntax {
                                            message: Text::from("expected 'for' after '$[' for repetition syntax"),
                                        });
                                    } else {
                                        // M409: Check for zip with mismatched lengths
                                        // Look for pattern: for ... in zip(var1, var2) { ... }
                                        self.check_repetition_lengths(inner_tokens, violations);
                                    }
                                    i += 1; // Skip the group
                                }
                                _ => {
                                    // Bare $ at end or followed by non-identifier - M401
                                    if i + 1 >= tokens.len() {
                                        violations.push(MetaError::UnquoteOutsideQuote);
                                    }
                                }
                            }
                        }
                    }
                    // Check for lift() calls and validate the argument type
                    // M406 is emitted for unliftable types (Expr, Type, Pattern, Item - AST nodes)
                    // lift can be tokenized as Ident, Keyword, or Punct (with text "Lift")
                    else if ((token.kind == TokenTreeKind::Ident || token.kind == TokenTreeKind::Keyword)
                             && token.text.as_str() == "lift")
                         || (token.kind == TokenTreeKind::Punct
                             && token.text.as_str().eq_ignore_ascii_case("lift"))
                    {
                        // #[cfg(debug_assertions)]
                        // eprintln!("[DEBUG] Found 'lift' at index {}", i);
                        if i + 1 < tokens.len() {
                            if let TokenTree::Group { delimiter: MacroDelimiter::Paren, tokens: lift_args, .. } = &tokens[i + 1] {
                                // Check if the lift argument is an unliftable type
                                // Look for the identifier being lifted
                                if let Some(TokenTree::Token(arg_tok)) = lift_args.first() {
                                    if arg_tok.kind == TokenTreeKind::Ident {
                                        let arg_name = Text::from(arg_tok.text.as_str());
                                        // Check if this variable is in meta scope and has an unliftable type
                                        if let Some(value) = self.get(&arg_name) {
                                            // M406: Expr, Type, Pattern, Item, Items variants cannot be lifted
                                            // These are AST nodes that don't have a syntactic representation as literals
                                            let is_unliftable = matches!(
                                                value,
                                                ConstValue::Expr(_) |
                                                ConstValue::Type(_) |
                                                ConstValue::Pattern(_) |
                                                ConstValue::Item(_) |
                                                ConstValue::Items(_)
                                            );
                                            if is_unliftable {
                                                // #[cfg(debug_assertions)]
                                                // eprintln!("[DEBUG] M406 triggered for lift({}) - unliftable AST type", arg_name.as_str());
                                                violations.push(MetaError::LiftTypeMismatch {
                                                    ty: Text::from(match &value {
                                                        ConstValue::Expr(_) => "Expr (closure/function)",
                                                        ConstValue::Type(_) => "Type",
                                                        ConstValue::Pattern(_) => "Pattern",
                                                        ConstValue::Item(_) => "Item",
                                                        ConstValue::Items(_) => "Items",
                                                        _ => "unknown",
                                                    }),
                                                    reason: Text::from("AST values cannot be lifted; they don't have a syntactic representation as literals"),
                                                });
                                            }
                                        }
                                    }
                                }
                                // #[cfg(debug_assertions)]
                                // eprintln!("[DEBUG] Processed lift() paren group at index {}", i + 1);
                                i += 1; // Skip the argument group
                            }
                        }
                    }
                    // Check for bare identifiers that might be accessing meta scope
                    else if token.kind == TokenTreeKind::Ident || token.kind == TokenTreeKind::Keyword {
                        let var_name = Text::from(token.text.as_str());

                        // Skip identifiers that are being DEFINED (e.g., fn name, type Name)
                        // These are not references and shouldn't be checked for hygiene
                        if definition_idents.contains(&i) {
                            // #[cfg(debug_assertions)]
                            // eprintln!("[DEBUG] Skipping definition identifier '{}' at index {}", var_name.as_str(), i);
                            i += 1;
                            continue;
                        }

                        // Skip identifiers that are type references (after -> or :)
                        // Type names are not captured from caller's scope
                        if type_reference_idents.contains(&i) {
                            // #[cfg(debug_assertions)]
                            // eprintln!("[DEBUG] Skipping type reference '{}' at index {}", var_name.as_str(), i);
                            i += 1;
                            continue;
                        }

                        // Skip identifiers that start with uppercase - these are typically type names
                        // In Verum (like many languages), types use PascalCase, variables use snake_case
                        if var_name.as_str().chars().next().map_or(false, |c| c.is_uppercase()) {
                            // #[cfg(debug_assertions)]
                            // eprintln!("[DEBUG] Skipping uppercase identifier '{}' (likely type name)", var_name.as_str());
                            i += 1;
                            continue;
                        }

                        // Skip reserved keywords, builtins, and locally declared identifiers
                        if !self.is_keyword_or_builtin(&var_name) && !local_bindings.contains(&var_name) {
                            // Check if this identifier exists in the meta scope
                            if self.get(&var_name).is_some() {
                                // #[cfg(debug_assertions)]
                                // eprintln!("[DEBUG] M405 triggered for identifier: '{}' (stage mismatch - meta binding used in quote)",
//                                    var_name.as_str());
                                // M405: Quote stage error - referencing stage 1 binding from stage 0 code
                                // The user should use $var_name (splice) or lift(var_name) to cross stages
                                violations.push(MetaError::QuoteStageError {
                                    target: 1,  // Meta level (where the variable is defined)
                                    current: 0, // Quote level (runtime code)
                                });
                            } else if self.is_transparent && !self.is_reserved_keyword(&var_name) {
                                // M402: In @transparent macros, bare identifiers that aren't
                                // defined locally could accidentally capture from caller's scope.
                                // This is dangerous - user should use explicit splice $ or lift().
                                // #[cfg(debug_assertions)]
                                // eprintln!("[DEBUG] M402 triggered for identifier: '{}' (potential capture in @transparent macro)",
//                                    var_name.as_str());
                                violations.push(MetaError::HygieneViolation {
                                    identifier: var_name,
                                    message: Text::from(format!(
                                        "bare identifier '{}' in @transparent macro could accidentally capture from caller's scope; use $ident or lift() to make capture explicit",
                                        token.text.as_str()
                                    )),
                                });
                                return; // Report first violation only
                            } else if !self.is_transparent && !self.is_reserved_keyword(&var_name) {
                                // M408: In non-transparent macros, using an identifier that:
                                // - Is NOT defined locally in the quote
                                // - Is NOT available via meta scope (to splice with $)
                                // - Is NOT a reserved keyword or type name
                                // means the user is referencing a variable without declaring the capture.
                                // This is M408 (CaptureNotDeclared) - the user may have intended to
                                // splice a meta-level binding but forgot to use $
                                // #[cfg(debug_assertions)]
                                // eprintln!("[DEBUG] M408 triggered for identifier: '{}' (undeclared capture)",
//                                    var_name.as_str());
                                violations.push(MetaError::CaptureNotDeclared {
                                    identifier: var_name,
                                    span: token.span,
                                });
                                return; // Report first violation only
                            }
                        }
                    }
                }
                TokenTree::Group { tokens: inner_tokens, delimiter, .. } => {
                    // Check if this is a nested quote block: `quote { ... }`
                    // If so, don't check for double-splice $$ in the nested quote
                    // because $$x is valid for inner quotes to access outer scope
                    // UNLESS we're inside a meta function body (new stage boundary)
                    let is_nested_quote = if i > 0 {
                        if let TokenTree::Token(prev) = &tokens[i - 1] {
                            // The quote keyword can be tokenized as:
                            // - kind=Keyword/Ident, text="quote"
                            // - kind=Punct, text="QuoteKeyword"
                            let is_quote_keyword =
                                (is_keyword_token(prev, "quote"))
                                || (prev.kind == TokenTreeKind::Punct && prev.text.as_str() == "QuoteKeyword");
                            is_quote_keyword && *delimiter == MacroDelimiter::Brace
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    // Check if this is a meta function body: `meta fn name(...) -> Type { ... }`
                    // If so, reset check_double_splice to true because we're entering a new
                    // meta function context where $$ would be escaping past the stage boundary
                    let is_meta_fn_body = if *delimiter == MacroDelimiter::Brace && i >= 2 {
                        // Look back for "meta fn" pattern before this brace group
                        // The pattern could be: meta fn name (...) -> Type { }
                        //                    or: meta fn name (...) { }
                        let mut saw_meta = false;
                        let mut is_meta_fn = false;
                        for j in i.saturating_sub(10)..i {
                            if let TokenTree::Token(t) = &tokens[j] {
                                if is_keyword_token(t, "meta") {
                                    saw_meta = true;
                                } else if saw_meta && is_keyword_token(t, "fn") {
                                    is_meta_fn = true;
                                    break;
                                } else if t.kind == TokenTreeKind::Punct
                                    && t.text.as_str() == ";"
                                {
                                    saw_meta = false; // Reset on semicolon (new statement)
                                }
                            }
                        }
                        is_meta_fn
                    } else {
                        false
                    };

                    #[cfg(debug_assertions)]
                    if is_meta_fn_body {
                        // eprintln!("[DEBUG] Detected meta fn body at group index {}, resetting check_double_splice", i);
                    }
                    #[cfg(debug_assertions)]
                    if is_nested_quote {
                        // eprintln!("[DEBUG] Detected nested quote at group index {}", i);
                    }

                    // Determine check_double_splice for inner tokens:
                    // - If inside a meta fn body, always check for $$ (stage boundary)
                    // - Nested quotes (not in meta fn): disable M407 check ($$x is valid)
                    // - Meta function body: enable M407 check and set inside_meta_fn flag
                    // - Regular groups: keep current value
                    let (inner_check_double_splice, inner_inside_meta_fn) = if is_meta_fn_body {
                        // Entering a meta function body - reset to true and track it
                        (true, true)
                    } else if is_nested_quote {
                        if inside_meta_fn {
                            // Inside a meta fn, nested quotes should still check $$
                            (true, true)
                        } else {
                            // Regular nested quote, disable $$ check
                            (false, false)
                        }
                    } else {
                        (check_double_splice, inside_meta_fn)
                    };

                    self.analyze_token_tree_hygiene_with_locals(
                        inner_tokens,
                        local_bindings,
                        violations,
                        inner_check_double_splice,
                        inner_inside_meta_fn,
                    );
                }
            }
            i += 1;
        }
    }

    /// Analyze a splice expression (${...}) for hygiene violations
    fn analyze_splice_expr(
        &self,
        tokens: &List<verum_ast::expr::TokenTree>,
        span: Span,
        violations: &mut Vec<MetaError>,
    ) {
        use verum_ast::expr::{TokenTree, TokenTreeKind};

        // Look for identifiers in the splice expression
        for token in tokens.iter() {
            match token {
                TokenTree::Token(t) if t.kind == TokenTreeKind::Ident => {
                    let var_name = Text::from(t.text.as_str());
                    // Skip keywords and builtins
                    if !self.is_keyword_or_builtin(&var_name) {
                        // Check if this variable is in scope
                        if self.get(&var_name).is_none() {
                            // M400: Invalid quote syntax - unbound splice variable in ${expr}
                            // Note: This was M408 but should be M400 since it's the same class of error
                            // as $undefined_var - the splice references an undefined variable
                            violations.push(MetaError::InvalidQuoteSyntax {
                                message: Text::from(format!(
                                    "unbound splice variable '{}' is not defined in meta scope",
                                    var_name.as_str()
                                )),
                            });
                            return; // Report first violation
                        }
                    }
                }
                TokenTree::Group { tokens: inner_tokens, .. } => {
                    // Recursively check nested groups
                    self.analyze_splice_expr(inner_tokens, span, violations);
                }
                _ => {}
            }
        }
    }

    /// Check if a name is a keyword or builtin that shouldn't be checked for scope
    fn is_keyword_or_builtin(&self, name: &Text) -> bool {
        let n = name.as_str();
        is_primitive_type_name(n) || matches!(n,
            // Keywords
            "let" | "fn" | "if" | "else" | "match" | "for" | "while" | "loop" |
            "return" | "break" | "continue" | "true" | "false" | "in" | "is" |
            "type" | "implement" | "meta" | "quote" | "using" | "provide" |
            // Common builtins
            "print" | "assert" | "panic" | "unreachable" | "sizeof" | "alignof" |
            // Meta builtins
            "type_name" | "fields_of" | "variants_of" | "ident" | "concat" |
            "stringify" | "lift" | "gensym" | "unquote"
        )
    }

    /// Check if a name is a reserved keyword that cannot be used as a variable name
    /// This excludes contextual keywords like 'result' that can be used as identifiers
    fn is_reserved_keyword(&self, name: &Text) -> bool {
        matches!(name.as_str(),
            // Reserved keywords that cannot be variable names
            "let" | "fn" | "if" | "else" | "match" | "for" | "while" | "loop" |
            "return" | "break" | "continue" | "true" | "false" | "in" | "is" |
            "type" | "implement" | "meta" | "quote" | "using" | "provide" |
            "async" | "await" | "spawn" | "mut" | "const" | "static" |
            "pub" | "public" | "private" | "internal" | "protected" |
            "module" | "protocol" | "extends" | "where" | "as" | "ref" | "move" |
            "unsafe" | "context" | "defer" | "stream" | "yield" | "self" | "Self" |
            variant_tags::NONE | variant_tags::SOME | variant_tags::OK | variant_tags::ERR
            // Note: 'result', 'ensures', 'requires', 'invariant' are contextual keywords
            // that CAN be used as variable names
        )
    }

    /// Check for M409: Repetition length mismatch in $[for ... in zip(var1, var2) {...}]
    ///
    /// This function analyzes the inner tokens of a $[...] repetition block to detect
    /// when zip() is called with arrays of different lengths.
    fn check_repetition_lengths(
        &self,
        tokens: &List<verum_ast::expr::TokenTree>,
        violations: &mut Vec<MetaError>,
    ) {
        use verum_ast::expr::{TokenTree, TokenTreeKind, MacroDelimiter};

        // Look for pattern: for ... in zip(var1, var2) { ... }
        // Tokens should be: for, pattern, in, zip, (args...), { body }

        // Find 'zip' followed by parentheses
        let mut i = 0;
        while i < tokens.len() {
            if let TokenTree::Token(tok) = &tokens[i] {
                if (tok.kind == TokenTreeKind::Ident || tok.kind == TokenTreeKind::Keyword)
                    && tok.text.as_str() == "zip"
                {
                    // Found 'zip', look for the argument list
                    if i + 1 < tokens.len() {
                        if let TokenTree::Group {
                            delimiter: MacroDelimiter::Paren,
                            tokens: args,
                            ..
                        } = &tokens[i + 1]
                        {
                            // Extract variable names from args
                            // Expected format: var1, var2 or (var1, var2)
                            let var_names = self.extract_zip_arg_names(args);

                            // #[cfg(debug_assertions)]
                            // eprintln!("[DEBUG] check_repetition_lengths: zip args = {:?}", var_names);

                            if var_names.len() >= 2 {
                                // Look up each variable and get its length
                                let mut lengths: Vec<(Text, usize)> = Vec::new();

                                for name in &var_names {
                                    if let Some(value) = self.get(name) {
                                        let len = match &value {
                                            ConstValue::Array(arr) => arr.len(),
                                            ConstValue::Tuple(tup) => tup.len(),
                                            _ => continue, // Skip non-iterable values
                                        };
                                        lengths.push((name.clone(), len));
                                    }
                                }

                                // Check if all lengths are the same
                                if lengths.len() >= 2 {
                                    let (first_name, first_len) = &lengths[0];
                                    for (other_name, other_len) in lengths.iter().skip(1) {
                                        if first_len != other_len {
                                            // #[cfg(debug_assertions)]
                                            // eprintln!("[DEBUG] M409 triggered: '{}' has {} elements, '{}' has {}",
//                                                first_name.as_str(), first_len,
//                                                other_name.as_str(), other_len);

                                            violations.push(MetaError::RepetitionMismatch {
                                                first_name: first_name.clone(),
                                                first_len: *first_len,
                                                second_name: other_name.clone(),
                                                second_len: *other_len,
                                            });
                                            return; // Report first mismatch only
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            i += 1;
        }
    }

    /// Extract variable names from zip() arguments
    /// Returns a list of variable names found in the argument tokens
    fn extract_zip_arg_names(
        &self,
        tokens: &List<verum_ast::expr::TokenTree>,
    ) -> Vec<Text> {
        use verum_ast::expr::{TokenTree, TokenTreeKind};

        let mut names = Vec::new();

        for token in tokens.iter() {
            if let TokenTree::Token(tok) = token {
                // Collect identifier tokens that aren't punctuation
                if tok.kind == TokenTreeKind::Ident {
                    names.push(Text::from(tok.text.as_str()));
                }
            }
        }

        names
    }
}

/// Convert a hygiene violation into a user-facing diagnostic.
///
/// The diagnostic carries the violation's M4xx error code (from
/// `HygieneViolation::error_code`) and human-readable message
/// (from `HygieneViolation::message`). Span resolution falls back
/// to the supplied `fallback_span` (the quote expression's outer
/// span) when the violation itself has no concrete location —
/// this happens for the few `HygieneViolation` variants whose
/// `span()` returns the dummy span (e.g. internal-error
/// `GensymCollision` produced without a real source location).
///
/// Severity is `Warning` — hygiene violations are non-fatal by
/// default; an embedder that wants hard-fail wires it through
/// `CheckerConfig::strict_mode`. The session diagnostic emitter
/// honours the severity for cargo build summary lines.
pub(crate) fn hygiene_violation_to_diagnostic(
    v: &crate::hygiene::HygieneViolation,
    fallback_span: verum_ast::span::Span,
) -> verum_diagnostics::Diagnostic {
    use verum_diagnostics::DiagnosticBuilder;
    let v_span = v.span();
    let resolved_span = if v_span.is_dummy() { fallback_span } else { v_span };
    DiagnosticBuilder::warning()
        .code(verum_common::Text::from(v.error_code()))
        .message(v.message())
        .span(crate::phases::ast_span_to_diagnostic_span(resolved_span, None))
        .build()
}

#[cfg(test)]
mod hygiene_diagnostic_tests {
    //! Pin tests for the HygieneViolation → Diagnostic conversion
    //! and the user-diagnostics emission path. Pre-fix violations
    //! only reached `tracing::warn!` — these tests pin that the
    //! conversion preserves M4xx error codes and routes the
    //! per-violation span through the standard span adapter.
    use super::*;
    use crate::hygiene::{HygieneViolation, scope::{HygienicIdent, ScopeSet}};
    use verum_ast::span::Span;
    use verum_common::Text;

    fn dummy_span_at(byte_offset: u32) -> Span {
        // Construct a non-dummy span with a known byte offset so
        // the resolved-span branch can be pinned (the dummy-span
        // fallback case is exercised by a sister test).
        verum_common::span::Span::new(
            byte_offset,
            byte_offset + 1,
            verum_ast::FileId::new(0),
        )
    }

    #[test]
    fn accidental_capture_yields_m402_warning() {
        let violation = HygieneViolation::AccidentalCapture {
            captured: HygienicIdent::new(
                Text::from("foo"),
                ScopeSet::default(),
                dummy_span_at(42),
            ),
            intended_binding: dummy_span_at(10),
            actual_binding: dummy_span_at(20),
        };
        let diag = hygiene_violation_to_diagnostic(&violation, Span::dummy());
        let msg = format!("{:?}", diag);
        assert!(
            msg.contains("M402"),
            "AccidentalCapture must carry the M402 error code (got: {})",
            msg
        );
        assert!(
            msg.contains("foo"),
            "diagnostic must mention the captured identifier (got: {})",
            msg
        );
    }

    #[test]
    fn shadow_conflict_yields_m402_warning() {
        let violation = HygieneViolation::ShadowConflict {
            shadowed: HygienicIdent::new(
                Text::from("bar"),
                ScopeSet::default(),
                dummy_span_at(7),
            ),
            introduced_at: dummy_span_at(7),
        };
        let diag = hygiene_violation_to_diagnostic(&violation, Span::dummy());
        let msg = format!("{:?}", diag);
        assert!(msg.contains("M402"));
        assert!(msg.contains("bar"));
    }

    #[test]
    fn stage_mismatch_yields_m405_warning() {
        let violation = HygieneViolation::StageMismatch {
            expected_stage: 1,
            actual_stage: 0,
            span: dummy_span_at(5),
        };
        let diag = hygiene_violation_to_diagnostic(&violation, Span::dummy());
        let msg = format!("{:?}", diag);
        assert!(msg.contains("M405"));
    }

    #[test]
    fn dummy_violation_span_falls_back_to_outer_quote_span() {
        // GensymCollision constructed with a dummy span — the
        // resolver must fall back to the supplied outer span.
        // We can't pin the rendered byte offset (the LineColSpan
        // adapter resolves spans against registered sources, which
        // are absent in unit-test isolation), but we CAN pin that
        // the fallback path produces a non-dummy primary label —
        // the dummy-span branch in the production code would
        // collapse the label onto a default location instead.
        let violation = HygieneViolation::GensymCollision {
            name: Text::from("x"),
            span: Span::dummy(),
        };
        let outer = dummy_span_at(99);
        let diag = hygiene_violation_to_diagnostic(&violation, outer);
        let msg = format!("{:?}", diag);
        // M403 code present (carried from violation.error_code()).
        assert!(msg.contains("M403"));
        // The diagnostic must have one primary label — the
        // fallback-span branch installs the outer span as the
        // primary location. Pinning label *count* is the stable
        // observable for unit tests without source registration.
        assert!(
            msg.contains("primary_labels: List { inner: [SpanLabel"),
            "diagnostic must carry a primary span label (got: {})",
            msg,
        );
    }

    #[test]
    fn meta_context_diagnostics_accumulates_after_recheck() {
        // End-to-end pin: verify that `recheck_post_splice_hygiene`
        // (the only production caller of the conversion) does NOT
        // error on an empty token tree (no violations → no
        // diagnostics added). This is the regression test for the
        // happy path — the violation-producing path is covered by
        // the unit tests above and the integration test that
        // populates the checker's binding table.
        let mut ctx = MetaContext::new();
        let initial = ctx.diagnostics.len();
        let empty_tokens: List<verum_ast::expr::TokenTree> = List::new();
        ctx.recheck_post_splice_hygiene(&empty_tokens, Span::dummy());
        assert_eq!(
            ctx.diagnostics.len(),
            initial,
            "no violations on empty input → diagnostics count unchanged"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eval_literal() {
        let mut ctx = MetaContext::new();
        let expr = MetaExpr::Literal(ConstValue::Int(42));
        let result = ctx.eval_meta_expr(&expr).unwrap();
        assert_eq!(result, ConstValue::Int(42));
    }

    #[test]
    fn test_eval_variable() {
        let mut ctx = MetaContext::new();
        ctx.bind(Text::from("x"), ConstValue::Int(10));
        let expr = MetaExpr::Variable(Text::from("x"));
        let result = ctx.eval_meta_expr(&expr).unwrap();
        assert_eq!(result, ConstValue::Int(10));
    }

    #[test]
    fn test_eval_if_true() {
        let mut ctx = MetaContext::new();
        let expr = MetaExpr::If {
            condition: Heap::new(MetaExpr::Literal(ConstValue::Bool(true))),
            then_branch: Heap::new(MetaExpr::Literal(ConstValue::Int(1))),
            else_branch: Maybe::Some(Heap::new(MetaExpr::Literal(ConstValue::Int(2)))),
        };
        let result = ctx.eval_meta_expr(&expr).unwrap();
        assert_eq!(result, ConstValue::Int(1));
    }

    #[test]
    fn test_eval_if_false() {
        let mut ctx = MetaContext::new();
        let expr = MetaExpr::If {
            condition: Heap::new(MetaExpr::Literal(ConstValue::Bool(false))),
            then_branch: Heap::new(MetaExpr::Literal(ConstValue::Int(1))),
            else_branch: Maybe::Some(Heap::new(MetaExpr::Literal(ConstValue::Int(2)))),
        };
        let result = ctx.eval_meta_expr(&expr).unwrap();
        assert_eq!(result, ConstValue::Int(2));
    }

    #[test]
    fn test_eval_let() {
        let mut ctx = MetaContext::new();
        let expr = MetaExpr::Let {
            name: Text::from("x"),
            value: Heap::new(MetaExpr::Literal(ConstValue::Int(5))),
            body: Heap::new(MetaExpr::Variable(Text::from("x"))),
        };
        let result = ctx.eval_meta_expr(&expr).unwrap();
        assert_eq!(result, ConstValue::Int(5));
        // x should not be bound after let expression
        assert!(!ctx.has(&Text::from("x")));
    }

    #[test]
    fn test_matches_wildcard() {
        let mut ctx = MetaContext::new();
        assert!(ctx.matches_pattern(&ConstValue::Int(42), &MetaPattern::Wildcard).unwrap());
    }

    #[test]
    fn test_matches_literal() {
        let mut ctx = MetaContext::new();
        assert!(ctx.matches_pattern(&ConstValue::Int(42), &MetaPattern::Literal(ConstValue::Int(42))).unwrap());
        assert!(!ctx.matches_pattern(&ConstValue::Int(42), &MetaPattern::Literal(ConstValue::Int(43))).unwrap());
    }

    #[test]
    fn test_matches_ident_binding() {
        let mut ctx = MetaContext::new();
        assert!(ctx.matches_pattern(&ConstValue::Int(42), &MetaPattern::Ident(Text::from("x"))).unwrap());
        assert_eq!(ctx.get(&Text::from("x")), Some(ConstValue::Int(42)));
    }

    #[test]
    fn test_abs_arity_error() {
        // Test that calling abs with wrong number of arguments returns ArityMismatch error
        let mut ctx = MetaContext::new();
        let expr = MetaExpr::Call(
            Text::from("abs"),
            vec![
                MetaExpr::Literal(ConstValue::Int(1)),
                MetaExpr::Literal(ConstValue::Int(2)),
            ].into_iter().collect()
        );
        let result = ctx.eval_meta_expr(&expr);
        assert!(result.is_err(), "Expected ArityMismatch error");
        match result.unwrap_err() {
            MetaError::ArityMismatch { expected, got } => {
                assert_eq!(expected, 1);
                assert_eq!(got, 2);
            }
            other => panic!("Expected ArityMismatch, got {:?}", other),
        }
    }

    #[test]
    fn test_block_with_call() {
        // Test that a block with a function call evaluates the call
        let mut ctx = MetaContext::new();
        let expr = MetaExpr::Block(vec![
            MetaStmt::Expr(MetaExpr::Call(
                Text::from("abs"),
                vec![
                    MetaExpr::Literal(ConstValue::Int(-5)),
                ].into_iter().collect()
            )),
        ].into_iter().collect());
        let result = ctx.eval_meta_expr(&expr);
        assert!(result.is_ok(), "Expected success, got {:?}", result.err());
        assert_eq!(result.unwrap(), ConstValue::Int(5));
    }

    #[test]
    fn test_block_with_arity_error() {
        // Test that a block with a function call with wrong arity propagates the error
        let mut ctx = MetaContext::new();
        let expr = MetaExpr::Block(vec![
            MetaStmt::Expr(MetaExpr::Call(
                Text::from("abs"),
                vec![
                    MetaExpr::Literal(ConstValue::Int(1)),
                    MetaExpr::Literal(ConstValue::Int(2)),
                ].into_iter().collect()
            )),
        ].into_iter().collect());
        let result = ctx.eval_meta_expr(&expr);
        assert!(result.is_err(), "Expected ArityMismatch error");
    }

    #[test]
    fn test_extract_qualified_path() {
        // Test extracting qualified path from simple path
        let path_expr = Expr::new(
            ExprKind::Path(Path::single(Ident::new("std", Span::dummy()))),
            Span::dummy(),
        );
        assert_eq!(extract_qualified_path(&path_expr), Some("std".to_string()));

        // Test extracting qualified path from field access: std.env
        let field_expr = Expr::new(
            ExprKind::Field {
                expr: Heap::new(Expr::new(
                    ExprKind::Path(Path::single(Ident::new("std", Span::dummy()))),
                    Span::dummy(),
                )),
                field: Ident::new("env", Span::dummy()),
            },
            Span::dummy(),
        );
        assert_eq!(extract_qualified_path(&field_expr), Some("std.env".to_string()));

        // Test extracting qualified path from nested field access: std.env.var
        let nested_field_expr = Expr::new(
            ExprKind::Field {
                expr: Heap::new(Expr::new(
                    ExprKind::Field {
                        expr: Heap::new(Expr::new(
                            ExprKind::Path(Path::single(Ident::new("std", Span::dummy()))),
                            Span::dummy(),
                        )),
                        field: Ident::new("env", Span::dummy()),
                    },
                    Span::dummy(),
                )),
                field: Ident::new("var", Span::dummy()),
            },
            Span::dummy(),
        );
        assert_eq!(extract_qualified_path(&nested_field_expr), Some("std.env.var".to_string()));
    }

    #[test]
    fn test_forbidden_direct_call_detection() {
        // Test that forbidden direct function calls are detected during evaluation
        let mut ctx = MetaContext::new();

        // Create a direct call to http_get (forbidden network operation)
        let call_expr = MetaExpr::Call(
            Text::from("http_get"),
            vec![MetaExpr::Literal(ConstValue::Text(Text::from("http://example.com")))].into_iter().collect(),
        );

        // This should fail with ForbiddenOperation during evaluation
        let result = ctx.eval_meta_expr(&call_expr);
        assert!(result.is_err(), "Expected ForbiddenOperation error for http_get, got {:?}", result);
        match result.unwrap_err() {
            super::MetaError::ForbiddenOperation { operation, .. } => {
                assert_eq!(operation, Text::from("http_get"));
            }
            other => panic!("Expected ForbiddenOperation, got {:?}", other),
        }
    }

    #[test]
    fn test_forbidden_method_call_detection() {
        // Test that forbidden method calls are detected during AST-to-MetaExpr conversion
        let ctx = MetaContext::new();

        // Create AST for std.env.var("HOME")
        // MethodCall { receiver: Field { expr: Path("std"), field: "env" }, method: "var", args: ["HOME"] }
        let method_call = Expr::new(
            ExprKind::MethodCall {
                receiver: Heap::new(Expr::new(
                    ExprKind::Field {
                        expr: Heap::new(Expr::new(
                            ExprKind::Path(Path::single(Ident::new("std", Span::dummy()))),
                            Span::dummy(),
                        )),
                        field: Ident::new("env", Span::dummy()),
                    },
                    Span::dummy(),
                )),
                method: Ident::new("var", Span::dummy()),
                type_args: List::new(),
                args: List::from(vec![
                    Expr::literal(verum_ast::Literal::string("HOME".into(), Span::dummy())),
                ]),
            },
            Span::dummy(),
        );

        // This should fail with ForbiddenOperation during conversion
        let result = ctx.ast_expr_to_meta_expr(&method_call);
        assert!(result.is_err(), "Expected ForbiddenOperation error for std.env.var");
        match result.unwrap_err() {
            super::MetaError::ForbiddenOperation { operation, .. } => {
                assert_eq!(operation, Text::from("std.env.var"));
            }
            other => panic!("Expected ForbiddenOperation, got {:?}", other),
        }
    }

}
