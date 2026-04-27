//! Verum AST → Lean 4 lowerer.
//!
//! Mirrors `ocaml_lower` shape but emits Lean 4 syntax. Lean 4's
//! term language is close to OCaml for the pure-functional core
//! (let-in, if-then-else, applications, infix operators), so the
//! same partial-coverage strategy applies: cover the common
//! subset, return `None` for anything else, caller falls back to
//! V12.1 metadata comment.
//!
//! Lean 4 differences from OCaml that drive translation choices:
//!   * `=` is propositional equality at the term level; we use
//!     `==` (Decidable) for runtime equality which matches Verum's
//!     `==` semantics.
//!   * `!=` is `≠` lexically but Lean accepts the ASCII fallback
//!     `!=` (mathlib4) — we emit `≠` for cleanliness.
//!   * Boolean ops are `&&` / `||` (same as Verum + OCaml).
//!   * Modulo is `%`, not `mod`.
//!   * Bitwise operators are `&&&` / `|||` / `^^^` / `<<<` / `>>>`
//!     (Lean 4 core).

use verum_ast::expr::{BinOp, Block, ConditionKind, Expr, ExprKind, IfCondition, UnOp};
use verum_ast::literal::{LiteralKind, StringLit};
use verum_common::Maybe;

pub(super) fn lower_block(block: &Block) -> Option<String> {
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
                // Lean 4 let-in: `let name := value; rest` (the `;`
                // separator is the term-mode chain).
                out.push_str(&format!("let {} := {}; ", name, v));
            }
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

pub(super) fn lower_expr(expr: &Expr) -> Option<String> {
    match &expr.kind {
        ExprKind::Literal(lit) => lower_literal(lit),
        ExprKind::Path(path) => Some(path.last_segment_name().to_string()),
        ExprKind::Paren(inner) => Some(format!("({})", lower_expr(inner)?)),
        ExprKind::Binary { op, left, right } => {
            let l = lower_expr(left)?;
            let r = lower_expr(right)?;
            let op_str = lean_bin_op(op)?;
            Some(format!("({} {} {})", l, op_str, r))
        }
        ExprKind::Unary { op, expr } => {
            let inner = lower_expr(expr)?;
            let op_str = lean_un_op(op)?;
            Some(format!("({} {})", op_str, inner))
        }
        ExprKind::Call { func, args, .. } => {
            let f = lower_expr(func)?;
            if args.is_empty() {
                // Lean 4 unit call: just apply to `()`.
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
            // Lean 4 `arr[i]!` — the panicking accessor matches
            // Verum's Tier-0 indexing semantics (refinement-typed
            // bounds checking is handled at verification time).
            let recv = lower_expr(expr)?;
            let idx = lower_expr(index)?;
            Some(format!("({}[{}]!)", recv, idx))
        }
        ExprKind::NullCoalesce { left, right } => {
            let l = lower_expr(left)?;
            let r = lower_expr(right)?;
            Some(format!("(match {} with | some v => v | none => {})", l, r))
        }
        ExprKind::Tuple(elems) => {
            if elems.iter().count() < 2 {
                return None;
            }
            let mut parts = Vec::with_capacity(elems.iter().count());
            for e in elems.iter() {
                parts.push(lower_expr(e)?);
            }
            Some(format!("({})", parts.join(", ")))
        }
        ExprKind::TupleIndex { expr, index } => {
            // Lean 4 supports `t.fst` / `t.snd` on Prod values.
            let recv = lower_expr(expr)?;
            match index {
                0 => Some(format!("({}.fst)", recv)),
                1 => Some(format!("({}.snd)", recv)),
                _ => None,
            }
        }
        ExprKind::Pipeline { left, right } => {
            // Lean 4 has `|>` natively (Mathlib + core).
            let l = lower_expr(left)?;
            let r = lower_expr(right)?;
            Some(format!("({} |> {})", l, r))
        }
        ExprKind::Closure { async_, move_, params, contexts, return_type: _, body } => {
            if *async_ || *move_ || contexts.iter().count() > 0 {
                return None;
            }
            if params.iter().count() == 0 {
                let b = lower_expr(body)?;
                return Some(format!("(fun () => {})", b));
            }
            let mut names = Vec::with_capacity(params.iter().count());
            for p in params.iter() {
                names.push(simple_pattern_name(&p.pattern)?);
            }
            let b = lower_expr(body)?;
            Some(format!("(fun {} => {})", names.join(" "), b))
        }
        ExprKind::MethodCall { receiver, method, type_args, args } => {
            // Lean 4 supports `recv.method args` natively; type-args
            // need named-arg syntax that the simple lowerer doesn't
            // model. Bail when present.
            if type_args.iter().count() > 0 {
                return None;
            }
            let recv = lower_expr(receiver)?;
            if args.iter().count() == 0 {
                Some(format!("({}.{})", recv, method.name.as_str()))
            } else {
                let mut parts = Vec::with_capacity(args.iter().count());
                for a in args.iter() {
                    parts.push(lower_expr(a)?);
                }
                Some(format!("({}.{} {})", recv, method.name.as_str(), parts.join(" ")))
            }
        }
        ExprKind::Field { expr, field } => {
            let recv = lower_expr(expr)?;
            Some(format!("{}.{}", recv, field.name.as_str()))
        }
        ExprKind::Match { expr, arms } => {
            let scrutinee = lower_expr(expr)?;
            let mut arm_strs = Vec::with_capacity(arms.iter().count());
            for arm in arms.iter() {
                if matches!(arm.guard, Maybe::Some(_)) {
                    return None;
                }
                let pat = lower_pattern(&arm.pattern)?;
                let body = lower_expr(&arm.body)?;
                arm_strs.push(format!("| {} => {}", pat, body));
            }
            Some(format!("(match {} with {})", scrutinee, arm_strs.join(" ")))
        }
        _ => None,
    }
}

fn lower_pattern(pat: &verum_ast::pattern::Pattern) -> Option<String> {
    use verum_ast::pattern::{PatternKind, VariantPatternData};
    match &pat.kind {
        PatternKind::Wildcard => Some("_".to_string()),
        PatternKind::Ident { name, subpattern, .. } => match subpattern {
            Maybe::None => Some(name.name.as_str().to_string()),
            // Lean 4 has no @-pattern; bail.
            Maybe::Some(_) => None,
        },
        PatternKind::Literal(lit) => match &lit.kind {
            LiteralKind::Bool(b) => Some(if *b { "true".to_string() } else { "false".to_string() }),
            LiteralKind::Int(i) => Some(format!("{}", i.value)),
            LiteralKind::Char(c) => Some(format!("'{}'", c)),
            LiteralKind::Text(StringLit::Regular(s)) | LiteralKind::Text(StringLit::MultiLine(s)) => {
                Some(format!("\"{}\"", escape_lean_string(s.as_str())))
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
                    Some(format!("{} {}", ctor, parts.join(" ")))
                }
                Maybe::Some(VariantPatternData::Record { .. }) => None,
            }
        }
        PatternKind::Or(_) => None, // Lean's `|` is at the arm level, not nested.
        PatternKind::Paren(inner) => Some(format!("({})", lower_pattern(inner)?)),
        _ => None,
    }
}

fn lower_if_condition(cond: &IfCondition) -> Option<String> {
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
            Some(format!("\"{}\"", escape_lean_string(s.as_str())))
        }
        LiteralKind::Char(c) => Some(format!("'{}'", c)),
        _ => None,
    }
}

fn lean_bin_op(op: &BinOp) -> Option<&'static str> {
    Some(match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::Concat => "++",
        BinOp::BitAnd => "&&&",
        BinOp::BitOr => "|||",
        BinOp::BitXor => "^^^",
        BinOp::Shl => "<<<",
        BinOp::Shr => ">>>",
        _ => return None,
    })
}

fn lean_un_op(op: &UnOp) -> Option<&'static str> {
    Some(match op {
        UnOp::Neg => "-",
        UnOp::Not => "!",
        _ => return None,
    })
}

fn simple_pattern_name(pat: &verum_ast::pattern::Pattern) -> Option<String> {
    use verum_ast::pattern::PatternKind;
    match &pat.kind {
        PatternKind::Ident { name, .. } => Some(name.name.as_str().to_string()),
        PatternKind::Wildcard => Some("_".to_string()),
        _ => None,
    }
}

fn escape_lean_string(s: &str) -> String {
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
    fn lean_int_literal() {
        assert_eq!(lower_expr(&int_lit(42)).as_deref(), Some("42"));
    }

    #[test]
    fn lean_bool_literal() {
        assert_eq!(lower_expr(&bool_lit(true)).as_deref(), Some("true"));
        assert_eq!(lower_expr(&bool_lit(false)).as_deref(), Some("false"));
    }

    #[test]
    fn lean_var() {
        assert_eq!(lower_expr(&var("x")).as_deref(), Some("x"));
    }

    #[test]
    fn lean_addition() {
        let e = Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: Heap::new(var("a")),
                right: Heap::new(var("b")),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(a + b)"));
    }

    #[test]
    fn lean_eq_uses_double_equals() {
        let e = Expr::new(
            ExprKind::Binary {
                op: BinOp::Eq,
                left: Heap::new(var("a")),
                right: Heap::new(var("b")),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(a == b)"));
    }

    #[test]
    fn lean_ne_uses_bang_equals() {
        let e = Expr::new(
            ExprKind::Binary {
                op: BinOp::Ne,
                left: Heap::new(var("a")),
                right: Heap::new(var("b")),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(a != b)"));
    }

    #[test]
    fn lean_modulo_uses_percent() {
        let e = Expr::new(
            ExprKind::Binary {
                op: BinOp::Rem,
                left: Heap::new(var("n")),
                right: Heap::new(int_lit(2)),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(n % 2)"));
    }

    #[test]
    fn lean_bitwise_uses_triple_chars() {
        for (op, expected) in [
            (BinOp::BitAnd, "(a &&& b)"),
            (BinOp::BitOr, "(a ||| b)"),
            (BinOp::BitXor, "(a ^^^ b)"),
            (BinOp::Shl, "(a <<< b)"),
            (BinOp::Shr, "(a >>> b)"),
        ] {
            let e = Expr::new(
                ExprKind::Binary {
                    op,
                    left: Heap::new(var("a")),
                    right: Heap::new(var("b")),
                },
                span(),
            );
            assert_eq!(lower_expr(&e).as_deref(), Some(expected));
        }
    }

    #[test]
    fn lean_concat_uses_plus_plus() {
        let e = Expr::new(
            ExprKind::Binary {
                op: BinOp::Concat,
                left: Heap::new(var("xs")),
                right: Heap::new(var("ys")),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(xs ++ ys)"));
    }

    #[test]
    fn lean_unary_neg() {
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
    fn lean_unary_not_uses_bang() {
        let e = Expr::new(
            ExprKind::Unary {
                op: UnOp::Not,
                expr: Heap::new(var("x")),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(! x)"));
    }

    #[test]
    fn lean_call_no_args_uses_unit() {
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
    fn lean_call_with_args() {
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
    fn lean_unsupported_returns_none() {
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
    fn lean_string_escapes() {
        let s = StringLit::Regular(Text::from("hi \"world\""));
        let e = Expr::literal(Literal::new(LiteralKind::Text(s), span()));
        assert_eq!(lower_expr(&e).as_deref(), Some("\"hi \\\"world\\\"\""));
    }

    use verum_ast::pattern::{MatchArm, Pattern, PatternKind, VariantPatternData};

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
    fn lean_match_lowers_variant_arms() {
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
            Some("(match opt with | None => 0 | Some v => v)"),
        );
    }

    #[test]
    fn lean_method_call_zero_args_uses_dot() {
        let e = Expr::new(
            ExprKind::MethodCall {
                receiver: Heap::new(var("xs")),
                method: Ident::new(Text::from("size"), span()),
                type_args: verum_common::List::new(),
                args: verum_common::List::new(),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(xs.size)"));
    }

    #[test]
    fn lean_method_call_with_args_uses_dot() {
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
        assert_eq!(lower_expr(&e).as_deref(), Some("(xs.get 2)"));
    }

    #[test]
    fn lean_field_access_uses_dot() {
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
    fn lean_index_uses_panicking_accessor() {
        let e = Expr::new(
            ExprKind::Index {
                expr: Heap::new(var("xs")),
                index: Heap::new(int_lit(0)),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(xs[0]!)"));
    }

    #[test]
    fn lean_null_coalesce_lowers_to_option_match() {
        let e = Expr::new(
            ExprKind::NullCoalesce {
                left: Heap::new(var("a")),
                right: Heap::new(int_lit(0)),
            },
            span(),
        );
        assert_eq!(
            lower_expr(&e).as_deref(),
            Some("(match a with | some v => v | none => 0)"),
        );
    }

    #[test]
    fn lean_tuple_two_elements() {
        let mut elems = verum_common::List::new();
        elems.push(int_lit(1));
        elems.push(int_lit(2));
        let e = Expr::new(ExprKind::Tuple(elems), span());
        assert_eq!(lower_expr(&e).as_deref(), Some("(1, 2)"));
    }

    #[test]
    fn lean_tuple_index_uses_fst_method() {
        let e = Expr::new(
            ExprKind::TupleIndex { expr: Heap::new(var("p")), index: 0 },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(p.fst)"));
    }

    #[test]
    fn lean_pipeline_uses_native_operator() {
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
    fn lean_closure_uses_fat_arrow() {
        let body = Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: Heap::new(var("x")),
                right: Heap::new(var("y")),
            },
            span(),
        );
        let e = closure(&["x", "y"], body);
        assert_eq!(lower_expr(&e).as_deref(), Some("(fun x y => (x + y))"));
    }

    #[test]
    fn lean_match_with_guard_returns_none() {
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
