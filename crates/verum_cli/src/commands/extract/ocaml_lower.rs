//! Verum AST → OCaml lowerer.
//!
//! Walks a [`verum_ast::expr::Expr`] (or [`verum_ast::expr::Block`])
//! and emits the corresponding OCaml source text. The lowerer is
//! **partial-coverage by design** — covering the common pure-
//! functional subset (literals / vars / binops / calls / let-blocks
//! / if-then-else) — and returns `None` for any construct outside
//! its current vocabulary so the caller can gracefully fall back
//! to the V12.1 metadata-comment scaffold.
//!
//! Architectural notes (per VVA semantic-honesty):
//!   * Pure functional subset matches OCaml's strict-evaluation
//!     ML core; effectful Verum constructs (mutation, async,
//!     contexts) are deliberately out of V12.2 scope and surface
//!     as `None`.
//!   * Identifier mangling is conservative: alphanumeric +
//!     underscore preserved, leading uppercase mapped to OCaml's
//!     constructor-naming convention (Cons → `Cons`, none → `none`).
//!   * Operator translation chooses the OCaml-canonical form
//!     (`==` → structural `=`, `!=` → `<>`, `&&` / `||` keep).

use verum_ast::expr::{BinOp, Block, Expr, ExprKind, ConditionKind, IfCondition, UnOp};
use verum_ast::literal::{LiteralKind, StringLit};
use verum_common::Maybe;

/// V12.2 entry point — lower a Verum function-body block into OCaml
/// source text. Returns `None` when any sub-construct falls outside
/// the lowerer's coverage.
pub(super) fn lower_block(block: &Block) -> Option<String> {
    // Strategy: walk statements in order.
    //   * `let x = e` → emit `let x = <lower(e)> in `
    //   * other stmts → unsupported (V12.2.1 may add side-effect
    //     handling); for now return None.
    //   * trailing tail expr → lower as the final expression.
    let mut out = String::new();
    for stmt in block.stmts.iter() {
        use verum_ast::stmt::StmtKind;
        match &stmt.kind {
            StmtKind::Let { pattern, value, .. } => {
                let name = simple_pattern_name(pattern)?;
                let v = match value {
                    Maybe::Some(v) => lower_expr(v)?,
                    Maybe::None => return None,
                };
                out.push_str(&format!("let {} = {} in ", mangle_ident(&name), v));
            }
            // Trailing-effect statements — unsupported in V12.2 so
            // we conservatively bail.
            _ => return None,
        }
    }
    let tail = match &block.expr {
        Maybe::Some(e) => lower_expr(e)?,
        Maybe::None => "()".to_string(),
    };
    out.push_str(&tail);
    Some(out)
}

/// V12.2 entry point — lower a single Verum expression into OCaml.
pub(super) fn lower_expr(expr: &Expr) -> Option<String> {
    match &expr.kind {
        ExprKind::Literal(lit) => lower_literal(lit),
        ExprKind::Path(path) => Some(mangle_ident(path.last_segment_name())),
        ExprKind::Paren(inner) => Some(format!("({})", lower_expr(inner)?)),
        ExprKind::Binary { op, left, right } => {
            let l = lower_expr(left)?;
            let r = lower_expr(right)?;
            let op_str = ocaml_bin_op(op)?;
            Some(format!("({} {} {})", l, op_str, r))
        }
        ExprKind::Unary { op, expr } => {
            let inner = lower_expr(expr)?;
            let op_str = ocaml_un_op(op)?;
            Some(format!("({} {})", op_str, inner))
        }
        ExprKind::Call { func, args, .. } => {
            let f = lower_expr(func)?;
            if args.is_empty() {
                Some(format!("({} ())", f))
            } else {
                let mut parts = Vec::with_capacity(args.iter().count());
                for a in args.iter() {
                    parts.push(lower_expr(a)?);
                }
                Some(format!("({} {})", f, parts.join(" ")))
            }
        }
        ExprKind::Block(b) => Some(format!("({})", lower_block(b)?)),
        ExprKind::If { condition, then_branch, else_branch } => {
            let c = lower_if_condition(condition)?;
            let t = lower_block(then_branch)?;
            let e = match else_branch {
                Maybe::Some(e) => lower_expr(e)?,
                Maybe::None => "()".to_string(),
            };
            Some(format!("(if {} then {} else {})", c, t, e))
        }
        ExprKind::Index { expr, index } => {
            let recv = lower_expr(expr)?;
            let idx = lower_expr(index)?;
            // OCaml's stdlib accessor for arrays/lists. The
            // exact target type isn't known at this layer; we
            // emit `Array.get` as the most common case for
            // verified extraction. Mismatched receivers will
            // surface as OCaml type errors during downstream
            // compilation, not silent miscomputation.
            Some(format!("(Array.get {} {})", recv, idx))
        }
        ExprKind::NullCoalesce { left, right } => {
            // Verum `a ?? b` short-circuits on Maybe::None.
            // Lower as the canonical Option-pattern.
            let l = lower_expr(left)?;
            let r = lower_expr(right)?;
            Some(format!("(match {} with Some v -> v | None -> {})", l, r))
        }
        ExprKind::Tuple(elems) => {
            if elems.iter().count() < 2 {
                // Single-element "tuple" is just `(e,)` in Verum
                // surface — no clean OCaml mapping. Bail.
                return None;
            }
            let mut parts = Vec::with_capacity(elems.iter().count());
            for e in elems.iter() {
                parts.push(lower_expr(e)?);
            }
            Some(format!("({})", parts.join(", ")))
        }
        ExprKind::TupleIndex { expr, index } => {
            // OCaml has no `tuple.N` syntax; for pairs we use
            // `fst` / `snd`. Higher arities need explicit
            // destructuring which the simple lowerer doesn't model.
            let recv = lower_expr(expr)?;
            match index {
                0 => Some(format!("(fst {})", recv)),
                1 => Some(format!("(snd {})", recv)),
                _ => None,
            }
        }
        ExprKind::Pipeline { left, right } => {
            // OCaml has `|>` natively with the same semantics
            // (`x |> f` ≡ `f x`), so the lowering is a direct
            // syntactic mirror.
            let l = lower_expr(left)?;
            let r = lower_expr(right)?;
            Some(format!("({} |> {})", l, r))
        }
        ExprKind::Closure { async_, move_, params, contexts, return_type: _, body } => {
            // OCaml has no surface form for `async`, `move`, or
            // Verum's context system — bail when any are present so
            // semantics aren't silently dropped.
            if *async_ || *move_ || contexts.iter().count() > 0 {
                return None;
            }
            if params.iter().count() == 0 {
                // OCaml has no zero-param closure surface; the
                // canonical encoding is `fun () -> body`.
                let b = lower_expr(body)?;
                return Some(format!("(fun () -> {})", b));
            }
            let mut names = Vec::with_capacity(params.iter().count());
            for p in params.iter() {
                names.push(simple_pattern_name(&p.pattern).map(|n| mangle_ident(&n))?);
            }
            let b = lower_expr(body)?;
            Some(format!("(fun {} -> {})", names.join(" "), b))
        }
        ExprKind::MethodCall { receiver, method, type_args, args } => {
            // Verum's value-uniform `recv.method(args)` lowers to a
            // free-function application `(method recv args)` because
            // OCaml has no first-class method dispatch on plain values
            // (record-method dispatch needs an object/class context
            // outside the partial-coverage subset). Type-arguments
            // have no surface form in OCaml — bail when present so
            // monomorphisation isn't silently lost.
            if type_args.iter().count() > 0 {
                return None;
            }
            let recv = lower_expr(receiver)?;
            let mut parts = Vec::with_capacity(args.iter().count() + 2);
            parts.push(mangle_ident(method.name.as_str()));
            parts.push(recv);
            for a in args.iter() {
                parts.push(lower_expr(a)?);
            }
            Some(format!("({})", parts.join(" ")))
        }
        ExprKind::Field { expr, field } => {
            let recv = lower_expr(expr)?;
            Some(format!("{}.{}", recv, mangle_ident(field.name.as_str())))
        }
        ExprKind::Match { expr, arms } => {
            let scrutinee = lower_expr(expr)?;
            let mut arm_strs = Vec::with_capacity(arms.iter().count());
            for arm in arms.iter() {
                // Guards on arms are out of coverage for the
                // partial-coverage lowerer — the OCaml `when`
                // form needs scope discipline that the simple
                // pattern translator below doesn't enforce.
                if matches!(arm.guard, Maybe::Some(_)) {
                    return None;
                }
                let pat = lower_pattern(&arm.pattern)?;
                let body = lower_expr(&arm.body)?;
                arm_strs.push(format!("| {} -> {}", pat, body));
            }
            Some(format!("(match {} with {})", scrutinee, arm_strs.join(" ")))
        }
        // Out of coverage for the partial lowerer: MethodCall (needs
        // receiver dispatch model), Closures, Cast, Try, Pipeline,
        // ranges, tuples-with-side-effects, etc. Return None so the
        // caller falls back to the metadata-comment scaffold.
        _ => None,
    }
}

/// Translate a Verum [`Pattern`] into OCaml pattern syntax.
/// Returns `None` for shapes outside the lowerer's coverage so the
/// caller can bail to the metadata-comment fallback.
fn lower_pattern(pat: &verum_ast::pattern::Pattern) -> Option<String> {
    use verum_ast::pattern::{PatternKind, VariantPatternData};
    match &pat.kind {
        PatternKind::Wildcard => Some("_".to_string()),
        PatternKind::Ident { name, subpattern, .. } => match subpattern {
            Maybe::None => Some(mangle_ident(name.name.as_str())),
            // `x @ subpat` → OCaml `(x as subpat)` form. We emit
            // `(subpat as x)` which OCaml accepts.
            Maybe::Some(sub) => {
                let inner = lower_pattern(sub)?;
                Some(format!("({} as {})", inner, mangle_ident(name.name.as_str())))
            }
        },
        PatternKind::Literal(lit) => match &lit.kind {
            LiteralKind::Bool(b) => Some(if *b { "true".to_string() } else { "false".to_string() }),
            LiteralKind::Int(i) => Some(format!("{}", i.value)),
            LiteralKind::Char(c) => Some(format!("'{}'", c)),
            LiteralKind::Text(StringLit::Regular(s)) | LiteralKind::Text(StringLit::MultiLine(s)) => {
                Some(format!("\"{}\"", escape_ocaml_string(s.as_str())))
            }
            _ => None,
        },
        PatternKind::Tuple(elems) => {
            let mut parts = Vec::with_capacity(elems.iter().count());
            for e in elems.iter() {
                parts.push(lower_pattern(e)?);
            }
            Some(format!("({})", parts.join(", ")))
        }
        PatternKind::Variant { path, data } => {
            let ctor = path.last_segment_name();
            match data {
                Maybe::None => Some(ctor.to_string()),
                Maybe::Some(VariantPatternData::Tuple(elems)) => {
                    let mut parts = Vec::with_capacity(elems.iter().count());
                    for e in elems.iter() {
                        parts.push(lower_pattern(e)?);
                    }
                    if parts.len() == 1 {
                        Some(format!("{} {}", ctor, parts[0]))
                    } else {
                        Some(format!("{} ({})", ctor, parts.join(", ")))
                    }
                }
                // Record-style variants need field-name plumbing
                // that's left for a follow-up pass.
                Maybe::Some(VariantPatternData::Record { .. }) => None,
            }
        }
        PatternKind::Or(alts) => {
            let mut parts = Vec::with_capacity(alts.iter().count());
            for a in alts.iter() {
                parts.push(lower_pattern(a)?);
            }
            Some(parts.join(" | "))
        }
        PatternKind::Paren(inner) => Some(format!("({})", lower_pattern(inner)?)),
        // Record patterns / slice patterns / range patterns / view
        // patterns / reference patterns / rest are out of coverage.
        _ => None,
    }
}

fn lower_if_condition(cond: &IfCondition) -> Option<String> {
    // V12.2 supports single-expression conditions; let-condition
    // chains (`if let pattern = expr && ...`) are out of scope.
    if cond.conditions.len() != 1 {
        return None;
    }
    match cond.conditions.first() {
        Some(ConditionKind::Expr(e)) => lower_expr(e),
        _ => None,
    }
}

fn lower_literal(lit: &verum_ast::literal::Literal) -> Option<String> {
    match &lit.kind {
        LiteralKind::Bool(b) => Some(if *b { "true".to_string() } else { "false".to_string() }),
        LiteralKind::Int(i) => Some(format!("{}", i.value)),
        LiteralKind::Float(f) => Some(format!("{}", f.value)),
        LiteralKind::Text(StringLit::Regular(s)) | LiteralKind::Text(StringLit::MultiLine(s)) => {
            Some(format!("\"{}\"", escape_ocaml_string(s.as_str())))
        }
        LiteralKind::Char(c) => Some(format!("'{}'", c)),
        // Bytes / interpolated / tagged / composite are out of V12.2 scope.
        _ => None,
    }
}

fn ocaml_bin_op(op: &BinOp) -> Option<&'static str> {
    Some(match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "mod",
        BinOp::Eq => "=",
        BinOp::Ne => "<>",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::Concat => "@",
        BinOp::BitAnd => "land",
        BinOp::BitOr => "lor",
        BinOp::BitXor => "lxor",
        BinOp::Shl => "lsl",
        BinOp::Shr => "lsr",
        // Pow / In / Imply / Iff / assignment ops have no clean OCaml
        // single-operator translation — fall through to None so the
        // caller bails to the V12.1 fallback.
        _ => return None,
    })
}

fn ocaml_un_op(op: &UnOp) -> Option<&'static str> {
    Some(match op {
        UnOp::Neg => "-",
        UnOp::Not => "not",
        _ => return None,
    })
}

/// Conservative ASCII identifier mangling matching OCaml's value
/// namespace. Verum identifiers are already alphanumeric+underscore
/// per the lexer; leading-uppercase names map to constructor-style
/// (preserved verbatim — OCaml accepts them as variant ctors).
fn mangle_ident(name: &str) -> String {
    if name.is_empty() {
        return "_".to_string();
    }
    let mut out = String::with_capacity(name.len());
    for (i, ch) in name.chars().enumerate() {
        if i == 0 && ch.is_ascii_digit() {
            out.push('_');
        }
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}

/// V12.2 helper — extract a single-name binding from a `Pattern`.
/// Returns `None` for compound patterns (V12.2.1 will lower them
/// to `match` expressions on the let-bound value).
fn simple_pattern_name(pat: &verum_ast::pattern::Pattern) -> Option<String> {
    use verum_ast::pattern::PatternKind;
    match &pat.kind {
        PatternKind::Ident { name, .. } => Some(name.name.as_str().to_string()),
        PatternKind::Wildcard => Some("_".to_string()),
        _ => None,
    }
}

fn escape_ocaml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::expr::Expr;
    use verum_ast::literal::{IntLit, Literal};
    use verum_ast::span::Span;
    use verum_ast::{Ident, Path};
    use verum_common::{Heap, Text};

    fn span() -> Span {
        Span::default()
    }

    fn int_lit(v: i128) -> Expr {
        Expr::literal(Literal::new(LiteralKind::Int(IntLit::new(v)), span()))
    }

    fn bool_lit(b: bool) -> Expr {
        Expr::literal(Literal::new(LiteralKind::Bool(b), span()))
    }

    fn var(name: &str) -> Expr {
        Expr::path(Path::single(Ident::new(Text::from(name), span())))
    }

    #[test]
    fn lower_int_literal() {
        assert_eq!(lower_expr(&int_lit(42)).as_deref(), Some("42"));
    }

    #[test]
    fn lower_bool_literal() {
        assert_eq!(lower_expr(&bool_lit(true)).as_deref(), Some("true"));
        assert_eq!(lower_expr(&bool_lit(false)).as_deref(), Some("false"));
    }

    #[test]
    fn lower_var_reference() {
        assert_eq!(lower_expr(&var("x")).as_deref(), Some("x"));
        assert_eq!(lower_expr(&var("plus_comm")).as_deref(), Some("plus_comm"));
    }

    #[test]
    fn lower_arithmetic_addition() {
        let e = Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: Heap::new(var("x")),
                right: Heap::new(var("y")),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(x + y)"));
    }

    #[test]
    fn lower_eq_uses_ocaml_structural_equality() {
        let e = Expr::new(
            ExprKind::Binary {
                op: BinOp::Eq,
                left: Heap::new(var("a")),
                right: Heap::new(var("b")),
            },
            span(),
        );
        // OCaml's `=` is structural equality (Verum `==` mapping).
        assert_eq!(lower_expr(&e).as_deref(), Some("(a = b)"));
    }

    #[test]
    fn lower_ne_uses_ocaml_lt_gt() {
        let e = Expr::new(
            ExprKind::Binary {
                op: BinOp::Ne,
                left: Heap::new(var("a")),
                right: Heap::new(var("b")),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(a <> b)"));
    }

    #[test]
    fn lower_unary_neg() {
        let e = Expr::new(
            ExprKind::Unary {
                op: UnOp::Neg,
                expr: Heap::new(var("x")),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(- x)"));
    }

    #[test]
    fn lower_function_call_no_args() {
        let e = Expr::new(
            ExprKind::Call {
                func: Heap::new(var("now")),
                type_args: verum_common::List::new(),
                args: verum_common::List::new(),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(now ())"));
    }

    #[test]
    fn lower_function_call_with_args() {
        let mut args = verum_common::List::new();
        args.push(var("a"));
        args.push(int_lit(2));
        let e = Expr::new(
            ExprKind::Call {
                func: Heap::new(var("add")),
                type_args: verum_common::List::new(),
                args,
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(add a 2)"));
    }

    #[test]
    fn lower_paren_preserves_grouping() {
        let inner = int_lit(7);
        let e = Expr::new(ExprKind::Paren(Heap::new(inner)), span());
        assert_eq!(lower_expr(&e).as_deref(), Some("(7)"));
    }

    #[test]
    fn lower_string_escapes_quotes_and_backslashes() {
        let s = StringLit::Regular(Text::from("hello \"world\" \\n"));
        let lit = Literal::new(LiteralKind::Text(s), span());
        let e = Expr::literal(lit);
        assert_eq!(lower_expr(&e).as_deref(), Some("\"hello \\\"world\\\" \\\\n\""));
    }

    #[test]
    fn lower_unsupported_returns_none() {
        // `**` (Pow) has no clean OCaml single-operator translation.
        let e = Expr::new(
            ExprKind::Binary {
                op: BinOp::Pow,
                left: Heap::new(var("x")),
                right: Heap::new(int_lit(2)),
            },
            span(),
        );
        assert!(lower_expr(&e).is_none());
    }

    #[test]
    fn mangle_preserves_alphanum_and_underscore() {
        assert_eq!(mangle_ident("foo_bar"), "foo_bar");
        assert_eq!(mangle_ident("Foo123"), "Foo123");
    }

    #[test]
    fn mangle_replaces_invalid_chars() {
        assert_eq!(mangle_ident("a-b"), "a_b");
        assert_eq!(mangle_ident("plus.comm"), "plus_comm");
    }

    #[test]
    fn mangle_prepends_underscore_to_leading_digit() {
        assert_eq!(mangle_ident("1foo"), "_1foo");
    }

    #[test]
    fn lower_logical_and_or() {
        let e = Expr::new(
            ExprKind::Binary {
                op: BinOp::And,
                left: Heap::new(bool_lit(true)),
                right: Heap::new(var("x")),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(true && x)"));

        let e = Expr::new(
            ExprKind::Binary {
                op: BinOp::Or,
                left: Heap::new(var("x")),
                right: Heap::new(bool_lit(false)),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(x || false)"));
    }

    #[test]
    fn lower_bitwise_uses_ocaml_keywords() {
        let e = Expr::new(
            ExprKind::Binary {
                op: BinOp::BitAnd,
                left: Heap::new(var("a")),
                right: Heap::new(var("b")),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(a land b)"));
    }

    use verum_ast::pattern::{MatchArm, Pattern, PatternKind, VariantPatternData};

    fn wildcard_pat() -> Pattern {
        Pattern::new(PatternKind::Wildcard, span())
    }

    fn ident_pat(name: &str) -> Pattern {
        Pattern::new(
            PatternKind::Ident {
                by_ref: false,
                mutable: false,
                name: Ident::new(Text::from(name), span()),
                subpattern: verum_common::Maybe::None,
            },
            span(),
        )
    }

    fn variant_pat(ctor: &str, args: Vec<Pattern>) -> Pattern {
        let mut list = verum_common::List::new();
        for a in args {
            list.push(a);
        }
        Pattern::new(
            PatternKind::Variant {
                path: Path::single(Ident::new(Text::from(ctor), span())),
                data: if list.iter().count() == 0 {
                    verum_common::Maybe::None
                } else {
                    verum_common::Maybe::Some(VariantPatternData::Tuple(list))
                },
            },
            span(),
        )
    }

    fn arm(p: Pattern, body: Expr) -> MatchArm {
        MatchArm::new(p, verum_common::Maybe::None, Heap::new(body), span())
    }

    #[test]
    fn lower_match_with_variant_arms() {
        let mut arms = verum_common::List::new();
        arms.push(arm(variant_pat("None", vec![]), int_lit(0)));
        arms.push(arm(
            variant_pat("Some", vec![ident_pat("v")]),
            var("v"),
        ));
        let e = Expr::new(
            ExprKind::Match {
                expr: Heap::new(var("opt")),
                arms,
            },
            span(),
        );
        assert_eq!(
            lower_expr(&e).as_deref(),
            Some("(match opt with | None -> 0 | Some v -> v)"),
        );
    }

    #[test]
    fn lower_match_wildcard_arm() {
        let mut arms = verum_common::List::new();
        arms.push(arm(ident_pat("x"), var("x")));
        arms.push(arm(wildcard_pat(), int_lit(0)));
        let e = Expr::new(
            ExprKind::Match {
                expr: Heap::new(var("n")),
                arms,
            },
            span(),
        );
        assert_eq!(
            lower_expr(&e).as_deref(),
            Some("(match n with | x -> x | _ -> 0)"),
        );
    }

    #[test]
    fn lower_method_call_zero_args() {
        let e = Expr::new(
            ExprKind::MethodCall {
                receiver: Heap::new(var("xs")),
                method: Ident::new(Text::from("len"), span()),
                type_args: verum_common::List::new(),
                args: verum_common::List::new(),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(len xs)"));
    }

    #[test]
    fn lower_method_call_with_args() {
        let mut args = verum_common::List::new();
        args.push(int_lit(2));
        let e = Expr::new(
            ExprKind::MethodCall {
                receiver: Heap::new(var("xs")),
                method: Ident::new(Text::from("get"), span()),
                type_args: verum_common::List::new(),
                args,
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(get xs 2)"));
    }

    #[test]
    fn lower_field_access() {
        let e = Expr::new(
            ExprKind::Field {
                expr: Heap::new(var("p")),
                field: Ident::new(Text::from("x"), span()),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("p.x"));
    }

    use verum_ast::expr::ClosureParam;

    fn closure(param_names: &[&str], body: Expr) -> Expr {
        let mut params = verum_common::List::new();
        for n in param_names {
            params.push(ClosureParam::new(ident_pat(n), verum_common::Maybe::None, span()));
        }
        Expr::new(
            ExprKind::Closure {
                async_: false,
                move_: false,
                params,
                contexts: verum_common::List::new(),
                return_type: verum_common::Maybe::None,
                body: Heap::new(body),
            },
            span(),
        )
    }

    #[test]
    fn lower_index_uses_array_get() {
        let e = Expr::new(
            ExprKind::Index {
                expr: Heap::new(var("xs")),
                index: Heap::new(int_lit(0)),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(Array.get xs 0)"));
    }

    #[test]
    fn lower_null_coalesce_lowers_to_match() {
        let e = Expr::new(
            ExprKind::NullCoalesce {
                left: Heap::new(var("a")),
                right: Heap::new(int_lit(0)),
            },
            span(),
        );
        assert_eq!(
            lower_expr(&e).as_deref(),
            Some("(match a with Some v -> v | None -> 0)"),
        );
    }

    #[test]
    fn lower_tuple_two_elements() {
        let mut elems = verum_common::List::new();
        elems.push(int_lit(1));
        elems.push(int_lit(2));
        let e = Expr::new(ExprKind::Tuple(elems), span());
        assert_eq!(lower_expr(&e).as_deref(), Some("(1, 2)"));
    }

    #[test]
    fn lower_tuple_index_zero_uses_fst() {
        let e = Expr::new(
            ExprKind::TupleIndex { expr: Heap::new(var("p")), index: 0 },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(fst p)"));
    }

    #[test]
    fn lower_tuple_index_two_returns_none() {
        let e = Expr::new(
            ExprKind::TupleIndex { expr: Heap::new(var("t")), index: 2 },
            span(),
        );
        assert!(lower_expr(&e).is_none());
    }

    #[test]
    fn lower_pipeline_uses_native_operator() {
        let e = Expr::new(
            ExprKind::Pipeline {
                left: Heap::new(var("xs")),
                right: Heap::new(var("len")),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(xs |> len)"));
    }

    #[test]
    fn lower_closure_with_two_params() {
        let body = Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: Heap::new(var("x")),
                right: Heap::new(var("y")),
            },
            span(),
        );
        let e = closure(&["x", "y"], body);
        assert_eq!(lower_expr(&e).as_deref(), Some("(fun x y -> (x + y))"));
    }

    #[test]
    fn lower_closure_zero_params_uses_unit() {
        let e = closure(&[], int_lit(42));
        assert_eq!(lower_expr(&e).as_deref(), Some("(fun () -> 42)"));
    }

    #[test]
    fn lower_match_with_guard_returns_none() {
        let mut arms = verum_common::List::new();
        arms.push(MatchArm::new(
            ident_pat("x"),
            verum_common::Maybe::Some(Heap::new(var("cond"))),
            Heap::new(var("x")),
            span(),
        ));
        let e = Expr::new(
            ExprKind::Match {
                expr: Heap::new(var("n")),
                arms,
            },
            span(),
        );
        assert!(lower_expr(&e).is_none());
    }
}
