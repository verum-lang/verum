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
    AgdaExprRenderer, AgdaTypeRenderer, CoqExprRenderer, CoqTypeRenderer,
    DeduktiExprRenderer, DeduktiTypeRenderer, ExprRenderer, IsabelleExprRenderer,
    IsabelleTypeRenderer, LeanExprRenderer, LeanTypeRenderer, TranslatedExpr,
    TranslatedType, TypeRenderer,
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
    /// placeholder unless `per_backend_proof_tactic` carries a real
    /// translation (#153 — proof-term emission).
    pub has_proof_body: bool,
    /// **Per-backend translated proof body (#153 / Phase 2 of
    /// proof-term emission)**.  When a backend's entry is present,
    /// the corresponding [`CorpusBackend`] uses that text in place
    /// of the `admit` / `sorry` placeholder — `Proof. <text> Qed.`
    /// (Coq) / `theorem ... := <text>` (Lean).  Keys are backend
    /// ids (`"coq"` / `"lean"`); values are the renderer's output.
    /// Absent entries trigger the placeholder fallback so partial
    /// translation coverage is safe — backends that haven't been
    /// taught a particular proof-body shape silently degrade to
    /// admitted-with-comment instead of producing malformed output.
    ///
    /// **Why a per-backend map (rather than a single proof_text)**:
    /// Coq and Lean tactic syntax diverges enough that a single
    /// canonical form would over-constrain the translator
    /// (e.g. Coq `apply X.` vs. Lean `exact X` for term-mode-leaning
    /// dispatch).  The map keeps each backend independent and lets
    /// translators land iteratively without touching the wire
    /// format.
    #[serde(default)]
    pub per_backend_proof_tactic: std::collections::BTreeMap<String, String>,
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
            Box::new(AgdaExprRenderer::new()) as Box<dyn ExprRenderer>,
            Box::new(IsabelleExprRenderer::new()) as Box<dyn ExprRenderer>,
            Box::new(DeduktiExprRenderer::new()) as Box<dyn ExprRenderer>,
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
                Box::new(AgdaTypeRenderer::new()) as Box<dyn TypeRenderer>,
                Box::new(IsabelleTypeRenderer::new()) as Box<dyn TypeRenderer>,
                Box::new(DeduktiTypeRenderer::new()) as Box<dyn TypeRenderer>,
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

    /// **Populate `per_backend_proof_tactic` (#153 / Phase 2)** by
    /// running the [`super::proof_body_translate::ProofBodyRenderer`]s
    /// over the supplied AST proof body.  Successful translations
    /// land in the per-backend map; fallbacks leave the entry absent
    /// so the per-format renderer reverts to `Admitted.` / `:= by sorry`.
    ///
    /// **Why a builder method (vs. inline construction)**: every
    /// audit walker that builds a TheoremSpec follows the same
    /// translate-the-translatable-pieces pattern; centralising the
    /// translator dispatch here keeps the shape consistent across
    /// `audit_cross_format_roundtrip` (the load-bearing site) and
    /// any future site that builds specs directly.
    pub fn with_translated_proof_body(mut self, body: &verum_ast::decl::ProofBody) -> Self {
        use super::proof_body_translate::{
            AgdaProofBodyRenderer, CoqProofBodyRenderer, DeduktiProofBodyRenderer,
            IsabelleProofBodyRenderer, LeanProofBodyRenderer, ProofBodyRenderer,
            TranslatedProofBody,
        };
        for renderer in [
            Box::new(CoqProofBodyRenderer::new()) as Box<dyn ProofBodyRenderer>,
            Box::new(LeanProofBodyRenderer::new()) as Box<dyn ProofBodyRenderer>,
            Box::new(AgdaProofBodyRenderer::new()) as Box<dyn ProofBodyRenderer>,
            Box::new(IsabelleProofBodyRenderer::new()) as Box<dyn ProofBodyRenderer>,
            Box::new(DeduktiProofBodyRenderer::new()) as Box<dyn ProofBodyRenderer>,
        ] {
            if let TranslatedProofBody::Translated { text } = renderer.render(body) {
                self.per_backend_proof_tactic
                    .insert(renderer.id().to_string(), text);
            }
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

    /// Canonical foreign-system handle.  Default implementation
    /// resolves [`id`](Self::id) via [`ForeignSystem::from_name`];
    /// override when the backend's ID doesn't match the canonical
    /// alias set.  Lets consumers dispatch by typed enum rather
    /// than string comparison.
    fn foreign_system(&self) -> Option<crate::foreign_system::ForeignSystem> {
        crate::foreign_system::ForeignSystem::from_name(self.id())
    }
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
        let signature = compute_provenance_signature(spec, "coq");
        let mut content = String::new();
        // Provenance signature header (#174) — third-party reviewers
        // recompute via `verum verify-signature <file>` to confirm the
        // file came from the named corpus state.
        content.push_str(&format!("(* verum_signature: {} *)\n", signature));
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
            // #153 / Phase 2: when a translated proof tactic is
            // available for this backend, emit it in place of
            // `admit.` + `Admitted.`  Closes the proof normally
            // with `Qed.` (constructive) instead of `Admitted.`
            // (un-checked).  Coq accepts multi-line tactic bodies;
            // we wrap the translation in a `Proof.`/`Qed.` block
            // and prefix each line with two-space indent so the
            // output stays readable.
            match spec.per_backend_proof_tactic.get("coq") {
                Some(tactic) if !tactic.trim().is_empty() => {
                    content.push_str(&format!(
                        "Theorem {}{}{} : {}.\nProof.\n",
                        spec.name, coq_generics, coq_params, coq_type,
                    ));
                    for line in tactic.lines() {
                        content.push_str("  ");
                        content.push_str(line);
                        content.push('\n');
                    }
                    content.push_str("Qed.\n");
                }
                _ => {
                    content.push_str(&format!(
                        "Theorem {}{}{} : {}.\n\
                         Proof.\n  \
                           admit.\n\
                         Admitted.\n",
                        spec.name, coq_generics, coq_params, coq_type,
                    ));
                }
            }
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
// Provenance signatures (#174 / certificate-bearing artifacts)
// =============================================================================

/// Compute a stable provenance signature for an emitted theorem file.
///
/// The signature is a BLAKE3 hash over the canonical source-state
/// fingerprint: `(kernel_version || backend_id || module_path ||
/// theorem_name || proposition_text || has_proof_body || declared_strategy)`.
/// Reviewers can independently recompute the signature from the
/// `TheoremSpec` they have on hand and verify that the emitted file
/// came from EXACTLY the corpus state Verum claims.
///
/// **The trust shift**: a third-party reviewer pulls the published
/// `theorem_5_1.v` file out of MSFS supplementary material, runs
/// `verum verify-signature theorem_5_1.v`, and gets a binary verdict
/// — the file is/isn't a faithful emission of the named theorem
/// against the named kernel version.  No need to re-run the whole
/// pipeline to verify provenance.
///
/// **Output format**: `<kernel_version>:<32-byte-hex-blake3>` — the
/// kernel version is included so future Verum versions can refuse
/// to verify signatures from incompatible kernel revisions.
pub fn compute_provenance_signature(
    spec: &TheoremSpec,
    backend_id: &str,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(KERNEL_VERSION.as_bytes());
    hasher.update(b"\x00");
    hasher.update(backend_id.as_bytes());
    hasher.update(b"\x00");
    hasher.update(spec.module_path.as_bytes());
    hasher.update(b"\x00");
    hasher.update(spec.name.as_bytes());
    hasher.update(b"\x00");
    hasher.update(spec.proposition_text.as_bytes());
    hasher.update(b"\x00");
    hasher.update(if spec.has_proof_body { b"T" } else { b"F" });
    hasher.update(b"\x00");
    // #153: include this backend's translated proof tactic in the
    // signature.  When the tactic changes (translator improvement,
    // refactor, soundness fix), the signature flips — third-party
    // reviewers see a hash mismatch and know the proof body has
    // shifted, not just the proposition.
    if let Some(tactic) = spec.per_backend_proof_tactic.get(backend_id) {
        hasher.update(tactic.as_bytes());
    }
    hasher.update(b"\x00");
    if let Some(s) = &spec.declared_strategy {
        hasher.update(s.as_bytes());
    }
    let hash = hasher.finalize();
    format!("{}:{}", KERNEL_VERSION, hash.to_hex())
}

/// Verum kernel version embedded in every provenance signature.
/// Bumping this value invalidates every prior signature; reviewers
/// then know they're verifying against a different kernel revision.
pub const KERNEL_VERSION: &str = env!("CARGO_PKG_VERSION");

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
        let signature = compute_provenance_signature(spec, "lean");
        let mut content = String::new();
        // Provenance signature header (#174).
        content.push_str(&format!("/-! verum_signature: {} -/\n", signature));
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
            // #153 / Phase 2: when a translated proof tactic is
            // available for this backend, emit it in place of
            // `:= by sorry`.  Lean 4 accepts both term-mode
            // (`:= <expr>`) and tactic-mode (`:= by <tactics>`)
            // proofs; the translator decides which form to emit
            // by prefixing the text with `by ` or not.  We pass
            // the translation through verbatim — translators are
            // responsible for emitting the leading `by ` themselves
            // when their output is a tactic block.
            match spec.per_backend_proof_tactic.get("lean") {
                Some(tactic) if !tactic.trim().is_empty() => {
                    content.push_str(&format!(
                        "theorem {}{}{} : {} := {}\n",
                        spec.name, lean_generics, lean_params, lean_type, tactic,
                    ));
                }
                _ => {
                    content.push_str(&format!(
                        "theorem {}{}{} : {} := by sorry\n",
                        spec.name, lean_generics, lean_params, lean_type,
                    ));
                }
            }
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
        Box::new(AgdaCorpusBackend::new()),
        Box::new(IsabelleCorpusBackend::new()),
        Box::new(DeduktiCorpusBackend::new()),
    ]
}

// =============================================================================
// AgdaCorpusBackend (#156 — third backend)
// =============================================================================

/// Agda backend for corpus theorems.  Emits per-theorem `.agda`
/// files using `postulate <name> : <type>` for axioms / proof-less
/// theorems and `<name> : <type>` followed by `<name> = <expr>` for
/// theorems whose proof body translated cleanly.
///
/// **Architectural significance**: Agda's MLTT foundation is the
/// only one in the multi-kernel set without UIP or impredicative
/// hierarchies — the corpus-theorem cross-check via Agda
/// independently exercises the constructive fragment of every
/// emitted theorem.  Combined with Coq (CIC + UIP) and Lean 4
/// (CIC-derived), the three-backend cross-validation covers two
/// distinct foundation families (MLTT + CIC).
pub struct AgdaCorpusBackend;

impl AgdaCorpusBackend {
    /// Construct a fresh backend.
    pub fn new() -> Self {
        Self
    }
}

impl Default for AgdaCorpusBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl CorpusBackend for AgdaCorpusBackend {
    fn id(&self) -> &'static str {
        "agda"
    }

    fn extension(&self) -> &'static str {
        "agda"
    }

    fn render_theorem(&self, spec: &TheoremSpec) -> RenderedTheorem {
        let filename = format!("{}.agda", spec.name);
        let signature = compute_provenance_signature(spec, "agda");
        let mut content = String::new();
        // Provenance signature header (#174).  Agda single-line
        // comment is `--`; per-line is the safest form (block
        // comments `{- ... -}` nest, which is fine, but `--` is
        // simpler and the convention for one-liners).
        content.push_str(&format!("-- verum_signature: {}\n", signature));
        content.push_str(&format!(
            "-- Auto-generated by verum_kernel::soundness::corpus_export::AgdaCorpusBackend\n\
             -- Source module: {}\n",
            spec.module_path,
        ));
        if let Some(strat) = &spec.declared_strategy {
            content.push_str(&format!("-- @verify({})\n", strat));
        }
        content.push('\n');
        content.push_str(&format!(
            "-- Proposition (Verum source): {}\n",
            sanitise_for_agda_line_comment(&spec.proposition_text),
        ));

        let agda_type: &str = spec
            .per_backend_proposition
            .get("agda")
            .map(|s| s.as_str())
            .unwrap_or("Set");
        let agda_generics = render_generics_for_backend(spec, "agda");
        let agda_params = render_params_for_backend(spec, "agda");

        if spec.has_proof_body {
            // #153 / Phase 2: when a translated proof tactic is
            // available for Agda, emit it as `<name> = <expr>`.
            // Otherwise fall back to `postulate` — Agda's idiomatic
            // form for unproven theorems (no `Admitted.` keyword
            // exists; postulate is the canonical replacement).
            match spec.per_backend_proof_tactic.get("agda") {
                Some(tactic) if !tactic.trim().is_empty() => {
                    content.push_str(&format!(
                        "{}{}{} : {}\n{}{}{} = {}\n",
                        spec.name,
                        agda_generics,
                        agda_params,
                        agda_type,
                        spec.name,
                        agda_generics,
                        agda_params,
                        tactic.trim(),
                    ));
                }
                _ => {
                    content.push_str(&format!(
                        "postulate\n  {}{}{} : {}\n",
                        spec.name, agda_generics, agda_params, agda_type,
                    ));
                }
            }
        } else {
            // Pure axiom — postulate.
            content.push_str(&format!(
                "postulate\n  {}{}{} : {}\n",
                spec.name, agda_generics, agda_params, agda_type,
            ));
        }
        RenderedTheorem { filename, content }
    }
}

/// Sanitise text for embedding in an Agda single-line comment.
/// Replaces newlines with spaces so the comment doesn't accidentally
/// terminate before its content ends.  Block-comment delimiters
/// `{-` / `-}` aren't relevant for line comments.
fn sanitise_for_agda_line_comment(text: &str) -> String {
    text.replace('\n', " ").replace('\r', " ")
}

// =============================================================================
// DeduktiCorpusBackend (#156 — fifth backend, closes the matrix)
// =============================================================================

/// Dedukti backend for corpus theorems.  Emits per-theorem `.dk`
/// files using the λΠ-modulo declaration syntax: `name : type.`
/// for axioms, `def name : type := body.` for definitions / proven
/// theorems with a translation.
///
/// **Architectural significance**: Dedukti is a **logical
/// framework** — it embeds many proof systems uniformly via
/// rewriting.  Adding it to the cross-format gate provides a
/// meta-validation layer: artefacts emitted from the four upstream
/// backends (Coq + Lean + Agda + Isabelle) all admit
/// Dedukti-translation pipelines (Coq via Coqine, Agda via
/// agda2dedukti, Lean via the Lean-to-Dedukti exporter), so
/// emitting directly to Dedukti complements the upstream
/// translations with a Verum-side independent path.
pub struct DeduktiCorpusBackend;

impl DeduktiCorpusBackend {
    /// Construct a fresh backend.
    pub fn new() -> Self {
        Self
    }
}

impl Default for DeduktiCorpusBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl CorpusBackend for DeduktiCorpusBackend {
    fn id(&self) -> &'static str {
        "dedukti"
    }

    fn extension(&self) -> &'static str {
        "dk"
    }

    fn render_theorem(&self, spec: &TheoremSpec) -> RenderedTheorem {
        let filename = format!("{}.dk", spec.name);
        let signature = compute_provenance_signature(spec, "dedukti");
        let mut content = String::new();
        // Provenance signature header.  Dedukti uses `(; ... ;)`
        // for block comments; line-by-line on `(;` is the cleanest
        // emission shape.
        content.push_str(&format!("(; verum_signature: {} ;)\n", signature));
        content.push_str(&format!(
            "(; Auto-generated by verum_kernel::soundness::corpus_export::DeduktiCorpusBackend ;)\n\
             (; Source module: {} ;)\n",
            spec.module_path,
        ));
        if let Some(strat) = &spec.declared_strategy {
            content.push_str(&format!("(; @verify({}) ;)\n", strat));
        }
        content.push('\n');
        content.push_str(&format!(
            "(; Proposition (Verum source): {} ;)\n",
            sanitise_for_dedukti_comment(&spec.proposition_text),
        ));

        let dk_type: &str = spec
            .per_backend_proposition
            .get("dedukti")
            .map(|s| s.as_str())
            .unwrap_or("Type");
        let dk_generics = render_generics_for_backend(spec, "dedukti");
        let dk_params = render_params_for_backend(spec, "dedukti");

        if spec.has_proof_body {
            // Translated proof — `def name : type := body.` form.
            // Dedukti's `def` keyword introduces a definitional
            // equality for the symbol, which is the cleanest match
            // for "this theorem has a constructive proof".
            match spec.per_backend_proof_tactic.get("dedukti") {
                Some(tactic) if !tactic.trim().is_empty() => {
                    content.push_str(&format!(
                        "def {}{}{} : {} := {}.\n",
                        spec.name,
                        dk_generics,
                        dk_params,
                        dk_type,
                        tactic.trim(),
                    ));
                }
                _ => {
                    // Untranslated proof body — emit as a constant
                    // declaration (equivalent to `Admitted.`/`sorry`
                    // / `postulate` in upstream backends).
                    content.push_str(&format!(
                        "{}{}{} : {}.\n",
                        spec.name, dk_generics, dk_params, dk_type,
                    ));
                }
            }
        } else {
            // Pure axiom — bare declaration.
            content.push_str(&format!(
                "{}{}{} : {}.\n",
                spec.name, dk_generics, dk_params, dk_type,
            ));
        }
        RenderedTheorem { filename, content }
    }
}

/// Sanitise text for embedding in a Dedukti `(; ... ;)` block
/// comment.  Replaces `;)` with `; )` to prevent the comment from
/// terminating prematurely, and `(;` with `( ;` to prevent
/// accidentally opening a nested block (Dedukti comments don't
/// nest in older parsers).
fn sanitise_for_dedukti_comment(text: &str) -> String {
    text.replace(";)", "; )").replace("(;", "( ;")
}

// =============================================================================
// IsabelleCorpusBackend (#156 — fourth backend)
// =============================================================================

/// Isabelle/HOL backend for corpus theorems.  Emits per-theorem
/// `.thy` files structured around Isabelle's session-theory model:
/// every theorem becomes one self-contained theory with the
/// statement and proof.
///
/// **Architectural significance**: Isabelle/HOL is classical
/// higher-order logic — neither CIC (Coq + Lean) nor MLTT (Agda).
/// Adding Isabelle gives the cross-format gate three distinct
/// foundation families on the verifier side: classical HOL,
/// constructive MLTT, and CIC.  A corpus theorem that re-checks
/// across all three is foundation-robust in a way no single
/// backend can attest.
pub struct IsabelleCorpusBackend;

impl IsabelleCorpusBackend {
    /// Construct a fresh backend.
    pub fn new() -> Self {
        Self
    }
}

impl Default for IsabelleCorpusBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl CorpusBackend for IsabelleCorpusBackend {
    fn id(&self) -> &'static str {
        "isabelle"
    }

    fn extension(&self) -> &'static str {
        "thy"
    }

    fn render_theorem(&self, spec: &TheoremSpec) -> RenderedTheorem {
        let filename = format!("{}.thy", spec.name);
        let signature = compute_provenance_signature(spec, "isabelle");
        let mut content = String::new();
        // Provenance signature header — Isabelle uses `(* ... *)`
        // block comments (same as Coq).
        content.push_str(&format!("(* verum_signature: {} *)\n", signature));
        content.push_str(&format!(
            "(* Auto-generated by verum_kernel::soundness::corpus_export::IsabelleCorpusBackend *)\n\
             (* Source module: {} *)\n",
            spec.module_path,
        ));
        if let Some(strat) = &spec.declared_strategy {
            content.push_str(&format!("(* @verify({}) *)\n", strat));
        }
        content.push('\n');

        // Isabelle requires every theorem to live inside a theory
        // block — `theory X imports Main begin ... end`.  The theory
        // name must match the filename stem.
        content.push_str(&format!("theory {} imports Main begin\n\n", spec.name));
        content.push_str(&format!(
            "(* Proposition (Verum source): {} *)\n",
            sanitise_for_comment(&spec.proposition_text),
        ));

        let isa_type: &str = spec
            .per_backend_proposition
            .get("isabelle")
            .map(|s| s.as_str())
            .unwrap_or("True");
        let isa_generics = render_generics_for_backend(spec, "isabelle");
        let isa_params = render_params_for_backend(spec, "isabelle");

        if spec.has_proof_body {
            // #153 / Phase 2: when a translated proof body is
            // available, emit it as the Isabelle `proof` block.
            // Otherwise fall back to `sorry` — Isabelle's
            // built-in admitted form (analogous to Lean's `sorry`).
            match spec.per_backend_proof_tactic.get("isabelle") {
                Some(tactic) if !tactic.trim().is_empty() => {
                    content.push_str(&format!(
                        "lemma {}{}{} : \"{}\"\n",
                        spec.name, isa_generics, isa_params, isa_type,
                    ));
                    content.push_str("  ");
                    content.push_str(tactic.trim());
                    content.push('\n');
                }
                _ => {
                    content.push_str(&format!(
                        "lemma {}{}{} : \"{}\"\n  sorry\n",
                        spec.name, isa_generics, isa_params, isa_type,
                    ));
                }
            }
        } else {
            // Pure axiom — Isabelle keyword.
            content.push_str(&format!(
                "axiomatization where\n  {}{}{} : \"{}\"\n",
                spec.name, isa_generics, isa_params, isa_type,
            ));
        }
        content.push_str("\nend\n");

        RenderedTheorem { filename, content }
    }
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
            per_backend_proof_tactic: std::collections::BTreeMap::new(),
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
            per_backend_proof_tactic: std::collections::BTreeMap::new(),
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

    // -------------------------------------------------------------
    // #153 — per_backend_proof_tactic: real proof-body emission
    // -------------------------------------------------------------

    #[test]
    fn coq_emits_proof_tactic_when_present() {
        let backend = CoqCorpusBackend::new();
        let mut spec = spec_proven("delegated_thm", "P");
        spec.per_backend_proof_tactic
            .insert("coq".to_string(), "apply other_thm.".to_string());
        let rendered = backend.render_theorem(&spec);
        assert!(
            rendered.content.contains("Proof.\n  apply other_thm.\nQed."),
            "tactic must replace `admit. Admitted.`; got:\n{}",
            rendered.content,
        );
        assert!(
            !rendered.content.contains("Admitted."),
            "Admitted must not appear when a proof tactic is present",
        );
    }

    #[test]
    fn lean_emits_proof_tactic_when_present() {
        let backend = LeanCorpusBackend::new();
        let mut spec = spec_proven("delegated_thm", "P");
        spec.per_backend_proof_tactic
            .insert("lean".to_string(), "by apply other_thm".to_string());
        let rendered = backend.render_theorem(&spec);
        assert!(
            rendered
                .content
                .contains("theorem delegated_thm : Prop := by apply other_thm"),
            "tactic must replace `:= by sorry`; got:\n{}",
            rendered.content,
        );
        assert!(
            !rendered.content.contains("sorry"),
            "sorry must not appear when a proof tactic is present",
        );
    }

    #[test]
    fn coq_falls_back_to_admitted_when_tactic_missing() {
        // Per-backend independence: even if Lean has a translation,
        // Coq without one falls back to Admitted.
        let backend = CoqCorpusBackend::new();
        let mut spec = spec_proven("partial_thm", "P");
        spec.per_backend_proof_tactic
            .insert("lean".to_string(), "by exact x".to_string());
        let rendered = backend.render_theorem(&spec);
        assert!(rendered.content.contains("Admitted."));
    }

    #[test]
    fn lean_falls_back_to_sorry_when_tactic_missing() {
        let backend = LeanCorpusBackend::new();
        let mut spec = spec_proven("partial_thm", "P");
        spec.per_backend_proof_tactic
            .insert("coq".to_string(), "apply x.".to_string());
        let rendered = backend.render_theorem(&spec);
        assert!(rendered.content.contains(":= by sorry"));
    }

    #[test]
    fn empty_proof_tactic_is_treated_as_missing() {
        // A whitespace-only tactic shouldn't be emitted as a proof.
        // Whitespace-only is the same as absent: fall back to admitted.
        let backend = CoqCorpusBackend::new();
        let mut spec = spec_proven("whitespace_thm", "P");
        spec.per_backend_proof_tactic
            .insert("coq".to_string(), "   \n  \t".to_string());
        let rendered = backend.render_theorem(&spec);
        assert!(rendered.content.contains("Admitted."));
    }

    #[test]
    fn coq_proof_tactic_supports_multiline_body() {
        let backend = CoqCorpusBackend::new();
        let mut spec = spec_proven("multistep_thm", "P");
        spec.per_backend_proof_tactic.insert(
            "coq".to_string(),
            "intros n.\napply lemma_1.\nexact H.".to_string(),
        );
        let rendered = backend.render_theorem(&spec);
        assert!(rendered.content.contains("  intros n."));
        assert!(rendered.content.contains("  apply lemma_1."));
        assert!(rendered.content.contains("  exact H."));
        assert!(rendered.content.ends_with("Qed.\n"));
    }

    #[test]
    fn provenance_signature_changes_when_proof_tactic_changes() {
        // The signature (#174) must change when the proof body
        // changes — otherwise reviewers can't detect proof-body
        // drift between releases.
        let mut spec_a = spec_proven("thm", "P");
        spec_a
            .per_backend_proof_tactic
            .insert("coq".to_string(), "apply v1.".to_string());
        let mut spec_b = spec_proven("thm", "P");
        spec_b
            .per_backend_proof_tactic
            .insert("coq".to_string(), "apply v2.".to_string());
        let sig_a = compute_provenance_signature(&spec_a, "coq");
        let sig_b = compute_provenance_signature(&spec_b, "coq");
        assert_ne!(
            sig_a, sig_b,
            "different proof tactics must produce different signatures",
        );
    }

    #[test]
    fn provenance_signature_independent_per_backend() {
        // The signature for backend X depends on backend X's tactic
        // only — not the union of all tactics.  Otherwise editing
        // Lean's translation would break the Coq signature.
        let mut spec = spec_proven("thm", "P");
        spec.per_backend_proof_tactic
            .insert("coq".to_string(), "apply x.".to_string());
        spec.per_backend_proof_tactic
            .insert("lean".to_string(), "by apply x".to_string());
        let coq_sig_with_lean = compute_provenance_signature(&spec, "coq");

        spec.per_backend_proof_tactic
            .insert("lean".to_string(), "by apply different_x".to_string());
        let coq_sig_after_lean_change = compute_provenance_signature(&spec, "coq");

        assert_eq!(
            coq_sig_with_lean, coq_sig_after_lean_change,
            "Coq signature must not depend on Lean's tactic",
        );
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
    fn all_corpus_backends_returns_five_known_backends() {
        // Closes #156 declared backend matrix:
        //   Coq + Lean (CIC) + Agda (MLTT) + Isabelle (HOL) +
        //   Dedukti (λΠ-modulo, logical framework).
        let backends = all_corpus_backends();
        let ids: Vec<&str> = backends.iter().map(|b| b.id()).collect();
        assert_eq!(ids.len(), 5);
        for required in ["coq", "lean", "agda", "isabelle", "dedukti"] {
            assert!(
                ids.contains(&required),
                "default registry must include `{}`",
                required,
            );
        }
    }

    #[test]
    fn dedukti_backend_emits_bare_declaration_for_axioms() {
        let backend = DeduktiCorpusBackend::new();
        let r = backend.render_theorem(&spec_axiom("ax_thm", "Type"));
        assert_eq!(r.filename, "ax_thm.dk");
        assert!(
            r.content.contains("ax_thm"),
            "axiom must be emitted by name",
        );
        assert!(r.content.contains(" : "));
        // Dedukti uses `(; ;)` block comments.
        assert!(r.content.contains("(; verum_signature:"));
        assert!(!r.content.contains("(*"));
    }

    #[test]
    fn dedukti_backend_emits_def_for_translated_proof() {
        let backend = DeduktiCorpusBackend::new();
        let mut spec = spec_proven("backed_thm", "P");
        spec.per_backend_proof_tactic
            .insert("dedukti".to_string(), "lemma_x".to_string());
        let r = backend.render_theorem(&spec);
        assert!(
            r.content.contains("def backed_thm"),
            "Dedukti uses `def` for proven theorems with body; got:\n{}",
            r.content,
        );
        assert!(r.content.contains(":= lemma_x."));
    }

    #[test]
    fn dedukti_backend_falls_back_to_declaration_without_translation() {
        let backend = DeduktiCorpusBackend::new();
        let r = backend.render_theorem(&spec_proven("bare_thm", "P"));
        // No translation → emits as bare declaration (axiom form).
        assert!(!r.content.contains("def "));
        assert!(r.content.contains("bare_thm "));
    }

    #[test]
    fn dedukti_signature_is_distinct_from_other_backends() {
        let spec = spec_axiom("thm", "P");
        let coq = compute_provenance_signature(&spec, "coq");
        let lean = compute_provenance_signature(&spec, "lean");
        let agda = compute_provenance_signature(&spec, "agda");
        let isa = compute_provenance_signature(&spec, "isabelle");
        let dk = compute_provenance_signature(&spec, "dedukti");
        for other in [&coq, &lean, &agda, &isa] {
            assert_ne!(*other, dk);
        }
    }

    #[test]
    fn isabelle_backend_emits_axiomatization_for_axioms() {
        let backend = IsabelleCorpusBackend::new();
        let r = backend.render_theorem(&spec_axiom("ax_thm", "True"));
        assert_eq!(r.filename, "ax_thm.thy");
        assert!(r.content.contains("theory ax_thm imports Main begin"));
        assert!(r.content.contains("axiomatization where"));
        assert!(r.content.contains("ax_thm : "));
        assert!(r.content.ends_with("\nend\n"));
    }

    #[test]
    fn isabelle_backend_emits_lemma_with_sorry_when_no_translation() {
        let backend = IsabelleCorpusBackend::new();
        let r = backend.render_theorem(&spec_proven("my_thm", "True"));
        assert!(r.content.contains("lemma my_thm"));
        assert!(
            r.content.contains("sorry"),
            "Isabelle uses `sorry` for admitted lemmas — fallback shape",
        );
    }

    #[test]
    fn isabelle_backend_emits_real_proof_when_translation_present() {
        let backend = IsabelleCorpusBackend::new();
        let mut spec = spec_proven("backed_thm", "P");
        spec.per_backend_proof_tactic
            .insert("isabelle".to_string(), "by (rule lemma_x)".to_string());
        let r = backend.render_theorem(&spec);
        assert!(r.content.contains("lemma backed_thm"));
        assert!(r.content.contains("by (rule lemma_x)"));
        assert!(
            !r.content.contains("sorry"),
            "sorry must not appear when a translation is present",
        );
    }

    #[test]
    fn isabelle_signature_is_distinct_from_other_backends() {
        let spec = spec_axiom("thm", "P");
        let coq = compute_provenance_signature(&spec, "coq");
        let lean = compute_provenance_signature(&spec, "lean");
        let agda = compute_provenance_signature(&spec, "agda");
        let isa = compute_provenance_signature(&spec, "isabelle");
        for other in [&coq, &lean, &agda] {
            assert_ne!(*other, isa);
        }
    }

    #[test]
    fn agda_backend_emits_postulate_for_axioms() {
        let backend = AgdaCorpusBackend::new();
        let r = backend.render_theorem(&spec_axiom("my_axiom", "true"));
        assert_eq!(r.filename, "my_axiom.agda");
        assert!(r.content.contains("postulate"));
        assert!(r.content.contains("my_axiom : "));
    }

    #[test]
    fn agda_backend_emits_postulate_for_proof_body_decls_without_translation() {
        let backend = AgdaCorpusBackend::new();
        let r = backend.render_theorem(&spec_proven("my_thm", "x ≡ x"));
        assert_eq!(r.filename, "my_thm.agda");
        assert!(
            r.content.contains("postulate"),
            "Agda has no Admitted; falls back to postulate when no translation",
        );
    }

    #[test]
    fn agda_backend_emits_definition_when_translation_present() {
        let backend = AgdaCorpusBackend::new();
        let mut spec = spec_proven("delegated_thm", "P");
        spec.per_backend_proof_tactic
            .insert("agda".to_string(), "lemma_x".to_string());
        let r = backend.render_theorem(&spec);
        // Both the type signature and the definition body must appear.
        assert!(
            r.content.contains("delegated_thm : "),
            "Agda type signature must be emitted",
        );
        assert!(
            r.content.contains("delegated_thm = lemma_x"),
            "Agda definition body must be emitted; got:\n{}",
            r.content,
        );
        assert!(
            !r.content.contains("postulate"),
            "postulate must not appear when a translation is present",
        );
    }

    #[test]
    fn agda_backend_uses_line_comments() {
        let backend = AgdaCorpusBackend::new();
        let r = backend.render_theorem(&spec_axiom("commented_thm", "P"));
        assert!(
            r.content.contains("-- verum_signature:"),
            "Agda uses single-line `--` comments for the provenance header",
        );
        assert!(!r.content.contains("(*"));
        assert!(!r.content.contains("/-!"));
    }

    #[test]
    fn agda_signature_is_distinct_from_coq_and_lean() {
        // Per-backend signatures must NOT collide — otherwise
        // signature verification couldn't distinguish the three
        // emitted artefacts.
        let spec = spec_axiom("thm", "P");
        let coq = compute_provenance_signature(&spec, "coq");
        let lean = compute_provenance_signature(&spec, "lean");
        let agda = compute_provenance_signature(&spec, "agda");
        assert_ne!(coq, lean);
        assert_ne!(lean, agda);
        assert_ne!(coq, agda);
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
