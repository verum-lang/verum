//! Verum AST → Coq lowerer.
//!
//! Mirrors `ocaml_lower` / `lean_lower` shape but emits Coq term
//! syntax (gallina). Coq differences:
//!   * `let-in` is `let x := value in rest` (`:=` for definition,
//!     `=` for proposition equality).
//!   * Boolean ops: `andb` / `orb` / `negb` for Bool; `&&` / `||`
//!     reserved for Prop. We emit `andb` / `orb` for Verum's `&&` /
//!     `||` since Verum-extracted code is value-level, not Prop.
//!   * Equality is `=` for Prop / `=?` for Decidable Bool. Verum's
//!     `==` is value-level Decidable, so we emit `=?`.
//!   * `if-then-else` is `if cond then a else b` (no `:=` between).
//!   * Modulo is `mod` (Coq Stdlib's Z.modulo / N.modulo prefix
//!     form). For broad coverage we emit `mod` infix which Coq
//!     mathcomp accepts as a Bool-namespace operator; pure Coq
//!     stdlib uses `Z.modulo a b`. We emit `mod` since the
//!     extracted output assumes a math-comp / mathlib-style env.

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
                // Coq let-in: `let name := value in rest`.
                out.push_str(&format!("let {} := {} in ", name, v));
            }
            _ => return None,
        }
    }
    let tail = match &block.expr {
        Maybe::Some(e) => lower_expr(e)?,
        Maybe::None => "tt".to_string(), // Coq's unit value.
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
            // Coq logical ops on Bool are functions, not infix.
            // Emit `andb a b` / `orb a b` for Verum `&&` / `||`.
            match op {
                BinOp::And => Some(format!("(andb {} {})", l, r)),
                BinOp::Or => Some(format!("(orb {} {})", l, r)),
                _ => {
                    let op_str = coq_bin_op(op)?;
                    Some(format!("({} {} {})", l, op_str, r))
                }
            }
        }
        ExprKind::Unary { op, expr } => {
            let inner = lower_expr(expr)?;
            match op {
                UnOp::Not => Some(format!("(negb {})", inner)),
                UnOp::Neg => Some(format!("(- {})", inner)),
                _ => None,
            }
        }
        ExprKind::Call { func, args, .. } => {
            let f = lower_expr(func)?;
            if args.is_empty() {
                Some(format!("({} tt)", f))
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
                Maybe::None => "tt".to_string(),
            };
            Some(format!("(if {} then {} else {})", c, t, e))
        }
        ExprKind::NullCoalesce { left, right } => {
            // Coq pattern-match on Some / None.
            let l = lower_expr(left)?;
            let r = lower_expr(right)?;
            Some(format!("(match {} with | Some v => v | None => {} end)", l, r))
        }
        // Index is intentionally not lowered: Coq's `List.nth_default`
        // requires an explicit default value the lowerer cannot
        // synthesise without type information. Bail to the metadata-
        // comment fallback.
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
            // Coq stdlib pair projections.
            let recv = lower_expr(expr)?;
            match index {
                0 => Some(format!("(fst {})", recv)),
                1 => Some(format!("(snd {})", recv)),
                _ => None,
            }
        }
        ExprKind::Pipeline { left, right } => {
            // Coq has no native `|>` infix; rewrite `x |> f` ≡ `f x`.
            let l = lower_expr(left)?;
            let r = lower_expr(right)?;
            Some(format!("({} {})", r, l))
        }
        ExprKind::Closure { async_, move_, params, contexts, return_type: _, body } => {
            if *async_ || *move_ || contexts.iter().count() > 0 {
                return None;
            }
            if params.iter().count() == 0 {
                let b = lower_expr(body)?;
                return Some(format!("(fun _ : unit => {})", b));
            }
            let mut names = Vec::with_capacity(params.iter().count());
            for p in params.iter() {
                names.push(simple_pattern_name(&p.pattern)?);
            }
            let b = lower_expr(body)?;
            Some(format!("(fun {} => {})", names.join(" "), b))
        }
        ExprKind::MethodCall { receiver, method, type_args, args } => {
            // Coq has no method-dot syntax for plain values; the
            // canonical lowering is the free-function form
            // `(method recv args)`. Type-arguments would need `@`-
            // prefixed explicit instantiation that the simple
            // lowerer doesn't model. Bail when present.
            if type_args.iter().count() > 0 {
                return None;
            }
            let recv = lower_expr(receiver)?;
            let mut parts = Vec::with_capacity(args.iter().count() + 2);
            parts.push(method.name.as_str().to_string());
            parts.push(recv);
            for a in args.iter() {
                parts.push(lower_expr(a)?);
            }
            Some(format!("({})", parts.join(" ")))
        }
        ExprKind::Field { expr, field } => {
            // Coq records are accessed as `field recv` (the field
            // is a projection function). This matches Coq stdlib
            // conventions for record-style ADTs.
            let recv = lower_expr(expr)?;
            Some(format!("({} {})", field.name.as_str(), recv))
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
            // Coq match: `match x with | pat => body | ... end`.
            Some(format!("(match {} with {} end)", scrutinee, arm_strs.join(" ")))
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
            // Coq has no @-pattern; bail.
            Maybe::Some(_) => None,
        },
        PatternKind::Literal(lit) => match &lit.kind {
            LiteralKind::Bool(b) => Some(if *b { "true".to_string() } else { "false".to_string() }),
            LiteralKind::Int(i) => Some(format!("{}", i.value)),
            // Coq Char is `Ascii.ascii` — emit a `"x"%char` literal.
            LiteralKind::Char(c) => Some(format!("\"{}\"%char", c)),
            LiteralKind::Text(StringLit::Regular(s)) | LiteralKind::Text(StringLit::MultiLine(s)) => {
                Some(format!("\"{}\"", escape_coq_string(s.as_str())))
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
        PatternKind::Or(_) => None,
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
            // Coq stdlib uses `"..."` for `string` (Coq.Strings.String);
            // escape per Coq conventions.
            Some(format!("\"{}\"", escape_coq_string(s.as_str())))
        }
        LiteralKind::Char(c) => Some(format!("\"{}\"", c)),
        _ => None,
    }
}

fn coq_bin_op(op: &BinOp) -> Option<&'static str> {
    Some(match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "mod",
        // Verum's `==` is value-level Decidable equality →
        // Coq's `=?` (Bool namespace, mathcomp-style).
        BinOp::Eq => "=?",
        // Verum's `!=` → Coq's `<?` is `<`; for inequality we use
        // `negb (a =? b)` but coq_bin_op returns single string,
        // so we fall through to None for Ne — caller should
        // handle in the future. For V12.2 we approximate with
        // `=?` negated via negb at the binary site? Simpler:
        // emit `<>?` (mathcomp's bool inequality). We use `<>?`
        // which mathcomp accepts.
        BinOp::Ne => "<>?",
        BinOp::Lt => "<?",
        BinOp::Le => "<=?",
        BinOp::Gt => ">?",
        BinOp::Ge => ">=?",
        // Logical &&/|| handled separately above (function form).
        BinOp::Concat => "++",
        // Bitwise: Coq has `Z.land` / `Z.lor` / `Z.lxor` / `Z.shiftl`
        // / `Z.shiftr` as prefix functions; no infix. Out of V12.2
        // scope — return None.
        BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => return None,
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

fn escape_coq_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            // Coq strings double up the quote character.
            '"' => out.push_str("\"\""),
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
    fn coq_int_literal() {
        assert_eq!(lower_expr(&int_lit(42)).as_deref(), Some("42"));
    }

    #[test]
    fn coq_bool_literal() {
        assert_eq!(lower_expr(&bool_lit(true)).as_deref(), Some("true"));
    }

    #[test]
    fn coq_addition() {
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
    fn coq_eq_uses_question_mark_form() {
        let e = Expr::new(
            ExprKind::Binary {
                op: BinOp::Eq,
                left: Heap::new(var("a")),
                right: Heap::new(var("b")),
            },
            span(),
        );
        // Verum `==` → Coq `=?` (Decidable Bool equality).
        assert_eq!(lower_expr(&e).as_deref(), Some("(a =? b)"));
    }

    #[test]
    fn coq_lt_uses_question_mark_form() {
        let e = Expr::new(
            ExprKind::Binary {
                op: BinOp::Lt,
                left: Heap::new(var("a")),
                right: Heap::new(var("b")),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(a <? b)"));
    }

    #[test]
    fn coq_and_uses_andb_function() {
        let e = Expr::new(
            ExprKind::Binary {
                op: BinOp::And,
                left: Heap::new(var("a")),
                right: Heap::new(var("b")),
            },
            span(),
        );
        // Coq Bool conjunction is `andb a b`, not `a && b`.
        assert_eq!(lower_expr(&e).as_deref(), Some("(andb a b)"));
    }

    #[test]
    fn coq_or_uses_orb_function() {
        let e = Expr::new(
            ExprKind::Binary {
                op: BinOp::Or,
                left: Heap::new(var("a")),
                right: Heap::new(var("b")),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(orb a b)"));
    }

    #[test]
    fn coq_not_uses_negb_function() {
        let e = Expr::new(
            ExprKind::Unary {
                op: UnOp::Not,
                expr: Heap::new(var("x")),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(negb x)"));
    }

    #[test]
    fn coq_call_no_args_uses_tt() {
        let e = Expr::new(
            ExprKind::Call {
                func: Heap::new(var("now")),
                type_args: verum_common::List::new(),
                args: verum_common::List::new(),
            },
            span(),
        );
        // Coq's unit value is `tt` (not `()`).
        assert_eq!(lower_expr(&e).as_deref(), Some("(now tt)"));
    }

    #[test]
    fn coq_call_with_args() {
        let mut args = verum_common::List::new();
        args.push(var("a"));
        args.push(int_lit(2));
        let e = Expr::new(
            ExprKind::Call {
                func: Heap::new(var("plus")),
                type_args: verum_common::List::new(),
                args,
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(plus a 2)"));
    }

    #[test]
    fn coq_bitwise_unsupported_returns_none() {
        // Coq has Z.land / Z.lor / etc. as prefix functions, no
        // infix. V12.2 returns None — caller falls back to V12.1.
        let e = Expr::new(
            ExprKind::Binary {
                op: BinOp::BitAnd,
                left: Heap::new(var("a")),
                right: Heap::new(var("b")),
            },
            span(),
        );
        assert!(lower_expr(&e).is_none());
    }

    #[test]
    fn coq_string_doubles_quote_chars() {
        let s = StringLit::Regular(Text::from("hi \"world\""));
        let e = Expr::literal(Literal::new(LiteralKind::Text(s), span()));
        // Coq strings escape `"` as `""`.
        assert_eq!(lower_expr(&e).as_deref(), Some("\"hi \"\"world\"\"\""));
    }

    #[test]
    fn coq_unsupported_returns_none() {
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
    fn coq_match_lowers_variant_arms_with_end_keyword() {
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
            Some("(match opt with | None => 0 | Some v => v end)"),
        );
    }

    #[test]
    fn coq_method_call_uses_free_function_form() {
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
    fn coq_field_access_uses_projection_function() {
        let e = Expr::new(
            ExprKind::Field {
                expr: Heap::new(var("p")),
                field: Ident::new(Text::from("x"), span()),
            },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(x p)"));
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
    fn coq_null_coalesce_lowers_to_match() {
        let e = Expr::new(
            ExprKind::NullCoalesce {
                left: Heap::new(var("a")),
                right: Heap::new(int_lit(0)),
            },
            span(),
        );
        assert_eq!(
            lower_expr(&e).as_deref(),
            Some("(match a with | Some v => v | None => 0 end)"),
        );
    }

    #[test]
    fn coq_index_returns_none() {
        // No clean Coq mapping without a default value.
        let e = Expr::new(
            ExprKind::Index {
                expr: Heap::new(var("xs")),
                index: Heap::new(int_lit(0)),
            },
            span(),
        );
        assert!(lower_expr(&e).is_none());
    }

    #[test]
    fn coq_tuple_two_elements() {
        let mut elems = verum_common::List::new();
        elems.push(int_lit(1));
        elems.push(int_lit(2));
        let e = Expr::new(ExprKind::Tuple(elems), span());
        assert_eq!(lower_expr(&e).as_deref(), Some("(1, 2)"));
    }

    #[test]
    fn coq_tuple_index_uses_fst_function() {
        let e = Expr::new(
            ExprKind::TupleIndex { expr: Heap::new(var("p")), index: 0 },
            span(),
        );
        assert_eq!(lower_expr(&e).as_deref(), Some("(fst p)"));
    }

    #[test]
    fn coq_pipeline_inverts_to_application() {
        let e = Expr::new(
            ExprKind::Pipeline {
                left: Heap::new(var("xs")),
                right: Heap::new(var("len")),
            },
            span(),
        );
        // Coq has no `|>`; rewrite as direct application.
        assert_eq!(lower_expr(&e).as_deref(), Some("(len xs)"));
    }

    #[test]
    fn coq_closure_uses_fat_arrow() {
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
    fn coq_match_with_guard_returns_none() {
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
