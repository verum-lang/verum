//! Proof-body translator — Verum proof bodies → Coq tactics / Lean tactics.
//!

//! Sibling to [`super::expr_translate`] (which translates
//! propositions); this module handles the load-bearing other half:
//! translating the actual proof body that closes a Verum theorem.
//!

//! # The shape problem
//!

//! Pre-this-module, every proof-bearing theorem emitted to Coq/Lean
//! ended in `Admitted.` / `:= by sorry`. The proof body was
//! ignored. This is task #153 / Phase 2 — replace the placeholder
//! with a real, foreign-tool-checkable proof.
//!

//! # Coverage strategy
//!

//! Translators land iteratively, smallest-shape-first. covers
//! the two highest-frequency shapes in the corpus:
//!

//!  * **Term-mode** ([`ProofBody::Term`]): the proof is an explicit
//!  expression (Curry-Howard). Pass through the existing
//!  [`super::expr_translate::ExprRenderer`]. Coq form:
//!  `exact (<expr>).` Lean form: `<expr>` (term-mode, no `by`).
//!

//!  * **Single-apply tactic-mode** ([`ProofBody::Tactic`] with
//!  [`TacticExpr::Apply`]): the proof is `apply <name>(args)`.
//!  This is the shape produced by the `@delegate(target)`
//!  attribute (#146) — every delegating MSFS theorem currently
//!  synthesises this body. Coq form: `apply <name>.` Lean
//!  form: `by apply <name>`.
//!

//! Other shapes ([`ProofBody::Structured`], [`ProofBody::ByMethod`],
//! complex tactic chains) fall back to [`TranslatedProofBody::Fallback`]
//! and the renderer reverts to `Admitted.` / `sorry` — partial
//! coverage is safe, no broken artefacts emitted.
//!

//! # Why this shape
//!

//! The MSFS corpus's @delegate-driven design (post-#146) makes
//! single-apply the dominant proof-body shape. Closing this case
//! converts the largest cohort of `Admitted.` to `Qed.` in a
//! single pass, materially shrinking the trust extension visible
//! to `verum audit --proof-honesty`.

use serde::{Deserialize, Serialize};

use verum_ast::decl::{ProofBody, ProofBodyKind, ProofMethod, ProofStepKind, TacticExpr};
use verum_ast::expr::{Expr, ExprKind};
use verum_ast::ty::PathSegment;
use verum_common::Maybe;

use super::expr_translate::{
    AgdaExprRenderer, CoqExprRenderer, DeduktiExprRenderer, ExprRenderer, IsabelleExprRenderer,
    LeanExprRenderer, TranslatedExpr,
};

// =============================================================================
// TranslatedProofBody
// =============================================================================

/// One translation outcome. Mirrors
/// [`super::expr_translate::TranslatedExpr`] for proof bodies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TranslatedProofBody {
    /// Translation succeeded. `text` is ready to substitute into
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

/// Per-format translator interface. Adding a new foreign format is
/// one new instance. Mirrors [`super::expr_translate::ExprRenderer`]
/// — same shape, different surface.
pub trait ProofBodyRenderer {
    /// Stable backend identifier — `"coq"` / `"lean"`. Matches the
    /// keys used by [`super::corpus_export::TheoremSpec::per_backend_proof_tactic`].
    fn id(&self) -> &'static str;

    /// Translate a Verum proof body into the backend's proof-text
    /// syntax. Returns
    /// [`TranslatedProofBody::Fallback`] for shapes outside the
    /// V0 coverage set.
    fn render(&self, body: &ProofBody) -> TranslatedProofBody;
}

// =============================================================================
// Helpers — shape recognition
// =============================================================================

/// If `expr` is `Path::Name(ident)` (single-segment path), return the
/// ident text. Used to detect bare-name lemma references in
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

/// If `expr` is a multi-segment `Path` (e.g., `a::b::c`) OR a Field
/// chain rooted at a single-segment Path (e.g., `mathlib4.lambda.ChurchRosser`,
/// which the parser produces as nested `Field` expressions),
/// return the dot-joined form. Used to recognise mathlib4-cited
/// / framework-cited apply targets that the corpus uses extensively.
///

/// Returns `None` for non-resolvable shapes (calls, closures,
/// generic-arg segments, etc.) — those need their own classification.
fn dotted_path_text(expr: &Expr) -> Option<String> {
    // Path form (Verum's `::` separator + namespace paths).
    if let ExprKind::Path(path) = &expr.kind {
        let mut parts: Vec<&str> = Vec::with_capacity(path.segments.len());
        for seg in path.segments.iter() {
            match seg {
                PathSegment::Name(ident) => parts.push(ident.name.as_str()),
                // Generic-argument segments (e.g., `foo<T>`) — fall
                // back; foreign-tool unification fills in implicits.
                _ => return None,
            }
        }
        if parts.is_empty() {
            return None;
        }
        return Some(parts.join("."));
    }
    // Field-chain form (Verum's `.` member access — the actual
    // shape produced by the parser for `a.b.c` source). Walk the
    // Field nesting, collect field names, ensure the innermost
    // receiver is a single-segment Path.
    field_chain_text(expr)
}

/// Walk a `Field { expr, field }` chain and produce the
/// dot-joined dotted-path text when the innermost receiver is a
/// single-segment `Path`. Returns `None` otherwise.
///

/// Example: `mathlib4.lambda.ChurchRosser` parses as
/// `Field { expr: Field { expr: Path(mathlib4), field: lambda }, field: ChurchRosser }`
/// and resolves to `"mathlib4.lambda.ChurchRosser"`.
fn field_chain_text(expr: &Expr) -> Option<String> {
    let mut tail: Vec<String> = Vec::new();
    let mut cursor = expr;
    loop {
        match &cursor.kind {
            ExprKind::Field { expr: inner, field } => {
                tail.push(field.as_str().to_string());
                cursor = inner.as_ref();
            }
            ExprKind::Path(path) if path.segments.len() == 1 => {
                if let PathSegment::Name(ident) = &path.segments[0] {
                    let head = ident.name.as_str();
                    if tail.is_empty() {
                        return Some(head.to_string());
                    }
                    let mut parts: Vec<String> = vec![head.to_string()];
                    parts.extend(tail.into_iter().rev());
                    return Some(parts.join("."));
                }
                return None;
            }
            _ => return None,
        }
    }
}

/// If `expr` is the parser's representation of `<name>(args)` —
/// `Call { func: Path(<name>), args, type_args: [] }` — return the
/// callee name and the call args. The fast parser produces this
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
    // explicit. For now, fall back so the renderer reverts to
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

///  * a bare single-segment path — e.g. `apply foo;` (Apply.args
///  carries the actual argument list); or
///  * a single-segment-callee Call — e.g. `apply foo(x, y);` (the
///  fast parser places the entire `f(args)` inside Apply.lemma
///  and leaves Apply.args empty).
///

/// Returned slice is the effective argument list — Apply.args for
/// the bare-path shape, the call's own args for the Call shape.
/// Helper so both `ProofBody::Tactic` and the structured-body
/// single-step case can share recognition.
fn classify_apply_tactic(tactic: &TacticExpr) -> Option<(String, &[Expr])> {
    let (lemma, outer_args) = match tactic {
        TacticExpr::Apply { lemma, args } => (lemma, args),
        _ => return None,
    };
    // Single-segment bare path: `apply foo;`
    if let Some(name) = single_segment_path_name(lemma.as_ref()) {
        return Some((name.to_string(), outer_args.as_slice()));
    }
    // Call shape with single-segment callee: `apply foo(x, y);`
    if let Some((name, call_args)) = call_with_single_segment_callee(lemma.as_ref()) {
        return Some((name.to_string(), call_args));
    }
    // Multi-segment dotted path: `apply mathlib4.lambda.ChurchRosser;`
    // — the dominant pattern for framework-cited theorems.
    if let Some(dotted) = dotted_path_text(lemma.as_ref()) {
        return Some((dotted, outer_args.as_slice()));
    }
    // Call shape over a multi-segment dotted callee:
    // `apply mathlib4.foo.bar(x, y);` — render the path dotted
    // and use the call args.
    if let ExprKind::Call {
        func,
        args,
        type_args,
    } = &lemma.as_ref().kind
    {
        if type_args.is_empty() {
            if let Some(dotted) = dotted_path_text(func.as_ref()) {
                return Some((dotted, args.as_slice()));
            }
        }
    }
    None
}

/// If `body` is one of the V0-recognised single-apply shapes, return
/// the lemma name + args:
///

///  * `ProofBody::Tactic(Apply{...})` — bare tactic-mode body.
///  * `ProofBody::Structured` with exactly one `Tactic(Apply{...})`
///  step and no `conclusion` — the shape produced by the parser
///  for `proof { apply <name>(args); }` blocks. This is the
///  dominant @delegate-driven shape in the MSFS corpus (#146).
///  * `ProofBody::Structured` with empty `steps` and a
///  `conclusion: Some(Apply{...})` — alternative parser shape for
///  the same source pattern.
///

/// Args are returned alongside so future translators can render them
/// as positional arguments to `apply`.
fn classify_single_apply(body: &ProofBody) -> Option<(String, &[Expr])> {
    match body {
        ProofBody::Tactic(t) => classify_apply_tactic(t),
        ProofBody::Structured(s) => {
            // `proof { apply foo(args); }` may parse either way:
            // the apply lands in the steps list (with conclusion=None)
            // or as the conclusion (with steps=[]). Cover both.
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

/// One recognised step in a multi-step tactic chain. Either an
/// `apply <name>` (with optional args, currently dropped per V0
/// convention) or a primitive tactic that the per-backend renderer
/// already knows how to translate. Used by
/// [`classify_tactic_chain`].
#[derive(Debug, Clone)]
enum ChainStep<'a> {
    /// `apply <name>` step. Args dropped at render-time — the
    /// foreign tool's unifier supplies implicits.
    Apply { name: String },
    /// A primitive tactic — `Trivial`, `Reflexivity`, `Auto{...}`,
    /// `Ring`, `Field`, `Omega`, `Assumption` — that has a direct
    /// per-backend translation.
    Primitive(&'a TacticExpr),
}

/// If `body` is `ProofBody::Structured` with **all** steps + the
/// optional conclusion classifying as either `apply <name>` or one
/// of the recognised primitive tactics, return the ordered chain.
/// Otherwise return `None`.
///
/// Single-step shapes are NOT returned here — they're already
/// handled by [`classify_single_apply`] / [`primitive_tactic`] and
/// the existing per-backend single-step renderers. This classifier
/// targets the **N≥2** shapes (multi-`apply` proofs, mixed
/// `apply` + primitive sequences, steps + conclusion) that
/// previously fell through to `Admitted.`/`sorry`.
///
/// **Why this matters for #153**: corpus theorems with two or
/// more sequential `apply` steps (composition lemmas, transport
/// chains) used to emit unconditional admits — the foreign tool
/// only verified the proposition's TYPE, not the proof structure.
/// With chain support, the foreign tool re-runs every step.
fn classify_tactic_chain(body: &ProofBody) -> Option<Vec<ChainStep<'_>>> {
    let s = match body {
        ProofBody::Structured(s) => s,
        _ => return None,
    };
    let step_count = s.steps.iter().count();
    let has_conclusion = matches!(s.conclusion, Maybe::Some(_));
    let total = step_count + if has_conclusion { 1 } else { 0 };
    // Reject single-step / empty shapes — those routes already
    // exist (classify_single_apply + primitive_tactic).
    if total < 2 {
        return None;
    }
    let mut chain: Vec<ChainStep<'_>> = Vec::with_capacity(total);
    for step in s.steps.iter() {
        let tactic = match &step.kind {
            ProofStepKind::Tactic(t) => t,
            // Have/Show/Suffices/Let/Obtain/Calc/Cases/Focus carry
            // sub-bodies that V0 doesn't yet render — fall back so
            // the renderer reverts to admitted/sorry.
            _ => return None,
        };
        chain.push(classify_chain_step(tactic)?);
    }
    if let Maybe::Some(t) = &s.conclusion {
        chain.push(classify_chain_step(t)?);
    }
    Some(chain)
}

/// Recognise one tactic as either an `apply` step or a primitive.
/// Returns `None` for shapes outside the chain-renderable set —
/// `Split`, `Intro`, hint-bearing `Auto`, etc. fall back here so
/// the whole chain bails out rather than emitting a half-translated
/// proof.
fn classify_chain_step(tactic: &TacticExpr) -> Option<ChainStep<'_>> {
    if let Some((name, _args)) = classify_apply_tactic(tactic) {
        return Some(ChainStep::Apply { name });
    }
    // Recognise primitives whose per-backend tactic forms exist —
    // probing each translator with this tactic and accepting the
    // first hit keeps coverage in lockstep with primitive_tactic_to_*.
    let coq_ok = primitive_tactic_to_coq(tactic).is_some();
    let lean_ok = primitive_tactic_to_lean(tactic).is_some();
    let isa_ok = primitive_tactic_to_isabelle(tactic).is_some();
    if coq_ok || lean_ok || isa_ok {
        return Some(ChainStep::Primitive(tactic));
    }
    None
}

// =============================================================================
// CoqProofBodyRenderer
// =============================================================================

/// Coq backend. Produces Coq tactic-mode proof-body text.
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
            ProofBodyKind::ByMethod => render_by_method_coq(body),
        }
    }
}

/// Coq term-mode proof: `exact (<expr>).` Reuses the existing
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

/// Coq single-apply proof: `apply <name>.` Args are not rendered
/// yet (Coq apply unification usually figures out the implicit
/// arguments from the goal). Covers both `ProofBody::Tactic(Apply)`
/// and the equivalent `ProofBody::Structured` shapes produced for
/// `proof { apply <name>(args); }` blocks. Also covers the simple
/// primitive tactics (Auto / Trivial / Reflexivity / Assumption /
/// Ring / Field / Omega) — each translates to a single Coq tactic
/// of the same name. Multi-step shapes route through
/// [`render_tactic_chain_coq`].
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
    if let Some(chain) = classify_tactic_chain(body) {
        return render_tactic_chain_coq(&chain);
    }
    TranslatedProofBody::Fallback {
        reason: "Coq: V0 covers single-apply + primitive tactics + multi-step tactic chains".to_string(),
    }
}

/// Coq multi-step tactic chain: each step on its own line, ending
/// in `.` — Coq's standard tactic-mode separator. Example:
///
/// ```text
/// apply intro_lemma.
/// apply transport.
/// reflexivity.
/// ```
///
/// Coq's `Proof. <body> Qed.` framing wraps this verbatim; each
/// `.` advances the proof state. If any step's primitive form
/// doesn't have a Coq translation (e.g., a hint-bearing `Auto`),
/// fall back so we don't emit a half-translated proof — chains
/// must be all-or-nothing for soundness.
fn render_tactic_chain_coq(chain: &[ChainStep<'_>]) -> TranslatedProofBody {
    let mut lines: Vec<String> = Vec::with_capacity(chain.len());
    for step in chain {
        match step {
            ChainStep::Apply { name } => lines.push(format!("apply {}.", name)),
            ChainStep::Primitive(t) => match primitive_tactic_to_coq(t) {
                Some(text) => lines.push(text),
                None => {
                    return TranslatedProofBody::Fallback {
                        reason: "Coq chain: at least one step has no Coq primitive translation"
                            .to_string(),
                    }
                }
            },
        }
    }
    TranslatedProofBody::Translated {
        text: lines.join("\n"),
    }
}

// =============================================================================
// LeanProofBodyRenderer
// =============================================================================

/// Lean 4 backend. Produces text that goes after `:=` in
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
            ProofBodyKind::Tactic | ProofBodyKind::Structured => render_single_apply_lean(body),
            ProofBodyKind::ByMethod => render_by_method_lean(body),
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

/// Lean single-apply proof: `by apply <name>`. Covers both bare
/// tactic-mode and structured-with-single-apply shapes. Also covers
/// the simple primitive tactics — each gets a `by <lean_name>`
/// translation. Multi-step shapes route through
/// [`render_tactic_chain_lean`].
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
    if let Some(chain) = classify_tactic_chain(body) {
        return render_tactic_chain_lean(&chain);
    }
    TranslatedProofBody::Fallback {
        reason: "Lean: V0 covers single-apply + primitive tactics + multi-step tactic chains".to_string(),
    }
}

/// Lean 4 multi-step tactic chain: a single `by` block with each
/// step on its own indented line. Example:
///
/// ```text
/// by
///   apply intro_lemma
///   apply transport
///   rfl
/// ```
///
/// Lean strips the leading `by` from each step's primitive form
/// (since `primitive_tactic_to_lean` returns the standalone
/// `by <tactic>` shape) — inside the block the bare tactic is what
/// Lean expects. Chains are all-or-nothing.
fn render_tactic_chain_lean(chain: &[ChainStep<'_>]) -> TranslatedProofBody {
    let mut lines: Vec<String> = Vec::with_capacity(chain.len() + 1);
    lines.push("by".to_string());
    for step in chain {
        match step {
            ChainStep::Apply { name } => lines.push(format!("  apply {}", name)),
            ChainStep::Primitive(t) => match primitive_tactic_to_lean(t) {
                Some(text) => {
                    // Strip the standalone `by ` prefix since we're
                    // inside an outer `by` block — Lean rejects nested
                    // `by` here.
                    let bare = text.strip_prefix("by ").unwrap_or(text.as_str());
                    lines.push(format!("  {}", bare));
                }
                None => {
                    return TranslatedProofBody::Fallback {
                        reason: "Lean chain: at least one step has no Lean primitive translation"
                            .to_string(),
                    }
                }
            },
        }
    }
    TranslatedProofBody::Translated {
        text: lines.join("\n"),
    }
}

// =============================================================================
// AgdaProofBodyRenderer (#156 — third backend)
// =============================================================================

/// Agda 4 backend. Agda has no tactic system in the vanilla
/// language (the experimental `Reflection` library is out of scope
/// for V0), so the translator's surface is necessarily smaller than
/// Coq/Lean: only term-mode proofs and `apply <name>(args)` shapes
/// translate cleanly. Everything else falls back to `postulate` at
/// the corpus-emission layer.
///

/// **Coverage**:
///  * `ProofBody::Term(expr)` → `<expr>` (Agda accepts term-mode
///  proofs as the right-hand side of `name = body`).
///  * `ProofBody::Tactic(Apply{lemma})` / structured single-apply
///  → `<lemma>` (Agda treats the apply as a bare-term proof).
///  * Primitive tactics (`auto`, `omega`, `ring`, ...) → fall back.
///  Agda has no built-in equivalents, and the experimental
///  `simp`/`automation` ecosystem is library-dependent.
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
            // Agda has no tactic system — by-method proofs (induction
            // / cases / contradiction) have no direct equivalent. A
            // future Agda backend could route to the experimental
            // Reflection library, but falls back to postulate.
            ProofBodyKind::ByMethod => TranslatedProofBody::Fallback {
                reason: "Agda: by-method proofs (induction/cases) have no term-mode equivalent in vanilla Agda".to_string(),
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
/// the bare lemma name as a term. The args are dropped — Agda's
/// unification fills in implicit arguments when the proof is
/// type-checked against the goal. Also covers the small set of
/// primitive tactics that have direct term-mode equivalents in
/// Agda's stdlib.
fn render_single_apply_agda(body: &ProofBody) -> TranslatedProofBody {
    if let Some((name, _args)) = classify_single_apply(body) {
        return TranslatedProofBody::Translated {
            text: name.to_string(),
        };
    }
    if let Some(tactic) = primitive_tactic(body) {
        if let Some(text) = primitive_tactic_to_agda(tactic) {
            return TranslatedProofBody::Translated { text };
        }
    }
    TranslatedProofBody::Fallback {
        reason: "Agda: V0 covers Term + single-apply + reflexivity/trivial primitives only"
            .to_string(),
    }
}

/// Translate a primitive tactic to its Agda term-mode equivalent.
/// Agda has no tactic system, so only tactics with direct term-mode
/// proof terms in Agda's stdlib translate; everything else falls
/// back to postulate.
///

/// Coverage:
///  * `Reflexivity` → `refl` — the propositional-equality
///  constructor from `Relation.Binary.PropositionalEquality`.
///  * `Trivial` → `tt` — the unit constructor for `⊤`
///  (the trivial proposition).
///

/// `Auto` / `Omega` / `Ring` etc. have no term-mode equivalents —
/// they're decision procedures that only exist as tactics in
/// Coq/Lean/Isabelle.
fn primitive_tactic_to_agda(tactic: &TacticExpr) -> Option<String> {
    Some(match tactic {
        TacticExpr::Reflexivity => "refl".to_string(),
        TacticExpr::Trivial => "tt".to_string(),
        _ => return None,
    })
}

// =============================================================================
// DeduktiProofBodyRenderer (#156 — fifth backend)
// =============================================================================

/// Dedukti backend. Dedukti is a logical framework — it has no
/// tactic system whatsoever, so the current surface is term-mode only.
/// Any proof body that doesn't reduce to a single term falls back
/// to the postulate (axiom-declaration) form at the corpus-emission
/// layer.
///

/// **Coverage** :
///  * `ProofBody::Term(expr)` → `<expr>` (same as Lean / Agda).
///  * Single-apply (Tactic + Structured) → bare lemma name as
///  a term (Dedukti's β-reduction supplies any implicit args).
///  * Primitive tactics — fall back; Dedukti has no built-in
///  tactic vocabulary.
pub struct DeduktiProofBodyRenderer;

impl DeduktiProofBodyRenderer {
    /// Construct a fresh renderer.
    pub fn new() -> Self {
        Self
    }
}

impl Default for DeduktiProofBodyRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl ProofBodyRenderer for DeduktiProofBodyRenderer {
    fn id(&self) -> &'static str {
        "dedukti"
    }

    fn render(&self, body: &ProofBody) -> TranslatedProofBody {
        match body.kind() {
            ProofBodyKind::Term => render_term_dedukti(body),
            ProofBodyKind::Tactic | ProofBodyKind::Structured => {
                render_single_apply_dedukti(body)
            }
            // Dedukti is a logical framework — no tactics, so
            // by-method proofs fall back. Future encoding libraries
            // could supply Π-style induction principles as terms.
            ProofBodyKind::ByMethod => TranslatedProofBody::Fallback {
                reason: "Dedukti: by-method proofs require an induction-principle library, not yet wired".to_string(),
            },
        }
    }
}

fn render_term_dedukti(body: &ProofBody) -> TranslatedProofBody {
    let expr = match body {
        ProofBody::Term(e) => e.as_ref(),
        _ => unreachable!("called from kind() == Term arm"),
    };
    match DeduktiExprRenderer::new().render(expr) {
        TranslatedExpr::Translated { text } => TranslatedProofBody::Translated { text },
        TranslatedExpr::Fallback { reason, .. } => TranslatedProofBody::Fallback {
            reason: format!("Dedukti term-mode: expr renderer fallback — {}", reason),
        },
    }
}

fn render_single_apply_dedukti(body: &ProofBody) -> TranslatedProofBody {
    if let Some((name, _args)) = classify_single_apply(body) {
        return TranslatedProofBody::Translated {
            text: name.to_string(),
        };
    }
    if let Some(tactic) = primitive_tactic(body) {
        if let Some(text) = primitive_tactic_to_dedukti(tactic) {
            return TranslatedProofBody::Translated { text };
        }
    }
    TranslatedProofBody::Fallback {
        reason: "Dedukti: V0 covers Term + single-apply + reflexivity/trivial primitives only"
            .to_string(),
    }
}

/// Translate a primitive tactic to its Dedukti term-mode
/// equivalent. Dedukti has no tactic system; only tactics with
/// direct term-form encodings in standard Dedukti libraries
/// translate.
///

/// Coverage:
///  * `Reflexivity` → `refl` — the propositional-equality
///  constructor (assumed to be present in the consumer's
///  theory; encodings vary by library).
///  * `Trivial` → `I` — the canonical inhabitant of `True`
///  in Coq-flavoured Dedukti encodings (LF-style).
fn primitive_tactic_to_dedukti(tactic: &TacticExpr) -> Option<String> {
    Some(match tactic {
        TacticExpr::Reflexivity => "refl".to_string(),
        TacticExpr::Trivial => "I".to_string(),
        _ => return None,
    })
}

// =============================================================================
// IsabelleProofBodyRenderer (#156 — fourth backend)
// =============================================================================

/// Isabelle/HOL backend. Isabelle's proof model is closer to Coq's
/// than Lean's — the `apply` keyword exists, classical-tactic
/// names like `auto`, `simp`, `blast` are first-class, and proofs
/// are typically structured as `proof - ... qed` blocks.
///

/// **Coverage** :
///  * `ProofBody::Term(expr)` → `by (rule <expr>)` — Isabelle's
///  reference-by-rule shape closest to Coq's `exact`.
///  * Single-apply (Tactic + Structured) → `by (rule <name>)`.
///  * Primitive tactics: Auto / Trivial / Reflexivity / Assumption /
///  Ring / Field / Omega → Isabelle's stock tactic library.
pub struct IsabelleProofBodyRenderer;

impl IsabelleProofBodyRenderer {
    /// Construct a fresh renderer.
    pub fn new() -> Self {
        Self
    }
}

impl Default for IsabelleProofBodyRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl ProofBodyRenderer for IsabelleProofBodyRenderer {
    fn id(&self) -> &'static str {
        "isabelle"
    }

    fn render(&self, body: &ProofBody) -> TranslatedProofBody {
        match body.kind() {
            ProofBodyKind::Term => render_term_isabelle(body),
            ProofBodyKind::Tactic | ProofBodyKind::Structured => render_single_apply_isabelle(body),
            ProofBodyKind::ByMethod => render_by_method_isabelle(body),
        }
    }
}

fn render_term_isabelle(body: &ProofBody) -> TranslatedProofBody {
    let expr = match body {
        ProofBody::Term(e) => e.as_ref(),
        _ => unreachable!("called from kind() == Term arm"),
    };
    match IsabelleExprRenderer::new().render(expr) {
        TranslatedExpr::Translated { text } => TranslatedProofBody::Translated {
            text: format!("by (rule {})", text),
        },
        TranslatedExpr::Fallback { reason, .. } => TranslatedProofBody::Fallback {
            reason: format!("Isabelle term-mode: expr renderer fallback — {}", reason),
        },
    }
}

fn render_single_apply_isabelle(body: &ProofBody) -> TranslatedProofBody {
    if let Some((name, _args)) = classify_single_apply(body) {
        return TranslatedProofBody::Translated {
            text: format!("by (rule {})", name),
        };
    }
    if let Some(tactic) = primitive_tactic(body) {
        if let Some(text) = primitive_tactic_to_isabelle(tactic) {
            return TranslatedProofBody::Translated { text };
        }
    }
    if let Some(chain) = classify_tactic_chain(body) {
        return render_tactic_chain_isabelle(&chain);
    }
    TranslatedProofBody::Fallback {
        reason: "Isabelle: V0 covers single-apply + primitive tactics + multi-step tactic chains".to_string(),
    }
}

/// Isabelle/HOL multi-step tactic chain: each step is a separate
/// `apply (...)` line, terminated with `done`. Example:
///
/// ```text
/// apply (rule intro_lemma)
/// apply (rule transport)
/// apply (rule refl)
/// done
/// ```
///
/// The terminating `done` closes the apply-chain — Isabelle rejects
/// the chain otherwise. Primitive tactics whose Isabelle form is
/// `by <name>` are converted to the apply-chain-internal `<name>`
/// shape (strip the leading `by ` so they fit inside the chain).
fn render_tactic_chain_isabelle(chain: &[ChainStep<'_>]) -> TranslatedProofBody {
    let mut lines: Vec<String> = Vec::with_capacity(chain.len() + 1);
    for step in chain {
        match step {
            ChainStep::Apply { name } => lines.push(format!("apply (rule {})", name)),
            ChainStep::Primitive(t) => match primitive_tactic_to_isabelle(t) {
                Some(text) => {
                    // primitive_tactic_to_isabelle returns the
                    // standalone `by <method>` form; inside an
                    // apply-chain we need `apply <method>`.
                    let bare = text.strip_prefix("by ").unwrap_or(text.as_str());
                    lines.push(format!("apply {}", bare));
                }
                None => {
                    return TranslatedProofBody::Fallback {
                        reason: "Isabelle chain: at least one step has no Isabelle primitive translation".to_string(),
                    }
                }
            },
        }
    }
    lines.push("done".to_string());
    TranslatedProofBody::Translated {
        text: lines.join("\n"),
    }
}

/// Translate a primitive tactic to its Isabelle/HOL equivalent.
/// Isabelle's tactic vocabulary is close to Coq's but with subtle
/// differences:
///  * `reflexivity` → `by simp` (Isabelle uses simp for refl-
///  equality goals; `(rule refl)` also works for bare `x = x`).
///  * `omega` → `by linarith` (Isabelle uses linarith for linear
///  arithmetic; `arith` is the older name).
///  * `field` → `by algebra` (Mathlib's `field_simp` analogue).
fn primitive_tactic_to_isabelle(tactic: &TacticExpr) -> Option<String> {
    Some(match tactic {
        TacticExpr::Trivial => "by simp".to_string(),
        TacticExpr::Assumption => "by assumption".to_string(),
        TacticExpr::Reflexivity => "by (rule refl)".to_string(),
        TacticExpr::Auto { with_hints } if with_hints.iter().count() == 0 => "by auto".to_string(),
        TacticExpr::Ring => "by algebra".to_string(),
        TacticExpr::Field => "by algebra".to_string(),
        TacticExpr::Omega => "by linarith".to_string(),
        _ => return None,
    })
}

// =============================================================================
// Primitive-tactic recognition + per-backend translation
// =============================================================================

/// Extract the inner tactic from a body that's either
/// `ProofBody::Tactic(t)` or `ProofBody::Structured` with exactly one
/// `Tactic(t)` step (and no conclusion) or `Structured` with empty
/// steps and `conclusion: Some(t)`. Mirrors [`classify_single_apply`]
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

/// Translate a primitive tactic to its Coq equivalent. Returns `None`
/// for non-primitive tactics (the caller falls back to admitted).
///

/// **Why these tactics**: each is decidable enough that the foreign
/// tool can re-discharge it with a single tactic invocation. The
/// translation is direct: the Verum tactic name matches the Coq
/// tactic name (Coq has all of these in its built-in tactic library).
fn primitive_tactic_to_coq(tactic: &TacticExpr) -> Option<String> {
    Some(match tactic {
        TacticExpr::Trivial => "trivial.".to_string(),
        TacticExpr::Assumption => "assumption.".to_string(),
        TacticExpr::Reflexivity => "reflexivity.".to_string(),
        TacticExpr::Auto { with_hints } if with_hints.iter().count() == 0 => "auto.".to_string(),
        TacticExpr::Ring => "ring.".to_string(),
        TacticExpr::Field => "field.".to_string(),
        TacticExpr::Omega => "lia.".to_string(), // modern Coq uses lia for omega
        _ => return None,
    })
}

/// Translate a primitive tactic to its Lean 4 equivalent. Returns
/// `None` for non-primitive tactics. Lean tactic names diverge
/// slightly from Coq:
///

///  * `reflexivity` → `rfl`
///  * `omega` → `omega` (Mathlib provides this)
///  * `field` → `field_simp` (Mathlib)
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
// ByMethod (induction / cases / contradiction) translation
// =============================================================================
//

// `proof by induction n;` parses to `ProofBody::ByMethod(Induction
// { on: Some(n), cases: [] })`. When the cases list is empty (the
// theorem leaves the per-case bodies to the foreign tool's
// automation), the translation is a single tactic:
//

//  Coq: `induction <n>.` / `destruct <e>.`
//  Lean: `by induction <n>` / `by cases <e>`
//  Isabelle: `by (induct <n>)` / `by (cases <e>)`
//

// When the cases list is non-empty, Verum's per-case proof bodies
// would need to be rendered alongside — that's V1. falls back
// to `Admitted.` / `sorry` for the explicit-cases shape.

fn classify_by_method(body: &ProofBody) -> Option<&ProofMethod> {
    match body {
        ProofBody::ByMethod(m) => Some(m),
        _ => None,
    }
}

/// Coq translation for `proof by induction <on>` / `proof by cases <on>`
/// when the cases list is empty (auto-discharged). Other shapes
/// (Contradiction, StrongInduction, WellFoundedInduction, or any
/// non-empty cases list) fall back.
fn render_by_method_coq(body: &ProofBody) -> TranslatedProofBody {
    let method = match classify_by_method(body) {
        Some(m) => m,
        None => unreachable!("called from kind() == ByMethod arm"),
    };
    match method {
        ProofMethod::Induction { on, cases } if cases.iter().count() == 0 => {
            match on {
                Maybe::Some(ident) => TranslatedProofBody::Translated {
                    text: format!("induction {}.", ident.name.as_str()),
                },
                // Bare `induction` without target — Coq accepts on
                // the topmost hypothesis.
                Maybe::None => TranslatedProofBody::Translated {
                    text: "induction.".to_string(),
                },
            }
        }
        ProofMethod::Cases { on, cases } if cases.iter().count() == 0 => {
            // Cases scrutinee is a Heap<Expr>; render it via the Coq
            // expression translator. When the expression doesn't
            // translate cleanly (rare for typical scrutinees),
            // fall back.
            match CoqExprRenderer::new().render(on.as_ref()) {
                TranslatedExpr::Translated { text } => TranslatedProofBody::Translated {
                    text: format!("destruct {}.", text),
                },
                TranslatedExpr::Fallback { reason, .. } => TranslatedProofBody::Fallback {
                    reason: format!("Coq cases: scrutinee untranslatable — {}", reason),
                },
            }
        }
        ProofMethod::StrongInduction { on, cases } if cases.iter().count() == 0 => {
            // Coq's strong-induction tactic is `induction ... using
            // strong_induction.` (or `well_founded_induction`); the
            // bare-name form is library-dependent. Default emits the
            // generic-induction form and lets Coq's automation
            // pick up the strong-induction principle from context.
            TranslatedProofBody::Translated {
                text: format!("induction {} using strong_induction.", on.name.as_str()),
            }
        }
        // Contradiction: requires per-step rendering of the proof
        // body. falls back.
        ProofMethod::Contradiction { .. } => TranslatedProofBody::Fallback {
            reason: "Coq contradiction: multi-step body not yet rendered (V0)".to_string(),
        },
        // Non-empty cases or well-founded: V1.
        _ => TranslatedProofBody::Fallback {
            reason: "Coq by-method: explicit-case bodies / well-founded induction land in V1"
                .to_string(),
        },
    }
}

/// Lean translation — same coverage as Coq, but Lean syntax:
/// `by induction <n>` / `by cases <e>`.
fn render_by_method_lean(body: &ProofBody) -> TranslatedProofBody {
    let method = match classify_by_method(body) {
        Some(m) => m,
        None => unreachable!("called from kind() == ByMethod arm"),
    };
    match method {
        ProofMethod::Induction { on, cases } if cases.iter().count() == 0 => match on {
            Maybe::Some(ident) => TranslatedProofBody::Translated {
                text: format!("by induction {}", ident.name.as_str()),
            },
            Maybe::None => TranslatedProofBody::Translated {
                text: "by induction".to_string(),
            },
        },
        ProofMethod::Cases { on, cases } if cases.iter().count() == 0 => {
            match LeanExprRenderer::new().render(on.as_ref()) {
                TranslatedExpr::Translated { text } => TranslatedProofBody::Translated {
                    text: format!("by cases {}", text),
                },
                TranslatedExpr::Fallback { reason, .. } => TranslatedProofBody::Fallback {
                    reason: format!("Lean cases: scrutinee untranslatable — {}", reason),
                },
            }
        }
        ProofMethod::StrongInduction { on, cases } if cases.iter().count() == 0 => {
            TranslatedProofBody::Translated {
                text: format!(
                    "by induction {} using Nat.strong_induction_on",
                    on.name.as_str(),
                ),
            }
        }
        ProofMethod::Contradiction { .. } => TranslatedProofBody::Fallback {
            reason: "Lean contradiction: multi-step body not yet rendered (V0)".to_string(),
        },
        _ => TranslatedProofBody::Fallback {
            reason: "Lean by-method: explicit-case bodies / well-founded induction land in V1"
                .to_string(),
        },
    }
}

/// Isabelle/HOL translation — `by (induct <n>)` / `by (cases <e>)`.
/// Isabelle's `induct` method does both structural induction and
/// case-split selection automatically.
fn render_by_method_isabelle(body: &ProofBody) -> TranslatedProofBody {
    let method = match classify_by_method(body) {
        Some(m) => m,
        None => unreachable!("called from kind() == ByMethod arm"),
    };
    match method {
        ProofMethod::Induction { on, cases } if cases.iter().count() == 0 => match on {
            Maybe::Some(ident) => TranslatedProofBody::Translated {
                text: format!("by (induct {})", ident.name.as_str()),
            },
            Maybe::None => TranslatedProofBody::Translated {
                text: "by induct_tac".to_string(),
            },
        },
        ProofMethod::Cases { on, cases } if cases.iter().count() == 0 => {
            match IsabelleExprRenderer::new().render(on.as_ref()) {
                TranslatedExpr::Translated { text } => TranslatedProofBody::Translated {
                    text: format!("by (cases \"{}\")", text),
                },
                TranslatedExpr::Fallback { reason, .. } => TranslatedProofBody::Fallback {
                    reason: format!("Isabelle cases: scrutinee untranslatable — {}", reason),
                },
            }
        }
        ProofMethod::StrongInduction { on, cases } if cases.iter().count() == 0 => {
            TranslatedProofBody::Translated {
                text: format!("by (induct {} rule: less_induct)", on.name.as_str()),
            }
        }
        ProofMethod::Contradiction { .. } => TranslatedProofBody::Fallback {
            reason: "Isabelle contradiction: multi-step body not yet rendered (V0)".to_string(),
        },
        _ => TranslatedProofBody::Fallback {
            reason: "Isabelle by-method: explicit-case bodies / well-founded induction land in V1"
                .to_string(),
        },
    }
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
    /// the outer Apply.args is empty. This is what the fast parser
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
        // emits `apply <name>.` and lets Coq unification figure
        // out the args. Future V1 may emit `apply (<name> arg1 arg2).`
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
    /// step and no conclusion. This is the shape produced by the
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

    /// Helper: build a Structured proof body whose `steps` is the
    /// given list of `apply <name>` tactics with no conclusion.
    fn structured_apply_chain(names: &[&str]) -> ProofBody {
        use verum_ast::decl::{ProofStep, ProofStructure};
        let make_step = |name: &str| ProofStep {
            kind: ProofStepKind::Tactic(TacticExpr::Apply {
                lemma: Heap::new(name_path_expr(name)),
                args: List::from(Vec::new()),
            }),
            span: span(),
        };
        let steps: Vec<ProofStep> = names.iter().map(|n| make_step(n)).collect();
        ProofBody::Structured(ProofStructure {
            steps: List::from(steps),
            conclusion: Maybe::None,
            span: span(),
        })
    }

    /// Helper: same as above, but the last apply lands in `conclusion`
    /// instead of `steps`. Some parser paths produce this shape.
    fn structured_apply_chain_with_conclusion(steps: &[&str], conclusion: &str) -> ProofBody {
        use verum_ast::decl::{ProofStep, ProofStructure};
        let make_step = |name: &str| ProofStep {
            kind: ProofStepKind::Tactic(TacticExpr::Apply {
                lemma: Heap::new(name_path_expr(name)),
                args: List::from(Vec::new()),
            }),
            span: span(),
        };
        let mid: Vec<ProofStep> = steps.iter().map(|n| make_step(n)).collect();
        ProofBody::Structured(ProofStructure {
            steps: List::from(mid),
            conclusion: Maybe::Some(TacticExpr::Apply {
                lemma: Heap::new(name_path_expr(conclusion)),
                args: List::from(Vec::new()),
            }),
            span: span(),
        })
    }

    // ----- Multi-step tactic chain (#153 Phase 2 extension) -----

    #[test]
    fn coq_renders_two_step_apply_chain() {
        let body = structured_apply_chain(&["intro_lemma", "transport"]);
        let r = CoqProofBodyRenderer::new().render(&body);
        assert_eq!(
            r.text(),
            Some("apply intro_lemma.\napply transport."),
            "Coq must emit one `apply <name>.` per step, newline-separated",
        );
    }

    #[test]
    fn lean_renders_two_step_apply_chain() {
        let body = structured_apply_chain(&["intro_lemma", "transport"]);
        let r = LeanProofBodyRenderer::new().render(&body);
        assert_eq!(
            r.text(),
            Some("by\n  apply intro_lemma\n  apply transport"),
            "Lean must emit a single `by` block with each apply on its own indented line",
        );
    }

    #[test]
    fn isabelle_renders_two_step_apply_chain() {
        let body = structured_apply_chain(&["intro_lemma", "transport"]);
        let r = IsabelleProofBodyRenderer::new().render(&body);
        assert_eq!(
            r.text(),
            Some("apply (rule intro_lemma)\napply (rule transport)\ndone"),
            "Isabelle must emit one `apply (rule <name>)` per step, terminated with `done`",
        );
    }

    #[test]
    fn coq_renders_three_step_chain_with_primitive() {
        // apply foo; apply bar; reflexivity. — mixed apply + primitive.
        use verum_ast::decl::{ProofStep, ProofStructure};
        let make_apply = |name: &str| ProofStep {
            kind: ProofStepKind::Tactic(TacticExpr::Apply {
                lemma: Heap::new(name_path_expr(name)),
                args: List::from(Vec::new()),
            }),
            span: span(),
        };
        let make_refl = || ProofStep {
            kind: ProofStepKind::Tactic(TacticExpr::Reflexivity),
            span: span(),
        };
        let body = ProofBody::Structured(ProofStructure {
            steps: List::from(vec![make_apply("foo"), make_apply("bar"), make_refl()]),
            conclusion: Maybe::None,
            span: span(),
        });
        let r = CoqProofBodyRenderer::new().render(&body);
        assert_eq!(
            r.text(),
            Some("apply foo.\napply bar.\nreflexivity."),
            "Mixed apply + primitive chain must render every step",
        );
    }

    #[test]
    fn lean_renders_three_step_chain_with_primitive() {
        use verum_ast::decl::{ProofStep, ProofStructure};
        let make_apply = |name: &str| ProofStep {
            kind: ProofStepKind::Tactic(TacticExpr::Apply {
                lemma: Heap::new(name_path_expr(name)),
                args: List::from(Vec::new()),
            }),
            span: span(),
        };
        let make_refl = || ProofStep {
            kind: ProofStepKind::Tactic(TacticExpr::Reflexivity),
            span: span(),
        };
        let body = ProofBody::Structured(ProofStructure {
            steps: List::from(vec![make_apply("foo"), make_apply("bar"), make_refl()]),
            conclusion: Maybe::None,
            span: span(),
        });
        let r = LeanProofBodyRenderer::new().render(&body);
        // `Reflexivity` → `by rfl` standalone — the `by ` prefix is
        // stripped inside the outer `by` block.
        assert_eq!(
            r.text(),
            Some("by\n  apply foo\n  apply bar\n  rfl"),
        );
    }

    #[test]
    fn isabelle_renders_three_step_chain_with_primitive() {
        use verum_ast::decl::{ProofStep, ProofStructure};
        let make_apply = |name: &str| ProofStep {
            kind: ProofStepKind::Tactic(TacticExpr::Apply {
                lemma: Heap::new(name_path_expr(name)),
                args: List::from(Vec::new()),
            }),
            span: span(),
        };
        let make_assumption = || ProofStep {
            kind: ProofStepKind::Tactic(TacticExpr::Assumption),
            span: span(),
        };
        let body = ProofBody::Structured(ProofStructure {
            steps: List::from(vec![
                make_apply("foo"),
                make_apply("bar"),
                make_assumption(),
            ]),
            conclusion: Maybe::None,
            span: span(),
        });
        let r = IsabelleProofBodyRenderer::new().render(&body);
        assert_eq!(
            r.text(),
            Some("apply (rule foo)\napply (rule bar)\napply assumption\ndone"),
        );
    }

    #[test]
    fn coq_renders_chain_with_step_plus_conclusion() {
        // The parser may place the final apply in `conclusion` while
        // earlier applies sit in `steps`. The chain renderer must
        // accept both shapes uniformly.
        let body = structured_apply_chain_with_conclusion(&["a"], "b");
        let r = CoqProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("apply a.\napply b."));
    }

    #[test]
    fn lean_renders_chain_with_step_plus_conclusion() {
        let body = structured_apply_chain_with_conclusion(&["a"], "b");
        let r = LeanProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("by\n  apply a\n  apply b"));
    }

    #[test]
    fn isabelle_renders_chain_with_step_plus_conclusion() {
        let body = structured_apply_chain_with_conclusion(&["a"], "b");
        let r = IsabelleProofBodyRenderer::new().render(&body);
        assert_eq!(
            r.text(),
            Some("apply (rule a)\napply (rule b)\ndone"),
        );
    }

    #[test]
    fn chain_with_unsupported_tactic_falls_back_atomically() {
        // Chain coverage is all-or-nothing: a single step that the
        // backend can't translate (`Split`) blocks the entire chain.
        // Half-translated proofs would be unsound.
        use verum_ast::decl::{ProofStep, ProofStructure};
        let make_apply = |name: &str| ProofStep {
            kind: ProofStepKind::Tactic(TacticExpr::Apply {
                lemma: Heap::new(name_path_expr(name)),
                args: List::from(Vec::new()),
            }),
            span: span(),
        };
        let make_split = || ProofStep {
            kind: ProofStepKind::Tactic(TacticExpr::Split),
            span: span(),
        };
        let body = ProofBody::Structured(ProofStructure {
            steps: List::from(vec![make_apply("foo"), make_split()]),
            conclusion: Maybe::None,
            span: span(),
        });
        // All three backends must fall back — `Split` isn't covered.
        assert!(matches!(
            CoqProofBodyRenderer::new().render(&body),
            TranslatedProofBody::Fallback { .. },
        ));
        assert!(matches!(
            LeanProofBodyRenderer::new().render(&body),
            TranslatedProofBody::Fallback { .. },
        ));
        assert!(matches!(
            IsabelleProofBodyRenderer::new().render(&body),
            TranslatedProofBody::Fallback { .. },
        ));
    }

    #[test]
    fn agda_falls_back_on_chain() {
        // Agda has no tactic chain syntax in vanilla form — chains
        // must fall back to postulate.
        let body = structured_apply_chain(&["foo", "bar"]);
        assert!(matches!(
            AgdaProofBodyRenderer::new().render(&body),
            TranslatedProofBody::Fallback { .. },
        ));
    }

    #[test]
    fn dedukti_falls_back_on_chain() {
        // Dedukti has no tactic chain syntax — chains must fall back.
        let body = structured_apply_chain(&["foo", "bar"]);
        assert!(matches!(
            DeduktiProofBodyRenderer::new().render(&body),
            TranslatedProofBody::Fallback { .. },
        ));
    }

    #[test]
    fn classify_tactic_chain_rejects_single_step() {
        // Single-step shapes route through classify_single_apply,
        // not the chain classifier — keep responsibility split.
        let body = structured_apply_chain(&["foo"]);
        assert!(classify_tactic_chain(&body).is_none());
    }

    #[test]
    fn classify_tactic_chain_rejects_have_step() {
        // `have h: P by t;` carries a sub-body that V0 doesn't render —
        // chain classifier must bail so the whole body falls back.
        use verum_ast::decl::{ProofStep, ProofStructure};
        let have_step = ProofStep {
            kind: ProofStepKind::Have {
                name: ident("h"),
                proposition: Heap::new(name_path_expr("P")),
                justification: TacticExpr::Trivial,
            },
            span: span(),
        };
        let apply_step = ProofStep {
            kind: ProofStepKind::Tactic(TacticExpr::Apply {
                lemma: Heap::new(name_path_expr("foo")),
                args: List::from(Vec::new()),
            }),
            span: span(),
        };
        let body = ProofBody::Structured(ProofStructure {
            steps: List::from(vec![have_step, apply_step]),
            conclusion: Maybe::None,
            span: span(),
        });
        assert!(classify_tactic_chain(&body).is_none());
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
        // step. Should translate just like the bare Tactic form.
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
    fn agda_translates_term_mode_primitives() {
        // Reflexivity + Trivial have direct term-mode equivalents in
        // Agda's stdlib (`refl` / `tt`). Everything else still
        // falls back since Agda has no tactic system.
        assert_eq!(
            AgdaProofBodyRenderer::new()
                .render(&ProofBody::Tactic(TacticExpr::Reflexivity))
                .text(),
            Some("refl"),
        );
        assert_eq!(
            AgdaProofBodyRenderer::new()
                .render(&ProofBody::Tactic(TacticExpr::Trivial))
                .text(),
            Some("tt"),
        );
    }

    #[test]
    fn agda_falls_back_on_decision_tactics() {
        // Auto / Omega / Ring have no term-mode equivalents in Agda.
        for tactic in [
            TacticExpr::Auto {
                with_hints: List::new(),
            },
            TacticExpr::Omega,
            TacticExpr::Ring,
        ] {
            let body = ProofBody::Tactic(tactic);
            let r = AgdaProofBodyRenderer::new().render(&body);
            assert!(
                matches!(r, TranslatedProofBody::Fallback { .. }),
                "Agda decision tactics fall back — no term-mode equivalents",
            );
        }
    }

    #[test]
    fn agda_translates_structured_apply_step() {
        let body = structured_apply_step("backbone_full", vec![]);
        let r = AgdaProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("backbone_full"));
    }

    // -------------------------------------------------------------
    // Isabelle/HOL translator (#156 — fourth backend)
    // -------------------------------------------------------------

    #[test]
    fn isabelle_renders_apply_as_by_rule() {
        let body = apply_body("backbone_full", vec![]);
        let r = IsabelleProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("by (rule backbone_full)"));
    }

    #[test]
    fn isabelle_renders_term_body_as_by_rule() {
        let body = term_body("h_x");
        let r = IsabelleProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("by (rule h_x)"));
    }

    #[test]
    fn isabelle_renders_primitive_tactics() {
        let body = primitive_body(TacticExpr::Trivial);
        assert_eq!(
            IsabelleProofBodyRenderer::new().render(&body).text(),
            Some("by simp"),
        );
        let body = primitive_body(TacticExpr::Auto {
            with_hints: List::new(),
        });
        assert_eq!(
            IsabelleProofBodyRenderer::new().render(&body).text(),
            Some("by auto"),
        );
        let body = primitive_body(TacticExpr::Omega);
        assert_eq!(
            IsabelleProofBodyRenderer::new().render(&body).text(),
            Some("by linarith"),
            "Isabelle uses linarith for omega-style decisions",
        );
        let body = primitive_body(TacticExpr::Reflexivity);
        assert_eq!(
            IsabelleProofBodyRenderer::new().render(&body).text(),
            Some("by (rule refl)"),
        );
    }

    #[test]
    fn isabelle_renders_call_shape_apply() {
        let body = apply_body_via_call("lemma_x", vec![name_path_expr("p")]);
        let r = IsabelleProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("by (rule lemma_x)"));
    }

    // -------------------------------------------------------------
    // Dedukti translator (#156 — fifth backend)
    // -------------------------------------------------------------

    #[test]
    fn dedukti_renders_apply_as_bare_term() {
        let body = apply_body("backbone_full", vec![]);
        let r = DeduktiProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("backbone_full"));
    }

    #[test]
    fn dedukti_renders_term_body_passthrough() {
        let body = term_body("h_x");
        let r = DeduktiProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("h_x"));
    }

    #[test]
    fn dedukti_translates_term_mode_primitives() {
        // Reflexivity + Trivial have direct term encodings in
        // standard Dedukti libraries.
        assert_eq!(
            DeduktiProofBodyRenderer::new()
                .render(&primitive_body(TacticExpr::Reflexivity))
                .text(),
            Some("refl"),
        );
        assert_eq!(
            DeduktiProofBodyRenderer::new()
                .render(&primitive_body(TacticExpr::Trivial))
                .text(),
            Some("I"),
        );
    }

    #[test]
    fn dedukti_falls_back_on_decision_tactics() {
        let body = primitive_body(TacticExpr::Auto {
            with_hints: List::new(),
        });
        let r = DeduktiProofBodyRenderer::new().render(&body);
        assert!(
            matches!(r, TranslatedProofBody::Fallback { .. }),
            "Dedukti decision tactics (auto/omega/ring) fall back",
        );
    }

    #[test]
    fn dedukti_renders_call_shape_apply() {
        let body = apply_body_via_call("lemma_x", vec![name_path_expr("p")]);
        let r = DeduktiProofBodyRenderer::new().render(&body);
        assert_eq!(r.text(), Some("lemma_x"));
    }

    // -------------------------------------------------------------
    // ByMethod (induction / cases / strong_induction) translation
    // -------------------------------------------------------------

    fn induction_body(on: Option<&str>) -> ProofBody {
        ProofBody::ByMethod(ProofMethod::Induction {
            on: match on {
                Some(n) => Maybe::Some(ident(n)),
                None => Maybe::None,
            },
            cases: List::from(Vec::new()),
        })
    }

    fn cases_body(scrutinee_name: &str) -> ProofBody {
        ProofBody::ByMethod(ProofMethod::Cases {
            on: Heap::new(name_path_expr(scrutinee_name)),
            cases: List::from(Vec::new()),
        })
    }

    fn strong_induction_body(on: &str) -> ProofBody {
        ProofBody::ByMethod(ProofMethod::StrongInduction {
            on: ident(on),
            cases: List::from(Vec::new()),
        })
    }

    fn contradiction_body(assumption: &str) -> ProofBody {
        ProofBody::ByMethod(ProofMethod::Contradiction {
            assumption: ident(assumption),
            proof: List::from(Vec::new()),
        })
    }

    // -------------------------------------------------------------
    // Multi-segment dotted-path apply targets (#153 — mathlib4 etc.)
    // -------------------------------------------------------------

    fn dotted_path_expr(segments: &[&str]) -> Expr {
        let segs: Vec<PathSegment> = segments
            .iter()
            .map(|s| PathSegment::Name(ident(s)))
            .collect();
        let path = Path::new(List::from(segs), span());
        Expr::new(ExprKind::Path(path), span())
    }

    fn apply_dotted(segments: &[&str], outer_args: Vec<Expr>) -> ProofBody {
        ProofBody::Tactic(TacticExpr::Apply {
            lemma: Heap::new(dotted_path_expr(segments)),
            args: List::from(outer_args),
        })
    }

    /// Build a `Field { expr, field }` chain — the parser's actual
    /// representation of source `a.b.c` (NOT a multi-segment Path).
    fn field_chain_expr(segments: &[&str]) -> Expr {
        assert!(!segments.is_empty(), "at least one segment required");
        let head = name_path_expr(segments[0]);
        let mut cur = head;
        for seg in &segments[1..] {
            cur = Expr::new(
                ExprKind::Field {
                    expr: Heap::new(cur),
                    field: ident(seg),
                },
                span(),
            );
        }
        cur
    }

    fn apply_field_chain(segments: &[&str], outer_args: Vec<Expr>) -> ProofBody {
        ProofBody::Tactic(TacticExpr::Apply {
            lemma: Heap::new(field_chain_expr(segments)),
            args: List::from(outer_args),
        })
    }

    #[test]
    fn coq_renders_apply_with_field_chain_path() {
        // Source: `apply mathlib4.lambda.ChurchRosser;`
        // Parser: Apply { lemma: Field-chain, args: [] }
        let body = apply_field_chain(&["mathlib4", "lambda", "ChurchRosser"], vec![]);
        assert_eq!(
            CoqProofBodyRenderer::new().render(&body).text(),
            Some("apply mathlib4.lambda.ChurchRosser."),
        );
    }

    #[test]
    fn lean_renders_apply_with_field_chain_path() {
        let body = apply_field_chain(&["Mathlib", "Computability", "ChurchRosser"], vec![]);
        assert_eq!(
            LeanProofBodyRenderer::new().render(&body).text(),
            Some("by apply Mathlib.Computability.ChurchRosser"),
        );
    }

    #[test]
    fn isabelle_renders_apply_with_field_chain_path() {
        let body = apply_field_chain(&["HOL", "List", "rev_rev"], vec![]);
        assert_eq!(
            IsabelleProofBodyRenderer::new().render(&body).text(),
            Some("by (rule HOL.List.rev_rev)"),
        );
    }

    #[test]
    fn coq_renders_apply_with_multi_segment_path() {
        let body = apply_dotted(&["mathlib4", "lambda", "ChurchRosser"], vec![]);
        assert_eq!(
            CoqProofBodyRenderer::new().render(&body).text(),
            Some("apply mathlib4.lambda.ChurchRosser."),
        );
    }

    #[test]
    fn lean_renders_apply_with_multi_segment_path() {
        let body = apply_dotted(&["Mathlib", "Computability", "ChurchRosser"], vec![]);
        assert_eq!(
            LeanProofBodyRenderer::new().render(&body).text(),
            Some("by apply Mathlib.Computability.ChurchRosser"),
        );
    }

    #[test]
    fn isabelle_renders_apply_with_multi_segment_path() {
        let body = apply_dotted(&["HOL", "List", "rev_rev"], vec![]);
        assert_eq!(
            IsabelleProofBodyRenderer::new().render(&body).text(),
            Some("by (rule HOL.List.rev_rev)"),
        );
    }

    #[test]
    fn agda_renders_apply_with_multi_segment_path() {
        let body = apply_dotted(&["Agda", "Builtin", "Equality", "refl"], vec![]);
        assert_eq!(
            AgdaProofBodyRenderer::new().render(&body).text(),
            Some("Agda.Builtin.Equality.refl"),
        );
    }

    #[test]
    fn dedukti_renders_apply_with_multi_segment_path() {
        let body = apply_dotted(&["std", "ChurchRosser"], vec![]);
        assert_eq!(
            DeduktiProofBodyRenderer::new().render(&body).text(),
            Some("std.ChurchRosser"),
        );
    }

    #[test]
    fn coq_renders_call_with_multi_segment_callee() {
        // `apply mathlib4.foo.bar(x, y);` — parser places the call
        // inside Apply.lemma.
        let dotted = dotted_path_expr(&["mathlib4", "foo", "bar"]);
        let call = Expr::new(
            ExprKind::Call {
                func: Heap::new(dotted),
                type_args: List::new(),
                args: List::from(vec![name_path_expr("x"), name_path_expr("y")]),
            },
            span(),
        );
        let body = ProofBody::Tactic(TacticExpr::Apply {
            lemma: Heap::new(call),
            args: List::new(),
        });
        assert_eq!(
            CoqProofBodyRenderer::new().render(&body).text(),
            Some("apply mathlib4.foo.bar."),
        );
    }

    #[test]
    fn coq_renders_induction_with_target() {
        let body = induction_body(Some("n"));
        assert_eq!(
            CoqProofBodyRenderer::new().render(&body).text(),
            Some("induction n."),
        );
    }

    #[test]
    fn coq_renders_induction_without_target() {
        let body = induction_body(None);
        assert_eq!(
            CoqProofBodyRenderer::new().render(&body).text(),
            Some("induction."),
        );
    }

    #[test]
    fn coq_renders_cases_on_scrutinee() {
        let body = cases_body("scrut");
        assert_eq!(
            CoqProofBodyRenderer::new().render(&body).text(),
            Some("destruct scrut."),
        );
    }

    #[test]
    fn coq_renders_strong_induction() {
        let body = strong_induction_body("n");
        assert_eq!(
            CoqProofBodyRenderer::new().render(&body).text(),
            Some("induction n using strong_induction."),
        );
    }

    #[test]
    fn coq_falls_back_on_contradiction() {
        let body = contradiction_body("h");
        assert!(matches!(
            CoqProofBodyRenderer::new().render(&body),
            TranslatedProofBody::Fallback { .. },
        ));
    }

    #[test]
    fn lean_renders_induction_with_target() {
        let body = induction_body(Some("n"));
        assert_eq!(
            LeanProofBodyRenderer::new().render(&body).text(),
            Some("by induction n"),
        );
    }

    #[test]
    fn lean_renders_cases_on_scrutinee() {
        let body = cases_body("scrut");
        assert_eq!(
            LeanProofBodyRenderer::new().render(&body).text(),
            Some("by cases scrut"),
        );
    }

    #[test]
    fn isabelle_renders_induction_with_target() {
        let body = induction_body(Some("n"));
        assert_eq!(
            IsabelleProofBodyRenderer::new().render(&body).text(),
            Some("by (induct n)"),
        );
    }

    #[test]
    fn isabelle_renders_cases_on_scrutinee() {
        let body = cases_body("scrut");
        assert_eq!(
            IsabelleProofBodyRenderer::new().render(&body).text(),
            Some("by (cases \"scrut\")"),
        );
    }

    #[test]
    fn agda_falls_back_on_by_method_induction() {
        // Agda has no tactic-mode induction in vanilla form.
        let body = induction_body(Some("n"));
        assert!(matches!(
            AgdaProofBodyRenderer::new().render(&body),
            TranslatedProofBody::Fallback { .. },
        ));
    }

    #[test]
    fn dedukti_falls_back_on_by_method() {
        let body = induction_body(Some("n"));
        assert!(matches!(
            DeduktiProofBodyRenderer::new().render(&body),
            TranslatedProofBody::Fallback { .. },
        ));
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
