//! Proof-term export — `verum_kernel::CoreTerm` → external proof
//! assistant syntax (Lean 4 / Coq / Agda / Dedukti / Metamath).
//! M-VVA-FU Sub-2.5/2.6/2.7 V0/V1 (deferred per VVA spec L1422).
//!
//! Pre-this-module the cross-format export emitted statement-level
//! declarations only — the @theorem signature followed by `sorry` /
//! `Admitted` / `postulate`. V1 (this module) lowers the structural
//! constructors that the kernel re-checks (Var / Universe / Pi / Lam
//! / App / Refl / Axiom / SmtProof) to the target proof-assistant
//! syntax. V2 (multi-month follow-up) extends to PathTy / HComp /
//! Transp / Glue / Quotient / Inductive / Elim, plus the cohesive
//! and modal modalities (Shape / Flat / Sharp / ModalBox /
//! ModalDiamond / EpsilonOf / AlphaOf).
//!
//! **Architectural shape.** Each target gets its own lowering module:
//!   * `lean::lower_term(t)` — `CoreTerm → String` (Lean 4 syntax).
//!   * `coq::lower_term(t)` — `CoreTerm → String` (Coq / Gallina).
//!   * `agda::lower_term(t)` — `CoreTerm → String` (Agda 2.6).
//!   * `dedukti::lower_term(t)` — `CoreTerm → String` (λΠ-modulo).
//!   * `metamath::lower_term(t)` — `CoreTerm → String` (label form).
//!
//! All five use the same conservative-fallback strategy: when an
//! unsupported constructor is encountered, return `sorry` (Lean) /
//! `admit` (Coq) / `?` (Agda) / `(* unsupported *)` (Dedukti) /
//! `( ?ctor )` (Metamath) with the constructor name embedded in a
//! tactic-mode comment for downstream re-derivation hints.
//!
//! References:
//!   * Lean 4 manual §6 (term mode).
//!   * Coq reference manual ch. 11 (Gallina).
//!   * Agda 2.6 manual ch. 4 (term language).
//!   * Dedukti — λΠ-calculus modulo, Saillard PhD ch. 3.
//!   * Metamath book ch. 2 — proof-substitution model.

use verum_kernel::{CoreTerm, UniverseLevel};

/// Lean 4 proof-term lowerer (V1).
pub mod lean {
    use super::*;

    /// Lower a `CoreTerm` to a Lean 4 term-mode string. Returns
    /// `sorry` for un-supported constructors, with a structured
    /// comment naming the constructor for downstream debugging.
    pub fn lower_term(t: &CoreTerm) -> String {
        match t {
            CoreTerm::Var(name) => name.as_str().to_string(),

            CoreTerm::Universe(UniverseLevel::Prop) => "Prop".to_string(),
            CoreTerm::Universe(UniverseLevel::Concrete(n)) => {
                if *n == 0 {
                    "Type".to_string()
                } else {
                    format!("Type {}", n)
                }
            }
            CoreTerm::Universe(UniverseLevel::Variable(name)) => {
                format!("Type {}", name.as_str())
            }
            CoreTerm::Universe(UniverseLevel::Succ(inner)) => {
                format!("Type ({} + 1)", lower_universe(inner.as_ref()))
            }
            CoreTerm::Universe(UniverseLevel::Max(a, b)) => {
                format!(
                    "Type (max {} {})",
                    lower_universe(a.as_ref()),
                    lower_universe(b.as_ref())
                )
            }

            CoreTerm::Pi { binder, domain, codomain } => {
                format!(
                    "({} : {}) → {}",
                    binder.as_str(),
                    lower_term(domain.as_ref()),
                    lower_term(codomain.as_ref())
                )
            }

            CoreTerm::Lam { binder, domain, body } => {
                format!(
                    "fun ({} : {}) => {}",
                    binder.as_str(),
                    lower_term(domain.as_ref()),
                    lower_term(body.as_ref())
                )
            }

            CoreTerm::App(f, a) => {
                format!("({}) ({})", lower_term(f.as_ref()), lower_term(a.as_ref()))
            }

            CoreTerm::Refl(_) => "rfl".to_string(),

            CoreTerm::Axiom { name, .. } => name.as_str().to_string(),

            // Sigma / Pair / Fst / Snd — direct Lean 4 mappings.
            CoreTerm::Sigma { binder, fst_ty, snd_ty } => {
                format!(
                    "Sigma (fun ({} : {}) => {})",
                    binder.as_str(),
                    lower_term(fst_ty.as_ref()),
                    lower_term(snd_ty.as_ref())
                )
            }
            CoreTerm::Pair(a, b) => {
                format!("⟨{}, {}⟩", lower_term(a.as_ref()), lower_term(b.as_ref()))
            }
            CoreTerm::Fst(p) => format!("({}).1", lower_term(p.as_ref())),
            CoreTerm::Snd(p) => format!("({}).2", lower_term(p.as_ref())),

            // PathTy — Lean's Eq for n=1 truncated path types.
            CoreTerm::PathTy { carrier, lhs, rhs } => {
                format!(
                    "@Eq {} {} {}",
                    lower_term(carrier.as_ref()),
                    lower_term(lhs.as_ref()),
                    lower_term(rhs.as_ref())
                )
            }

            // SmtProof — emit a `decide` tactic block; the certificate
            // re-replay happens in the kernel, not in Lean. Lean's
            // own `decide` may or may not close the goal independently;
            // when it does the proof is doubly-checked.
            CoreTerm::SmtProof(_) => "by decide".to_string(),

            // Inductive — emit just the type-name path (Lean resolves
            // it via its own type registry).
            CoreTerm::Inductive { path, .. } => path.as_str().to_string(),

            // Refinement — emit the base type with a comment naming
            // the predicate. Lean 4's subtype `{x : T // p x}` is the
            // closest match; the predicate is opaque at this layer.
            CoreTerm::Refine { base, binder, predicate } => {
                format!(
                    "{{{} : {} // {}}}",
                    binder.as_str(),
                    lower_term(base.as_ref()),
                    lower_term(predicate.as_ref())
                )
            }

            // Fallback for unsupported V2+ constructors.
            other => format!("sorry /- unsupported: {} -/", constructor_name(other)),
        }
    }

    fn lower_universe(level: &UniverseLevel) -> String {
        match level {
            UniverseLevel::Concrete(n) => n.to_string(),
            UniverseLevel::Prop => "0".to_string(),
            UniverseLevel::Variable(name) => name.as_str().to_string(),
            UniverseLevel::Succ(inner) => format!("({} + 1)", lower_universe(inner.as_ref())),
            UniverseLevel::Max(a, b) => format!("(max {} {})", lower_universe(a.as_ref()), lower_universe(b.as_ref())),
        }
    }
}

/// Coq / Gallina proof-term lowerer (V1). Mirrors lean::lower_term
/// with Coq syntax differences.
pub mod coq {
    use super::*;

    pub fn lower_term(t: &CoreTerm) -> String {
        match t {
            CoreTerm::Var(name) => name.as_str().to_string(),

            CoreTerm::Universe(UniverseLevel::Prop) => "Prop".to_string(),
            CoreTerm::Universe(UniverseLevel::Concrete(n)) => {
                if *n == 0 {
                    "Set".to_string()
                } else {
                    format!("Type@{{{}}}", n)
                }
            }
            CoreTerm::Universe(UniverseLevel::Variable(name)) => {
                format!("Type@{{{}}}", name.as_str())
            }
            CoreTerm::Universe(UniverseLevel::Succ(inner)) => {
                format!("Type@{{{} + 1}}", lower_universe(inner.as_ref()))
            }
            CoreTerm::Universe(UniverseLevel::Max(a, b)) => {
                format!(
                    "Type@{{max {} {}}}",
                    lower_universe(a.as_ref()),
                    lower_universe(b.as_ref())
                )
            }

            CoreTerm::Pi { binder, domain, codomain } => {
                format!(
                    "forall ({} : {}), {}",
                    binder.as_str(),
                    lower_term(domain.as_ref()),
                    lower_term(codomain.as_ref())
                )
            }

            CoreTerm::Lam { binder, domain, body } => {
                format!(
                    "fun ({} : {}) => {}",
                    binder.as_str(),
                    lower_term(domain.as_ref()),
                    lower_term(body.as_ref())
                )
            }

            CoreTerm::App(f, a) => {
                format!("({}) ({})", lower_term(f.as_ref()), lower_term(a.as_ref()))
            }

            CoreTerm::Refl(_) => "eq_refl".to_string(),

            CoreTerm::Axiom { name, .. } => name.as_str().to_string(),

            CoreTerm::Sigma { binder, fst_ty, snd_ty } => {
                format!(
                    "{{{} : {} & {}}}",
                    binder.as_str(),
                    lower_term(fst_ty.as_ref()),
                    lower_term(snd_ty.as_ref())
                )
            }
            CoreTerm::Pair(a, b) => {
                format!("(existT _ {} {})", lower_term(a.as_ref()), lower_term(b.as_ref()))
            }
            CoreTerm::Fst(p) => format!("(projT1 {})", lower_term(p.as_ref())),
            CoreTerm::Snd(p) => format!("(projT2 {})", lower_term(p.as_ref())),

            CoreTerm::PathTy { carrier, lhs, rhs } => {
                format!(
                    "@eq {} {} {}",
                    lower_term(carrier.as_ref()),
                    lower_term(lhs.as_ref()),
                    lower_term(rhs.as_ref())
                )
            }

            CoreTerm::SmtProof(_) => "ltac:(decide_eq || abstract auto)".to_string(),

            CoreTerm::Inductive { path, .. } => path.as_str().to_string(),

            CoreTerm::Refine { base, binder, predicate } => {
                format!(
                    "{{{} : {} | {}}}",
                    binder.as_str(),
                    lower_term(base.as_ref()),
                    lower_term(predicate.as_ref())
                )
            }

            other => format!("admit (* unsupported: {} *)", constructor_name(other)),
        }
    }

    fn lower_universe(level: &UniverseLevel) -> String {
        match level {
            UniverseLevel::Concrete(n) => n.to_string(),
            UniverseLevel::Prop => "0".to_string(),
            UniverseLevel::Variable(name) => name.as_str().to_string(),
            UniverseLevel::Succ(inner) => format!("{} + 1", lower_universe(inner.as_ref())),
            UniverseLevel::Max(a, b) => format!("max {} {}", lower_universe(a.as_ref()), lower_universe(b.as_ref())),
        }
    }
}

/// Agda 2.6 proof-term lowerer (V1).
pub mod agda {
    use super::*;

    pub fn lower_term(t: &CoreTerm) -> String {
        match t {
            CoreTerm::Var(name) => name.as_str().to_string(),

            CoreTerm::Universe(UniverseLevel::Prop) => "Prop".to_string(),
            CoreTerm::Universe(UniverseLevel::Concrete(n)) => {
                if *n == 0 {
                    "Set".to_string()
                } else {
                    format!("Set{}", n)
                }
            }
            CoreTerm::Universe(UniverseLevel::Variable(name)) => {
                format!("Set {}", name.as_str())
            }
            CoreTerm::Universe(UniverseLevel::Succ(inner)) => {
                format!("Set (suc {})", lower_universe(inner.as_ref()))
            }
            CoreTerm::Universe(UniverseLevel::Max(a, b)) => {
                format!(
                    "Set ({} ⊔ {})",
                    lower_universe(a.as_ref()),
                    lower_universe(b.as_ref())
                )
            }

            CoreTerm::Pi { binder, domain, codomain } => {
                format!(
                    "({} : {}) → {}",
                    binder.as_str(),
                    lower_term(domain.as_ref()),
                    lower_term(codomain.as_ref())
                )
            }

            CoreTerm::Lam { binder, domain, body } => {
                format!(
                    "λ ({} : {}) → {}",
                    binder.as_str(),
                    lower_term(domain.as_ref()),
                    lower_term(body.as_ref())
                )
            }

            CoreTerm::App(f, a) => {
                format!("({}) ({})", lower_term(f.as_ref()), lower_term(a.as_ref()))
            }

            CoreTerm::Refl(_) => "refl".to_string(),

            CoreTerm::Axiom { name, .. } => name.as_str().to_string(),

            CoreTerm::Sigma { binder, fst_ty, snd_ty } => {
                format!(
                    "Σ {} ∶ {} , {}",
                    binder.as_str(),
                    lower_term(fst_ty.as_ref()),
                    lower_term(snd_ty.as_ref())
                )
            }
            CoreTerm::Pair(a, b) => {
                format!("({} , {})", lower_term(a.as_ref()), lower_term(b.as_ref()))
            }
            CoreTerm::Fst(p) => format!("proj₁ ({})", lower_term(p.as_ref())),
            CoreTerm::Snd(p) => format!("proj₂ ({})", lower_term(p.as_ref())),

            CoreTerm::PathTy { carrier, lhs, rhs } => {
                format!(
                    "_≡_ {{{}}} {} {}",
                    lower_term(carrier.as_ref()),
                    lower_term(lhs.as_ref()),
                    lower_term(rhs.as_ref())
                )
            }

            CoreTerm::SmtProof(_) => "{!!}".to_string(),

            CoreTerm::Inductive { path, .. } => path.as_str().to_string(),

            CoreTerm::Refine { base, binder, predicate } => {
                format!(
                    "Σ ({} : {}) , {}",
                    binder.as_str(),
                    lower_term(base.as_ref()),
                    lower_term(predicate.as_ref())
                )
            }

            other => format!("? {{- unsupported: {} -}}", constructor_name(other)),
        }
    }

    fn lower_universe(level: &UniverseLevel) -> String {
        match level {
            UniverseLevel::Concrete(n) => n.to_string(),
            UniverseLevel::Prop => "0".to_string(),
            UniverseLevel::Variable(name) => name.as_str().to_string(),
            UniverseLevel::Succ(inner) => format!("(suc {})", lower_universe(inner.as_ref())),
            UniverseLevel::Max(a, b) => format!("({} ⊔ {})", lower_universe(a.as_ref()), lower_universe(b.as_ref())),
        }
    }
}

/// Dedukti (λΠ-modulo) proof-term lowerer (V1). Dedukti is the
/// universal proof-assistant interchange format — encodes Lean,
/// Coq, Agda, HOL via λΠ-calculus modulo β/η + rewrite rules.
pub mod dedukti {
    use super::*;

    pub fn lower_term(t: &CoreTerm) -> String {
        match t {
            CoreTerm::Var(name) => name.as_str().to_string(),

            CoreTerm::Universe(UniverseLevel::Prop) => "prop".to_string(),
            CoreTerm::Universe(UniverseLevel::Concrete(n)) => {
                if *n == 0 { "Type".to_string() } else { format!("(Type {})", n) }
            }
            CoreTerm::Universe(UniverseLevel::Variable(name)) => name.as_str().to_string(),
            CoreTerm::Universe(UniverseLevel::Succ(_)) => "(s _)".to_string(),
            CoreTerm::Universe(UniverseLevel::Max(_, _)) => "(max _ _)".to_string(),

            // Dedukti: `binder : Domain -> Codomain` is dependent product.
            CoreTerm::Pi { binder, domain, codomain } => {
                format!(
                    "{} : {} -> {}",
                    binder.as_str(),
                    lower_term(domain.as_ref()),
                    lower_term(codomain.as_ref())
                )
            }

            // Dedukti: `binder : Domain => body` is λ-abstraction.
            CoreTerm::Lam { binder, domain, body } => {
                format!(
                    "{} : {} => {}",
                    binder.as_str(),
                    lower_term(domain.as_ref()),
                    lower_term(body.as_ref())
                )
            }

            CoreTerm::App(f, a) => {
                format!("({} {})", lower_term(f.as_ref()), lower_term(a.as_ref()))
            }

            CoreTerm::Refl(_) => "refl".to_string(),

            CoreTerm::Axiom { name, .. } => name.as_str().to_string(),

            CoreTerm::Sigma { binder, fst_ty, snd_ty } => {
                format!(
                    "(sig ({}) ({} : {} => {}))",
                    lower_term(fst_ty.as_ref()),
                    binder.as_str(),
                    lower_term(fst_ty.as_ref()),
                    lower_term(snd_ty.as_ref())
                )
            }
            CoreTerm::Pair(a, b) => {
                format!("(pair {} {})", lower_term(a.as_ref()), lower_term(b.as_ref()))
            }
            CoreTerm::Fst(p) => format!("(proj1 {})", lower_term(p.as_ref())),
            CoreTerm::Snd(p) => format!("(proj2 {})", lower_term(p.as_ref())),

            CoreTerm::PathTy { carrier, lhs, rhs } => {
                format!(
                    "(eq {} {} {})",
                    lower_term(carrier.as_ref()),
                    lower_term(lhs.as_ref()),
                    lower_term(rhs.as_ref())
                )
            }

            CoreTerm::SmtProof(_) => "(* smt-proof — replay via verum *)".to_string(),

            CoreTerm::Inductive { path, .. } => path.as_str().replace('.', "__"),

            CoreTerm::Refine { base, binder, predicate } => {
                format!(
                    "(refine ({}) ({} : {} => {}))",
                    lower_term(base.as_ref()),
                    binder.as_str(),
                    lower_term(base.as_ref()),
                    lower_term(predicate.as_ref())
                )
            }

            other => format!("(* unsupported: {} *)", constructor_name(other)),
        }
    }
}

/// Metamath proof-term lowerer (V1). Metamath uses a minimalist
/// substitution-based proof-checking model — every step is a label
/// + substitution. We lower CoreTerm to a Metamath-style label
/// chain that downstream tools can translate into the actual
/// `$p ... $.` proof block.
pub mod metamath {
    use super::*;

    pub fn lower_term(t: &CoreTerm) -> String {
        match t {
            CoreTerm::Var(name) => name.as_str().to_string(),

            CoreTerm::Universe(UniverseLevel::Prop) => "wff".to_string(),
            CoreTerm::Universe(UniverseLevel::Concrete(n)) => {
                if *n == 0 { "set".to_string() } else { format!("class{}", n) }
            }
            CoreTerm::Universe(UniverseLevel::Variable(name)) => name.as_str().to_string(),
            CoreTerm::Universe(UniverseLevel::Succ(_)) => "succU".to_string(),
            CoreTerm::Universe(UniverseLevel::Max(_, _)) => "maxU".to_string(),

            CoreTerm::Pi { binder, domain, codomain } => {
                format!(
                    "( wpi {} {} {} )",
                    binder.as_str(),
                    lower_term(domain.as_ref()),
                    lower_term(codomain.as_ref())
                )
            }

            CoreTerm::Lam { binder, domain, body } => {
                format!(
                    "( wlam {} {} {} )",
                    binder.as_str(),
                    lower_term(domain.as_ref()),
                    lower_term(body.as_ref())
                )
            }

            CoreTerm::App(f, a) => {
                format!("( wapp {} {} )", lower_term(f.as_ref()), lower_term(a.as_ref()))
            }

            CoreTerm::Refl(t) => format!("( wrefl {} )", lower_term(t.as_ref())),

            CoreTerm::Axiom { name, .. } => name.as_str().to_string(),

            CoreTerm::Sigma { binder, fst_ty, snd_ty } => {
                format!(
                    "( wsigma {} {} {} )",
                    binder.as_str(),
                    lower_term(fst_ty.as_ref()),
                    lower_term(snd_ty.as_ref())
                )
            }
            CoreTerm::Pair(a, b) => {
                format!("( wpair {} {} )", lower_term(a.as_ref()), lower_term(b.as_ref()))
            }
            CoreTerm::Fst(p) => format!("( wfst {} )", lower_term(p.as_ref())),
            CoreTerm::Snd(p) => format!("( wsnd {} )", lower_term(p.as_ref())),

            CoreTerm::PathTy { carrier, lhs, rhs } => {
                format!(
                    "( weq {} {} {} )",
                    lower_term(carrier.as_ref()),
                    lower_term(lhs.as_ref()),
                    lower_term(rhs.as_ref())
                )
            }

            CoreTerm::SmtProof(_) => "( wsmt )".to_string(),

            CoreTerm::Inductive { path, .. } => path.as_str().replace('.', "_"),

            CoreTerm::Refine { base, binder, predicate } => {
                format!(
                    "( wrefine {} {} {} )",
                    binder.as_str(),
                    lower_term(base.as_ref()),
                    lower_term(predicate.as_ref())
                )
            }

            other => format!("( ?{} )", constructor_name(other)),
        }
    }
}

/// Diagnostic helper: name an unsupported constructor for the
/// fallback comment. Lists every CoreTerm variant by string tag.
fn constructor_name(t: &CoreTerm) -> &'static str {
    match t {
        CoreTerm::Var(_) => "Var",
        CoreTerm::Universe(_) => "Universe",
        CoreTerm::Pi { .. } => "Pi",
        CoreTerm::Lam { .. } => "Lam",
        CoreTerm::App(_, _) => "App",
        CoreTerm::Sigma { .. } => "Sigma",
        CoreTerm::Pair(_, _) => "Pair",
        CoreTerm::Fst(_) => "Fst",
        CoreTerm::Snd(_) => "Snd",
        CoreTerm::PathTy { .. } => "PathTy",
        CoreTerm::Refl(_) => "Refl",
        CoreTerm::PathOver { .. } => "PathOver",
        CoreTerm::HComp { .. } => "HComp",
        CoreTerm::Transp { .. } => "Transp",
        CoreTerm::Glue { .. } => "Glue",
        CoreTerm::Refine { .. } => "Refine",
        CoreTerm::Quotient { .. } => "Quotient",
        CoreTerm::QuotIntro { .. } => "QuotIntro",
        CoreTerm::QuotElim { .. } => "QuotElim",
        CoreTerm::Inductive { .. } => "Inductive",
        CoreTerm::Elim { .. } => "Elim",
        CoreTerm::SmtProof(_) => "SmtProof",
        CoreTerm::Axiom { .. } => "Axiom",
        CoreTerm::EpsilonOf(_) => "EpsilonOf",
        CoreTerm::AlphaOf(_) => "AlphaOf",
        CoreTerm::ModalBox(_) => "ModalBox",
        CoreTerm::ModalDiamond(_) => "ModalDiamond",
        CoreTerm::ModalBigAnd(_) => "ModalBigAnd",
        CoreTerm::Shape(_) => "Shape",
        CoreTerm::Flat(_) => "Flat",
        CoreTerm::Sharp(_) => "Sharp",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_common::{Heap, Text};

    fn var(name: &str) -> CoreTerm {
        CoreTerm::Var(Text::from(name))
    }

    // ---- Lean 4 lowering ----

    #[test]
    fn lean_var() {
        assert_eq!(lean::lower_term(&var("x")), "x");
    }

    #[test]
    fn lean_universe_prop() {
        assert_eq!(
            lean::lower_term(&CoreTerm::Universe(UniverseLevel::Prop)),
            "Prop"
        );
    }

    #[test]
    fn lean_universe_concrete() {
        assert_eq!(
            lean::lower_term(&CoreTerm::Universe(UniverseLevel::Concrete(0))),
            "Type"
        );
        assert_eq!(
            lean::lower_term(&CoreTerm::Universe(UniverseLevel::Concrete(2))),
            "Type 2"
        );
    }

    #[test]
    fn lean_pi_type() {
        let pi = CoreTerm::Pi {
            binder: Text::from("x"),
            domain: Heap::new(var("Nat")),
            codomain: Heap::new(var("Bool")),
        };
        assert_eq!(lean::lower_term(&pi), "(x : Nat) → Bool");
    }

    #[test]
    fn lean_lambda() {
        let lam = CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(var("Int")),
            body: Heap::new(var("x")),
        };
        assert_eq!(lean::lower_term(&lam), "fun (x : Int) => x");
    }

    #[test]
    fn lean_app() {
        let app = CoreTerm::App(Heap::new(var("f")), Heap::new(var("x")));
        assert_eq!(lean::lower_term(&app), "(f) (x)");
    }

    #[test]
    fn lean_refl() {
        assert_eq!(lean::lower_term(&CoreTerm::Refl(Heap::new(var("a")))), "rfl");
    }

    // ---- Coq lowering ----

    #[test]
    fn coq_pi_uses_forall() {
        let pi = CoreTerm::Pi {
            binder: Text::from("x"),
            domain: Heap::new(var("nat")),
            codomain: Heap::new(var("Prop")),
        };
        assert_eq!(coq::lower_term(&pi), "forall (x : nat), Prop");
    }

    #[test]
    fn coq_refl_uses_eq_refl() {
        assert_eq!(coq::lower_term(&CoreTerm::Refl(Heap::new(var("a")))), "eq_refl");
    }

    #[test]
    fn coq_universe_concrete_zero_is_set() {
        assert_eq!(
            coq::lower_term(&CoreTerm::Universe(UniverseLevel::Concrete(0))),
            "Set"
        );
    }

    // ---- Agda lowering ----

    #[test]
    fn agda_lambda_uses_unicode_lambda() {
        let lam = CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(var("ℕ")),
            body: Heap::new(var("x")),
        };
        assert_eq!(agda::lower_term(&lam), "λ (x : ℕ) → x");
    }

    #[test]
    fn agda_universe_concrete_with_subscript() {
        assert_eq!(
            agda::lower_term(&CoreTerm::Universe(UniverseLevel::Concrete(0))),
            "Set"
        );
        assert_eq!(
            agda::lower_term(&CoreTerm::Universe(UniverseLevel::Concrete(3))),
            "Set3"
        );
    }

    #[test]
    fn agda_refl_lowercase() {
        assert_eq!(agda::lower_term(&CoreTerm::Refl(Heap::new(var("a")))), "refl");
    }

    // ---- Fallback ----

    #[test]
    fn lean_hcomp_falls_back_to_sorry_with_diagnostic() {
        let hcomp = CoreTerm::HComp {
            phi: Heap::new(var("phi")),
            walls: Heap::new(var("walls")),
            base: Heap::new(var("base")),
        };
        let out = lean::lower_term(&hcomp);
        assert!(out.starts_with("sorry"));
        assert!(out.contains("HComp"));
    }

    #[test]
    fn coq_hcomp_falls_back_to_admit() {
        let hcomp = CoreTerm::HComp {
            phi: Heap::new(var("phi")),
            walls: Heap::new(var("walls")),
            base: Heap::new(var("base")),
        };
        let out = coq::lower_term(&hcomp);
        assert!(out.starts_with("admit"));
        assert!(out.contains("HComp"));
    }

    #[test]
    fn agda_hcomp_falls_back_to_questionmark() {
        let hcomp = CoreTerm::HComp {
            phi: Heap::new(var("phi")),
            walls: Heap::new(var("walls")),
            base: Heap::new(var("base")),
        };
        let out = agda::lower_term(&hcomp);
        assert!(out.starts_with("?"));
        assert!(out.contains("HComp"));
    }

    // ---- Composition ----

    #[test]
    fn lean_id_lambda_composition() {
        // λ (x : Nat) → x  applied to itself in Pi context.
        let id_pi = CoreTerm::Pi {
            binder: Text::from("x"),
            domain: Heap::new(var("Nat")),
            codomain: Heap::new(var("Nat")),
        };
        let id_lam = CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(var("Nat")),
            body: Heap::new(var("x")),
        };
        assert_eq!(lean::lower_term(&id_pi), "(x : Nat) → Nat");
        assert_eq!(lean::lower_term(&id_lam), "fun (x : Nat) => x");
    }

    // ---- Dedukti lowering ----

    #[test]
    fn dedukti_var() {
        assert_eq!(dedukti::lower_term(&var("x")), "x");
    }

    #[test]
    fn dedukti_pi_uses_arrow() {
        let pi = CoreTerm::Pi {
            binder: Text::from("x"),
            domain: Heap::new(var("Nat")),
            codomain: Heap::new(var("Bool")),
        };
        assert_eq!(dedukti::lower_term(&pi), "x : Nat -> Bool");
    }

    #[test]
    fn dedukti_lambda_uses_double_arrow() {
        let lam = CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(var("Nat")),
            body: Heap::new(var("x")),
        };
        assert_eq!(dedukti::lower_term(&lam), "x : Nat => x");
    }

    #[test]
    fn dedukti_refl_lowercase() {
        assert_eq!(dedukti::lower_term(&CoreTerm::Refl(Heap::new(var("a")))), "refl");
    }

    #[test]
    fn dedukti_inductive_dot_to_double_underscore() {
        let ind = CoreTerm::Inductive {
            path: Text::from("core.math.Set"),
            args: verum_common::List::new(),
        };
        // Dedukti identifiers can't contain dots; mangle to __.
        assert_eq!(dedukti::lower_term(&ind), "core__math__Set");
    }

    // ---- Metamath lowering ----

    #[test]
    fn metamath_var() {
        assert_eq!(metamath::lower_term(&var("ph")), "ph");
    }

    #[test]
    fn metamath_pi_uses_wpi_label() {
        let pi = CoreTerm::Pi {
            binder: Text::from("x"),
            domain: Heap::new(var("nat")),
            codomain: Heap::new(var("bool")),
        };
        assert_eq!(metamath::lower_term(&pi), "( wpi x nat bool )");
    }

    #[test]
    fn metamath_lambda_uses_wlam_label() {
        let lam = CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(var("nat")),
            body: Heap::new(var("x")),
        };
        assert_eq!(metamath::lower_term(&lam), "( wlam x nat x )");
    }

    #[test]
    fn metamath_refl_uses_wrefl() {
        assert_eq!(
            metamath::lower_term(&CoreTerm::Refl(Heap::new(var("a")))),
            "( wrefl a )"
        );
    }

    #[test]
    fn metamath_universe_concrete_zero_is_set() {
        assert_eq!(
            metamath::lower_term(&CoreTerm::Universe(UniverseLevel::Concrete(0))),
            "set"
        );
    }
}
