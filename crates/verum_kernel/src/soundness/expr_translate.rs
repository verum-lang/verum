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
use verum_ast::ty::{Type, TypeKind};
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
        ExprKind::MethodCall {
            receiver,
            method,
            args,
            ..
        } => {
            // `obj.method(a, b)` → `(method obj a b)`. Type args are
            // dropped; both Coq and Lean infer them from the receiver
            // and arg types in the surrounding context.
            let receiver_text = render_expr_coq(receiver)?;
            let mut out = method.as_str().to_string();
            out.push(' ');
            out.push_str(&parens_if_complex(&receiver_text));
            for a in args.iter() {
                let arg_text = render_expr_coq(a)?;
                out.push(' ');
                out.push_str(&parens_if_complex(&arg_text));
            }
            Some(format!("({})", out))
        }
        ExprKind::Field { expr: inner, field } => {
            // `obj.field` → `(field obj)`. Coq accepts `obj.(field)`
            // record syntax too, but applicative form is uniform with
            // method projections and avoids needing record-context
            // resolution.
            let recv_text = render_expr_coq(inner)?;
            Some(format!(
                "({} {})",
                field.as_str(),
                parens_if_complex(&recv_text)
            ))
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
        ExprKind::MethodCall {
            receiver,
            method,
            args,
            ..
        } => {
            let receiver_text = render_expr_lean(receiver)?;
            let mut out = method.as_str().to_string();
            out.push(' ');
            out.push_str(&parens_if_complex(&receiver_text));
            for a in args.iter() {
                let arg_text = render_expr_lean(a)?;
                out.push(' ');
                out.push_str(&parens_if_complex(&arg_text));
            }
            Some(format!("({})", out))
        }
        ExprKind::Field { expr: inner, field } => {
            let recv_text = render_expr_lean(inner)?;
            Some(format!(
                "({} {})",
                field.as_str(),
                parens_if_complex(&recv_text)
            ))
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

// =============================================================================
// TypeRenderer — verum_ast::Type → Coq/Lean type syntax (#141 / MSFS-L4.8)
// =============================================================================
//
// Sibling to [`ExprRenderer`] — translates types into the foreign
// tool's surface syntax so theorem parameters can be declared in the
// emitted file.  Without this, every theorem's free variables are
// undeclared and `coqc`/`lean` reject before they get to type-
// checking the proposition.

/// Output of translating a type.  Same discriminated-union shape as
/// [`TranslatedExpr`] — successful render or a fallback diagnostic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TranslatedType {
    /// Type translated cleanly — `text` is ready to be embedded as
    /// the parameter type in `(name : <text>)`.
    Translated {
        /// Foreign-tool type-syntax text (e.g., `Z`, `nat`, `bool`,
        /// `list nat`).
        text: String,
    },
    /// The type's shape couldn't be translated faithfully (refinement
    /// types with predicates, complex generics, function types with
    /// effects, etc.).  Caller decides how to embed (typical: declare
    /// the parameter as a fresh `Set` / `Type` variable).
    Fallback {
        /// Why the fallback fired.
        reason: String,
        /// Verum-side rendering preserved verbatim for the comment.
        original: String,
    },
}

impl TranslatedType {
    /// `true` iff translation succeeded.
    pub fn is_translated(&self) -> bool {
        matches!(self, TranslatedType::Translated { .. })
    }

    /// Get the translated text or `None`.
    pub fn text(&self) -> Option<&str> {
        match self {
            TranslatedType::Translated { text } => Some(text.as_str()),
            TranslatedType::Fallback { .. } => None,
        }
    }
}

/// Per-format type translator.  Adding a new foreign format is a
/// single new instance.
pub trait TypeRenderer {
    /// Stable id matching the corresponding [`ExprRenderer`].
    fn id(&self) -> &'static str;
    /// Translate a Verum type into the foreign tool's type syntax.
    fn render(&self, ty: &Type) -> TranslatedType;
}

// -----------------------------------------------------------------------------
// CoqTypeRenderer
// -----------------------------------------------------------------------------

/// Coq type backend.
pub struct CoqTypeRenderer;

impl CoqTypeRenderer {
    /// Construct a fresh renderer.
    pub fn new() -> Self {
        Self
    }
}

impl Default for CoqTypeRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeRenderer for CoqTypeRenderer {
    fn id(&self) -> &'static str {
        "coq"
    }

    fn render(&self, ty: &Type) -> TranslatedType {
        match render_type_coq(ty) {
            Some(text) => TranslatedType::Translated { text },
            None => TranslatedType::Fallback {
                reason: classify_unrenderable_type(ty).to_string(),
                original: format!("{:?}", ty.kind),
            },
        }
    }
}

fn render_type_coq(ty: &Type) -> Option<String> {
    match &ty.kind {
        TypeKind::Unit => Some("unit".to_string()),
        TypeKind::Bool => Some("bool".to_string()),
        TypeKind::Int => Some("Z".to_string()),
        TypeKind::Float => Some("R".to_string()),
        TypeKind::Char => Some("ascii".to_string()),
        TypeKind::Text => Some("string".to_string()),
        TypeKind::Path(path) => path.as_ident().map(|i| {
            // Common Verum stdlib types map to Coq's stdlib counterparts.
            match i.as_str() {
                "Int" => "Z".to_string(),
                "Bool" => "bool".to_string(),
                "Float" => "R".to_string(),
                "Text" => "string".to_string(),
                "Char" => "ascii".to_string(),
                "Unit" => "unit".to_string(),
                "Nat" => "nat".to_string(),
                other => other.to_string(),
            }
        }),
        TypeKind::Tuple(elems) => {
            let parts: Vec<String> = elems
                .iter()
                .map(render_type_coq)
                .collect::<Option<Vec<_>>>()?;
            if parts.is_empty() {
                Some("unit".to_string())
            } else if parts.len() == 1 {
                Some(parts.into_iter().next().unwrap())
            } else {
                // Coq tuple syntax: `T1 * T2 * ...`.
                Some(format!("({})", parts.join(" * ")))
            }
        }
        TypeKind::Slice(inner) | TypeKind::Array { element: inner, .. } => {
            // Coq stdlib lists: `list T`.
            let t = render_type_coq(inner)?;
            Some(format!("(list {})", t))
        }
        TypeKind::Generic { base, args, .. } => {
            // Verum `Foo<A, B>` → Coq `Foo A B` (curried application).
            // Common stdlib mappings: List<T> → list T, Maybe<T> → option T.
            let base_text = render_type_coq(base)?;
            let mapped_base = match base_text.as_str() {
                "List" => "list",
                "Maybe" | "Option" => "option",
                "Result" => "sum",
                other => other,
            }
            .to_string();
            let arg_parts: Vec<String> = args
                .iter()
                .filter_map(generic_arg_to_coq_text)
                .collect();
            if arg_parts.is_empty() {
                Some(mapped_base)
            } else {
                Some(format!("({} {})", mapped_base, arg_parts.join(" ")))
            }
        }
        TypeKind::Reference { inner, .. }
        | TypeKind::CheckedReference { inner, .. }
        | TypeKind::UnsafeReference { inner, .. } => {
            // References don't translate into a meaningful Coq type;
            // embed as the underlying carrier — sufficient for
            // statement-level type-checking.
            render_type_coq(inner)
        }
        _ => None,
    }
}

fn generic_arg_to_coq_text(arg: &verum_ast::ty::GenericArg) -> Option<String> {
    use verum_ast::ty::GenericArg;
    match arg {
        GenericArg::Type(t) => render_type_coq(t),
        // Const args / lifetime args don't translate cleanly — fall
        // back to a placeholder and let the caller decide.
        _ => Some("_".to_string()),
    }
}

// -----------------------------------------------------------------------------
// LeanTypeRenderer
// -----------------------------------------------------------------------------

/// Lean 4 type backend.
pub struct LeanTypeRenderer;

impl LeanTypeRenderer {
    /// Construct a fresh renderer.
    pub fn new() -> Self {
        Self
    }
}

impl Default for LeanTypeRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeRenderer for LeanTypeRenderer {
    fn id(&self) -> &'static str {
        "lean"
    }

    fn render(&self, ty: &Type) -> TranslatedType {
        match render_type_lean(ty) {
            Some(text) => TranslatedType::Translated { text },
            None => TranslatedType::Fallback {
                reason: classify_unrenderable_type(ty).to_string(),
                original: format!("{:?}", ty.kind),
            },
        }
    }
}

fn render_type_lean(ty: &Type) -> Option<String> {
    match &ty.kind {
        TypeKind::Unit => Some("Unit".to_string()),
        TypeKind::Bool => Some("Bool".to_string()),
        TypeKind::Int => Some("Int".to_string()),
        TypeKind::Float => Some("Float".to_string()),
        TypeKind::Char => Some("Char".to_string()),
        TypeKind::Text => Some("String".to_string()),
        TypeKind::Path(path) => path.as_ident().map(|i| {
            match i.as_str() {
                "Int" => "Int".to_string(),
                "Bool" => "Bool".to_string(),
                "Float" => "Float".to_string(),
                "Text" => "String".to_string(),
                "Char" => "Char".to_string(),
                "Unit" => "Unit".to_string(),
                "Nat" => "Nat".to_string(),
                other => other.to_string(),
            }
        }),
        TypeKind::Tuple(elems) => {
            let parts: Vec<String> = elems
                .iter()
                .map(render_type_lean)
                .collect::<Option<Vec<_>>>()?;
            if parts.is_empty() {
                Some("Unit".to_string())
            } else if parts.len() == 1 {
                Some(parts.into_iter().next().unwrap())
            } else {
                // Lean tuple syntax: `T1 × T2 × ...`.
                Some(format!("({})", parts.join(" × ")))
            }
        }
        TypeKind::Slice(inner) | TypeKind::Array { element: inner, .. } => {
            let t = render_type_lean(inner)?;
            Some(format!("(List {})", t))
        }
        TypeKind::Generic { base, args, .. } => {
            let base_text = render_type_lean(base)?;
            let mapped_base = match base_text.as_str() {
                "List" => "List",
                "Maybe" | "Option" => "Option",
                "Result" => "Sum",
                other => other,
            }
            .to_string();
            let arg_parts: Vec<String> = args
                .iter()
                .filter_map(generic_arg_to_lean_text)
                .collect();
            if arg_parts.is_empty() {
                Some(mapped_base)
            } else {
                Some(format!("({} {})", mapped_base, arg_parts.join(" ")))
            }
        }
        TypeKind::Reference { inner, .. }
        | TypeKind::CheckedReference { inner, .. }
        | TypeKind::UnsafeReference { inner, .. } => render_type_lean(inner),
        _ => None,
    }
}

fn generic_arg_to_lean_text(arg: &verum_ast::ty::GenericArg) -> Option<String> {
    use verum_ast::ty::GenericArg;
    match arg {
        GenericArg::Type(t) => render_type_lean(t),
        _ => Some("_".to_string()),
    }
}

/// Diagnostic classifier for unrenderable type shapes.
fn classify_unrenderable_type(ty: &Type) -> &'static str {
    match &ty.kind {
        TypeKind::Refined { .. } => "refinement type",
        TypeKind::Function { .. } => "function type",
        TypeKind::Rank2Function { .. } => "rank-2 function type",
        TypeKind::PathType { .. } => "path-equality type",
        TypeKind::DependentApp { .. } => "dependent-application type",
        TypeKind::Pointer { .. } => "raw pointer",
        TypeKind::VolatilePointer { .. } => "volatile pointer",
        TypeKind::Never => "never type",
        TypeKind::Unknown => "unknown type",
        _ => "unsupported type shape",
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

    // =========================================================================
    // MethodCall + Field translator tests (#142 / MSFS-L4.9)
    // =========================================================================

    fn method_call(receiver: Expr, method: &str, args: Vec<Expr>) -> Expr {
        let mut arg_list = verum_common::List::new();
        for a in args {
            arg_list.push(a);
        }
        Expr::new(
            ExprKind::MethodCall {
                receiver: Heap::new(receiver),
                method: Ident::new(method, Span::dummy()),
                type_args: verum_common::List::new(),
                args: arg_list,
            },
            Span::dummy(),
        )
    }

    fn field(receiver: Expr, name: &str) -> Expr {
        Expr::new(
            ExprKind::Field {
                expr: Heap::new(receiver),
                field: Ident::new(name, Span::dummy()),
            },
            Span::dummy(),
        )
    }

    #[test]
    fn coq_translates_zero_arg_method_call() {
        // obj.foo() → (foo obj)
        let e = method_call(ident_expr("obj"), "foo", vec![]);
        let text = CoqExprRenderer::new().render(&e).text().unwrap().to_string();
        assert_eq!(text, "(foo obj)");
    }

    #[test]
    fn lean_translates_zero_arg_method_call() {
        let e = method_call(ident_expr("obj"), "foo", vec![]);
        let text = LeanExprRenderer::new().render(&e).text().unwrap().to_string();
        assert_eq!(text, "(foo obj)");
    }

    #[test]
    fn coq_translates_method_call_with_args() {
        // obj.foo(a, b) → (foo obj a b)
        let e = method_call(
            ident_expr("obj"),
            "foo",
            vec![ident_expr("a"), ident_expr("b")],
        );
        let text = CoqExprRenderer::new().render(&e).text().unwrap().to_string();
        assert_eq!(text, "(foo obj a b)");
    }

    #[test]
    fn coq_translates_chained_method_calls() {
        // cand.articulation_view().cond_F_S().has_phi_X()
        //   → (has_phi_X (cond_F_S (articulation_view cand)))
        // This is the dominant MSFS proposition shape.
        let chain = method_call(
            method_call(
                method_call(ident_expr("cand"), "articulation_view", vec![]),
                "cond_F_S",
                vec![],
            ),
            "has_phi_X",
            vec![],
        );
        let text = CoqExprRenderer::new()
            .render(&chain)
            .text()
            .unwrap()
            .to_string();
        assert_eq!(
            text,
            "(has_phi_X (cond_F_S (articulation_view cand)))"
        );
    }

    #[test]
    fn lean_translates_chained_method_calls() {
        let chain = method_call(
            method_call(
                method_call(ident_expr("cand"), "articulation_view", vec![]),
                "cond_F_S",
                vec![],
            ),
            "has_phi_X",
            vec![],
        );
        let text = LeanExprRenderer::new()
            .render(&chain)
            .text()
            .unwrap()
            .to_string();
        // Both backends produce the same applicative form.
        assert_eq!(
            text,
            "(has_phi_X (cond_F_S (articulation_view cand)))"
        );
    }

    #[test]
    fn coq_translates_method_chain_in_logical_and() {
        // w.exhibitor_in_outer_stratum() && w.exhibitor_not_in_inner_stratum()
        // — pin for proposition_2_2_iii_cls_strict_above_clsmax.
        let e = binop(
            BinOp::And,
            method_call(ident_expr("w"), "exhibitor_in_outer_stratum", vec![]),
            method_call(ident_expr("w"), "exhibitor_not_in_inner_stratum", vec![]),
        );
        let text = CoqExprRenderer::new().render(&e).text().unwrap().to_string();
        assert!(text.contains("(exhibitor_in_outer_stratum w)"));
        assert!(text.contains("(exhibitor_not_in_inner_stratum w)"));
        assert!(text.contains("/\\"));
    }

    #[test]
    fn coq_translates_field_access() {
        // obj.field → (field obj)
        let e = field(ident_expr("obj"), "depth");
        let text = CoqExprRenderer::new().render(&e).text().unwrap().to_string();
        assert_eq!(text, "(depth obj)");
    }

    #[test]
    fn lean_translates_field_access() {
        let e = field(ident_expr("obj"), "depth");
        let text = LeanExprRenderer::new().render(&e).text().unwrap().to_string();
        assert_eq!(text, "(depth obj)");
    }

    #[test]
    fn method_call_falls_back_when_arg_unrenderable() {
        // obj.foo(<tuple>) → tuple is unrenderable, so the whole
        // method-call expression must surface as a fallback.  Pre-#142
        // every MethodCall surfaced as fallback regardless of args;
        // post-#142 only sub-expression failures bubble up.
        let unrenderable_arg = Expr::new(
            ExprKind::Tuple({
                let mut elems = verum_common::List::new();
                elems.push(ident_expr("x"));
                elems.push(ident_expr("y"));
                elems
            }),
            Span::dummy(),
        );
        let e = method_call(ident_expr("obj"), "foo", vec![unrenderable_arg]);
        let r = CoqExprRenderer::new().render(&e);
        match r {
            TranslatedExpr::Fallback { .. } => { /* expected */ }
            other => panic!("expected Fallback, got {:?}", other),
        }
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

    // =========================================================================
    // Type translator tests (#141 / MSFS-L4.8)
    // =========================================================================

    fn ty(kind: TypeKind) -> Type {
        Type::new(kind, Span::dummy())
    }

    fn path_ty(name: &str) -> Type {
        ty(TypeKind::Path(Path::single(Ident::new(name, Span::dummy()))))
    }

    #[test]
    fn coq_translates_primitive_types() {
        let r = CoqTypeRenderer::new();
        assert_eq!(r.render(&ty(TypeKind::Int)).text(), Some("Z"));
        assert_eq!(r.render(&ty(TypeKind::Bool)).text(), Some("bool"));
        assert_eq!(r.render(&ty(TypeKind::Text)).text(), Some("string"));
        assert_eq!(r.render(&ty(TypeKind::Float)).text(), Some("R"));
        assert_eq!(r.render(&ty(TypeKind::Unit)).text(), Some("unit"));
        assert_eq!(r.render(&ty(TypeKind::Char)).text(), Some("ascii"));
    }

    #[test]
    fn lean_translates_primitive_types() {
        let r = LeanTypeRenderer::new();
        assert_eq!(r.render(&ty(TypeKind::Int)).text(), Some("Int"));
        assert_eq!(r.render(&ty(TypeKind::Bool)).text(), Some("Bool"));
        assert_eq!(r.render(&ty(TypeKind::Text)).text(), Some("String"));
        assert_eq!(r.render(&ty(TypeKind::Float)).text(), Some("Float"));
        assert_eq!(r.render(&ty(TypeKind::Unit)).text(), Some("Unit"));
        assert_eq!(r.render(&ty(TypeKind::Char)).text(), Some("Char"));
    }

    #[test]
    fn coq_remaps_path_named_primitives() {
        // `Int` written as a Path identifier (not a Primitive
        // TypeKind::Int) — should still map to `Z`.
        let r = CoqTypeRenderer::new();
        assert_eq!(r.render(&path_ty("Int")).text(), Some("Z"));
        assert_eq!(r.render(&path_ty("Bool")).text(), Some("bool"));
        assert_eq!(r.render(&path_ty("Nat")).text(), Some("nat"));
    }

    #[test]
    fn lean_remaps_path_named_primitives() {
        let r = LeanTypeRenderer::new();
        assert_eq!(r.render(&path_ty("Int")).text(), Some("Int"));
        assert_eq!(r.render(&path_ty("Bool")).text(), Some("Bool"));
        assert_eq!(r.render(&path_ty("Nat")).text(), Some("Nat"));
    }

    #[test]
    fn coq_translates_user_named_path_verbatim() {
        // User-defined types pass through unchanged.
        let r = CoqTypeRenderer::new();
        assert_eq!(r.render(&path_ty("MyType")).text(), Some("MyType"));
    }

    #[test]
    fn coq_translates_tuple_to_product() {
        let r = CoqTypeRenderer::new();
        let t = ty(TypeKind::Tuple({
            let mut v = verum_common::List::new();
            v.push(ty(TypeKind::Int));
            v.push(ty(TypeKind::Bool));
            v
        }));
        assert_eq!(r.render(&t).text(), Some("(Z * bool)"));
    }

    #[test]
    fn lean_translates_tuple_to_product() {
        let r = LeanTypeRenderer::new();
        let t = ty(TypeKind::Tuple({
            let mut v = verum_common::List::new();
            v.push(ty(TypeKind::Int));
            v.push(ty(TypeKind::Bool));
            v
        }));
        assert_eq!(r.render(&t).text(), Some("(Int × Bool)"));
    }

    #[test]
    fn coq_translates_slice_to_list() {
        let r = CoqTypeRenderer::new();
        let t = ty(TypeKind::Slice(verum_common::Heap::new(ty(TypeKind::Int))));
        assert_eq!(r.render(&t).text(), Some("(list Z)"));
    }

    #[test]
    fn lean_translates_slice_to_list() {
        let r = LeanTypeRenderer::new();
        let t = ty(TypeKind::Slice(verum_common::Heap::new(ty(TypeKind::Int))));
        assert_eq!(r.render(&t).text(), Some("(List Int)"));
    }

    #[test]
    fn coq_translates_generic_list_maybe() {
        // Generic { base: List, args: [Int] } → `(list Z)`.
        let r = CoqTypeRenderer::new();
        let mut args: verum_common::List<verum_ast::ty::GenericArg> =
            verum_common::List::new();
        args.push(verum_ast::ty::GenericArg::Type(ty(TypeKind::Int)));
        let t = ty(TypeKind::Generic {
            base: verum_common::Heap::new(path_ty("List")),
            args,
        });
        assert_eq!(r.render(&t).text(), Some("(list Z)"));

        // Maybe<Bool> → option bool.
        let mut args2: verum_common::List<verum_ast::ty::GenericArg> =
            verum_common::List::new();
        args2.push(verum_ast::ty::GenericArg::Type(ty(TypeKind::Bool)));
        let t2 = ty(TypeKind::Generic {
            base: verum_common::Heap::new(path_ty("Maybe")),
            args: args2,
        });
        assert_eq!(r.render(&t2).text(), Some("(option bool)"));
    }

    #[test]
    fn lean_translates_generic_list_option() {
        let r = LeanTypeRenderer::new();
        let mut args: verum_common::List<verum_ast::ty::GenericArg> =
            verum_common::List::new();
        args.push(verum_ast::ty::GenericArg::Type(ty(TypeKind::Int)));
        let t = ty(TypeKind::Generic {
            base: verum_common::Heap::new(path_ty("List")),
            args,
        });
        assert_eq!(r.render(&t).text(), Some("(List Int)"));

        let mut args2: verum_common::List<verum_ast::ty::GenericArg> =
            verum_common::List::new();
        args2.push(verum_ast::ty::GenericArg::Type(ty(TypeKind::Bool)));
        let t2 = ty(TypeKind::Generic {
            base: verum_common::Heap::new(path_ty("Maybe")),
            args: args2,
        });
        assert_eq!(r.render(&t2).text(), Some("(Option Bool)"));
    }

    #[test]
    fn type_renderer_falls_back_for_unsupported_shape() {
        // Function type isn't translatable by the V0 renderer.
        let r = CoqTypeRenderer::new();
        let t = ty(TypeKind::Never);
        match r.render(&t) {
            TranslatedType::Fallback { reason, .. } => {
                assert_eq!(reason, "never type");
            }
            other => panic!("expected Fallback, got {:?}", other),
        }
    }

    #[test]
    fn type_renderer_id_is_stable() {
        assert_eq!(CoqTypeRenderer::new().id(), "coq");
        assert_eq!(LeanTypeRenderer::new().id(), "lean");
    }

    #[test]
    fn translated_type_text_accessor_works() {
        let translated = TranslatedType::Translated { text: "Z".into() };
        assert_eq!(translated.text(), Some("Z"));
        let fallback = TranslatedType::Fallback {
            reason: "never type".into(),
            original: "Never".into(),
        };
        assert_eq!(fallback.text(), None);
    }
}
