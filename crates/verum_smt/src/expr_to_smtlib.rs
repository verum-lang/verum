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

/// Signature of one member (protocol/impl method) for reflection.
#[derive(Debug, Clone)]
pub struct MemberSig {
    /// Sorts of the explicit arguments (self excluded).
    pub arg_sorts: Vec<String>,
    /// Sort of the return value.
    pub ret_sort: String,
    /// The return type's NAME when it is a named (opaque) type —
    /// this is what lets a chained call keep resolving members.
    pub ret_type_name: Option<String>,
}

/// Module-level type facts the member arms translate against.
///
/// The core translator is deliberately context-free; member access
/// (`w.field`, `p.cond()`) needs to know the receiver's TYPE to name
/// its projection symbol, so the reflection entry points thread this
/// env through. An empty env refuses every member expression — the
/// pre-env behaviour, byte for byte.
#[derive(Debug, Clone, Default)]
pub struct ReflectionTypeEnv {
    /// Value name (function parameter) → named type.
    pub bindings: std::collections::HashMap<String, String>,
    /// Record type → field → (sort, named type when opaque).
    pub record_fields:
        std::collections::HashMap<String, std::collections::HashMap<String, (String, Option<String>)>>,
    /// Type (record, protocol, impl target) → method → signature.
    pub methods: std::collections::HashMap<String, std::collections::HashMap<String, MemberSig>>,
}

impl ReflectionTypeEnv {
    /// Harvest record fields and protocol/impl method signatures from
    /// a module — the same single-file scope the reflection scan
    /// itself walks.
    pub fn from_module(module: &verum_ast::Module) -> Self {
        use verum_ast::ItemKind;
        use verum_ast::decl::{ImplItemKind, ImplKind, ProtocolItemKind, TypeDeclBody};
        let mut env = Self::default();

        let mut add_methods =
            |env: &mut Self, type_name: &str, funcs: &mut dyn Iterator<Item = &verum_ast::FunctionDecl>| {
                for fd in funcs {
                    let arg_sorts: Vec<String> = fd
                        .params
                        .iter()
                        .filter_map(|p| match &p.kind {
                            verum_ast::decl::FunctionParamKind::Regular { ty, .. } => {
                                Some(type_to_sort(ty))
                            }
                            _ => None,
                        })
                        .collect();
                    let (ret_sort, ret_type_name) = fd
                        .return_type
                        .as_ref()
                        .map(type_to_sort_and_name)
                        .unwrap_or_else(|| ("Bool".to_string(), None));
                    env.methods
                        .entry(type_name.to_string())
                        .or_default()
                        .insert(
                            fd.name.name.as_str().to_string(),
                            MemberSig {
                                arg_sorts,
                                ret_sort,
                                ret_type_name,
                            },
                        );
                }
            };

        for item in &module.items {
            match &item.kind {
                ItemKind::Type(td) => match &td.body {
                    TypeDeclBody::Record(fields) => {
                        let m = env
                            .record_fields
                            .entry(td.name.name.as_str().to_string())
                            .or_default();
                        for f in fields.iter() {
                            m.insert(
                                f.name.name.as_str().to_string(),
                                type_to_sort_and_name(&f.ty),
                            );
                        }
                    }
                    TypeDeclBody::Protocol(p) => {
                        let mut fns = p.items.iter().filter_map(|it| match &it.kind {
                            ProtocolItemKind::Function { decl, .. } => Some(decl),
                            _ => None,
                        });
                        add_methods(&mut env, td.name.name.as_str(), &mut fns);
                    }
                    _ => {}
                },
                ItemKind::Protocol(p) => {
                    let mut fns = p.items.iter().filter_map(|it| match &it.kind {
                        ProtocolItemKind::Function { decl, .. } => Some(decl),
                        _ => None,
                    });
                    add_methods(&mut env, p.name.name.as_str(), &mut fns);
                }
                ItemKind::Impl(imp) => {
                    let target = match &imp.kind {
                        ImplKind::Inherent(ty) => type_to_sort_and_name(ty).1,
                        ImplKind::Protocol { for_type, .. } => type_to_sort_and_name(for_type).1,
                    };
                    if let Some(target) = target {
                        let mut fns = imp.items.iter().filter_map(|it| match &it.kind {
                            ImplItemKind::Function(fd) => Some(fd),
                            _ => None,
                        });
                        add_methods(&mut env, target.as_str(), &mut fns);
                    }
                }
                _ => {}
            }
        }
        env
    }
}

/// The named type of a member-bearing expression, resolved through
/// the env: a parameter carries its declared type; a field carries
/// the record's field type; a method call carries its return type.
fn member_type_name(expr: &Expr, env: &ReflectionTypeEnv) -> Option<String> {
    match &expr.kind {
        ExprKind::Path(p) => {
            let id = p.as_ident()?;
            env.bindings.get(id.as_str()).cloned()
        }
        ExprKind::Field { expr: obj, field } => {
            let t = member_type_name(obj, env)?;
            env.record_fields
                .get(&t)?
                .get(field.name.as_str())?
                .1
                .clone()
        }
        ExprKind::MethodCall {
            receiver, method, ..
        } => {
            let t = member_type_name(receiver, env)?;
            env.methods
                .get(&t)?
                .get(method.name.as_str())?
                .ret_type_name
                .clone()
        }
        ExprKind::Paren(inner) => member_type_name(inner, env),
        _ => None,
    }
}

/// Note an auxiliary declaration; `Verum!`-prefixed sorts get their
/// `declare-sort` alongside. BTreeSet keeps emission deterministic.
fn note_decl(aux: &mut std::collections::BTreeSet<String>, decl: String) {
    aux.insert(decl);
}

fn note_sort(aux: &mut std::collections::BTreeSet<String>, sort: &str) {
    if sort.starts_with("Verum!") {
        // NOTE: lexicographically "(declare-fun" < "(declare-sort",
        // so raw BTreeSet order would emit uses before declarations —
        // the registry's block renderer partitions sorts first.
        aux.insert(format!("(declare-sort {} 0)", sort));
    }
}

/// Translate a Verum AST expression into an SMT-LIB2 string.
///
/// Context-free wrapper: member expressions (field access, method
/// calls) need [`ReflectionTypeEnv`] facts and are refused here.
pub fn expr_to_smtlib(expr: &Expr) -> SmtResult {
    let env = ReflectionTypeEnv::default();
    let mut aux = std::collections::BTreeSet::new();
    expr_to_smtlib_env(expr, &env, &mut aux)
}

/// Translate with module type facts; auxiliary `declare-sort` /
/// `declare-fun` lines for projection symbols accumulate in `aux`.
pub fn expr_to_smtlib_env(
    expr: &Expr,
    env: &ReflectionTypeEnv,
    aux: &mut std::collections::BTreeSet<String>,
) -> SmtResult {
    match &expr.kind {
        ExprKind::Literal(lit) => literal_to_smtlib(lit),

        ExprKind::Path(path) => path_to_smtlib(path),

        ExprKind::Binary { op, left, right } => {
            let l = expr_to_smtlib_env(left, env, aux)?;
            let r = expr_to_smtlib_env(right, env, aux)?;
            let smt_op = binop_to_smtlib(*op)?;
            match op {
                BinOp::Ne => Ok(format!("(not (= {} {}))", l, r)),
                _ => Ok(format!("({} {} {})", smt_op, l, r)),
            }
        }

        ExprKind::Unary { op, expr: inner } => {
            let inner_smt = expr_to_smtlib_env(inner, env, aux)?;
            match op {
                UnOp::Not => Ok(format!("(not {})", inner_smt)),
                UnOp::Neg => Ok(format!("(- {})", inner_smt)),
                _ => Err(SmtTranslateError::UnsupportedOp {
                    op: format!("{:?}", op),
                }),
            }
        }

        ExprKind::Paren(inner) => expr_to_smtlib_env(inner, env, aux),

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
                expr_to_smtlib_env(e, env, aux)?
            } else {
                return Err(SmtTranslateError::UnsupportedExpr {
                    description: "non-expression if-condition".to_string(),
                });
            };

            // Then branch: Block with optional tail expression.
            let then_smt = if let verum_common::Maybe::Some(tail) = &then_branch.expr {
                expr_to_smtlib_env(tail, env, aux)?
            } else {
                return Err(SmtTranslateError::UnsupportedExpr {
                    description: "if-then branch without tail expression".to_string(),
                });
            };

            // Else branch
            let else_smt = if let verum_common::Maybe::Some(eb) = else_branch {
                expr_to_smtlib_env(eb, env, aux)?
            } else {
                return Err(SmtTranslateError::UnsupportedExpr {
                    description: "if without else branch".to_string(),
                });
            };

            Ok(format!("(ite {} {} {})", cond_smt, then_smt, else_smt))
        }

        ExprKind::Call { func, args, .. } => {
            // **#161 V3 fast path** — recognise separation-logic
            // predicate calls and route through the structured
            // `sep_*` SMT-LIB rendering so downstream Z3 setup can
            // dispatch on the prefix to install the matching theory.
            // Generic opaque-function translation is the fallback
            // (preserves pre-V3 behaviour for non-separation calls).
            if let Some(result) =
                crate::separation_recognizer::try_translate_sep_predicate_to_smtlib(expr)
            {
                return result;
            }
            let func_name = expr_to_smtlib_env(func, env, aux)?;
            let mut parts = vec![func_name];
            for a in args.iter() {
                parts.push(expr_to_smtlib_env(a, env, aux)?);
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
                    return expr_to_smtlib_env(tail, env, aux);
                }
            }
            Err(SmtTranslateError::UnsupportedExpr {
                description: "block with statements".to_string(),
            })
        }

        ExprKind::Tuple(elements) if elements.len() == 1 => expr_to_smtlib_env(&elements[0], env, aux),

        // Enum-dispatch reflection: `match k { K.A => e1, K.B => e2 }` over a
        // variant scrutinee becomes a right-to-left `ite` chain, each arm
        // guarded by `(= k path_K.A)` — byte-identical to the trusted Z3-AST
        // translator (`translate.rs`) and the variant-disjointness axioms, so
        // the reflected body and the goal name the same solver constant.
        //
        // SOUNDNESS: this body is emitted as an axiom asserted TRUE, so —
        // unlike the goal-side `translate_match`, which may safely
        // over-approximate a match it is trying to prove — every arm must be
        // represented EXACTLY or the whole match is refused. A guard, a
        // payload-binding variant pattern, or any non-nullary-variant pattern
        // makes this `Err` (conservative-refuse is always sound).
        ExprKind::Match { expr: scrut, arms } => {
            use verum_ast::pattern::PatternKind;
            let scrut_smt = expr_to_smtlib_env(scrut, env, aux)?;
            let mut chain: Option<String> = None;
            for arm in arms.iter().rev() {
                if arm.guard.is_some() {
                    return Err(SmtTranslateError::UnsupportedExpr {
                        description: "guarded match arm".to_string(),
                    });
                }
                let body_smt = expr_to_smtlib_env(&arm.body, env, aux)?;
                match &arm.pattern.kind {
                    // Binds anything → the fallthrough (else) branch.
                    PatternKind::Wildcard | PatternKind::Ident { .. } => {
                        chain = Some(body_smt);
                    }
                    // A nullary variant `K.A`. A payload (`Some(x)`) has no SMT
                    // form for `x`, so refuse rather than emit a wrong body.
                    PatternKind::Variant { path, data } => {
                        if data.is_some() {
                            return Err(SmtTranslateError::UnsupportedExpr {
                                description: "variant pattern with payload".to_string(),
                            });
                        }
                        let cond = format!("(= {} {})", scrut_smt, path_to_smtlib(path)?);
                        let existing = chain.clone().unwrap_or_else(|| body_smt.clone());
                        chain = Some(format!("(ite {} {} {})", cond, body_smt, existing));
                    }
                    // A nullary variant sometimes parses as an empty record.
                    PatternKind::Record { path, fields, .. } if fields.is_empty() => {
                        let cond = format!("(= {} {})", scrut_smt, path_to_smtlib(path)?);
                        let existing = chain.clone().unwrap_or_else(|| body_smt.clone());
                        chain = Some(format!("(ite {} {} {})", cond, body_smt, existing));
                    }
                    _ => {
                        return Err(SmtTranslateError::UnsupportedExpr {
                            description: "non-nullary-variant match pattern".to_string(),
                        });
                    }
                }
            }
            chain.ok_or_else(|| SmtTranslateError::UnsupportedExpr {
                description: "match with no arms".to_string(),
            })
        }

        // Record-field projection: `w.field` where `w`'s named type is
        // known to the env becomes an application of the projection
        // symbol `Verum!proj!<Type>!<field>`, declared alongside as an
        // uninterpreted function over the type's opaque sort. The
        // solver learns nothing about the field's VALUE — only that
        // the same receiver always projects to the same value, which
        // is exactly what lets a reflected field-conjunction body and
        // a hypothesis about the same receiver meet.
        ExprKind::Field { expr: obj, field } => {
            let tn = member_type_name(obj, env).ok_or_else(|| SmtTranslateError::UnsupportedExpr {
                description: format!(
                    "field access on a value whose named type is unknown to reflection: .{}",
                    field.name.as_str()
                ),
            })?;
            let (fsort, _) = env
                .record_fields
                .get(&tn)
                .and_then(|m| m.get(field.name.as_str()))
                .cloned()
                .ok_or_else(|| SmtTranslateError::UnsupportedExpr {
                    description: format!("unknown field {}.{}", tn, field.name.as_str()),
                })?;
            let recv = expr_to_smtlib_env(obj, env, aux)?;
            let tsort = crate::solver_symbols::opaque_sort(&tn);
            note_sort(aux, &tsort);
            note_sort(aux, &fsort);
            let proj = crate::solver_symbols::projection(&tn, field.name.as_str());
            note_decl(
                aux,
                format!("(declare-fun {} ({}) {})", proj, tsort, fsort),
            );
            Ok(format!("({} {})", proj, recv))
        }

        // Protocol/impl method call: `p.cond()` (chains included —
        // the receiver's type is resolved through method return
        // types) becomes `(Verum!method!<Type>!<name> recv args…)`,
        // an uninterpreted function of the receiver and arguments.
        ExprKind::MethodCall {
            receiver,
            method,
            args,
            ..
        } => {
            let tn = member_type_name(receiver, env).ok_or_else(|| {
                SmtTranslateError::UnsupportedExpr {
                    description: format!(
                        "method call on a value whose named type is unknown to reflection: .{}()",
                        method.name.as_str()
                    ),
                }
            })?;
            let sig = env
                .methods
                .get(&tn)
                .and_then(|m| m.get(method.name.as_str()))
                .cloned()
                .ok_or_else(|| SmtTranslateError::UnsupportedExpr {
                    description: format!("unknown method {}.{}()", tn, method.name.as_str()),
                })?;
            if sig.arg_sorts.len() != args.len() {
                return Err(SmtTranslateError::UnsupportedExpr {
                    description: format!(
                        "method {}.{}() arity mismatch: declared {}, called with {}",
                        tn,
                        method.name.as_str(),
                        sig.arg_sorts.len(),
                        args.len()
                    ),
                });
            }
            let recv = expr_to_smtlib_env(receiver, env, aux)?;
            let tsort = crate::solver_symbols::opaque_sort(&tn);
            note_sort(aux, &tsort);
            note_sort(aux, &sig.ret_sort);
            for s in &sig.arg_sorts {
                note_sort(aux, s);
            }
            let m = crate::solver_symbols::method(&tn, method.name.as_str());
            let mut decl_args = vec![tsort];
            decl_args.extend(sig.arg_sorts.iter().cloned());
            note_decl(
                aux,
                format!(
                    "(declare-fun {} ({}) {})",
                    m,
                    decl_args.join(" "),
                    sig.ret_sort
                ),
            );
            let mut parts = vec![m, recv];
            for a in args.iter() {
                parts.push(expr_to_smtlib_env(a, env, aux)?);
            }
            Ok(format!("({})", parts.join(" ")))
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

/// Infer the SMT-LIB sort name from a Verum AST type.
///
/// Mirrors `translate.rs::create_var` family-by-family — the two
/// translators MUST spell the same sort for the same type, or a
/// reflected `(declare-fun P (S) Bool)` and the goal-side application
/// of `P` to a variable of that type name conflicting symbols and Z3
/// treats them as distinct uninterpreted functions. The old body
/// substituted `Int` for anything it did not know — the exact sibling
/// defect the `create_var` opaque arm names in its comment: over a
/// list-shaped `Int`, `xs.len() > 0` becomes arithmetic that means
/// nothing and the solver can then "prove" it. A named type the
/// translator does not model is opaque UNDER ITS OWN NAME
/// (`Verum!<Name>`, byte-identical to `create_var`), never a scalar.
pub fn type_to_sort(ty: &verum_ast::ty::Type) -> String {
    type_to_sort_and_name(ty).0
}

/// [`type_to_sort`] plus the NAMED-type identity when the sort is an
/// opaque `Verum!<Name>` — the name is what member translation keys
/// record-field and protocol-method lookups by.
pub fn type_to_sort_and_name(ty: &verum_ast::ty::Type) -> (String, Option<String>) {
    use verum_ast::ty::TypeKind;
    match &ty.kind {
        TypeKind::Int => ("Int".to_string(), None),
        TypeKind::Bool => ("Bool".to_string(), None),
        TypeKind::Float => ("Real".to_string(), None),
        TypeKind::Text => ("String".to_string(), None),
        // Refinement and reference wrappers translate as their base:
        // a `&T` parameter carries T's facts, and `Int{ it > 0 }` is
        // an Int with an assumption, not a new sort.
        TypeKind::Refined { base, .. } => type_to_sort_and_name(base),
        TypeKind::Reference { inner, .. } => type_to_sort_and_name(inner),
        TypeKind::Path(path) => {
            let Some(ident) = path.as_ident() else {
                return ("Verum!Path".to_string(), None);
            };
            let tn = ident.as_str();
            if verum_common::well_known_types::type_names::is_integer_type(tn) {
                return ("Int".to_string(), None);
            }
            if verum_common::well_known_types::type_names::is_float_type(tn) {
                return ("Real".to_string(), None);
            }
            match tn {
                "Bool" | "bool" => ("Bool".to_string(), None),
                "Text" | "String" | "str" => ("String".to_string(), None),
                other => (
                    crate::solver_symbols::opaque_sort(other),
                    Some(other.to_string()),
                ),
            }
        }
        // Everything else is opaque under its SHAPE tag — the same
        // authority (`translate::type_kind_tag`) the Z3-AST side's
        // catch-all consults, so both spell the same sort.
        other => (
            crate::solver_symbols::opaque_sort(crate::translate::type_kind_tag(other)),
            None,
        ),
    }
}

/// Translate a path to its SMT-LIB symbol. A single `Name` segment is the
/// bare identifier (`xs`); a 2+-segment variant path `K.A` becomes the
/// canonical `path_K.A` constant — byte-identical to the Z3-AST translator
/// (`translate.rs::translate_expr`, `format!("path_{}", segs.join("."))`) and
/// to `variant_disjointness_axioms`, so a reflected body and the goal it feeds
/// name the *same* Int solver constant. (Dots are legal SMT-LIB symbol chars.)
fn path_to_smtlib(path: &verum_ast::ty::Path) -> SmtResult {
    let mut names: Vec<&str> = Vec::new();
    for seg in path.segments.iter() {
        match seg {
            verum_ast::ty::PathSegment::Name(ident) => names.push(ident.name.as_str()),
            _ => {
                return Err(SmtTranslateError::UnsupportedExpr {
                    description: "non-name path segment".to_string(),
                })
            }
        }
    }
    match names.len() {
        0 => Err(SmtTranslateError::UnsupportedExpr {
            description: "empty path".to_string(),
        }),
        1 => Ok(names[0].to_string()),
        _ => Ok(format!("path_{}", names.join("."))),
    }
}

/// Extract parameter names from a function's parameter list.
/// Returns `(name, sort)` pairs suitable for `ReflectedFunction`.
pub fn extract_params(func: &verum_ast::FunctionDecl) -> Vec<(String, String)> {
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
///
/// Context-free wrapper over [`try_reflect_function_with_env`]:
/// member expressions in the body (field access, method calls) are
/// refused without an env.
pub fn try_reflect_function(
    func: &verum_ast::FunctionDecl,
) -> Option<crate::refinement_reflection::ReflectedFunction> {
    try_reflect_function_with_env(func, &ReflectionTypeEnv::default())
}

/// [`try_reflect_function`] with module type facts: bodies over
/// record fields (`w.field`) and protocol/impl methods
/// (`p.cond_F_S().has_phi_X()`) reflect as applications of
/// uninterpreted projection symbols, whose `declare-sort` /
/// `declare-fun` lines travel with the entry as `aux_decls`.
pub fn try_reflect_function_with_env(
    func: &verum_ast::FunctionDecl,
    module_env: &ReflectionTypeEnv,
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

    // Parameter bindings: names of NAMED-typed parameters feed the
    // member arms (a field access or method call is resolvable only
    // on a value whose type name the env knows).
    let mut env = module_env.clone();
    for p in func.params.iter() {
        if let verum_ast::decl::FunctionParamKind::Regular { pattern, ty, .. } = &p.kind {
            if let verum_ast::pattern::PatternKind::Ident { name, .. } = &pattern.kind {
                if let (_, Some(tn)) = type_to_sort_and_name(ty) {
                    env.bindings.insert(name.name.as_str().to_string(), tn);
                }
            }
        }
    }

    // Translate the body to SMT-LIB, collecting projection decls.
    let mut aux = std::collections::BTreeSet::new();
    let body_smtlib = match expr_to_smtlib_env(tail_expr, &env, &mut aux) {
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
        .map(type_to_sort)
        .unwrap_or_else(|| "Int".to_string());

    // Parameter sorts may themselves be opaque (`Verum!T`) — their
    // declare-sort lines must travel with the entry too, or the
    // block's own `(declare-fun f (Verum!T) Bool)` names an
    // undeclared sort.
    for (_, s) in &params {
        if s.starts_with("Verum!") {
            aux.insert(format!("(declare-sort {} 0)", s));
        }
    }
    if return_sort.starts_with("Verum!") {
        aux.insert(format!("(declare-sort {} 0)", return_sort));
    }

    Some(crate::refinement_reflection::ReflectedFunction {
        name: Text::from(func.name.name.as_str()),
        parameters: params.iter().map(|(n, _)| Text::from(n.as_str())).collect(),
        body_smtlib: Text::from(body_smtlib),
        return_sort: Text::from(return_sort),
        parameter_sorts: params.iter().map(|(_, s)| Text::from(s.as_str())).collect(),
        aux_decls: aux.into_iter().map(Text::from).collect(),
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
        resolved_call_target: None,
        }
    }

    fn bool_expr(b: bool) -> Expr {
        Expr {
            kind: ExprKind::Literal(Literal::new(verum_ast::literal::LiteralKind::Bool(b), sp())),
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
        resolved_call_target: None,
        }
    }

    fn var_expr(name: &str) -> Expr {
        Expr {
            kind: ExprKind::Path(Path::single(Ident::new(name, sp()))),
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
        resolved_call_target: None,
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
        resolved_call_target: None,
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
        resolved_call_target: None,
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
        resolved_call_target: None,
        };
        assert_eq!(expr_to_smtlib(&e).unwrap(), "(- x)");
    }

    #[test]
    fn nested_arithmetic() {
        // (a + b) * c
        let sum = binop(BinOp::Add, var_expr("a"), var_expr("b"));
        let product = binop(BinOp::Mul, sum, var_expr("c"));
        assert_eq!(expr_to_smtlib(&product).unwrap(), "(* (+ a b) c)");
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
        resolved_call_target: None,
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
        resolved_call_target: None,
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

    // -------------------------------------------------------------
    // #161 V3 — separation-recogniser fast path integration
    // -------------------------------------------------------------

    fn call_expr(callee: &str, args: Vec<Expr>) -> Expr {
        Expr {
            kind: ExprKind::Call {
                func: verum_common::Heap::new(var_expr(callee)),
                type_args: verum_common::List::new(),
                args: args.into_iter().collect(),
            },
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
        resolved_call_target: None,
        }
    }

    #[test]
    fn separation_recogniser_routes_emp_to_structured_form() {
        // emp() — pre-V3 emitted `(emp)` (opaque function).
        // Post-V3: routes through recogniser, emits structured `sep_emp`.
        let e = call_expr("emp", vec![]);
        assert_eq!(expr_to_smtlib(&e).unwrap(), "sep_emp");
    }

    #[test]
    fn separation_recogniser_routes_points_to_to_structured_form() {
        let e = call_expr("points_to", vec![var_expr("a"), var_expr("av")]);
        assert_eq!(expr_to_smtlib(&e).unwrap(), "(sep_pt a av)");
    }

    #[test]
    fn separation_recogniser_routes_sep_conj_recursively() {
        // sep_conj(emp(), points_to(a, av))
        let inner = call_expr("points_to", vec![var_expr("a"), var_expr("av")]);
        let outer = call_expr("sep_conj", vec![call_expr("emp", vec![]), inner]);
        assert_eq!(
            expr_to_smtlib(&outer).unwrap(),
            "(sep_star sep_emp (sep_pt a av))",
        );
    }

    #[test]
    fn unrecognised_call_falls_back_to_opaque_translation() {
        // Generic user function — should still translate as opaque.
        let e = call_expr("user_function", vec![var_expr("arg")]);
        assert_eq!(expr_to_smtlib(&e).unwrap(), "(user_function arg)");
    }

    #[test]
    fn separation_recogniser_with_unrecognised_inner_falls_back_at_outer_level() {
        // sep_conj(emp(), user_function()) — outer recogniser bails
        // because inner isn't a sep predicate; full call falls through
        // to opaque translation: (sep_conj emp (user_function)).
        // (Note: the inner emp() inside fallback path translates
        // to its OPAQUE form (sep_emp resolves there too via
        // bare-name resolution under generic translation? Let me
        // verify what the test sees.)
        let outer = call_expr(
            "sep_conj",
            vec![call_expr("emp", vec![]), call_expr("user_function", vec![])],
        );
        // The all-or-nothing recogniser returns None for outer because
        // user_function is not a sep predicate. Generic Call
        // translation kicks in: each arg is recursively translated;
        // the inner emp() IS recognised individually and renders as
        // `sep_emp`; user_function() renders opaque as `(user_function)`.
        // Result: (sep_conj sep_emp (user_function))
        assert_eq!(
            expr_to_smtlib(&outer).unwrap(),
            "(sep_conj sep_emp (user_function))",
        );
    }

    // ---- T0843: member-bearing bodies reflect as projection symbols ----

    fn field_expr(obj: Expr, field: &str) -> Expr {
        Expr {
            kind: ExprKind::Field {
                expr: verum_common::Heap::new(obj),
                field: Ident::new(field, sp()),
            },
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
            resolved_call_target: None,
        }
    }

    fn method_call_expr(recv: Expr, method: &str, args: Vec<Expr>) -> Expr {
        Expr {
            kind: ExprKind::MethodCall {
                receiver: verum_common::Heap::new(recv),
                method: Ident::new(method, sp()),
                type_args: verum_common::List::default(),
                args: args.into_iter().collect(),
            },
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
            resolved_call_target: None,
        }
    }

    fn witness_env() -> ReflectionTypeEnv {
        let mut env = ReflectionTypeEnv::default();
        env.bindings.insert("w".into(), "Witness".into());
        env.record_fields.insert(
            "Witness".into(),
            [("pnt_asymptotic".to_string(), ("Bool".to_string(), None))]
                .into_iter()
                .collect(),
        );
        env
    }

    /// The (D1) leaf: `w.field` over a record witness translates as an
    /// application of the projection symbol, whose declarations travel
    /// in `aux` — the shape whose refusal made every predicate goal
    /// over a record witness unprovable.
    #[test]
    fn record_field_projects_through_an_uninterpreted_symbol() {
        let env = witness_env();
        let mut aux = std::collections::BTreeSet::new();
        let e = field_expr(var_expr("w"), "pnt_asymptotic");
        assert_eq!(
            expr_to_smtlib_env(&e, &env, &mut aux).unwrap(),
            "(Verum!proj!Witness!pnt_asymptotic w)"
        );
        assert!(aux.contains("(declare-sort Verum!Witness 0)"));
        assert!(aux.contains(
            "(declare-fun Verum!proj!Witness!pnt_asymptotic (Verum!Witness) Bool)"
        ));
    }

    /// The msfs leaf: a CHAINED protocol-method call — the receiver
    /// type of the outer call is the return type of the inner one.
    #[test]
    fn chained_protocol_methods_project_through_uninterpreted_symbols() {
        let mut env = ReflectionTypeEnv::default();
        env.bindings.insert("candidate".into(), "Candidate".into());
        env.methods.insert(
            "Candidate".into(),
            [(
                "cond_F_S".to_string(),
                MemberSig {
                    arg_sorts: vec![],
                    ret_sort: "Verum!CondFS".to_string(),
                    ret_type_name: Some("CondFS".to_string()),
                },
            )]
            .into_iter()
            .collect(),
        );
        env.methods.insert(
            "CondFS".into(),
            [(
                "has_phi_X".to_string(),
                MemberSig {
                    arg_sorts: vec![],
                    ret_sort: "Bool".to_string(),
                    ret_type_name: None,
                },
            )]
            .into_iter()
            .collect(),
        );
        let mut aux = std::collections::BTreeSet::new();
        let chain = method_call_expr(
            method_call_expr(var_expr("candidate"), "cond_F_S", vec![]),
            "has_phi_X",
            vec![],
        );
        assert_eq!(
            expr_to_smtlib_env(&chain, &env, &mut aux).unwrap(),
            "(Verum!method!CondFS!has_phi_X (Verum!method!Candidate!cond_F_S candidate))"
        );
        assert!(aux.contains("(declare-sort Verum!Candidate 0)"));
        assert!(aux.contains("(declare-sort Verum!CondFS 0)"));
        assert!(aux.contains(
            "(declare-fun Verum!method!Candidate!cond_F_S (Verum!Candidate) Verum!CondFS)"
        ));
        assert!(aux.contains(
            "(declare-fun Verum!method!CondFS!has_phi_X (Verum!CondFS) Bool)"
        ));
    }

    /// The context-free wrapper keeps the pre-env behaviour: member
    /// expressions are refused, not mistranslated.
    #[test]
    fn member_access_without_env_is_refused() {
        let e = field_expr(var_expr("w"), "pnt_asymptotic");
        assert!(expr_to_smtlib(&e).is_err());
    }

    /// An unknown type must NOT collapse to a scalar sort — the exact
    /// sibling defect `translate.rs::create_var`'s opaque arm pins:
    /// over a list-shaped Int, `xs.len() > 0` becomes provable noise.
    #[test]
    fn named_types_sort_opaque_under_their_own_name_never_int() {
        use verum_ast::ty::{Path as TyPath, Type, TypeKind};
        let named = Type::new(
            TypeKind::Path(TyPath::single(Ident::new("UserRecord", sp()))),
            sp(),
        );
        assert_eq!(type_to_sort(&named), "Verum!UserRecord");
        let reference = Type::new(
            TypeKind::Reference {
                mutable: false,
                inner: verum_common::Heap::new(named),
            },
            sp(),
        );
        // `&T` carries T's facts — same sort as T, matching the
        // Reference arm added to `create_var`.
        assert_eq!(type_to_sort(&reference), "Verum!UserRecord");
    }
}
