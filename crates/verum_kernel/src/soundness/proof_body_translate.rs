//! Proof-body translator — Verum proof bodies → Coq tactics / Lean tactics.
//!
//! Sibling to [`super::expr_translate`] (which translates
//! propositions); this module handles the load-bearing other half:
//! translating the actual proof body that closes a Verum theorem.
//!
//! # The shape problem
//!
//! Pre-this-module, every proof-bearing theorem emitted to Coq/Lean
//! ended in `Admitted.` / `:= by sorry`.  The proof body was
//! ignored.  This is task #153 / Phase 2 — replace the placeholder
//! with a real, foreign-tool-checkable proof.
//!
//! # Coverage strategy
//!
//! Translators land iteratively, smallest-shape-first.  V0 covers
//! the two highest-frequency shapes in the corpus:
//!
//!   * **Term-mode** ([`ProofBody::Term`]): the proof is an explicit
//!     expression (Curry-Howard).  Pass through the existing
//!     [`super::expr_translate::ExprRenderer`].  Coq form:
//!     `exact (<expr>).`  Lean form: `<expr>` (term-mode, no `by`).
//!
//!   * **Single-apply tactic-mode** ([`ProofBody::Tactic`] with
//!     [`TacticExpr::Apply`]): the proof is `apply <name>(args)`.
//!     This is the shape produced by the `@delegate(target)`
//!     attribute (#146) — every delegating MSFS theorem currently
//!     synthesises this body.  Coq form: `apply <name>.`  Lean
//!     form: `by apply <name>`.
//!
//! Other shapes ([`ProofBody::Structured`], [`ProofBody::ByMethod`],
//! complex tactic chains) fall back to [`TranslatedProofBody::Fallback`]
//! and the renderer reverts to `Admitted.` / `sorry` — partial
//! coverage is safe, no broken artefacts emitted.
//!
//! # Why this shape
//!
//! The MSFS corpus's @delegate-driven design (post-#146) makes
//! single-apply the dominant proof-body shape.  Closing this case
//! converts the largest cohort of `Admitted.` to `Qed.` in a
//! single pass, materially shrinking the trust extension visible
//! to `verum audit --proof-honesty`.

use serde::{Deserialize, Serialize};

use verum_ast::decl::{ProofBody, ProofBodyKind, ProofStepKind, TacticExpr};
use verum_ast::expr::{Expr, ExprKind};
use verum_ast::ty::PathSegment;
use verum_common::Maybe;

use super::expr_translate::{
    AgdaExprRenderer, CoqExprRenderer, ExprRenderer, LeanExprRenderer, TranslatedExpr,
};

// =============================================================================
// TranslatedProofBody
// =============================================================================

/// One translation outcome.  Mirrors
/// [`super::expr_translate::TranslatedExpr`] for proof bodies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TranslatedProofBody {
    /// Translation succeeded.  `text` is ready to substitute into
    /// the renderer's proof slot — for Coq, the body of
    /// `Proof. <text> Qed.`; for Lean, the right-hand side of
    /// `theorem foo : T := <text>` (which may include a leading
    /// `by ` if the translator chose tactic mode).
    Translated {
        /// Foreign-tool source text — e.g. `apply lemma_x.` (Coq)
        /// or `by apply lemma_x` (Lean).
        text: String,
    },
    /// The proof body's shape isn't covered by this translator.
    /// Caller falls back to `Admitted.` / `sorry`.
    Fallback {
        /// Why the fallback fired — used for diagnostics.
        reason: String,
    },
}

impl TranslatedProofBody {
    /// `true` iff the translation succeeded.
    pub fn is_translated(&self) -> bool {
        matches!(self, TranslatedProofBody::Translated { .. })
    }

    /// Get the translated text, or `None` for fallbacks.
    pub fn text(&self) -> Option<&str> {
        match self {
            TranslatedProofBody::Translated { text } => Some(text.as_str()),
            TranslatedProofBody::Fallback { .. } => None,
        }
    }
}

// =============================================================================
// ProofBodyRenderer trait
// =============================================================================

/// Per-format translator interface.  Adding a new foreign format is
/// one new instance.  Mirrors [`super::expr_translate::ExprRenderer`]
/// — same shape, different surface.
pub trait ProofBodyRenderer {
    /// Stable backend identifier — `"coq"` / `"lean"`.  Matches the
    /// keys used by [`super::corpus_export::TheoremSpec::per_backend_proof_tactic`].
    fn id(&self) -> &'static str;

    /// Translate a Verum proof body into the backend's proof-text
    /// syntax.  Returns
    /// [`TranslatedProofBody::Fallback`] for shapes outside the
    /// V0 coverage set.
    fn render(&self, body: &ProofBody) -> TranslatedProofBody;
}

// =============================================================================
// Helpers — shape recognition
// =============================================================================

/// If `expr` is `Path::Name(ident)` (single-segment path), return the
/// ident text.  Used to detect bare-name lemma references in
/// `apply <name>` tactics.
fn single_segment_path_name(expr: &Expr) -> Option<&str> {
    match &expr.kind {
        ExprKind::Path(path) if path.segments.len() == 1 => match &path.segments[0] {
            PathSegment::Name(ident) => Some(ident.name.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// If `expr` is the parser's representation of `<name>(args)` —
/// `Call { func: Path(<name>), args, type_args: [] }` — return the
/// callee name and the call args.  The fast parser produces this
/// shape inside `TacticExpr::Apply.lemma` for source like
/// `apply foo(x, y);` (the Apply variant's own `args` list stays
/// empty in this case).
fn call_with_single_segment_callee(expr: &Expr) -> Option<(&str, &[Expr])> {
    let (func, args, type_args) = match &expr.kind {
        ExprKind::Call {
            func,
            args,
            type_args,
        } => (func, args, type_args),
        _ => return None,
    };
    // V0: skip calls carrying explicit type arguments — Coq/Lean
    // unify implicits, so we can drop them safely, but recording
    // the choice as a future-V1 enhancement keeps the contract
    // explicit.  For now, fall back so the renderer reverts to
    // admitted; a future translator can render `apply (<name> @T)`.
    if !type_args.is_empty() {
        return None;
    }
    let name = single_segment_path_name(func.as_ref())?;
    Some((name, args.as_slice()))
}

/// If `tactic` is `Apply{lemma, args}` return the lemma name when the
/// lemma expression resolves to either:
///
///   * a bare single-segment path — e.g. `apply foo;`  (Apply.args
///     carries the actual argument list); or
///   * a single-segment-callee Call — e.g. `apply foo(x, y);`  (the
///     fast parser places the entire `f(args)` inside Apply.lemma
///     and leaves Apply.args empty).
///
/// Returned slice is the effective argument list — Apply.args for
/// the bare-path shape, the call's own args for the Call shape.
/// Helper so both `ProofBody::Tactic` and the structured-body
/// single-step case can share recognition.
fn classify_apply_tactic(tactic: &TacticExpr) -> Option<(&str, &[Expr])> {
    let (lemma, outer_args) = match tactic {
        TacticExpr::Apply { lemma, args } => (lemma, args),
        _ => return None,
    };
    if let Some(name) = single_segment_path_name(lemma.as_ref()) {
        return Some((name, outer_args.as_slice()));
    }
    if let Some((name, call_args)) = call_with_single_segment_callee(lemma.as_ref()) {
        return Some((name, call_args));
    }
    None
}

/// If `body` is one of the V0-recognised single-apply shapes, return
/// the lemma name + args:
///
///   * `ProofBody::Tactic(Apply{...})` — bare tactic-mode body.
///   * `ProofBody::Structured` with exactly one `Tactic(Apply{...})`
///     step and no `conclusion` — the shape produced by the parser
///     for `proof { apply <name>(args); }` blocks.  This is the
///     dominant @delegate-driven shape in the MSFS corpus (#146).
///   * `ProofBody::Structured` with empty `steps` and a
///     `conclusion: Some(Apply{...})` — alternative parser shape for
///     the same source pattern.
///
/// Args are returned alongside so future translators can render them
/// as positional arguments to `apply`.
fn classify_single_apply(body: &ProofBody) -> Option<(&str, &[Expr])> {
    match body {
        ProofBody::Tactic(t) => classify_apply_tactic(t),
        ProofBody::Structured(s) => {
            // `proof { apply foo(args); }` may parse either way:
            // the apply lands in the steps list (with conclusion=None)
            // or as the conclusion (with steps=[]).  Cover both.
            let steps_count = s.steps.iter().count();
            match (steps_count, &s.conclusion) {
                (1, Maybe::None) => {
                    let step = s.steps.iter().next()?;
                    if let ProofStepKind::Tactic(t) = &step.kind {
                        classify_apply_tactic(t)
                    } else {
                        None
                    }
                }
                (0, Maybe::Some(t)) => classify_apply_tactic(t),
                _ => None,
            }
        }
        _ => None,
    }
}

// =============================================================================
// CoqProofBodyRenderer
// =============================================================================

/// Coq backend.  Produces Coq tactic-mode proof-body text.
pub struct CoqProofBodyRenderer;

impl CoqProofBodyRenderer {
    /// Construct a fresh renderer.
    pub fn new() -> Self {
        Self
    }
}

impl Default for CoqProofBodyRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl ProofBodyRenderer for CoqProofBodyRenderer {
    fn id(&self) -> &'static str {
        "coq"
    }

    fn render(&self, body: &ProofBody) -> TranslatedProofBody {
        match body.kind() {
            ProofBodyKind::Term => render_term_coq(body),
            ProofBodyKind::Tactic | ProofBodyKind::Structured => {
                // Both shapes can carry a single-apply payload (the
                // structured shape is produced for `proof { apply ...; }`).
                // `render_single_apply_coq` handles both via
                // `classify_single_apply`.
                render_single_apply_coq(body)
            }
            other => TranslatedProofBody::Fallback {
                reason: format!(
                    "Coq translator: proof-body kind {:?} not yet covered (V0 covers Term + single-apply)",
                    other,
                ),
            },
        }
    }
}

/// Coq term-mode proof: `exact (<expr>).`  Reuses the existing
/// proposition translator since term-mode proofs are just
/// expressions.
fn render_term_coq(body: &ProofBody) -> TranslatedProofBody {
    let expr = match body {
        ProofBody::Term(e) => e.as_ref(),
        _ => unreachable!("called from kind() == Term arm"),
    };
    match CoqExprRenderer::new().render(expr) {
        TranslatedExpr::Translated { text } => TranslatedProofBody::Translated {
            text: format!("exact ({}).", text),
        },
        TranslatedExpr::Fallback { reason, .. } => TranslatedProofBody::Fallback {
            reason: format!("Coq term-mode: expr renderer fallback — {}", reason),
        },
    }
}

/// Coq single-apply proof: `apply <name>.`  Args are not rendered
/// yet (Coq apply unification usually figures out the implicit
/// arguments from the goal).  Covers both `ProofBody::Tactic(Apply)`
/// and the equivalent `ProofBody::Structured` shapes produced for
/// `proof { apply <name>(args); }` blocks.  Also covers the simple
/// primitive tactics (Auto / Trivial / Reflexivity / Assumption /
/// Ring / Field / Omega) — each translates to a single Coq tactic
/// of the same name.
fn render_single_apply_coq(body: &ProofBody) -> TranslatedProofBody {
    if let Some((name, _args)) = classify_single_apply(body) {
        return TranslatedProofBody::Translated {
            text: format!("apply {}.", name),
        };
    }
    if let Some(tactic) = primitive_tactic(body) {
        if let Some(text) = primitive_tactic_to_coq(tactic) {
            return TranslatedProofBody::Translated { text };
        }
    }
    TranslatedProofBody::Fallback {
        reason: "Coq: V0 covers single-apply + primitive tactics (auto/trivial/refl/assumption/ring/field/omega)".to_string(),
    }
}

// =============================================================================
// LeanProofBodyRenderer
// =============================================================================

/// Lean 4 backend.  Produces text that goes after `:=` in
/// `theorem foo : T := <text>` — may be either term-mode (no leading
/// `by `) or tactic-mode (`by ...`).
pub struct LeanProofBodyRenderer;

impl LeanProofBodyRenderer {
    /// Construct a fresh renderer.
    pub fn new() -> Self {
        Self
    }
}

impl Default for LeanProofBodyRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl ProofBodyRenderer for LeanProofBodyRenderer {
    fn id(&self) -> &'static str {
        "lean"
    }

    fn render(&self, body: &ProofBody) -> TranslatedProofBody {
        match body.kind() {
            ProofBodyKind::Term => render_term_lean(body),
            ProofBodyKind::Tactic | ProofBodyKind::Structured => {
                render_single_apply_lean(body)
            }
            other => TranslatedProofBody::Fallback {
                reason: format!(
                    "Lean translator: proof-body kind {:?} not yet covered (V0 covers Term + single-apply)",
                    other,
                ),
            },
        }
    }
}

/// Lean term-mode proof: `<expr>` — Lean accepts term-mode after `:=`
/// without a leading `by`.
fn render_term_lean(body: &ProofBody) -> TranslatedProofBody {
    let expr = match body {
        ProofBody::Term(e) => e.as_ref(),
        _ => unreachable!("called from kind() == Term arm"),
    };
    match LeanExprRenderer::new().render(expr) {
        TranslatedExpr::Translated { text } => TranslatedProofBody::Translated { text },
        TranslatedExpr::Fallback { reason, .. } => TranslatedProofBody::Fallback {
            reason: format!("Lean term-mode: expr renderer fallback — {}", reason),
        },
    }
}

/// Lean single-apply proof: `by apply <name>`.  Covers both bare
/// tactic-mode and structured-with-single-apply shapes.  Also covers
/// the simple primitive tactics — each gets a `by <lean_name>`
/// translation.
fn render_single_apply_lean(body: &ProofBody) -> TranslatedProofBody {
    if let Some((name, _args)) = classify_single_apply(body) {
        return TranslatedProofBody::Translated {
            text: format!("by apply {}", name),
        };
    }
    if let Some(tactic) = primitive_tactic(body) {
        if let Some(text) = primitive_tactic_to_lean(tactic) {
            return TranslatedProofBody::Translated { text };
        }
    }
    TranslatedProofBody::Fallback {
        reason: "Lean: V0 covers single-apply + primitive tactics (auto/trivial/refl/assumption/ring/field/omega)".to_string(),
    }
}

// =============================================================================
// AgdaProofBodyRenderer (#156 — third backend)
// =============================================================================

/// Agda 4 backend.  Agda has no tactic system in the vanilla
/// language (the experimental `Reflection` library is out of scope
/// for V0), so the translator's surface is necessarily smaller than
/// Coq/Lean: only term-mode proofs and `apply <name>(args)` shapes
/// translate cleanly.  Everything else falls back to `postulate` at
/// the corpus-emission layer.
///
/// **Coverage**:
///   * `ProofBody::Term(expr)` → `<expr>` (Agda accepts term-mode
///     proofs as the right-hand side of `name = body`).
///   * `ProofBody::Tactic(Apply{lemma})` / structured single-apply
///     → `<lemma>` (Agda treats the apply as a bare-term proof).
///   * Primitive tactics (`auto`, `omega`, `ring`, ...) → fall back.
///     Agda has no built-in equivalents, and the experimental
///     `simp`/`automation` ecosystem is library-dependent.
pub struct AgdaProofBodyRenderer;

impl AgdaProofBodyRenderer {
    /// Construct a fresh renderer.
    pub fn new() -> Self {
        Self
    }
}

impl Default for AgdaProofBodyRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl ProofBodyRenderer for AgdaProofBodyRenderer {
    fn id(&self) -> &'static str {
        "agda"
    }

    fn render(&self, body: &ProofBody) -> TranslatedProofBody {
        match body.kind() {
            ProofBodyKind::Term => render_term_agda(body),
            ProofBodyKind::Tactic | ProofBodyKind::Structured => {
                render_single_apply_agda(body)
            }
            other => TranslatedProofBody::Fallback {
                reason: format!(
                    "Agda translator: proof-body kind {:?} not yet covered (Agda has no tactic system; V0 covers Term + single-apply only)",
                    other,
                ),
            },
        }
    }
}

/// Agda term-mode proof: `<expr>` — same shape as Lean's term-mode.
fn render_term_agda(body: &ProofBody) -> TranslatedProofBody {
    let expr = match body {
        ProofBody::Term(e) => e.as_ref(),
        _ => unreachable!("called from kind() == Term arm"),
    };
    match AgdaExprRenderer::new().render(expr) {
        TranslatedExpr::Translated { text } => TranslatedProofBody::Translated { text },
        TranslatedExpr::Fallback { reason, .. } => TranslatedProofBody::Fallback {
            reason: format!("Agda term-mode: expr renderer fallback — {}", reason),
        },
    }
}

/// Agda single-apply proof: `<name>` — Agda has no `apply` keyword
/// and no tactic mode in vanilla form, so the translation is just
/// the bare lemma name as a term.  The args are dropped — Agda's
/// unification fills in implicit arguments when the proof is
/// type-checked against the goal.
fn render_single_apply_agda(body: &ProofBody) -> TranslatedProofBody {
    if let Some((name, _args)) = classify_single_apply(body) {
        return TranslatedProofBody::Translated {
            text: name.to_string(),
        };
    }
    TranslatedProofBody::Fallback {
        reason: "Agda: no tactic system; only Term and single-apply translate in V0".to_string(),
    }
}

// =============================================================================
// Primitive-tactic recognition + per-backend translation
// =============================================================================

/// Extract the inner tactic from a body that's either
/// `ProofBody::Tactic(t)` or `ProofBody::Structured` with exactly one
/// `Tactic(t)` step (and no conclusion) or `Structured` with empty
/// steps and `conclusion: Some(t)`.  Mirrors [`classify_single_apply`]
/// but returns the raw tactic instead of the apply payload — the
/// caller decides how to translate it.
fn primitive_tactic(body: &ProofBody) -> Option<&TacticExpr> {
    match body {
        ProofBody::Tactic(t) => Some(t),
        ProofBody::Structured(s) => {
            let steps_count = s.steps.iter().count();
            match (steps_count, &s.conclusion) {
                (1, Maybe::None) => {
                    let step = s.steps.iter().next()?;
                    if let ProofStepKind::Tactic(t) = &step.kind {
                        Some(t)
                    } else {
                        None
                    }
                }
                (0, Maybe::Some(t)) => Some(t),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Translate a primitive tactic to its Coq equivalent.  Returns `None`
/// for non-primitive tactics (the caller falls back to admitted).
///
/// **Why these tactics**: each is decidable enough that the foreign
/// tool can re-discharge it with a single tactic invocation.  The
/// translation is direct: the Verum tactic name matches the Coq
/// tactic name (Coq has all of these in its built-in tactic library).
fn primitive_tactic_to_coq(tactic: &TacticExpr) -> Option<String> {
    Some(match tactic {
        TacticExpr::Trivial => "trivial.".to_string(),
        TacticExpr::Assumption => "assumption.".to_string(),
        TacticExpr::Reflexivity => "reflexivity.".to_string(),
        TacticExpr::Auto { with_hints } if with_hints.iter().count() == 0 => {
            "auto.".to_string()
        }
        TacticExpr::Ring => "ring.".to_string(),
        TacticExpr::Field => "field.".to_string(),
        TacticExpr::Omega => "lia.".to_string(), // modern Coq uses lia for omega
        _ => return None,
    })
}

/// Translate a primitive tactic to its Lean 4 equivalent.  Returns
/// `None` for non-primitive tactics.  Lean tactic names diverge
/// slightly from Coq:
///
///   * `reflexivity` → `rfl`
///   * `omega` → `omega` (Mathlib provides this)
///   * `field` → `field_simp` (Mathlib)
fn primitive_tactic_to_lean(tactic: &TacticExpr) -> Option<String> {
    Some(match tactic {
        TacticExpr::Trivial => "by trivial".to_string(),
        TacticExpr::Assumption => "by assumption".to_string(),
        TacticExpr::Reflexivity => "by rfl".to_string(),
        TacticExpr::Auto { with_hints } if with_hints.iter().count() == 0 => {
            "by simp_all".to_string()
        }
        TacticExpr::Ring => "by ring".to_string(),
        TacticExpr::Field => "by field_simp".to_string(),
        TacticExpr::Omega => "by omega".to_string(),
        _ => return None,
    })
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::expr::{Expr, ExprKind};
    use verum_ast::ty::{Ident, Path, PathSegment};
    use verum_common::span::Span;
    use verum_common::{Heap, List, Maybe};

    fn span() -> Span {
        Span::dummy()
    }

    fn ident(name: &str) -> Ident {
        Ident {
            name: name.into(),
            span: span(),
        }
    }

    fn name_path_expr(name: &str) -> Expr {
        let path = Path::new(List::from(vec![PathSegment::Name(ident(name))]), span());
        Expr::new(ExprKind::Path(path), span())
    }

    fn apply_body(lemma_name: &str, args: Vec<Expr>) -> ProofBody {
        ProofBody::Tactic(TacticExpr::Apply {
            lemma: Heap::new(name_path_expr(lemma_name)),
            args: List::from(args),
        })
    }

    /// Build the parser's actual representation of `apply foo(x, y);`:
    /// the lemma is `Call { func: Path(foo), args: [x, y] }` and
    /// the outer Apply.args is empty.  This is what the fast parser
    /// produces — see `crates/verum_fast_parser/src/proof.rs:2733`.
    fn apply_body_via_call(lemma_name: &str, call_args: Vec<Expr>) -> ProofBody {
        let call = Expr::new(
            ExprKind::Call {
                func: Heap::new(name_path_expr(lemma_name)),
                type_args: List::new(),
                args: List::from(call_args),
            },
            span(),
        );
        ProofBody::Tactic(TacticExpr::Apply {
            lemma: Heap::new(call),
            args: List::new(),
        })
    }

    fn term_body(name: &str) -> ProofBody {
        ProofBody::Term(Heap::new(name_path_expr(name)))
    }

    // ----- single_segment_path_name + classify_single_apply -----

    #[test]
    fn single_segment_path_name_extracts_name() {
        let e = name_path_expr("foo");
        assert_eq!(single_segment_path_name(&e), Some("foo"));
    }

    #[test]
    fn classify_single_apply_finds_lemma() {
        let body = apply_body("backbone_full", vec![name_path_expr("n")]);
        let (name, args) = classify_single_apply(&body).expect("recognises Apply shape");
        assert_eq!(name, "backbone_full");
        assert_eq!(args.len(), 1);
    }

    #[test]
    fn classify_single_apply_rejects_non_apply_tactic() {
        let body = ProofBody::Tactic(TacticExpr::Trivial);
        assert!(classify_single_apply(&body).is_none());
    }

    #[test]
    fn classify_single_apply_rejects_term_body() {
        let body = term_body("anything");
        assert!(classify_single_apply(&body).is_none());
    }

    // ----- Coq renderer -----

    #[test]
    fn coq_renders_apply_to_apply_dot() {
        let body = apply_body("lemma_3_4", vec![]);
        let r = CoqProofBodyRenderer::new().render(&body);
        assert_eq!(
            r.text(),
            Some("apply lemma_3_4."),
            "Coq must emit `apply <name>.`",
        );
    }

    #[test]
    fn coq_renders_apply_with_args_drops_args_in_v0() {
        // V0 emits `apply <name>.` and lets Coq unification figure
        // out the args.  Future V1 may emit `apply (<name> arg1 arg2).`
        let body = apply_body("h", vec![name_path_expr("x"), name_path_expr("y")]);
        let r = CoqProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("apply h."));
    }

    #[test]
    fn coq_recognises_apply_via_call_shape() {
        // The parser's actual shape for `apply foo(x, y);` —
        // lemma is Call(path=foo, args=[x, y]).
        let body = apply_body_via_call(
            "syn_mod_lemma_3_4_steps_2_3",
            vec![name_path_expr("x"), name_path_expr("membership")],
        );
        let r = CoqProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("apply syn_mod_lemma_3_4_steps_2_3."));
    }

    #[test]
    fn lean_recognises_apply_via_call_shape() {
        let body = apply_body_via_call(
            "syn_mod_lemma_3_4_steps_2_3",
            vec![name_path_expr("x"), name_path_expr("membership")],
        );
        let r = LeanProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("by apply syn_mod_lemma_3_4_steps_2_3"));
    }

    #[test]
    fn structured_apply_via_call_shape_translates() {
        // The full @delegate-driven shape: structured body with a
        // single Tactic(Apply) step where the lemma is a Call.
        use verum_ast::decl::{ProofStep, ProofStructure};
        let call = Expr::new(
            ExprKind::Call {
                func: Heap::new(name_path_expr("backbone_full")),
                type_args: List::new(),
                args: List::from(vec![name_path_expr("n")]),
            },
            span(),
        );
        let step = ProofStep {
            kind: ProofStepKind::Tactic(TacticExpr::Apply {
                lemma: Heap::new(call),
                args: List::new(),
            }),
            span: span(),
        };
        let body = ProofBody::Structured(ProofStructure {
            steps: List::from(vec![step]),
            conclusion: Maybe::None,
            span: span(),
        });
        let r = CoqProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("apply backbone_full."));
        let r = LeanProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("by apply backbone_full"));
    }

    #[test]
    fn coq_falls_back_on_unsupported_tactic() {
        // Use a tactic outside the V0 primitive set — `Split` (∧-intro)
        // isn't translated yet, so the renderer must fall back.
        let body = ProofBody::Tactic(TacticExpr::Split);
        let r = CoqProofBodyRenderer::new().render(&body);
        assert!(matches!(r, TranslatedProofBody::Fallback { .. }));
    }

    #[test]
    fn coq_renders_term_body_as_exact() {
        let body = term_body("h_x");
        let r = CoqProofBodyRenderer::new().render(&body);
        assert_eq!(
            r.text(),
            Some("exact (h_x)."),
            "Coq term-mode must wrap as `exact (<expr>).`",
        );
    }

    // ----- Lean renderer -----

    #[test]
    fn lean_renders_apply_as_by_apply() {
        let body = apply_body("lemma_3_4", vec![]);
        let r = LeanProofBodyRenderer::new().render(&body);
        assert_eq!(
            r.text(),
            Some("by apply lemma_3_4"),
            "Lean must emit `by apply <name>`",
        );
    }

    #[test]
    fn lean_renders_apply_with_args_drops_args_in_v0() {
        let body = apply_body("h", vec![name_path_expr("x")]);
        let r = LeanProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("by apply h"));
    }

    #[test]
    fn lean_falls_back_on_unsupported_tactic() {
        let body = ProofBody::Tactic(TacticExpr::Split);
        let r = LeanProofBodyRenderer::new().render(&body);
        assert!(matches!(r, TranslatedProofBody::Fallback { .. }));
    }

    #[test]
    fn lean_renders_term_body_passthrough() {
        let body = term_body("h_x");
        let r = LeanProofBodyRenderer::new().render(&body);
        // Lean term-mode after `:=` is bare expression — no `by`.
        assert_eq!(r.text(), Some("h_x"));
    }

    // ----- structured/by_method falls back uniformly -----

    #[test]
    fn renderers_fall_back_on_empty_structured_body() {
        // Empty structured body (no steps, no conclusion) — nothing
        // to translate.
        use verum_ast::decl::ProofStructure;
        let body = ProofBody::Structured(ProofStructure {
            steps: List::from(Vec::new()),
            conclusion: Maybe::None,
            span: span(),
        });
        assert!(matches!(
            CoqProofBodyRenderer::new().render(&body),
            TranslatedProofBody::Fallback { .. },
        ));
        assert!(matches!(
            LeanProofBodyRenderer::new().render(&body),
            TranslatedProofBody::Fallback { .. },
        ));
    }

    /// Helper: build the canonical `proof { apply <name>(args); }`
    /// shape — `ProofBody::Structured` with one `ProofStep::Tactic(Apply)`
    /// step and no conclusion.  This is the shape produced by the
    /// MSFS @delegate(target) attribute (#146).
    fn structured_apply_step(name: &str, args: Vec<Expr>) -> ProofBody {
        use verum_ast::decl::{ProofStep, ProofStructure};
        let tactic = TacticExpr::Apply {
            lemma: Heap::new(name_path_expr(name)),
            args: List::from(args),
        };
        let step = ProofStep {
            kind: ProofStepKind::Tactic(tactic),
            span: span(),
        };
        ProofBody::Structured(ProofStructure {
            steps: List::from(vec![step]),
            conclusion: Maybe::None,
            span: span(),
        })
    }

    /// Alternative parser shape: empty steps + Some(Apply) conclusion.
    fn structured_apply_conclusion(name: &str, args: Vec<Expr>) -> ProofBody {
        use verum_ast::decl::ProofStructure;
        let tactic = TacticExpr::Apply {
            lemma: Heap::new(name_path_expr(name)),
            args: List::from(args),
        };
        ProofBody::Structured(ProofStructure {
            steps: List::from(Vec::new()),
            conclusion: Maybe::Some(tactic),
            span: span(),
        })
    }

    #[test]
    fn coq_translates_structured_apply_step() {
        let body = structured_apply_step("backbone_full", vec![name_path_expr("n")]);
        let r = CoqProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("apply backbone_full."));
    }

    #[test]
    fn lean_translates_structured_apply_step() {
        let body = structured_apply_step("backbone_full", vec![name_path_expr("n")]);
        let r = LeanProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("by apply backbone_full"));
    }

    #[test]
    fn coq_translates_structured_apply_conclusion() {
        let body = structured_apply_conclusion("lemma_x", vec![]);
        let r = CoqProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("apply lemma_x."));
    }

    #[test]
    fn lean_translates_structured_apply_conclusion() {
        let body = structured_apply_conclusion("lemma_x", vec![]);
        let r = LeanProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("by apply lemma_x"));
    }

    #[test]
    fn structured_with_two_steps_falls_back() {
        // Multiple steps don't reduce to a single apply.
        use verum_ast::decl::{ProofStep, ProofStructure};
        let make_step = |name: &str| ProofStep {
            kind: ProofStepKind::Tactic(TacticExpr::Apply {
                lemma: Heap::new(name_path_expr(name)),
                args: List::from(Vec::new()),
            }),
            span: span(),
        };
        let body = ProofBody::Structured(ProofStructure {
            steps: List::from(vec![make_step("a"), make_step("b")]),
            conclusion: Maybe::None,
            span: span(),
        });
        let r = CoqProofBodyRenderer::new().render(&body);
        assert!(matches!(r, TranslatedProofBody::Fallback { .. }));
    }

    #[test]
    fn structured_with_step_and_conclusion_falls_back() {
        // A non-trivial structure — both steps and a conclusion —
        // is multi-tactic and outside V0 coverage.
        use verum_ast::decl::{ProofStep, ProofStructure};
        let body = ProofBody::Structured(ProofStructure {
            steps: List::from(vec![ProofStep {
                kind: ProofStepKind::Tactic(TacticExpr::Apply {
                    lemma: Heap::new(name_path_expr("a")),
                    args: List::from(Vec::new()),
                }),
                span: span(),
            }]),
            conclusion: Maybe::Some(TacticExpr::Apply {
                lemma: Heap::new(name_path_expr("b")),
                args: List::from(Vec::new()),
            }),
            span: span(),
        });
        let r = CoqProofBodyRenderer::new().render(&body);
        assert!(matches!(r, TranslatedProofBody::Fallback { .. }));
    }

    // ----- TranslatedProofBody surface -----

    #[test]
    fn translated_proof_body_text_returns_some_only_when_translated() {
        let t = TranslatedProofBody::Translated {
            text: "ok".to_string(),
        };
        let f = TranslatedProofBody::Fallback {
            reason: "no".to_string(),
        };
        assert_eq!(t.text(), Some("ok"));
        assert_eq!(f.text(), None);
        assert!(t.is_translated());
        assert!(!f.is_translated());
    }

    // -------------------------------------------------------------
    // Primitive tactic translation (Auto, Trivial, Refl, Ring, ...)
    // -------------------------------------------------------------

    fn primitive_body(t: TacticExpr) -> ProofBody {
        ProofBody::Tactic(t)
    }

    #[test]
    fn coq_renders_primitive_tactics() {
        assert_eq!(
            CoqProofBodyRenderer::new()
                .render(&primitive_body(TacticExpr::Trivial))
                .text(),
            Some("trivial."),
        );
        assert_eq!(
            CoqProofBodyRenderer::new()
                .render(&primitive_body(TacticExpr::Assumption))
                .text(),
            Some("assumption."),
        );
        assert_eq!(
            CoqProofBodyRenderer::new()
                .render(&primitive_body(TacticExpr::Reflexivity))
                .text(),
            Some("reflexivity."),
        );
        assert_eq!(
            CoqProofBodyRenderer::new()
                .render(&primitive_body(TacticExpr::Auto {
                    with_hints: List::new(),
                }))
                .text(),
            Some("auto."),
        );
        assert_eq!(
            CoqProofBodyRenderer::new()
                .render(&primitive_body(TacticExpr::Ring))
                .text(),
            Some("ring."),
        );
        assert_eq!(
            CoqProofBodyRenderer::new()
                .render(&primitive_body(TacticExpr::Field))
                .text(),
            Some("field."),
        );
        assert_eq!(
            CoqProofBodyRenderer::new()
                .render(&primitive_body(TacticExpr::Omega))
                .text(),
            Some("lia."),
            "modern Coq uses `lia` for linear-integer-arithmetic decisions",
        );
    }

    #[test]
    fn lean_renders_primitive_tactics() {
        assert_eq!(
            LeanProofBodyRenderer::new()
                .render(&primitive_body(TacticExpr::Trivial))
                .text(),
            Some("by trivial"),
        );
        assert_eq!(
            LeanProofBodyRenderer::new()
                .render(&primitive_body(TacticExpr::Reflexivity))
                .text(),
            Some("by rfl"),
            "Lean uses `rfl` for reflexivity, not `reflexivity`",
        );
        assert_eq!(
            LeanProofBodyRenderer::new()
                .render(&primitive_body(TacticExpr::Auto {
                    with_hints: List::new(),
                }))
                .text(),
            Some("by simp_all"),
        );
        assert_eq!(
            LeanProofBodyRenderer::new()
                .render(&primitive_body(TacticExpr::Field))
                .text(),
            Some("by field_simp"),
            "Lean Mathlib uses `field_simp`, not `field`",
        );
        assert_eq!(
            LeanProofBodyRenderer::new()
                .render(&primitive_body(TacticExpr::Omega))
                .text(),
            Some("by omega"),
        );
    }

    #[test]
    fn auto_with_hints_falls_back_in_v0() {
        // Hint sets aren't translated yet — fall back to admitted.
        let body = primitive_body(TacticExpr::Auto {
            with_hints: List::from(vec![ident("hint_lemma")]),
        });
        assert!(matches!(
            CoqProofBodyRenderer::new().render(&body),
            TranslatedProofBody::Fallback { .. },
        ));
        assert!(matches!(
            LeanProofBodyRenderer::new().render(&body),
            TranslatedProofBody::Fallback { .. },
        ));
    }

    #[test]
    fn primitive_tactic_via_structured_body_translates() {
        // `proof { trivial; }` parses as Structured with one Tactic(Trivial)
        // step.  Should translate just like the bare Tactic form.
        use verum_ast::decl::{ProofStep, ProofStructure};
        let step = ProofStep {
            kind: ProofStepKind::Tactic(TacticExpr::Trivial),
            span: span(),
        };
        let body = ProofBody::Structured(ProofStructure {
            steps: List::from(vec![step]),
            conclusion: Maybe::None,
            span: span(),
        });
        assert_eq!(
            CoqProofBodyRenderer::new().render(&body).text(),
            Some("trivial."),
        );
        assert_eq!(
            LeanProofBodyRenderer::new().render(&body).text(),
            Some("by trivial"),
        );
    }

    // -------------------------------------------------------------
    // Agda translator (#156 — third backend)
    // -------------------------------------------------------------

    #[test]
    fn agda_renders_apply_as_bare_term() {
        // Agda has no `apply` keyword — the translation is just the
        // lemma name as a term (Agda unification fills implicits).
        let body = apply_body("backbone_full", vec![name_path_expr("n")]);
        let r = AgdaProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("backbone_full"));
    }

    #[test]
    fn agda_renders_call_shape_apply() {
        let body = apply_body_via_call(
            "syn_mod_lemma_3_4_steps_2_3",
            vec![name_path_expr("x"), name_path_expr("membership")],
        );
        let r = AgdaProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("syn_mod_lemma_3_4_steps_2_3"));
    }

    #[test]
    fn agda_renders_term_body_passthrough() {
        let body = term_body("h_x");
        let r = AgdaProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("h_x"));
    }

    #[test]
    fn agda_falls_back_on_primitive_tactic() {
        // Agda has no built-in `auto` / `omega` / `ring`.  The
        // primitive tactics fall back; the corpus-emission layer
        // then uses `postulate` for these theorems.
        for tactic in [
            TacticExpr::Trivial,
            TacticExpr::Reflexivity,
            TacticExpr::Auto {
                with_hints: List::new(),
            },
            TacticExpr::Omega,
        ] {
            let body = ProofBody::Tactic(tactic);
            let r = AgdaProofBodyRenderer::new().render(&body);
            assert!(
                matches!(r, TranslatedProofBody::Fallback { .. }),
                "Agda must fall back on primitive tactics — no built-in equivalents",
            );
        }
    }

    #[test]
    fn agda_translates_structured_apply_step() {
        let body = structured_apply_step("backbone_full", vec![]);
        let r = AgdaProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("backbone_full"));
    }

    #[test]
    fn translated_proof_body_serde_round_trip() {
        let t = TranslatedProofBody::Translated {
            text: "by apply x".to_string(),
        };
        let json = serde_json::to_string(&t).unwrap();
        let restored: TranslatedProofBody = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, t);
    }
}
