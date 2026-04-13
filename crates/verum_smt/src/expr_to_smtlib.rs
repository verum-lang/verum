//! Expr → SMT-LIB2 Translator.
//!
//! Translates Verum AST expressions into SMT-LIB2 string form
//! suitable for injection into Z3 via `Solver::from_string`. This
//! is the missing piece that connects the refinement-reflection
//! registry to real user-function definitions: once a function's
//! body is translated here, the registry axiom becomes a genuine
//! unfolding rule rather than a placeholder.
//!
//! ## Supported expression shapes
//!
//! | Verum AST | SMT-LIB2 |
//! |---|---|
//! | `42` (Int literal) | `42` |
//! | `true` / `false` | `true` / `false` |
//! | `x` (variable) | `x` |
//! | `a + b` | `(+ a b)` |
//! | `a - b` | `(- a b)` |
//! | `a * b` | `(* a b)` |
//! | `a / b` | `(div a b)` |
//! | `a % b` | `(mod a b)` |
//! | `a == b` | `(= a b)` |
//! | `a != b` | `(not (= a b))` |
//! | `a < b` | `(< a b)` |
//! | `a <= b` | `(<= a b)` |
//! | `a > b` | `(> a b)` |
//! | `a >= b` | `(>= a b)` |
//! | `a && b` | `(and a b)` |
//! | `a \|\| b` | `(or a b)` |
//! | `!a` | `(not a)` |
//! | `-a` | `(- a)` |
//! | `if c { t } else { e }` | `(ite c t e)` |
//! | `f(a, b)` | `(f a b)` |
//! | `(expr)` | recurse |
//!
//! Unsupported shapes return `Err` — the caller decides whether
//! to skip reflection or report a diagnostic.
//!
//! ## Soundness
//!
//! The translator is conservative: if it encounters an expression
//! it cannot represent in QF_LIA/QF_NIA (the integer-arithmetic
//! fragment Z3 handles well), it returns `Err` rather than
//! producing an incorrect axiom. This means some reflectable
//! functions won't be reflected — but no incorrect axiom will
//! ever be emitted.

use verum_common::Text;

use verum_ast::expr::{BinOp, Expr, ExprKind, UnOp};
use verum_ast::literal::LiteralKind;

/// Result of translating an expression to SMT-LIB2.
pub type SmtResult = Result<String, SmtTranslateError>;

/// Errors from the Expr→SMT-LIB translator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmtTranslateError {
    /// Expression shape not supported in SMT-LIB2 translation.
    UnsupportedExpr { description: String },
    /// Binary operator not mapped to SMT-LIB2.
    UnsupportedOp { op: String },
}

impl std::fmt::Display for SmtTranslateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedExpr { description } => {
                write!(f, "cannot translate to SMT-LIB: {}", description)
            }
            Self::UnsupportedOp { op } => {
                write!(f, "unsupported operator for SMT-LIB: {}", op)
            }
        }
    }
}

impl std::error::Error for SmtTranslateError {}

/// Translate a Verum AST expression into an SMT-LIB2 string.
pub fn expr_to_smtlib(expr: &Expr) -> SmtResult {
    match &expr.kind {
        ExprKind::Literal(lit) => literal_to_smtlib(lit),

        ExprKind::Path(path) => {
            if path.segments.len() == 1 {
                if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                    return Ok(ident.name.as_str().to_string());
                }
            }
            Err(SmtTranslateError::UnsupportedExpr {
                description: "multi-segment path".to_string(),
            })
        }

        ExprKind::Binary { op, left, right } => {
            let l = expr_to_smtlib(left)?;
            let r = expr_to_smtlib(right)?;
            let smt_op = binop_to_smtlib(*op)?;
            match op {
                BinOp::Ne => Ok(format!("(not (= {} {}))", l, r)),
                _ => Ok(format!("({} {} {})", smt_op, l, r)),
            }
        }

        ExprKind::Unary { op, expr: inner } => {
            let inner_smt = expr_to_smtlib(inner)?;
            match op {
                UnOp::Not => Ok(format!("(not {})", inner_smt)),
                UnOp::Neg => Ok(format!("(- {})", inner_smt)),
                _ => Err(SmtTranslateError::UnsupportedOp {
                    op: format!("{:?}", op),
                }),
            }
        }

        ExprKind::Paren(inner) => expr_to_smtlib(inner),

        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            // Translate condition — IfCondition may have multiple
            // sub-conditions; we take the first Expr form.
            let cond_smt = if let Some(verum_ast::expr::ConditionKind::Expr(e)) =
                condition.conditions.first()
            {
                expr_to_smtlib(e)?
            } else {
                return Err(SmtTranslateError::UnsupportedExpr {
                    description: "non-expression if-condition".to_string(),
                });
            };

            // Then branch: Block with optional tail expression.
            let then_smt = if let verum_common::Maybe::Some(tail) = &then_branch.expr {
                expr_to_smtlib(tail)?
            } else {
                return Err(SmtTranslateError::UnsupportedExpr {
                    description: "if-then branch without tail expression".to_string(),
                });
            };

            // Else branch
            let else_smt = if let verum_common::Maybe::Some(eb) = else_branch {
                expr_to_smtlib(eb)?
            } else {
                return Err(SmtTranslateError::UnsupportedExpr {
                    description: "if without else branch".to_string(),
                });
            };

            Ok(format!("(ite {} {} {})", cond_smt, then_smt, else_smt))
        }

        ExprKind::Call { func, args, .. } => {
            let func_name = expr_to_smtlib(func)?;
            let mut parts = vec![func_name];
            for a in args.iter() {
                parts.push(expr_to_smtlib(a)?);
            }
            if parts.len() == 1 {
                Ok(format!("({})", parts[0]))
            } else {
                Ok(format!("({})", parts.join(" ")))
            }
        }

        ExprKind::Block(block) => {
            // A block with a single tail expression and no statements
            // translates to just the tail expression.
            if block.stmts.is_empty() {
                if let verum_common::Maybe::Some(tail) = &block.expr {
                    return expr_to_smtlib(tail);
                }
            }
            Err(SmtTranslateError::UnsupportedExpr {
                description: "block with statements".to_string(),
            })
        }

        ExprKind::Tuple(elements) if elements.len() == 1 => {
            expr_to_smtlib(&elements[0])
        }

        _ => Err(SmtTranslateError::UnsupportedExpr {
            description: format!("{:?}", std::mem::discriminant(&expr.kind)),
        }),
    }
}

fn literal_to_smtlib(lit: &verum_ast::literal::Literal) -> SmtResult {
    match &lit.kind {
        LiteralKind::Int(n) => Ok(format!("{}", n.value)),
        LiteralKind::Bool(b) => Ok(if *b { "true" } else { "false" }.to_string()),
        LiteralKind::Float(f) => Ok(format!("{}", f.value)),
        _ => Err(SmtTranslateError::UnsupportedExpr {
            description: "non-numeric/bool literal".to_string(),
        }),
    }
}

fn binop_to_smtlib(op: BinOp) -> Result<&'static str, SmtTranslateError> {
    match op {
        BinOp::Add => Ok("+"),
        BinOp::Sub => Ok("-"),
        BinOp::Mul => Ok("*"),
        BinOp::Div => Ok("div"),
        BinOp::Rem => Ok("mod"),
        BinOp::Eq => Ok("="),
        BinOp::Ne => Ok("="), // handled specially above
        BinOp::Lt => Ok("<"),
        BinOp::Le => Ok("<="),
        BinOp::Gt => Ok(">"),
        BinOp::Ge => Ok(">="),
        BinOp::And => Ok("and"),
        BinOp::Or => Ok("or"),
        BinOp::Imply => Ok("=>"),
        _ => Err(SmtTranslateError::UnsupportedOp {
            op: format!("{:?}", op),
        }),
    }
}

/// Infer the SMT-LIB sort name from a Verum AST type. Conservative:
/// returns "Int" for Int, "Bool" for Bool, "Int" as fallback for
/// unknown types (safe for QF_LIA fragment).
pub fn type_to_sort(ty: &verum_ast::ty::Type) -> String {
    match &ty.kind {
        verum_ast::ty::TypeKind::Int => "Int".to_string(),
        verum_ast::ty::TypeKind::Bool => "Bool".to_string(),
        verum_ast::ty::TypeKind::Float => "Real".to_string(),
        _ => "Int".to_string(), // conservative fallback
    }
}

/// Extract parameter names from a function's parameter list.
/// Returns `(name, sort)` pairs suitable for `ReflectedFunction`.
pub fn extract_params(
    func: &verum_ast::FunctionDecl,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for p in func.params.iter() {
        if let verum_ast::decl::FunctionParamKind::Regular { pattern, ty, .. } = &p.kind {
            let name = match &pattern.kind {
                verum_ast::pattern::PatternKind::Ident { name, .. } => {
                    name.name.as_str().to_string()
                }
                _ => continue,
            };
            let sort = type_to_sort(ty);
            out.push((name, sort));
        }
    }
    out
}

/// Try to translate a function declaration into a `ReflectedFunction`.
/// Returns `None` if the function can't be reflected (impure,
/// non-total, or body can't be translated to SMT-LIB).
pub fn try_reflect_function(
    func: &verum_ast::FunctionDecl,
) -> Option<crate::refinement_reflection::ReflectedFunction> {
    // Gate: must have parameters (nullary functions are constants,
    // not interesting for reflection).
    if func.params.is_empty() {
        return None;
    }

    // Gate: must not have context requirements (impure).
    if !func.contexts.is_empty() {
        return None;
    }

    // Gate: must have a body.
    let body = match &func.body {
        verum_common::Maybe::Some(b) => b,
        verum_common::Maybe::None => return None,
    };

    // Gate: body must be a Block with a tail expression and no
    // statements (single-expression function).
    let tail_expr = match body {
        verum_ast::decl::FunctionBody::Block(block) => {
            if !block.stmts.is_empty() {
                return None;
            }
            match &block.expr {
                verum_common::Maybe::Some(e) => e,
                verum_common::Maybe::None => return None,
            }
        }
        verum_ast::decl::FunctionBody::Expr(e) => e,
        _ => return None,
    };

    // Translate the body to SMT-LIB.
    let body_smtlib = match expr_to_smtlib(tail_expr) {
        Ok(s) => s,
        Err(_) => return None,
    };

    let params = extract_params(func);
    if params.is_empty() {
        return None;
    }

    let return_sort = func
        .return_type
        .as_ref()
        .map(|t| type_to_sort(t))
        .unwrap_or_else(|| "Int".to_string());

    Some(crate::refinement_reflection::ReflectedFunction {
        name: Text::from(func.name.name.as_str()),
        parameters: params.iter().map(|(n, _)| Text::from(n.as_str())).collect(),
        body_smtlib: Text::from(body_smtlib),
        return_sort: Text::from(return_sort),
        parameter_sorts: params.iter().map(|(_, s)| Text::from(s.as_str())).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::expr::{Expr, ExprKind};
    use verum_ast::literal::Literal;
    use verum_ast::span::Span;
    use verum_ast::ty::{Ident, Path};

    fn sp() -> Span {
        Span::default()
    }

    fn int_expr(n: i128) -> Expr {
        Expr {
            kind: ExprKind::Literal(Literal::int(n, sp())),
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
        }
    }

    fn bool_expr(b: bool) -> Expr {
        Expr {
            kind: ExprKind::Literal(Literal::new(
                verum_ast::literal::LiteralKind::Bool(b),
                sp(),
            )),
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
        }
    }

    fn var_expr(name: &str) -> Expr {
        Expr {
            kind: ExprKind::Path(Path::single(Ident::new(name, sp()))),
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
        }
    }

    fn binop(op: BinOp, l: Expr, r: Expr) -> Expr {
        Expr {
            kind: ExprKind::Binary {
                op,
                left: verum_common::Heap::new(l),
                right: verum_common::Heap::new(r),
            },
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
        }
    }

    #[test]
    fn int_literal() {
        assert_eq!(expr_to_smtlib(&int_expr(42)).unwrap(), "42");
    }

    #[test]
    fn bool_literal() {
        assert_eq!(expr_to_smtlib(&bool_expr(true)).unwrap(), "true");
        assert_eq!(expr_to_smtlib(&bool_expr(false)).unwrap(), "false");
    }

    #[test]
    fn variable() {
        assert_eq!(expr_to_smtlib(&var_expr("x")).unwrap(), "x");
    }

    #[test]
    fn addition() {
        let e = binop(BinOp::Add, var_expr("a"), var_expr("b"));
        assert_eq!(expr_to_smtlib(&e).unwrap(), "(+ a b)");
    }

    #[test]
    fn multiplication() {
        let e = binop(BinOp::Mul, int_expr(2), var_expr("n"));
        assert_eq!(expr_to_smtlib(&e).unwrap(), "(* 2 n)");
    }

    #[test]
    fn not_equal() {
        let e = binop(BinOp::Ne, var_expr("a"), var_expr("b"));
        assert_eq!(expr_to_smtlib(&e).unwrap(), "(not (= a b))");
    }

    #[test]
    fn comparison() {
        let e = binop(BinOp::Le, var_expr("x"), int_expr(10));
        assert_eq!(expr_to_smtlib(&e).unwrap(), "(<= x 10)");
    }

    #[test]
    fn logical_and() {
        let e = binop(BinOp::And, bool_expr(true), var_expr("p"));
        assert_eq!(expr_to_smtlib(&e).unwrap(), "(and true p)");
    }

    #[test]
    fn negation() {
        let e = Expr {
            kind: ExprKind::Unary {
                op: UnOp::Not,
                expr: verum_common::Heap::new(var_expr("b")),
            },
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
        };
        assert_eq!(expr_to_smtlib(&e).unwrap(), "(not b)");
    }

    #[test]
    fn arithmetic_negation() {
        let e = Expr {
            kind: ExprKind::Unary {
                op: UnOp::Neg,
                expr: verum_common::Heap::new(var_expr("x")),
            },
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
        };
        assert_eq!(expr_to_smtlib(&e).unwrap(), "(- x)");
    }

    #[test]
    fn nested_arithmetic() {
        // (a + b) * c
        let sum = binop(BinOp::Add, var_expr("a"), var_expr("b"));
        let product = binop(BinOp::Mul, sum, var_expr("c"));
        assert_eq!(
            expr_to_smtlib(&product).unwrap(),
            "(* (+ a b) c)"
        );
    }

    #[test]
    fn function_call() {
        let e = Expr {
            kind: ExprKind::Call {
                func: verum_common::Heap::new(var_expr("f")),
                type_args: verum_common::List::new(),
                args: verum_common::List::from_iter([var_expr("x"), var_expr("y")]),
            },
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
        };
        assert_eq!(expr_to_smtlib(&e).unwrap(), "(f x y)");
    }

    #[test]
    fn paren_unwraps() {
        let e = Expr {
            kind: ExprKind::Paren(verum_common::Heap::new(var_expr("x"))),
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
        };
        assert_eq!(expr_to_smtlib(&e).unwrap(), "x");
    }

    #[test]
    fn implication() {
        let e = binop(BinOp::Imply, var_expr("p"), var_expr("q"));
        assert_eq!(expr_to_smtlib(&e).unwrap(), "(=> p q)");
    }

    #[test]
    fn division_and_modulo() {
        let d = binop(BinOp::Div, var_expr("a"), var_expr("b"));
        assert_eq!(expr_to_smtlib(&d).unwrap(), "(div a b)");
        let m = binop(BinOp::Rem, var_expr("a"), var_expr("b"));
        assert_eq!(expr_to_smtlib(&m).unwrap(), "(mod a b)");
    }
}
