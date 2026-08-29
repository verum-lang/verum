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
    /// Sum type → its constructors, in declaration order, each with the
    /// SORTS of its payload positions.
    ///
    /// A nullary constructor has an empty payload list and needs no
    /// machinery beyond the constant it already is. One with a payload
    /// gets a discriminant predicate and one projection per position —
    /// see `solver_symbols::discriminant` / `::payload`.
    pub variants: std::collections::HashMap<String, Vec<(String, Vec<String>)>>,
}

impl ReflectionTypeEnv {
    /// Harvest record fields and protocol/impl method signatures from
    /// a module — the same single-file scope the reflection scan
    /// itself walks.
    pub fn from_module(module: &verum_ast::Module) -> Self {
        let mut env = Self::default();
        env.absorb_types_from(module);
        env
    }

    /// Add the TYPE SHAPES this module declares — record fields,
    /// protocol and impl methods — to an env that already exists.
    ///
    /// Existing entries are kept, so the env's FIRST module wins any
    /// name clash. That is the file under verification: a type it
    /// declares itself is the one it means, whatever a sibling calls
    /// the same name.
    ///
    /// This exists because a type a module MOUNTS is not in its
    /// `items`, so a theorem parameter typed by one had no declared
    /// shape and kept the `Int` default while the predicate it was
    /// passed to was declared over that type's own sort — the
    /// application was then refused and the claim could not be stated
    /// at all (T0904).
    pub fn absorb_types_from(&mut self, module: &verum_ast::Module) {
        use verum_ast::ItemKind;
        use verum_ast::decl::{ImplItemKind, ImplKind, ProtocolItemKind, TypeDeclBody};
        let mut env = &mut *self;

        let add_methods =
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
                    TypeDeclBody::Variant(vs) => {
                        // First declarer wins — see `absorb_types_from`.
                        let name = td.name.name.as_str().to_string();
                        if env.variants.contains_key(&name) {
                            continue;
                        }
                        let ctors: Vec<(String, Vec<String>)> = vs
                            .iter()
                            .map(|v| {
                                let payload = match &v.data {
                                    verum_common::Maybe::Some(
                                        verum_ast::decl::VariantData::Tuple(tys),
                                    ) => tys.iter().map(type_to_sort).collect(),
                                    verum_common::Maybe::Some(
                                        verum_ast::decl::VariantData::Record(fs),
                                    ) => fs.iter().map(|f| type_to_sort(&f.ty)).collect(),
                                    verum_common::Maybe::None => Vec::new(),
                                };
                                (v.name.name.as_str().to_string(), payload)
                            })
                            .collect();
                        env.variants.insert(name, ctors);
                    }
                    TypeDeclBody::Record(fields) => {
                        // First declarer wins — see `absorb_types_from`.
                        let name = td.name.name.as_str().to_string();
                        if env.record_fields.contains_key(&name) {
                            continue;
                        }
                        let m = env.record_fields.entry(name).or_default();
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

/// Build one arm of a `match` whose pattern carries a PAYLOAD.
///
/// Answers `(condition, body)`. The condition is a discriminant
/// predicate over the scrutinee; the body is the arm translated with
/// each bound name replaced by a projection of that scrutinee.
///
/// Declines — and so declines the whole reflection — when it cannot
/// name the scrutinee's TYPE, when that type's constructors are not
/// known, when the constructor named is not one of them, or when the
/// pattern binds anything but plain identifiers. A partial answer here
/// would be a body that is not the function's.
fn payload_arm(
    scrut: &Expr,
    scrut_smt: &str,
    path: &verum_ast::ty::Path,
    data: &verum_ast::pattern::VariantPatternData,
    body: &Expr,
    env: &ReflectionTypeEnv,
    aux: &mut std::collections::BTreeSet<String>,
) -> Result<(String, String), SmtTranslateError> {
    use verum_ast::pattern::PatternKind;

    let decline = |what: &str| SmtTranslateError::UnsupportedExpr {
        description: what.to_string(),
    };

    // The constructor is the LAST segment; the type is either the
    // segment before it (`Result.Ok`) or the scrutinee's own type
    // (a bare `Ok`).
    let segs: Vec<&str> = path
        .segments
        .iter()
        .filter_map(|seg| match seg {
            verum_ast::ty::PathSegment::Name(id) => Some(id.name.as_str()),
            _ => None,
        })
        .collect();
    let ctor = *segs
        .last()
        .ok_or_else(|| decline("variant pattern with no constructor name"))?;
    let scrut_type = member_type_name(scrut, env)
        .or_else(|| {
            if segs.len() >= 2 {
                Some(segs[segs.len() - 2].to_string())
            } else {
                None
            }
        })
        .ok_or_else(|| decline("payload match on a scrutinee of unknown type"))?;

    let ctors = env
        .variants
        .get(&scrut_type)
        .ok_or_else(|| decline("payload match on a type with no known constructors"))?;
    let payload_sorts = ctors
        .iter()
        .find(|(name, _)| name == ctor)
        .map(|(_, sorts)| sorts.clone())
        .ok_or_else(|| decline("payload match on an unknown constructor"))?;

    // The bound names, in order.
    let bound: Vec<&verum_ast::pattern::Pattern> = match data {
        verum_ast::pattern::VariantPatternData::Tuple(ps) => ps.iter().collect(),
        verum_ast::pattern::VariantPatternData::Record { fields, rest } => {
            if *rest {
                return Err(decline("payload pattern with `..`"));
            }
            // A record-style field pattern may elide its sub-pattern
            // (`Error { code }` binds `code` by its own name); this
            // fragment only carries the explicit form, and declines
            // the shorthand rather than guessing at the binding.
            let mut out: Vec<&verum_ast::pattern::Pattern> = Vec::new();
            for fp in fields.iter() {
                match &fp.pattern {
                    Some(p) => out.push(p),
                    None => return Err(decline("record payload pattern using field shorthand")),
                }
            }
            out
        }
    };
    if bound.len() != payload_sorts.len() {
        return Err(decline("payload pattern with the wrong number of bindings"));
    }

    let scrut_sort = crate::solver_symbols::opaque_sort(&scrut_type);
    note_sort(aux, &scrut_sort);

    // Each binding becomes a projection application, substituted into
    // the arm's body before translation.
    let mut substituted = body.clone();
    for (i, pat) in bound.iter().enumerate() {
        let name = match &pat.kind {
            PatternKind::Ident { name, .. } => name.name.as_str().to_string(),
            PatternKind::Wildcard => continue,
            _ => return Err(decline("payload pattern binding a non-identifier")),
        };
        let proj = crate::solver_symbols::payload(&scrut_type, ctor, i);
        note_decl(
            aux,
            format!("(declare-fun {} ({}) {})", proj, scrut_sort, payload_sorts[i]),
        );
        note_sort(aux, &payload_sorts[i]);
        substituted = substitute_ident_with_symbol(
            &substituted,
            &name,
            &format!("({} {})", proj, scrut_smt),
        );
    }

    let disc = crate::solver_symbols::discriminant(&scrut_type, ctor);
    note_decl(aux, format!("(declare-fun {} ({}) Bool)", disc, scrut_sort));
    note_variant_discriminant_facts(&scrut_type, ctors, &scrut_sort, aux);

    let body_smt = expr_to_smtlib_env(&substituted, env, aux)?;
    Ok((format!("({} {})", disc, scrut_smt), body_smt))
}

/// Declare every constructor's discriminant for a type and say what
/// they mean, once.
///
/// Three facts, and each is needed by a different kind of claim:
///
///   * EXHAUSTIVE — some constructor holds. Without it the solver may
///     pick a value that is none of them.
///   * DISJOINT — no two hold together. This is what makes "it is an
///     `Err`, so it is not an `Ok`" available, which is the whole of an
///     `accepts exactly when` claim.
///   * for a NULLARY constructor, the discriminant is exactly equality
///     with its constant, so the two spellings of the same test —
///     `(= k path_K.A)` from a nullary arm and `(Verum!is!K!A k)` from
///     a payload one — agree in a match that mixes them.
///
/// Emitted for the whole TYPE rather than the one constructor in hand,
/// because disjointness is a statement about pairs and a partial
/// version of it is worse than none.
fn note_variant_discriminant_facts(
    type_name: &str,
    ctors: &[(String, Vec<String>)],
    sort: &str,
    aux: &mut std::collections::BTreeSet<String>,
) {
    if ctors.is_empty() {
        return;
    }
    let disc_of = |c: &str| crate::solver_symbols::discriminant(type_name, c);
    for (name, _) in ctors {
        note_decl(aux, format!("(declare-fun {} ({}) Bool)", disc_of(name), sort));
    }

    let applied: Vec<String> = ctors
        .iter()
        .map(|(n, _)| format!("({} x)", disc_of(n)))
        .collect();
    if applied.len() == 1 {
        note_decl(
            aux,
            format!("(assert (forall ((x {})) {}))", sort, applied[0]),
        );
    } else {
        note_decl(
            aux,
            format!(
                "(assert (forall ((x {})) (or {})))",
                sort,
                applied.join(" ")
            ),
        );
        for i in 0..ctors.len() {
            for j in (i + 1)..ctors.len() {
                note_decl(
                    aux,
                    format!(
                        "(assert (forall ((x {})) (not (and {} {}))))",
                        sort, applied[i], applied[j]
                    ),
                );
            }
        }
    }

    for (name, payload) in ctors {
        if payload.is_empty() {
            note_decl(
                aux,
                format!(
                    "(assert (forall ((x {})) (= ({} x) (= x path_{}.{}))))",
                    sort,
                    disc_of(name),
                    type_name,
                    name
                ),
            );
        }
    }
}

/// Replace a bare-path reference by a RAW SMT-LIB symbol.
///
/// The payload projection `(Verum!payload!Result!Ok!0 r)` is not a
/// Verum expression, so it cannot be substituted as one. A marker
/// expression carries it through the AST and the translator's Path arm
/// emits it verbatim.
fn substitute_ident_with_symbol(expr: &Expr, name: &str, symbol: &str) -> Expr {
    let marker = Expr::new(
        ExprKind::Path(verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
            format!("{}{}", RAW_SMT_PREFIX, symbol).as_str(),
            expr.span,
        ))),
        expr.span,
    );
    substitute_ident(expr, name, &marker)
}

/// Prefix marking an identifier that is really a raw SMT-LIB term.
/// Chosen so no Verum identifier can collide with it.
const RAW_SMT_PREFIX: &str = "\u{0}smt:";

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
                // A PAYLOAD arm's body names the bound payload, which
                // has no meaning until the arm is built, so its
                // translation cannot be attempted here. Nullary arms
                // are unaffected — they bind nothing.
                let is_payload_arm = matches!(
                    &arm.pattern.kind,
                    PatternKind::Variant { data: Some(_), .. }
                );
                let body_smt = if is_payload_arm {
                    String::new()
                } else {
                    expr_to_smtlib_env(&arm.body, env, aux)?
                };
                match &arm.pattern.kind {
                    // Binds anything → the fallthrough (else) branch.
                    PatternKind::Wildcard | PatternKind::Ident { .. } => {
                        chain = Some(body_smt);
                    }
                    // A variant pattern. Nullary is a CONSTANT, so the
                    // test is `(= k path_K.A)`. One with a payload is
                    // not a constant: the test is a discriminant
                    // predicate and each bound name is a projection of
                    // the scrutinee — the same device a record field
                    // uses, and for the same reason. The solver learns
                    // nothing about the payload's value, only that one
                    // scrutinee projects to one payload, which is what
                    // an arm's body and a hypothesis about the same
                    // value need in order to meet.
                    PatternKind::Variant { path, data } => {
                        if let Some(d) = data {
                            let (cond, body) =
                                payload_arm(scrut, &scrut_smt, path, d, &arm.body, env, aux)?;
                            let existing = chain.clone().unwrap_or_else(|| body.clone());
                            chain = Some(format!("(ite {} {} {})", cond, body, existing));
                            continue;
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
            // TYPE-QUALIFIED first: `Held.None` is a variant CONSTANT
            // written in expression position, and the parser gives it
            // the same shape as a field access. It is not one — there
            // is no receiver whose type could be looked up, which is
            // why this used to decline with "field access on a value
            // whose named type is unknown: .None", and why a guard
            // comparing a variant against its own constant was
            // unreflectable while the same comparison inside a `match`
            // arm worked (that arm builds the constant from the
            // PATTERN's path and never reaches here). Same constant,
            // same spelling, one authority (T0906).
            if let ExprKind::Path(p) = &obj.kind
                && let Some(type_ident) = p.as_ident()
                && let Some(ctors) = env.variants.get(type_ident.as_str())
                && ctors.iter().any(|(n, _)| n == field.name.as_str())
            {
                return Ok(format!("path_{}.{}", type_ident.as_str(), field.name.as_str()));
            }
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

        // Reading one element: `xs[i]`, as an application of the same
        // uninterpreted symbol the goal side emits.
        //
        // This arm did not exist, and neither did the goal side's — so a
        // reflected body containing an index was declined here while the
        // goal containing one failed to translate there. Both are the
        // same missing rule, and writing it on one side only would have
        // been worse than writing neither: a definition that spells the
        // read differently from the goal can never reach it, which is
        // exactly the drift `solver_symbols` exists to prevent (T0962).
        ExprKind::Index { expr: base, index } => {
            let b = expr_to_smtlib_env(base, env, aux)?;
            let i = expr_to_smtlib_env(index, env, aux)?;
            // `Int` is the base sort this side can name: a reflected
            // body reaches here with the container rendered as an
            // ordinary term, and the goal side spells the same symbol
            // for an `Int`-sorted base. A container that arrives at the
            // goal under an opaque sort is a different symbol by
            // construction, which is the point — the two would
            // otherwise share a name and disagree about its signature
            // (T0962).
            let sym = crate::solver_symbols::index("Int");
            note_decl(aux, format!("(declare-fun {} (Int Int) Int)", sym));
            Ok(format!("({} {} {})", sym, b, i))
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
        // A marker planted by `substitute_ident_with_symbol`: the name
        // IS an SMT-LIB term already. Nothing a Verum source can spell
        // reaches this arm — the prefix contains a NUL.
        1 if names[0].starts_with(RAW_SMT_PREFIX) => {
            Ok(names[0][RAW_SMT_PREFIX.len()..].to_string())
        }
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
/// Fold a straight-line body into ONE expression.
///
/// Two statement shapes are carried, and both are shapes people
/// actually write when a predicate stops fitting on a line:
///
///   * a `let` binding — `let m = n; m == n` becomes `n == n`;
///   * an EARLY-RETURN GUARD — `if c { return e; } tail` becomes
///     `if c { e } else { tail }`, which the translator already renders
///     as an `ite`.
///
/// The guard shape is what a comparison chain looks like:
///
/// ```text
/// if a.major != b.major { return a.major < b.major; }
/// if a.minor != b.minor { return a.minor < b.minor; }
/// a.patch < b.patch
/// ```
///
/// Returns `None` — declining the whole reflection — for anything else:
/// a statement that is neither, a destructuring or otherwise
/// non-identifier pattern, a `let` with no initialiser, an `if` with an
/// `else` or with anything but a single `return` inside, or a chain
/// long enough that substitution would blow the term up.
///
/// Declining is the point. A PARTIAL fold would hand the solver a body
/// that is not the function's, and a proof about the wrong body is
/// worse than no proof.
///
/// ORDER is load-bearing. A guard is substituted with the bindings seen
/// BEFORE it and not with any that follow, because a `let` after a
/// guard is only reached when that guard did not fire — which is
/// exactly the else-branch the guard becomes.
/// The single expression a straight-line block computes, or `None` when
/// the block does something this substitution model does not represent.
///
/// Public because two places need the SAME answer to "what does this body
/// compute": reflection, which unfolds a call at another site, and the
/// verifier's own result binding, which ties `result` to the body. They
/// used to answer separately — reflection by folding, the result binding
/// by asserting each `let` into the solver and ignoring assignments — and
/// the second one then argued against its own postcondition with the
/// function's initial values.
pub fn fold_block_to_expr(
    stmts: &[verum_ast::stmt::Stmt],
    tail: &Expr,
) -> Option<Expr> {
    fold_let_bindings(stmts, tail)
}

fn fold_let_bindings(
    stmts: &[verum_ast::stmt::Stmt],
    tail: &Expr,
) -> Option<Expr> {
    use verum_ast::stmt::StmtKind;

    // Bounded: each substitution can duplicate its bound expression
    // once per use, so a long chain multiplies. Eight is far past any
    // predicate anyone writes and short enough that the worst case
    // stays small.
    const MAX_BINDINGS: usize = 8;
    if stmts.len() > MAX_BINDINGS {
        return None;
    }

    let mut bindings: Vec<(String, Expr)> = Vec::with_capacity(stmts.len());
    let mut guards: Vec<(Expr, Expr)> = Vec::new();
    let apply = |e: &Expr, bs: &[(String, Expr)]| -> Expr {
        let mut out = e.clone();
        for (bound, replacement) in bs {
            out = substitute_ident(&out, bound, replacement);
        }
        out
    };

    for stmt in stmts {
        match &stmt.kind {
            StmtKind::Let { pattern, value, .. } => {
                let verum_ast::pattern::PatternKind::Ident { name, .. } = &pattern.kind else {
                    return None;
                };
                let verum_common::Maybe::Some(init) = value else {
                    return None;
                };
                // Substitute the bindings already seen INTO this
                // initialiser, so `let a = n; let b = a + 1; b` folds to
                // `n + 1` rather than leaving a dangling `a`.
                let init = apply(init, &bindings);
                bindings.push((name.name.as_str().to_string(), init));
            }
            StmtKind::Expr { expr, .. } => {
                // An assignment to a local is a REBINDING, and folding it as
                // one is what lets an imperative body be reflected at all.
                //
                // `let mut acc = 0; acc = acc + 1; acc` means the same thing
                // as `let acc0 = 0; let acc1 = acc0 + 1; acc1`, and the fold
                // already knows how to inline the second form. Resolving the
                // right-hand side against the CURRENT bindings before
                // replacing the old one is what makes that equivalence hold:
                // the stored value closes over the state the assignment saw,
                // so a later `acc` reads the new value and the one inside
                // `acc + 1` still reads the old.
                //
                // Without this the whole function was declined — "body has a
                // statement that is neither a `let` nor an early return" —
                // and a declined body reflects as nothing, so its
                // postcondition had nothing to be proved against. Measured:
                // `{ let mut acc = 0; acc = 1; acc }` with
                // `ensures result == 1` did not verify, while the same
                // function written with an immutable binding did. Every
                // undischarged loop obligation in the registry showcase is
                // downstream of this, the loops being a symptom.
                if let ExprKind::Binary { op, left, right } = &expr.kind
                    && op.is_assignment()
                    && let ExprKind::Path(path) = &left.kind
                    && let Some(ident) = path.as_ident()
                {
                    let name = ident.name.as_str().to_string();
                    // Only a variable this fold introduced. A write to
                    // anything else — a field, an index, a captured
                    // binding — is a state change the substitution model
                    // does not represent, and guessing is how a reflection
                    // starts describing a different function.
                    if !bindings.iter().any(|(b, _)| b == &name) {
                        return None;
                    }
                    // Compound forms carry the read implicitly: `acc += e`
                    // is `acc = acc + e`. Only the plain form is folded
                    // here; the rest decline rather than be approximated.
                    let value_expr = match op {
                        verum_ast::expr::BinOp::Assign => right.as_ref().clone(),
                        _ => return None,
                    };
                    let resolved = apply(&value_expr, &bindings);
                    for b in bindings.iter_mut() {
                        if b.0 == name {
                            b.1 = resolved.clone();
                        }
                    }
                    continue;
                }
                let (cond, value) = early_return_guard(expr)?;
                guards.push((apply(&cond, &bindings), apply(&value, &bindings)));
            }
            _ => return None,
        }
    }

    let mut folded = apply(tail, &bindings);
    // Innermost guard last: the FIRST guard written is the outermost
    // test, so wrapping runs in reverse.
    for (cond, value) in guards.iter().rev() {
        folded = Expr::new(
            ExprKind::If {
                condition: verum_common::Heap::new(verum_ast::expr::IfCondition {
                    conditions: std::iter::once(verum_ast::expr::ConditionKind::Expr(
                        cond.clone(),
                    ))
                    .collect(),
                    span: cond.span,
                }),
                then_branch: verum_ast::expr::Block::new(
                    verum_common::List::new(),
                    verum_common::Maybe::Some(verum_common::Heap::new(value.clone())),
                    value.span,
                ),
                else_branch: verum_common::Maybe::Some(verum_common::Heap::new(folded)),
            },
            cond.span,
        );
    }
    Some(folded)
}

/// `if c { return e; }` — with no `else` and nothing else in the block
/// — as the pair `(c, e)`. `None` for any other expression.
///
/// Deliberately narrow. An `if` with an `else` is already an
/// expression the translator handles, and an `if` whose block does more
/// than return one value is not a guard: treating it as one would drop
/// whatever else it does.
fn early_return_guard(expr: &Expr) -> Option<(Expr, Expr)> {
    use verum_ast::stmt::StmtKind;
    let ExprKind::If {
        condition,
        then_branch,
        else_branch,
    } = &expr.kind
    else {
        return None;
    };
    if matches!(else_branch, verum_common::Maybe::Some(_)) {
        return None;
    }
    if condition.conditions.len() != 1 {
        return None;
    }
    let Some(verum_ast::expr::ConditionKind::Expr(cond)) = condition.conditions.first() else {
        return None;
    };

    // The returned value: either the block's tail, or its single
    // statement — both spellings of the same one-line block.
    let returned = match (&then_branch.expr, then_branch.stmts.len()) {
        (verum_common::Maybe::Some(tail), 0) => (**tail).clone(),
        (verum_common::Maybe::None, 1) => match &then_branch.stmts[0].kind {
            StmtKind::Expr { expr, .. } => expr.clone(),
            _ => return None,
        },
        _ => return None,
    };
    let ExprKind::Return(verum_common::Maybe::Some(value)) = &returned.kind else {
        return None;
    };
    Some((cond.clone(), (**value).clone()))
}

/// Replace every bare-path reference to `name` with `replacement`.
///
/// Structural and total: an expression kind this does not know is
/// returned unchanged, which is safe because an unsubstituted binding
/// then shows up as a free variable and the reflection's own
/// translation declines it — a miss becomes "not proved", never
/// "proved about something else".
fn substitute_ident(expr: &Expr, name: &str, replacement: &Expr) -> Expr {
    let sub = |e: &Expr| substitute_ident(e, name, replacement);
    let kind = match &expr.kind {
        ExprKind::Path(path) => {
            if path.segments.len() == 1
                && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
                && ident.name.as_str() == name
            {
                return replacement.clone();
            }
            return expr.clone();
        }
        ExprKind::Binary { op, left, right } => ExprKind::Binary {
            op: *op,
            left: verum_common::Heap::new(sub(left)),
            right: verum_common::Heap::new(sub(right)),
        },
        ExprKind::Unary { op, expr: inner } => ExprKind::Unary {
            op: *op,
            expr: verum_common::Heap::new(sub(inner)),
        },
        ExprKind::Paren(inner) => ExprKind::Paren(verum_common::Heap::new(sub(inner))),
        ExprKind::Field { expr: base, field } => ExprKind::Field {
            expr: verum_common::Heap::new(sub(base)),
            field: field.clone(),
        },
        ExprKind::Call { func, args, type_args } => ExprKind::Call {
            func: verum_common::Heap::new(sub(func)),
            args: args.iter().map(&sub).collect(),
            type_args: type_args.clone(),
        },
        ExprKind::MethodCall {
            receiver,
            method,
            args,
            type_args,
        } => ExprKind::MethodCall {
            receiver: verum_common::Heap::new(sub(receiver)),
            method: method.clone(),
            args: args.iter().map(&sub).collect(),
            type_args: type_args.clone(),
        },
        _ => return expr.clone(),
    };
    let mut out = expr.clone();
    out.kind = kind;
    out
}

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
    let decline = |reason: &str| -> Option<crate::refinement_reflection::ReflectedFunction> {
        // Every decline says WHY, on the tracing channel.
        //
        // This used to answer a bare `None`, so the only way to learn
        // why a predicate stayed an uninterpreted symbol — and every
        // claim about it therefore unprovable — was to guess and
        // re-measure. The reason was computed and then thrown away.
        tracing::debug!(
            "reflection declined `{}`: {}",
            func.name.name.as_str(),
            reason
        );
        None
    };

    if func.params.is_empty() {
        return decline("nullary — a constant, not a definition to unfold");
    }

    // Gate: must not have context requirements (impure).
    if !func.contexts.is_empty() {
        return decline("declares contexts, so its result may depend on injected state");
    }

    // Gate: must have a body.
    let body = match &func.body {
        verum_common::Maybe::Some(b) => b,
        verum_common::Maybe::None => return decline("no body"),
    };

    // Gate: body must be a Block with a tail expression. STRAIGHT-LINE
    // `let` bindings are folded into that tail by substitution;
    // anything else in the statement list still declines.
    //
    // Refusing every body with a statement made `let m = n; m == n`
    // unprovable while `n == n` proved — the SAME claim, refused for
    // its shape. And naming an intermediate value is the first thing
    // anyone does when a predicate stops fitting on one line, so the
    // fragment excluded exactly the properties worth writing down.
    //
    // Substitution is sound here without further conditions because
    // reflection already refuses impure functions (the context gate
    // above), so a bound expression has no effects to duplicate or
    // reorder — only solver work, which is why the fold is bounded
    // below.
    let folded_tail: Expr;
    let tail_expr = match body {
        verum_ast::decl::FunctionBody::Block(block) => {
            let tail = match &block.expr {
                verum_common::Maybe::Some(e) => e,
                verum_common::Maybe::None => {
                    return decline("block body with no tail expression");
                }
            };
            if block.stmts.is_empty() {
                tail
            } else {
                match fold_let_bindings(&block.stmts, tail) {
                    Some(f) => {
                        // The ACCEPT counterpart of `decline`, on the same
                        // channel. A decline says why; an accept has to say
                        // INTO WHAT, because the next failure mode after
                        // "declined everything" is "folded it wrongly", and
                        // that one is silent — the reflection succeeds and
                        // simply describes a different function.
                        tracing::debug!(
                            "reflection accepted `{}`: folded to {:?}",
                            func.name.name.as_str(),
                            f.kind
                        );
                        folded_tail = f;
                        &folded_tail
                    }
                    None => {
                        return decline(
                            "body has a statement that is neither a `let`, an \
                             assignment to a local, nor an early-return guard",
                        );
                    }
                }
            }
        }
        verum_ast::decl::FunctionBody::Expr(e) => e,
        _ => return decline("body is not a block or an expression"),
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
        Err(e) => return decline(&format!("body does not translate: {}", e)),
    };

    let params = extract_params(func);
    if params.is_empty() {
        return decline("no regular parameters");
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
