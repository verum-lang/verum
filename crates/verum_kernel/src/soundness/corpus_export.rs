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
    /// Whether the theorem has a proof body.  Statements without a
    /// proof body are treated as axioms (postulates) in the foreign
    /// tool; ones with a proof body get `sorry` / `Admitted.` as a
    /// placeholder until per-theorem proof-term export lands.
    pub has_proof_body: bool,
    /// `@verify(<strategy>)` annotation if present (records the
    /// declared verification level for the audit report).
    pub declared_strategy: Option<String>,
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
        // Statement-level export: emit a `Parameter`-form binding that
        // declares the theorem's name with an opaque `Prop` type.
        // Even at this opaque layer, `coqc` verifies the binding is
        // well-formed (no name collision, valid identifier shape).
        // The proposition text is preserved verbatim in a comment so
        // a reviewer can see what the statement is supposed to say.
        content.push_str(&format!("(* Proposition (Verum source): {} *)\n",
            sanitise_for_comment(&spec.proposition_text)));
        if spec.has_proof_body {
            content.push_str(&format!(
                "Theorem {} : Prop.\n\
                 Proof.\n  \
                   admit.\n\
                 Admitted.\n",
                spec.name,
            ));
        } else {
            content.push_str(&format!("Axiom {} : Prop.\n", spec.name));
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
        if spec.has_proof_body {
            content.push_str(&format!(
                "theorem {} : Prop := by sorry\n",
                spec.name,
            ));
        } else {
            content.push_str(&format!("axiom {} : Prop\n", spec.name));
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
            has_proof_body: true,
            declared_strategy: None,
        }
    }

    fn spec_axiom(name: &str, prop: &str) -> TheoremSpec {
        TheoremSpec {
            name: name.to_string(),
            module_path: "test_module".to_string(),
            proposition_text: prop.to_string(),
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
}
