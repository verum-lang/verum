//! Corpus-theorem cross-format export — task #138 / MSFS-L4.5.
//!
//! Sibling to the kernel-soundness export (`coq.rs` / `lean.rs`) but
//! aimed at *corpus theorems* (the theorems users write in their
//! `.vr` modules, not the kernel's own meta-theory).  Produces
//! per-theorem foreign-tool source files that `coqc` / `lean` can
//! re-check independently.
//!
//! ## Architecture (protocol-driven)
//!
//! Single trait [`CorpusBackend`] with one implementation per
//! foreign tool.  Adding Isabelle / Agda / Dedukti is a single new
//! [`CorpusBackend`] instance — the walker, the audit gate, and the
//! cross-format runner are all parameterised over the trait.
//!
//! ## What gets emitted
//!
//! For every `@theorem name(...) ensures E proof { … }` declaration:
//!
//!   * The theorem name as the foreign-tool identifier
//!     (sanitised — Verum `snake_case` maps directly; non-ASCII
//!     and reserved-word collisions get a `verum_` prefix).
//!   * The proposition rendered as a textual statement (best-effort
//!     for the V0 surface; opaque `Prop` parameter for non-trivial
//!     shapes — sufficient to verify the statement *type-checks* in
//!     the foreign system).
//!   * A `sorry` (Lean) / `Admitted.` (Coq) marker because the
//!     proof body itself isn't being exported in this round.  The
//!     CI-gate semantics: *the statement is well-formed in the
//!     foreign system*.  Proof-term export is a separate piece
//!     (proof_replay backends).
//!
//! This is the **statement-level CI gate**.  Even at this scope it
//! catches:
//!
//!   * Statements that don't type-check in the foreign system (e.g.,
//!     malformed quantifier scoping, undefined operators, missing
//!     imports).
//!   * Tooling regressions — `coqc` / `lean` version drift surfaces
//!     when the gate runs.
//!
//! Proof-term re-check is a strictly stronger gate that requires the
//! kernel's CoreTerm proof to be lowered to each foreign system's
//! tactic language; that lives in `verum_smt::proof_replay::*` and is
//! not in scope here.

use serde::{Deserialize, Serialize};

use super::expr_translate::{
    CoqExprRenderer, CoqTypeRenderer, ExprRenderer, LeanExprRenderer, LeanTypeRenderer,
    TranslatedExpr, TranslatedType, TypeRenderer,
};

/// One theorem parameter — name + per-backend type text (#141).
/// `per_backend_type_text` keys are backend ids (`"coq"`, `"lean"`);
/// when a backend's translator succeeded, the value is the foreign-
/// tool type syntax for the parameter.  When it fell back, the entry
/// for that backend is absent and the per-format renderer emits a
/// generic `Type` placeholder so the binding still exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheoremParam {
    /// Parameter name (already sanitised to a foreign-tool identifier).
    pub name: String,
    /// Per-backend rendered type text — keys: `"coq"` / `"lean"`.
    pub per_backend_type_text: std::collections::BTreeMap<String, String>,
}

/// One generic type parameter (#145 / MSFS-L4.11).  Theorems with
/// generic parameters like `<S: RichS>` are emitted as Coq/Lean
/// implicit arguments (`{S : Type}`) preceding the value parameters,
/// so foreign tools see the universe-quantified statement rather than
/// an undeclared identifier.
///
/// Protocol bounds (`S: RichS`) are preserved as a per-backend comment
/// hint rather than as a typeclass instance, because instance lowering
/// requires the protocol to be exported as a Coq Class / Lean class
/// — which the cross-format gate doesn't currently emit (a future
/// pass can flip the emission to instance arguments once protocol-
/// to-class export lands).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheoremGeneric {
    /// Generic parameter name (e.g., `S`).
    pub name: String,
    /// Bound text — a human-readable description of the protocol bound,
    /// preserved into the foreign-tool emission as an annotation.
    /// Example: `"S : RichS"` for `<S: RichS>`.  Empty when the
    /// generic carries no bound.
    pub bound_annotation: String,
}

/// One theorem's cross-format export specification — what the
/// per-format renderer needs.  Constructed from the AST by the
/// audit walker; consumed by [`CorpusBackend::render_theorem`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheoremSpec {
    /// Theorem name as written in the source (sanitised by the
    /// walker before reaching the renderer — non-ASCII / reserved-
    /// word collisions are already handled).
    pub name: String,
    /// Module path of the source file (e.g.,
    /// `theorems.msfs.05_afnt_alpha.theorem_5_1`).  Preserved into
    /// the foreign-tool file's header comment.
    pub module_path: String,
    /// Proposition rendered as text — best-effort.  When the
    /// statement involves Verum-specific shapes the renderer can't
    /// translate, this falls back to `Prop` and the foreign-tool
    /// re-check verifies only that the *name* binding is well-formed.
    pub proposition_text: String,
    /// **Per-format translated proposition (#140 / MSFS-L4.7)**.
    /// When `Some`, the corresponding [`CorpusBackend`] uses this
    /// rendered proposition as the theorem's TYPE in the foreign
    /// tool — e.g., `Theorem foo : (n = 7).` instead of
    /// `Theorem foo : Prop.`  Keys are backend ids
    /// (`"coq"`, `"lean"`); values are the renderer's output.
    /// When the translation falls back, the entry stays absent and
    /// the renderer reverts to `Prop` with the original text in a
    /// comment.
    #[serde(default)]
    pub per_backend_proposition: std::collections::BTreeMap<String, String>,
    /// **Theorem parameters (#141 / MSFS-L4.8)**.  Each entry is a
    /// (name, per-backend-type-text) pair where the inner map keys
    /// are backend ids (`"coq"` / `"lean"`).  When a parameter's
    /// type translates cleanly for a backend, the corresponding
    /// renderer emits the parameter binding before the colon-
    /// separator: `Theorem foo (n : Z) : (n = 7).` instead of
    /// `Theorem foo : (n = 7).`  Without parameter declarations the
    /// foreign tool rejects on undeclared identifier — so this is
    /// the load-bearing piece that makes the type-structure gate
    /// (#140) actually fire.
    ///
    /// Parameters whose types fall back are still emitted with a
    /// generic `Type` placeholder so the binding exists; the
    /// foreign tool then validates that the proposition's free
    /// variables are at least DECLARED, even when their types
    /// can't be translated faithfully.
    #[serde(default)]
    pub params: Vec<TheoremParam>,
    /// **Generic type parameters (#145 / MSFS-L4.11)**.  Each entry
    /// is one type-parameter binding (`<S: RichS>`-style).  Emitted as
    /// Coq/Lean implicit arguments preceding the value parameters:
    /// `Theorem foo {S : Type} (s : S) : ...`.  Without this, theorems
    /// like MSFS §7.1 (which bind `<S: RichS>`) emit with `S` as a
    /// free variable, producing a foreign-tool rejection.
    #[serde(default)]
    pub generics: Vec<TheoremGeneric>,
    /// Whether the theorem has a proof body.  Statements without a
    /// proof body are treated as axioms (postulates) in the foreign
    /// tool; ones with a proof body get `sorry` / `Admitted.` as a
    /// placeholder until per-theorem proof-term export lands.
    pub has_proof_body: bool,
    /// `@verify(<strategy>)` annotation if present (records the
    /// declared verification level for the audit report).
    pub declared_strategy: Option<String>,
}

impl TheoremSpec {
    /// Populate `per_backend_proposition` for the standard backend
    /// set (Coq + Lean) by running the corresponding [`ExprRenderer`]
    /// over the supplied AST proposition.  Successful translations
    /// land in the map; fallbacks are recorded as absent entries
    /// (the per-format renderer then emits `Prop` with the original
    /// text in a comment).
    pub fn with_translated_proposition(mut self, proposition: &verum_ast::Expr) -> Self {
        for renderer in [
            Box::new(CoqExprRenderer::new()) as Box<dyn ExprRenderer>,
            Box::new(LeanExprRenderer::new()) as Box<dyn ExprRenderer>,
        ] {
            match renderer.render(proposition) {
                TranslatedExpr::Translated { text } => {
                    self.per_backend_proposition
                        .insert(renderer.id().to_string(), text);
                }
                TranslatedExpr::Fallback { .. } => {
                    // Absence is the signal for the per-format
                    // renderer to emit `Prop` with the original text
                    // in a comment.
                }
            }
        }
        self
    }

    /// Populate `params` by running the [`TypeRenderer`]s over each
    /// parameter's type (#141 / MSFS-L4.8).  Per-backend translation
    /// failures leave that backend's entry absent for the parameter;
    /// the renderer then emits a generic `Type` placeholder so the
    /// binding still exists at the foreign-tool level.
    ///
    /// `params` is a slice of `(parameter_name, parameter_type)` pairs
    /// extracted from the theorem's AST by the audit walker.  Names
    /// should already be sanitised to valid foreign-tool identifiers.
    pub fn with_translated_params(
        mut self,
        params: &[(String, &verum_ast::ty::Type)],
    ) -> Self {
        for (name, ty) in params {
            let mut per_backend_type_text: std::collections::BTreeMap<String, String> =
                std::collections::BTreeMap::new();
            for renderer in [
                Box::new(CoqTypeRenderer::new()) as Box<dyn TypeRenderer>,
                Box::new(LeanTypeRenderer::new()) as Box<dyn TypeRenderer>,
            ] {
                if let TranslatedType::Translated { text } = renderer.render(ty) {
                    per_backend_type_text.insert(renderer.id().to_string(), text);
                }
            }
            self.params.push(TheoremParam {
                name: name.clone(),
                per_backend_type_text,
            });
        }
        self
    }

    /// Populate `generics` from the theorem's generic-parameter list
    /// (#145 / MSFS-L4.11).  Each input is a `(name, bound_annotation)`
    /// pair extracted from the AST by the audit walker; the bound is
    /// already rendered into a single line like `"S : RichS"`.  Empty
    /// bound text means a bare generic (no bound).
    pub fn with_generics(mut self, generics: &[(String, String)]) -> Self {
        for (name, bound_annotation) in generics {
            self.generics.push(TheoremGeneric {
                name: name.clone(),
                bound_annotation: bound_annotation.clone(),
            });
        }
        self
    }
}

/// One backend's per-theorem render of [`TheoremSpec`].  Returned
/// by [`CorpusBackend::render_theorem`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedTheorem {
    /// Output filename (e.g., `theorem_5_1.v` for Coq, `Theorem51.lean`
    /// for Lean — the Lean filename rules differ from Coq).
    pub filename: String,
    /// File contents the foreign-tool checker will run on.
    pub content: String,
}

/// The per-format protocol.  One trait, multiple instances —
/// `CoqCorpusBackend` and `LeanCorpusBackend` ship in this module.
/// Adding Isabelle / Agda / Dedukti is a single new instance.
pub trait CorpusBackend {
    /// Stable identifier — `"coq"`, `"lean"`, etc.  Used as the
    /// `certificates/<id>/` subdirectory and in audit reports.
    fn id(&self) -> &'static str;
    /// File extension WITHOUT the leading dot.
    fn extension(&self) -> &'static str;
    /// Render a single theorem as a self-contained foreign-tool
    /// source file ready for `coqc` / `lean` invocation.
    fn render_theorem(&self, spec: &TheoremSpec) -> RenderedTheorem;
}

// =============================================================================
// CoqCorpusBackend
// =============================================================================

/// Coq backend for corpus theorems.  Emits per-theorem `.v` files
/// with `Axiom <name> : Prop.` for proof-less statements and
/// `Theorem <name> : Prop. Admitted.` for proof-bearing ones.
pub struct CoqCorpusBackend;

impl CoqCorpusBackend {
    /// Construct a fresh backend.
    pub fn new() -> Self {
        Self
    }
}

impl Default for CoqCorpusBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl CorpusBackend for CoqCorpusBackend {
    fn id(&self) -> &'static str {
        "coq"
    }

    fn extension(&self) -> &'static str {
        "v"
    }

    fn render_theorem(&self, spec: &TheoremSpec) -> RenderedTheorem {
        let filename = format!("{}.v", spec.name);
        let mut content = String::new();
        content.push_str(&format!(
            "(* Auto-generated by verum_kernel::soundness::corpus_export::CoqCorpusBackend *)\n\
             (* Source module: {} *)\n",
            spec.module_path,
        ));
        if let Some(strat) = &spec.declared_strategy {
            content.push_str(&format!("(* @verify({}) *)\n", strat));
        }
        content.push('\n');
        // Statement-level export — type structure varies by whether
        // the AST translator (#140 / MSFS-L4.7) successfully rendered
        // the proposition into Coq syntax:
        //
        //   * `per_backend_proposition["coq"]` is `Some(t)` → use `t`
        //     as the theorem's TYPE.  `coqc` validates the
        //     proposition's type structure as well as its name binding.
        //   * Translation fell back → use `Prop` and embed the
        //     original Verum-side text in a comment.  `coqc` validates
        //     only the name binding.
        let coq_type: &str = spec
            .per_backend_proposition
            .get("coq")
            .map(|s| s.as_str())
            .unwrap_or("Prop");
        content.push_str(&format!(
            "(* Proposition (Verum source): {} *)\n",
            sanitise_for_comment(&spec.proposition_text)
        ));
        let coq_generics = render_generics_for_backend(spec, "coq");
        let coq_params = render_params_for_backend(spec, "coq");
        if spec.has_proof_body {
            content.push_str(&format!(
                "Theorem {}{}{} : {}.\n\
                 Proof.\n  \
                   admit.\n\
                 Admitted.\n",
                spec.name, coq_generics, coq_params, coq_type,
            ));
        } else {
            content.push_str(&format!(
                "Axiom {}{}{} : {}.\n",
                spec.name, coq_generics, coq_params, coq_type
            ));
        }
        RenderedTheorem { filename, content }
    }
}

// =============================================================================
// LeanCorpusBackend
// =============================================================================

/// Lean 4 backend for corpus theorems.  Emits per-theorem `.lean`
/// files with `axiom <name> : Prop` for proof-less statements and
/// `theorem <name> : Prop := sorry` for proof-bearing ones.
pub struct LeanCorpusBackend;

impl LeanCorpusBackend {
    /// Construct a fresh backend.
    pub fn new() -> Self {
        Self
    }
}

impl Default for LeanCorpusBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl CorpusBackend for LeanCorpusBackend {
    fn id(&self) -> &'static str {
        "lean"
    }

    fn extension(&self) -> &'static str {
        "lean"
    }

    fn render_theorem(&self, spec: &TheoremSpec) -> RenderedTheorem {
        // Lean filenames conventionally use PascalCase; the audit walker
        // produces sanitised theorem names already, so we just append
        // the extension here and rely on file-system case-sensitivity.
        let filename = format!("{}.lean", spec.name);
        let mut content = String::new();
        content.push_str(&format!(
            "/-! Auto-generated by verum_kernel::soundness::corpus_export::LeanCorpusBackend\n\
             Source module: {} -/\n",
            spec.module_path,
        ));
        if let Some(strat) = &spec.declared_strategy {
            content.push_str(&format!("/-! @verify({}) -/\n", strat));
        }
        content.push('\n');
        content.push_str(&format!(
            "/-! Proposition (Verum source): {} -/\n",
            sanitise_for_comment(&spec.proposition_text),
        ));
        // Same translator-driven type-structure choice as the Coq
        // backend (#140 / MSFS-L4.7).
        let lean_type: &str = spec
            .per_backend_proposition
            .get("lean")
            .map(|s| s.as_str())
            .unwrap_or("Prop");
        let lean_generics = render_generics_for_backend(spec, "lean");
        let lean_params = render_params_for_backend(spec, "lean");
        if spec.has_proof_body {
            content.push_str(&format!(
                "theorem {}{}{} : {} := by sorry\n",
                spec.name, lean_generics, lean_params, lean_type,
            ));
        } else {
            content.push_str(&format!(
                "axiom {}{}{} : {}\n",
                spec.name, lean_generics, lean_params, lean_type
            ));
        }
        RenderedTheorem { filename, content }
    }
}

/// Sanitise a string for embedding in a foreign-tool comment block.
/// Strips characters that would close the comment delimiter
/// prematurely.  Sufficient because comments in both Coq (`(* … *)`)
/// and Lean (`/-! … -/`) only require the closing-delimiter avoidance.
fn sanitise_for_comment(text: &str) -> String {
    text.replace("*)", "* )").replace("-/", "- /")
}

/// Render the parameter-binding text for a given backend.  Produces
/// the form ` (n1 : T1) (n2 : T2) ...` (with a leading space)
/// suitable for embedding between the theorem name and the colon-
/// separator.  Empty when no params.
///
/// Type-translation failures fall back to a generic `Type` placeholder
/// (Coq) / `Type` (Lean) so the parameter binding still exists at the
/// foreign-tool level — the foreign tool then validates that the
/// proposition's free variables are at least DECLARED, even when
/// their types can't be translated faithfully.
fn render_params_for_backend(spec: &TheoremSpec, backend_id: &str) -> String {
    if spec.params.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for p in &spec.params {
        let ty_text = p
            .per_backend_type_text
            .get(backend_id)
            .map(|s| s.as_str())
            .unwrap_or("Type");
        out.push_str(&format!(" ({} : {})", p.name, ty_text));
    }
    out
}

/// Render the generic-parameter binding text for a given backend
/// (#145 / MSFS-L4.11).  Produces the form
/// ` {S : Type} {T : Type} ...` (with a leading space) suitable for
/// embedding between the theorem name and the value parameters.
/// Empty when no generics.
///
/// Both Coq and Lean accept the curly-brace implicit-argument form.
/// Bound annotations (e.g., `S : RichS`) are preserved as a per-
/// generic comment directly preceding the binding so foreign-tool
/// reviewers see the constraint without it gating compilation.  Once
/// the cross-format gate emits protocols as Coq Class / Lean class,
/// the bound can be lowered into a typeclass instance argument
/// (`{S : Type} [SRichS : RichS S]`).
fn render_generics_for_backend(spec: &TheoremSpec, _backend_id: &str) -> String {
    if spec.generics.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for g in &spec.generics {
        if !g.bound_annotation.is_empty() {
            // Inline the bound annotation as a comment so the foreign
            // tool's source still records the protocol bound.  Both
            // Coq and Lean treat `(*…*)` (Coq) / `/-…-/` (Lean) the
            // same way at parameter-binder positions; we use Coq's
            // form because the implicit-arg syntax `{...}` is
            // identical between backends and the comment delimiter
            // doesn't affect Lean's tokeniser inside `{}`.
            out.push_str(&format!(" (* bound: {} *)", g.bound_annotation));
        }
        out.push_str(&format!(" {{{} : Type}}", g.name));
    }
    out
}

// =============================================================================
// Trait dispatcher — used by the audit gate to enumerate backends
// =============================================================================

/// Box<dyn> for dynamic dispatch over the per-format backends.  The
/// audit gate iterates `[Box::new(CoqCorpusBackend), …]` and calls
/// each one's `render_theorem` over every walked corpus theorem.
pub fn all_corpus_backends() -> Vec<Box<dyn CorpusBackend>> {
    vec![
        Box::new(CoqCorpusBackend::new()),
        Box::new(LeanCorpusBackend::new()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_proven(name: &str, prop: &str) -> TheoremSpec {
        TheoremSpec {
            name: name.to_string(),
            module_path: "test_module".to_string(),
            proposition_text: prop.to_string(),
            per_backend_proposition: std::collections::BTreeMap::new(),
            params: Vec::new(),
            generics: Vec::new(),
            has_proof_body: true,
            declared_strategy: None,
        }
    }

    fn spec_axiom(name: &str, prop: &str) -> TheoremSpec {
        TheoremSpec {
            name: name.to_string(),
            module_path: "test_module".to_string(),
            proposition_text: prop.to_string(),
            per_backend_proposition: std::collections::BTreeMap::new(),
            params: Vec::new(),
            generics: Vec::new(),
            has_proof_body: false,
            declared_strategy: None,
        }
    }

    #[test]
    fn coq_emits_theorem_admitted_for_proof_body_decls() {
        let backend = CoqCorpusBackend::new();
        let rendered = backend.render_theorem(&spec_proven("my_thm", "x = x"));
        assert_eq!(rendered.filename, "my_thm.v");
        assert!(rendered.content.contains("Theorem my_thm : Prop."));
        assert!(rendered.content.contains("Admitted."));
        assert!(rendered.content.contains("(* Proposition (Verum source): x = x *)"));
    }

    #[test]
    fn coq_emits_axiom_for_proofless_decls() {
        let backend = CoqCorpusBackend::new();
        let rendered = backend.render_theorem(&spec_axiom("my_axiom", "true"));
        assert_eq!(rendered.filename, "my_axiom.v");
        assert!(rendered.content.contains("Axiom my_axiom : Prop."));
        assert!(!rendered.content.contains("Theorem"));
        assert!(!rendered.content.contains("Admitted."));
    }

    #[test]
    fn lean_emits_theorem_sorry_for_proof_body_decls() {
        let backend = LeanCorpusBackend::new();
        let rendered = backend.render_theorem(&spec_proven("my_thm", "x = x"));
        assert_eq!(rendered.filename, "my_thm.lean");
        assert!(rendered.content.contains("theorem my_thm : Prop := by sorry"));
    }

    #[test]
    fn lean_emits_axiom_for_proofless_decls() {
        let backend = LeanCorpusBackend::new();
        let rendered = backend.render_theorem(&spec_axiom("my_axiom", "true"));
        assert_eq!(rendered.filename, "my_axiom.lean");
        assert!(rendered.content.contains("axiom my_axiom : Prop"));
        assert!(!rendered.content.contains("theorem"));
        assert!(!rendered.content.contains("sorry"));
    }

    #[test]
    fn comment_sanitiser_escapes_coq_close_delimiter() {
        // "*)" in proposition would prematurely close the (* ... *) block
        // and corrupt the file.  Sanitiser must escape it.
        let backend = CoqCorpusBackend::new();
        let rendered = backend.render_theorem(&spec_axiom("evil", "*)"));
        assert!(!rendered.content.contains("(* Proposition (Verum source): *) *)"));
        assert!(rendered.content.contains("* )"));
    }

    #[test]
    fn comment_sanitiser_escapes_lean_close_delimiter() {
        let backend = LeanCorpusBackend::new();
        let rendered = backend.render_theorem(&spec_axiom("evil", "-/"));
        // The Lean comment is /-! ... -/; closing-delimiter "-/" must be
        // escaped to "- /" to avoid corrupting the file.
        assert!(rendered.content.contains("- /"));
    }

    #[test]
    fn declared_strategy_is_preserved_in_header() {
        let mut spec = spec_proven("annotated", "x = x");
        spec.declared_strategy = Some("formal".to_string());
        let coq_rendered = CoqCorpusBackend::new().render_theorem(&spec);
        assert!(coq_rendered.content.contains("(* @verify(formal) *)"));
        let lean_rendered = LeanCorpusBackend::new().render_theorem(&spec);
        assert!(lean_rendered.content.contains("/-! @verify(formal) -/"));
    }

    #[test]
    fn all_corpus_backends_returns_two_known_backends() {
        let backends = all_corpus_backends();
        let ids: Vec<&str> = backends.iter().map(|b| b.id()).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"coq"));
        assert!(ids.contains(&"lean"));
    }

    #[test]
    fn rendered_theorems_carry_module_path_in_header() {
        let mut spec = spec_proven("with_module", "true");
        spec.module_path = "theorems.msfs.05_afnt_alpha.theorem_5_1".to_string();
        let r = CoqCorpusBackend::new().render_theorem(&spec);
        assert!(r.content.contains("theorems.msfs.05_afnt_alpha.theorem_5_1"));
        let r = LeanCorpusBackend::new().render_theorem(&spec);
        assert!(r.content.contains("theorems.msfs.05_afnt_alpha.theorem_5_1"));
    }

    #[test]
    fn translator_lifts_coq_theorem_type_when_proposition_translates() {
        // When per_backend_proposition["coq"] is populated, the
        // emitted Coq file's `Theorem foo : <type>.` carries the
        // translated proposition instead of `Prop`.  Pre-fix the
        // type was always `Prop` and `coqc` only validated name
        // binding; post-fix it validates the proposition's TYPE
        // structure too.
        let mut spec = spec_proven("eq_thm", "n == 7");
        spec.per_backend_proposition.insert("coq".into(), "(n = 7)".into());
        spec.per_backend_proposition.insert("lean".into(), "(n = 7)".into());
        let coq = CoqCorpusBackend::new().render_theorem(&spec);
        assert!(
            coq.content.contains("Theorem eq_thm : (n = 7)."),
            "Coq output must use translated type; got:\n{}",
            coq.content,
        );
        let lean = LeanCorpusBackend::new().render_theorem(&spec);
        assert!(
            lean.content.contains("theorem eq_thm : (n = 7) := by sorry"),
            "Lean output must use translated type; got:\n{}",
            lean.content,
        );
    }

    #[test]
    fn translator_falls_back_to_prop_when_translation_absent() {
        // Untranslated proposition (no entry in per_backend_proposition)
        // → renderer falls back to `Prop` while preserving the
        // original Verum text in a comment.
        let spec = spec_proven("untranslated_thm", "complex match shape");
        let coq = CoqCorpusBackend::new().render_theorem(&spec);
        assert!(coq.content.contains("Theorem untranslated_thm : Prop."));
        assert!(coq.content.contains("complex match shape"));
        let lean = LeanCorpusBackend::new().render_theorem(&spec);
        assert!(lean.content.contains("theorem untranslated_thm : Prop := by sorry"));
        assert!(lean.content.contains("complex match shape"));
    }

    #[test]
    fn translator_lifts_axiom_form_too() {
        // The translation lift applies to proofless decls (axiom
        // form) the same way it applies to proof-bearing decls.
        let mut spec = spec_axiom("nat_zero_pos", "0 == 0");
        spec.per_backend_proposition.insert("coq".into(), "(0 = 0)".into());
        spec.per_backend_proposition.insert("lean".into(), "(0 = 0)".into());
        let coq = CoqCorpusBackend::new().render_theorem(&spec);
        assert!(coq.content.contains("Axiom nat_zero_pos : (0 = 0)."));
        let lean = LeanCorpusBackend::new().render_theorem(&spec);
        assert!(lean.content.contains("axiom nat_zero_pos : (0 = 0)"));
    }

    #[test]
    fn theorem_param_emission_with_int_parameter_in_coq() {
        // Theorem `foo(n: Int) ensures n == 7` → Coq emission must
        // declare `n : Z` before the colon-separator.
        let mut spec = spec_proven("eq_thm", "n == 7");
        spec.per_backend_proposition.insert("coq".into(), "(n = 7)".into());
        spec.params.push(TheoremParam {
            name: "n".to_string(),
            per_backend_type_text: {
                let mut m = std::collections::BTreeMap::new();
                m.insert("coq".to_string(), "Z".to_string());
                m
            },
        });
        let coq = CoqCorpusBackend::new().render_theorem(&spec);
        assert!(
            coq.content.contains("Theorem eq_thm (n : Z) : (n = 7)."),
            "Coq output must declare param before colon; got:\n{}",
            coq.content,
        );
    }

    #[test]
    fn theorem_param_emission_with_int_parameter_in_lean() {
        let mut spec = spec_proven("eq_thm", "n == 7");
        spec.per_backend_proposition.insert("lean".into(), "(n = 7)".into());
        spec.params.push(TheoremParam {
            name: "n".to_string(),
            per_backend_type_text: {
                let mut m = std::collections::BTreeMap::new();
                m.insert("lean".to_string(), "Int".to_string());
                m
            },
        });
        let lean = LeanCorpusBackend::new().render_theorem(&spec);
        assert!(
            lean.content.contains("theorem eq_thm (n : Int) : (n = 7) := by sorry"),
            "Lean output must declare param before colon; got:\n{}",
            lean.content,
        );
    }

    #[test]
    fn untranslated_param_type_falls_back_to_type_placeholder() {
        // Parameter whose type didn't translate (per_backend_type_text
        // for that backend is absent) → emission falls back to `Type`
        // so the binding still exists.  Foreign tool then validates
        // that the proposition's free variables are at least
        // DECLARED.  Pin both Coq and Lean independently — each
        // backend gets its own per_backend_proposition entry so the
        // test isolates param-fallback behavior from proposition
        // fallback.
        let mut spec = spec_proven("opaque_thm", "P x");
        spec.per_backend_proposition.insert("coq".into(), "(P x)".into());
        spec.per_backend_proposition.insert("lean".into(), "(P x)".into());
        spec.params.push(TheoremParam {
            name: "x".to_string(),
            per_backend_type_text: std::collections::BTreeMap::new(),
        });
        let coq = CoqCorpusBackend::new().render_theorem(&spec);
        assert!(
            coq.content.contains("Theorem opaque_thm (x : Type) : (P x)."),
            "Coq emission with untranslated param type must use `Type` placeholder; got:\n{}",
            coq.content,
        );
        let lean = LeanCorpusBackend::new().render_theorem(&spec);
        assert!(
            lean.content.contains("theorem opaque_thm (x : Type) : (P x) := by sorry"),
            "Lean emission with untranslated param type must use `Type` placeholder; got:\n{}",
            lean.content,
        );
    }

    #[test]
    fn multiple_params_emit_in_declaration_order() {
        let mut spec = spec_proven("multi_param_thm", "a == b");
        spec.per_backend_proposition.insert("coq".into(), "(a = b)".into());
        for (name, ty) in [("a", "Z"), ("b", "Z")] {
            spec.params.push(TheoremParam {
                name: name.to_string(),
                per_backend_type_text: {
                    let mut m = std::collections::BTreeMap::new();
                    m.insert("coq".to_string(), ty.to_string());
                    m
                },
            });
        }
        let coq = CoqCorpusBackend::new().render_theorem(&spec);
        assert!(coq.content.contains("Theorem multi_param_thm (a : Z) (b : Z) : (a = b)."));
    }

    #[test]
    fn params_with_translated_proposition_compose_correctly() {
        // The two with_* methods compose: with_translated_params
        // populates `params`, with_translated_proposition populates
        // `per_backend_proposition`.  Both end up in the emitted
        // theorem header.
        use verum_ast::expr::ExprKind;
        use verum_ast::ty::{Ident as TyIdent, Path as TyPath, Type, TypeKind};
        use verum_ast::Span;
        use verum_common::Heap;

        let lit_zero = verum_ast::Expr::new(
            ExprKind::Literal(verum_ast::Literal::int(0, Span::dummy())),
            Span::dummy(),
        );
        let path_n = verum_ast::Expr::new(
            ExprKind::Path(TyPath::single(TyIdent::new("n", Span::dummy()))),
            Span::dummy(),
        );
        let prop = verum_ast::Expr::new(
            ExprKind::Binary {
                op: verum_ast::BinOp::Eq,
                left: Heap::new(path_n),
                right: Heap::new(lit_zero),
            },
            Span::dummy(),
        );
        let int_ty = Type::new(TypeKind::Int, Span::dummy());
        let spec = spec_proven("zero_thm", "n == 0")
            .with_translated_params(&[("n".to_string(), &int_ty)])
            .with_translated_proposition(&prop);
        let coq = CoqCorpusBackend::new().render_theorem(&spec);
        assert!(
            coq.content.contains("Theorem zero_thm (n : Z) : (n = 0)."),
            "Coq emission must combine param + translated proposition; got:\n{}",
            coq.content,
        );
        let lean = LeanCorpusBackend::new().render_theorem(&spec);
        assert!(
            lean.content.contains("theorem zero_thm (n : Int) : (n = 0) := by sorry"),
            "Lean emission must combine param + translated proposition; got:\n{}",
            lean.content,
        );
    }

    #[test]
    fn with_translated_proposition_runs_both_backends() {
        // Round-trip: TheoremSpec::with_translated_proposition takes
        // an Expr and populates per_backend_proposition for every
        // standard backend (Coq + Lean).
        use verum_ast::expr::ExprKind;
        use verum_ast::ty::{Ident, Path};
        use verum_ast::Span;
        use verum_common::Heap;

        let lit_zero = verum_ast::Expr::new(
            ExprKind::Literal(verum_ast::Literal::int(0, Span::dummy())),
            Span::dummy(),
        );
        let path_n = verum_ast::Expr::new(
            ExprKind::Path(Path::single(Ident::new("n", Span::dummy()))),
            Span::dummy(),
        );
        let prop = verum_ast::Expr::new(
            ExprKind::Binary {
                op: verum_ast::BinOp::Eq,
                left: Heap::new(path_n),
                right: Heap::new(lit_zero),
            },
            Span::dummy(),
        );
        let spec = spec_proven("zero_thm", "n == 0")
            .with_translated_proposition(&prop);
        assert_eq!(
            spec.per_backend_proposition.get("coq").map(|s| s.as_str()),
            Some("(n = 0)"),
        );
        assert_eq!(
            spec.per_backend_proposition.get("lean").map(|s| s.as_str()),
            Some("(n = 0)"),
        );
    }

    // =========================================================================
    // Generic-parameter emission tests (#145 / MSFS-L4.11)
    // =========================================================================

    #[test]
    fn coq_emits_generic_implicit_arg() {
        // `<S>` (no bound) → ` {S : Type}` precedes value params.
        let spec = spec_proven("generic_thm", "true")
            .with_generics(&[("S".to_string(), String::new())]);
        let coq = CoqCorpusBackend::new().render_theorem(&spec);
        assert!(
            coq.content.contains("Theorem generic_thm {S : Type} : Prop."),
            "Coq emission must declare generic before colon; got:\n{}",
            coq.content,
        );
    }

    #[test]
    fn lean_emits_generic_implicit_arg() {
        let spec = spec_proven("generic_thm", "true")
            .with_generics(&[("S".to_string(), String::new())]);
        let lean = LeanCorpusBackend::new().render_theorem(&spec);
        assert!(
            lean.content.contains("theorem generic_thm {S : Type} : Prop := by sorry"),
            "Lean emission must declare generic before colon; got:\n{}",
            lean.content,
        );
    }

    #[test]
    fn generic_with_bound_preserves_bound_as_comment() {
        // `<S: RichS>` → `(* bound: S : RichS *) {S : Type}` — the
        // bound text is preserved as a comment so foreign-tool
        // reviewers see the protocol constraint without it gating
        // compilation.
        let spec = spec_proven("bounded_generic", "true")
            .with_generics(&[("S".to_string(), "S : RichS".to_string())]);
        let coq = CoqCorpusBackend::new().render_theorem(&spec);
        assert!(
            coq.content.contains("(* bound: S : RichS *) {S : Type}"),
            "Coq emission must preserve protocol bound as comment; got:\n{}",
            coq.content,
        );
    }

    #[test]
    fn generics_compose_with_value_params() {
        // The MSFS §7 shape: `theorem foo<S: RichS>(s: &S, c: &LAbsCandidate)`
        // → `Theorem foo (* bound: S : RichS *) {S : Type} (s : Type) (c : LAbsCandidate) : ...`.
        let mut spec = spec_proven("five_axis_thm", "false")
            .with_generics(&[("S".to_string(), "S : RichS".to_string())]);
        spec.per_backend_proposition.insert("coq".into(), "False".into());
        spec.params.push(TheoremParam {
            name: "s".to_string(),
            per_backend_type_text: std::collections::BTreeMap::new(),
        });
        spec.params.push(TheoremParam {
            name: "c".to_string(),
            per_backend_type_text: {
                let mut m = std::collections::BTreeMap::new();
                m.insert("coq".to_string(), "LAbsCandidate".to_string());
                m
            },
        });
        let coq = CoqCorpusBackend::new().render_theorem(&spec);
        assert!(
            coq.content.contains(
                "Theorem five_axis_thm (* bound: S : RichS *) {S : Type} (s : Type) (c : LAbsCandidate) : False."
            ),
            "Coq emission must compose generics with value params in order; got:\n{}",
            coq.content,
        );
    }

    #[test]
    fn empty_generics_emits_no_braces() {
        // No generics → no leading ` {...}` text.  Pin so that simple
        // theorems don't accidentally pick up empty-brace artifacts.
        let spec = spec_proven("simple_thm", "true");
        let coq = CoqCorpusBackend::new().render_theorem(&spec);
        assert!(
            coq.content.contains("Theorem simple_thm : Prop."),
            "Coq emission for no-generic theorem must not add braces; got:\n{}",
            coq.content,
        );
        assert!(!coq.content.contains("{"), "no braces expected when generics empty");
    }
}
