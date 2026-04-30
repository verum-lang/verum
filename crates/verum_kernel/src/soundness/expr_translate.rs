//! Verum AST → Coq/Lean Prop syntax translator.
//!
//! Task #140 / MSFS-L4.7 — the foundational piece that lifts the
//! cross-format roundtrip's foreign-tool re-check from "this name
//! exists in Prop" to "this proposition's TYPE STRUCTURE is
//! well-formed in the target system."
//!
//! ## Architecture (protocol-driven)
//!
//! Single trait [`ExprRenderer`] with one implementation per foreign
//! format.  `CoqExprRenderer` and `LeanExprRenderer` ship in this
//! module.  Adding Isabelle / Agda / Dedukti is a single new
//! [`ExprRenderer`] instance — the corpus-export walker and the
//! cross-format gate are unchanged.
//!
//! ## What gets translated
//!
//!   * Literals: `Int n` → `n` (Coq `nat`/`Z` infer; Lean `Nat`/`Int` infer);
//!     `Bool b` → `True` / `False`; `Text s` → `"s"`.
//!   * Path: identifier reference rendered verbatim (segments joined
//!     by `_` per foreign-tool conventions; sanitisation matches
//!     `corpus_export::sanitise_theorem_name`).
//!   * Binary operators (the prop-level subset): `==` → `=`, `&&` →
//!     `/\` (Coq) / `∧` (Lean), `||` → `\/` / `∨`, `->` → `->` / `→`,
//!     `<->` → `<->` / `↔`, comparison operators map directly,
//!     arithmetic likewise.
//!   * Unary: `!x` → `~x` (Coq) / `¬x` (Lean); `-x` → `-x`.
//!   * Quantifiers: `forall x, body` (Coq) / `∀ x, body` (Lean);
//!     `exists` mirrors.
//!   * Parens / parenthesised expressions: pass through.
//!   * Application: `f a b` (curried) — works directly in both Coq
//!     and Lean.
//!
//! ## What falls back to placeholder
//!
//! Match expressions, refinement types, tagged literals, complex
//! pattern bindings — these don't map cleanly to a one-line Prop
//! syntax, so the renderer returns `Prop` and emits the original
//! Verum text in a comment.  The CI gate's invariant degrades from
//! "TYPE structure verified" to "name binding verified" for these
//! shapes — same level as the pre-translator placeholder.

use serde::{Deserialize, Serialize};
use verum_ast::expr::{Expr, ExprKind};
use verum_ast::literal::{LiteralKind, StringLit};
use verum_ast::{BinOp, UnOp};

/// Output of translating one expression.  Carries either a successful
/// rendering or a fall-back diagnostic so the caller can decide how
/// to embed the result (full prop substitution vs. comment-only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TranslatedExpr {
    /// The expression translated cleanly.  `text` is ready to be
    /// embedded after `Theorem foo : ` (Coq) / `theorem foo : ` (Lean).
    Translated {
        /// Foreign-tool source text — e.g. `n = 7` or `forall x, P x`.
        text: String,
    },
    /// The expression's shape couldn't be translated faithfully
    /// (refinement types, complex pattern binders, etc.).  The
    /// caller falls back to `Prop` and embeds `original` in a
    /// comment so the foreign-tool reviewer can see what the
    /// statement was supposed to say.
    Fallback {
        /// Why the fallback fired (e.g., "match expression",
        /// "refinement type").  Goes into the diagnostic comment.
        reason: String,
        /// The Verum-side rendering (via `verum_ast::pretty::format_expr`)
        /// preserved verbatim for the comment.
        original: String,
    },
}

impl TranslatedExpr {
    /// `true` iff the translation succeeded.
    pub fn is_translated(&self) -> bool {
        matches!(self, TranslatedExpr::Translated { .. })
    }

    /// Get the translated text, or `None` for fallbacks.
    pub fn text(&self) -> Option<&str> {
        match self {
            TranslatedExpr::Translated { text } => Some(text.as_str()),
            TranslatedExpr::Fallback { .. } => None,
        }
    }
}

/// Per-format translator interface.  Adding a new foreign format is
/// one new instance.
pub trait ExprRenderer {
    /// Stable identifier — `"coq"`, `"lean"`, etc.
    fn id(&self) -> &'static str;

    /// Translate a complete expression into the foreign-tool's
    /// proposition syntax.  Returns [`TranslatedExpr::Fallback`] when
    /// the shape can't be translated cleanly.
    fn render(&self, expr: &Expr) -> TranslatedExpr;
}

// =============================================================================
// CoqExprRenderer
// =============================================================================

/// Coq backend for proposition translation.
pub struct CoqExprRenderer;

impl CoqExprRenderer {
    /// Construct a fresh renderer.
    pub fn new() -> Self {
        Self
    }
}

impl Default for CoqExprRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl ExprRenderer for CoqExprRenderer {
    fn id(&self) -> &'static str {
        "coq"
    }

    fn render(&self, expr: &Expr) -> TranslatedExpr {
        match render_expr_coq(expr) {
            Some(text) => TranslatedExpr::Translated { text },
            None => TranslatedExpr::Fallback {
                reason: classify_unrenderable(expr).to_string(),
                original: verum_ast::pretty::format_expr(expr).to_string(),
            },
        }
    }
}

fn render_expr_coq(expr: &Expr) -> Option<String> {
    match &expr.kind {
        ExprKind::Literal(lit) => render_literal_coq(&lit.kind),
        ExprKind::Path(path) => path.as_ident().map(|i| i.as_str().to_string()),
        ExprKind::Paren(inner) => render_expr_coq(inner).map(|t| format!("({})", t)),
        ExprKind::Unary { op, expr: inner } => {
            let inner_text = render_expr_coq(inner)?;
            match op {
                UnOp::Not => Some(format!("~ {}", parens_if_complex(&inner_text))),
                UnOp::Neg => Some(format!("- {}", parens_if_complex(&inner_text))),
                _ => None,
            }
        }
        ExprKind::Binary { op, left, right } => {
            let l = render_expr_coq(left)?;
            let r = render_expr_coq(right)?;
            let coq_op = match op {
                BinOp::Eq => Some("="),
                BinOp::Ne => Some("<>"),
                BinOp::Lt => Some("<"),
                BinOp::Le => Some("<="),
                BinOp::Gt => Some(">"),
                BinOp::Ge => Some(">="),
                BinOp::And => Some("/\\"),
                BinOp::Or => Some("\\/"),
                BinOp::Imply => Some("->"),
                BinOp::Iff => Some("<->"),
                BinOp::Add => Some("+"),
                BinOp::Sub => Some("-"),
                BinOp::Mul => Some("*"),
                BinOp::Div => Some("/"),
                BinOp::Rem => Some("mod"),
                _ => None,
            }?;
            Some(format!(
                "({} {} {})",
                parens_if_complex(&l),
                coq_op,
                parens_if_complex(&r)
            ))
        }
        ExprKind::Call { func, args, .. } => {
            // Curry-style function application: `f a b c`.
            let head = render_expr_coq(func)?;
            let mut out = head;
            for a in args.iter() {
                let arg_text = render_expr_coq(a)?;
                out.push(' ');
                out.push_str(&parens_if_complex(&arg_text));
            }
            Some(format!("({})", out))
        }
        ExprKind::Forall { bindings, body } => {
            let names: Vec<String> = bindings
                .iter()
                .filter_map(quantifier_binding_name)
                .collect();
            if names.is_empty() {
                return None;
            }
            let body_text = render_expr_coq(body)?;
            Some(format!("(forall {}, {})", names.join(" "), body_text))
        }
        ExprKind::Exists { bindings, body } => {
            let names: Vec<String> = bindings
                .iter()
                .filter_map(quantifier_binding_name)
                .collect();
            if names.is_empty() {
                return None;
            }
            let body_text = render_expr_coq(body)?;
            // Coq `exists` requires nesting: `exists x, exists y, body`.
            let mut out = body_text;
            for name in names.into_iter().rev() {
                out = format!("(exists {}, {})", name, out);
            }
            Some(out)
        }
        _ => None,
    }
}

fn render_literal_coq(kind: &LiteralKind) -> Option<String> {
    match kind {
        LiteralKind::Int(int_lit) => Some(int_lit.value.to_string()),
        LiteralKind::Bool(true) => Some("True".to_string()),
        LiteralKind::Bool(false) => Some("False".to_string()),
        LiteralKind::Text(s) => match s {
            StringLit::Regular(t) | StringLit::MultiLine(t) => {
                Some(format!("\"{}\"", t.as_str().replace('"', "\\\"")))
            }
        },
        LiteralKind::Float(f) => Some(f.value.to_string()),
        _ => None,
    }
}

// =============================================================================
// LeanExprRenderer
// =============================================================================

/// Lean 4 backend for proposition translation.
pub struct LeanExprRenderer;

impl LeanExprRenderer {
    /// Construct a fresh renderer.
    pub fn new() -> Self {
        Self
    }
}

impl Default for LeanExprRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl ExprRenderer for LeanExprRenderer {
    fn id(&self) -> &'static str {
        "lean"
    }

    fn render(&self, expr: &Expr) -> TranslatedExpr {
        match render_expr_lean(expr) {
            Some(text) => TranslatedExpr::Translated { text },
            None => TranslatedExpr::Fallback {
                reason: classify_unrenderable(expr).to_string(),
                original: verum_ast::pretty::format_expr(expr).to_string(),
            },
        }
    }
}

fn render_expr_lean(expr: &Expr) -> Option<String> {
    match &expr.kind {
        ExprKind::Literal(lit) => render_literal_lean(&lit.kind),
        ExprKind::Path(path) => path.as_ident().map(|i| i.as_str().to_string()),
        ExprKind::Paren(inner) => render_expr_lean(inner).map(|t| format!("({})", t)),
        ExprKind::Unary { op, expr: inner } => {
            let inner_text = render_expr_lean(inner)?;
            match op {
                UnOp::Not => Some(format!("¬ {}", parens_if_complex(&inner_text))),
                UnOp::Neg => Some(format!("- {}", parens_if_complex(&inner_text))),
                _ => None,
            }
        }
        ExprKind::Binary { op, left, right } => {
            let l = render_expr_lean(left)?;
            let r = render_expr_lean(right)?;
            let lean_op = match op {
                BinOp::Eq => Some("="),
                BinOp::Ne => Some("≠"),
                BinOp::Lt => Some("<"),
                BinOp::Le => Some("≤"),
                BinOp::Gt => Some(">"),
                BinOp::Ge => Some("≥"),
                BinOp::And => Some("∧"),
                BinOp::Or => Some("∨"),
                BinOp::Imply => Some("→"),
                BinOp::Iff => Some("↔"),
                BinOp::Add => Some("+"),
                BinOp::Sub => Some("-"),
                BinOp::Mul => Some("*"),
                BinOp::Div => Some("/"),
                BinOp::Rem => Some("%"),
                _ => None,
            }?;
            Some(format!(
                "({} {} {})",
                parens_if_complex(&l),
                lean_op,
                parens_if_complex(&r)
            ))
        }
        ExprKind::Call { func, args, .. } => {
            let head = render_expr_lean(func)?;
            let mut out = head;
            for a in args.iter() {
                let arg_text = render_expr_lean(a)?;
                out.push(' ');
                out.push_str(&parens_if_complex(&arg_text));
            }
            Some(format!("({})", out))
        }
        ExprKind::Forall { bindings, body } => {
            let names: Vec<String> = bindings
                .iter()
                .filter_map(quantifier_binding_name)
                .collect();
            if names.is_empty() {
                return None;
            }
            let body_text = render_expr_lean(body)?;
            Some(format!("(∀ {}, {})", names.join(" "), body_text))
        }
        ExprKind::Exists { bindings, body } => {
            let names: Vec<String> = bindings
                .iter()
                .filter_map(quantifier_binding_name)
                .collect();
            if names.is_empty() {
                return None;
            }
            let body_text = render_expr_lean(body)?;
            let mut out = body_text;
            for name in names.into_iter().rev() {
                out = format!("(∃ {}, {})", name, out);
            }
            Some(out)
        }
        _ => None,
    }
}

fn render_literal_lean(kind: &LiteralKind) -> Option<String> {
    match kind {
        LiteralKind::Int(int_lit) => Some(int_lit.value.to_string()),
        LiteralKind::Bool(true) => Some("True".to_string()),
        LiteralKind::Bool(false) => Some("False".to_string()),
        LiteralKind::Text(s) => match s {
            StringLit::Regular(t) | StringLit::MultiLine(t) => {
                Some(format!("\"{}\"", t.as_str().replace('"', "\\\"")))
            }
        },
        LiteralKind::Float(f) => Some(f.value.to_string()),
        _ => None,
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Project: extract the binder name from a `QuantifierBinding`.  Only
/// Ident-pattern binders translate cleanly; any other shape returns
/// `None`.
fn quantifier_binding_name(qb: &verum_ast::expr::QuantifierBinding) -> Option<String> {
    use verum_ast::pattern::PatternKind;
    match &qb.pattern.kind {
        PatternKind::Ident { name, .. } => Some(name.as_str().to_string()),
        _ => None,
    }
}

/// Wrap text in `(…)` when it isn't already parenthesised AND it
/// looks complex enough to need disambiguation in op-precedence
/// contexts.  Heuristic: any whitespace inside means there's a
/// substructure worth bracketing.
fn parens_if_complex(text: &str) -> String {
    if text.starts_with('(') && text.ends_with(')') {
        text.to_string()
    } else if text.contains(' ') {
        format!("({})", text)
    } else {
        text.to_string()
    }
}

/// Diagnostic classifier for unrenderable shapes.  Goes into the
/// `Fallback.reason` field.  Only enumerates the common shapes the
/// MSFS corpus actually carries — everything else falls through to
/// the generic catch-all.
fn classify_unrenderable(expr: &Expr) -> &'static str {
    match &expr.kind {
        ExprKind::Match { .. } => "match expression",
        ExprKind::If { .. } => "if-then-else",
        ExprKind::Block(_) => "block expression",
        ExprKind::Closure { .. } => "closure expression",
        ExprKind::MethodCall { .. } => "method call",
        ExprKind::Tuple(_) => "tuple expression",
        ExprKind::Array(_) => "array literal",
        ExprKind::Range { .. } => "range expression",
        ExprKind::Index { .. } => "index expression",
        ExprKind::Cast { .. } => "cast expression",
        ExprKind::Try(_) => "try expression",
        ExprKind::Await { .. } => "await expression",
        ExprKind::Loop { .. } => "loop expression",
        ExprKind::While { .. } => "while expression",
        ExprKind::For { .. } => "for expression",
        ExprKind::Return { .. } => "return expression",
        _ => "unsupported expression shape",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::literal::IntLit;
    use verum_ast::pattern::{Pattern, PatternKind};
    use verum_ast::ty::{Ident, Path};
    use verum_ast::Span;
    use verum_ast::Literal;
    use verum_common::Heap;

    fn ident_expr(name: &str) -> Expr {
        Expr::new(
            ExprKind::Path(Path::single(Ident::new(name, Span::dummy()))),
            Span::dummy(),
        )
    }

    fn int_lit(n: i64) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal::int(n as i128, Span::dummy())),
            Span::dummy(),
        )
    }

    fn bool_lit(b: bool) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal::bool(b, Span::dummy())),
            Span::dummy(),
        )
    }

    fn binop(op: BinOp, l: Expr, r: Expr) -> Expr {
        Expr::new(
            ExprKind::Binary {
                op,
                left: Heap::new(l),
                right: Heap::new(r),
            },
            Span::dummy(),
        )
    }

    fn ident_binding(name: &str) -> verum_ast::expr::QuantifierBinding {
        verum_ast::expr::QuantifierBinding {
            pattern: Pattern {
                kind: PatternKind::Ident {
                    by_ref: false,
                    mutable: false,
                    name: Ident::new(name, Span::dummy()),
                    subpattern: verum_common::Maybe::None,
                },
                span: Span::dummy(),
            },
            ty: verum_common::Maybe::None,
            domain: verum_common::Maybe::None,
            guard: verum_common::Maybe::None,
            span: Span::dummy(),
        }
    }

    #[test]
    fn coq_translates_integer_equality() {
        let e = binop(BinOp::Eq, ident_expr("n"), int_lit(7));
        let r = CoqExprRenderer::new().render(&e);
        match r {
            TranslatedExpr::Translated { text } => assert_eq!(text, "(n = 7)"),
            other => panic!("expected Translated, got {:?}", other),
        }
    }

    #[test]
    fn lean_translates_integer_equality() {
        let e = binop(BinOp::Eq, ident_expr("n"), int_lit(7));
        let r = LeanExprRenderer::new().render(&e);
        match r {
            TranslatedExpr::Translated { text } => assert_eq!(text, "(n = 7)"),
            other => panic!("expected Translated, got {:?}", other),
        }
    }

    #[test]
    fn coq_translates_logical_and() {
        let e = binop(
            BinOp::And,
            binop(BinOp::Lt, ident_expr("a"), int_lit(0)),
            binop(BinOp::Gt, ident_expr("b"), int_lit(0)),
        );
        let r = CoqExprRenderer::new().render(&e);
        let text = r.text().expect("translation must succeed");
        // Coq /\ for &&
        assert!(text.contains("/\\"));
        assert!(text.contains("(a < 0)"));
        assert!(text.contains("(b > 0)"));
    }

    #[test]
    fn lean_translates_logical_and() {
        let e = binop(
            BinOp::And,
            binop(BinOp::Lt, ident_expr("a"), int_lit(0)),
            binop(BinOp::Gt, ident_expr("b"), int_lit(0)),
        );
        let r = LeanExprRenderer::new().render(&e);
        let text = r.text().expect("translation must succeed");
        // Lean ∧ for &&
        assert!(text.contains("∧"));
    }

    #[test]
    fn coq_translates_imply() {
        let e = binop(BinOp::Imply, bool_lit(true), bool_lit(false));
        let r = CoqExprRenderer::new().render(&e);
        let text = r.text().unwrap();
        assert_eq!(text, "(True -> False)");
    }

    #[test]
    fn lean_translates_imply() {
        let e = binop(BinOp::Imply, bool_lit(true), bool_lit(false));
        let r = LeanExprRenderer::new().render(&e);
        let text = r.text().unwrap();
        assert_eq!(text, "(True → False)");
    }

    #[test]
    fn coq_translates_negation() {
        let e = Expr::new(
            ExprKind::Unary {
                op: UnOp::Not,
                expr: Heap::new(ident_expr("p")),
            },
            Span::dummy(),
        );
        let text = CoqExprRenderer::new().render(&e).text().unwrap().to_string();
        assert_eq!(text, "~ p");
    }

    #[test]
    fn lean_translates_negation() {
        let e = Expr::new(
            ExprKind::Unary {
                op: UnOp::Not,
                expr: Heap::new(ident_expr("p")),
            },
            Span::dummy(),
        );
        let text = LeanExprRenderer::new().render(&e).text().unwrap().to_string();
        assert_eq!(text, "¬ p");
    }

    #[test]
    fn coq_translates_forall() {
        let body = binop(BinOp::Eq, ident_expr("x"), ident_expr("x"));
        let e = Expr::new(
            ExprKind::Forall {
                bindings: {
                    let mut bs = verum_common::List::new();
                    bs.push(ident_binding("x"));
                    bs
                },
                body: Heap::new(body),
            },
            Span::dummy(),
        );
        let text = CoqExprRenderer::new().render(&e).text().unwrap().to_string();
        assert_eq!(text, "(forall x, (x = x))");
    }

    #[test]
    fn lean_translates_forall() {
        let body = binop(BinOp::Eq, ident_expr("x"), ident_expr("x"));
        let e = Expr::new(
            ExprKind::Forall {
                bindings: {
                    let mut bs = verum_common::List::new();
                    bs.push(ident_binding("x"));
                    bs
                },
                body: Heap::new(body),
            },
            Span::dummy(),
        );
        let text = LeanExprRenderer::new().render(&e).text().unwrap().to_string();
        assert_eq!(text, "(∀ x, (x = x))");
    }

    #[test]
    fn coq_translates_exists() {
        let body = binop(BinOp::Eq, ident_expr("y"), int_lit(0));
        let e = Expr::new(
            ExprKind::Exists {
                bindings: {
                    let mut bs = verum_common::List::new();
                    bs.push(ident_binding("y"));
                    bs
                },
                body: Heap::new(body),
            },
            Span::dummy(),
        );
        let text = CoqExprRenderer::new().render(&e).text().unwrap().to_string();
        assert_eq!(text, "(exists y, (y = 0))");
    }

    #[test]
    fn lean_translates_exists() {
        let body = binop(BinOp::Eq, ident_expr("y"), int_lit(0));
        let e = Expr::new(
            ExprKind::Exists {
                bindings: {
                    let mut bs = verum_common::List::new();
                    bs.push(ident_binding("y"));
                    bs
                },
                body: Heap::new(body),
            },
            Span::dummy(),
        );
        let text = LeanExprRenderer::new().render(&e).text().unwrap().to_string();
        assert_eq!(text, "(∃ y, (y = 0))");
    }

    #[test]
    fn fallback_for_unsupported_shape_carries_reason() {
        // Tuple expressions don't translate cleanly to a one-line
        // Coq Prop syntax (would require Coq's pair / sigma encoding
        // and we'd need type-context to disambiguate).  Renderer
        // falls back with a non-empty reason and original text.
        let e = Expr::new(
            ExprKind::Tuple({
                let mut elems = verum_common::List::new();
                elems.push(ident_expr("x"));
                elems.push(ident_expr("y"));
                elems
            }),
            Span::dummy(),
        );
        let r = CoqExprRenderer::new().render(&e);
        match r {
            TranslatedExpr::Fallback { reason, original } => {
                assert_eq!(reason, "tuple expression");
                assert!(!original.is_empty());
            }
            other => panic!("expected Fallback, got {:?}", other),
        }
    }

    #[test]
    fn coq_translates_arithmetic_expression() {
        // (a + b) * 2
        let e = binop(
            BinOp::Mul,
            binop(BinOp::Add, ident_expr("a"), ident_expr("b")),
            int_lit(2),
        );
        let text = CoqExprRenderer::new().render(&e).text().unwrap().to_string();
        // Outer parenthesisation guarantees no precedence ambiguity.
        assert!(text.starts_with('('));
        assert!(text.contains("(a + b)"));
        assert!(text.contains("2"));
        assert!(text.contains("*"));
    }

    #[test]
    fn lean_translates_comparison_with_ne() {
        let e = binop(BinOp::Ne, ident_expr("p"), bool_lit(true));
        let text = LeanExprRenderer::new().render(&e).text().unwrap().to_string();
        assert!(text.contains("≠"));
    }

    #[test]
    fn coq_translates_call_to_curried_form() {
        // f(a, b) → (f a b)
        let e = Expr::new(
            ExprKind::Call {
                func: Heap::new(ident_expr("f")),
                args: {
                    let mut args = verum_common::List::new();
                    args.push(ident_expr("a"));
                    args.push(ident_expr("b"));
                    args
                },
                type_args: verum_common::List::new(),
            },
            Span::dummy(),
        );
        let text = CoqExprRenderer::new().render(&e).text().unwrap().to_string();
        assert_eq!(text, "(f a b)");
    }

    #[test]
    fn lean_translates_call_to_curried_form() {
        let e = Expr::new(
            ExprKind::Call {
                func: Heap::new(ident_expr("f")),
                args: {
                    let mut args = verum_common::List::new();
                    args.push(ident_expr("a"));
                    args.push(ident_expr("b"));
                    args
                },
                type_args: verum_common::List::new(),
            },
            Span::dummy(),
        );
        let text = LeanExprRenderer::new().render(&e).text().unwrap().to_string();
        assert_eq!(text, "(f a b)");
    }

    #[test]
    fn translated_text_accessor_works() {
        let translated = TranslatedExpr::Translated { text: "x = y".into() };
        assert_eq!(translated.text(), Some("x = y"));
        let fallback = TranslatedExpr::Fallback {
            reason: "match".into(),
            original: "match e { … }".into(),
        };
        assert_eq!(fallback.text(), None);
    }

    #[test]
    fn renderer_id_is_stable() {
        assert_eq!(CoqExprRenderer::new().id(), "coq");
        assert_eq!(LeanExprRenderer::new().id(), "lean");
    }
}
