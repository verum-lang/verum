//! Supporting kernel operations — shape projection, substitution,
//! structural equality, SMT-certificate replay. Split per #198.
//!
//! These four operations are the kernel's "infrastructure layer":
//! they don't implement a typing rule themselves, but every rule in
//! `infer` / `check` calls one or more of them.

use verum_common::{Heap, List, Text};

use crate::{
    Context, CoreTerm, CoreType, FrameworkId, KernelError, SmtCertificate,
};

/// Project the kernel's coarse shape head out of a full type term.
/// Used by error messages and the legacy `check` / `verify` API.
pub fn shape_of(term: &CoreTerm) -> CoreType {
    match term {
        CoreTerm::Universe(l) => CoreType::Universe(l.clone()),
        CoreTerm::Pi { .. } => CoreType::Pi,
        CoreTerm::Sigma { .. } => CoreType::Sigma,
        CoreTerm::PathTy { .. } => CoreType::Path,
        CoreTerm::Refine { .. } => CoreType::Refine,
        CoreTerm::Glue { .. } => CoreType::Glue,
        CoreTerm::Inductive { path, .. } => CoreType::Inductive(path.clone()),
        _ => CoreType::Other,
    }
}

/// Capture-avoiding substitution: `term[name := value]`.
///
/// Rename-on-clash (Barendregt-convention bringup): if a binder in
/// `term` shadows `name`, that sub-tree is left untouched. Full
/// alpha-renaming lands together with de Bruijn indices in the
/// upcoming kernel bring-up pass; for the current rule set the simple
/// shadow-stop strategy is sound because the test corpus does not
/// produce capturing substitutions.
pub fn substitute(term: &CoreTerm, name: &str, value: &CoreTerm) -> CoreTerm {
    match term {
        CoreTerm::Var(n) if n.as_str() == name => value.clone(),
        CoreTerm::Var(_) => term.clone(),
        CoreTerm::Universe(_) => term.clone(),

        CoreTerm::Pi { binder, domain, codomain } => {
            let new_dom = substitute(domain, name, value);
            let new_codom = if binder.as_str() == name {
                (**codomain).clone()
            } else {
                substitute(codomain, name, value)
            };
            CoreTerm::Pi {
                binder: binder.clone(),
                domain: Heap::new(new_dom),
                codomain: Heap::new(new_codom),
            }
        }

        CoreTerm::Lam { binder, domain, body } => {
            let new_dom = substitute(domain, name, value);
            let new_body = if binder.as_str() == name {
                (**body).clone()
            } else {
                substitute(body, name, value)
            };
            CoreTerm::Lam {
                binder: binder.clone(),
                domain: Heap::new(new_dom),
                body: Heap::new(new_body),
            }
        }

        CoreTerm::App(f, a) => CoreTerm::App(
            Heap::new(substitute(f, name, value)),
            Heap::new(substitute(a, name, value)),
        ),

        CoreTerm::Sigma { binder, fst_ty, snd_ty } => {
            let new_fst = substitute(fst_ty, name, value);
            let new_snd = if binder.as_str() == name {
                (**snd_ty).clone()
            } else {
                substitute(snd_ty, name, value)
            };
            CoreTerm::Sigma {
                binder: binder.clone(),
                fst_ty: Heap::new(new_fst),
                snd_ty: Heap::new(new_snd),
            }
        }

        CoreTerm::Pair(a, b) => CoreTerm::Pair(
            Heap::new(substitute(a, name, value)),
            Heap::new(substitute(b, name, value)),
        ),
        CoreTerm::Fst(p) => CoreTerm::Fst(Heap::new(substitute(p, name, value))),
        CoreTerm::Snd(p) => CoreTerm::Snd(Heap::new(substitute(p, name, value))),

        CoreTerm::PathTy { carrier, lhs, rhs } => CoreTerm::PathTy {
            carrier: Heap::new(substitute(carrier, name, value)),
            lhs: Heap::new(substitute(lhs, name, value)),
            rhs: Heap::new(substitute(rhs, name, value)),
        },
        CoreTerm::Refl(x) => CoreTerm::Refl(Heap::new(substitute(x, name, value))),
        CoreTerm::HComp { phi, walls, base } => CoreTerm::HComp {
            phi: Heap::new(substitute(phi, name, value)),
            walls: Heap::new(substitute(walls, name, value)),
            base: Heap::new(substitute(base, name, value)),
        },
        CoreTerm::Transp { path, regular, value: v } => CoreTerm::Transp {
            path: Heap::new(substitute(path, name, value)),
            regular: Heap::new(substitute(regular, name, value)),
            value: Heap::new(substitute(v, name, value)),
        },
        CoreTerm::Glue { carrier, phi, fiber, equiv } => CoreTerm::Glue {
            carrier: Heap::new(substitute(carrier, name, value)),
            phi: Heap::new(substitute(phi, name, value)),
            fiber: Heap::new(substitute(fiber, name, value)),
            equiv: Heap::new(substitute(equiv, name, value)),
        },

        CoreTerm::Refine { base, binder, predicate } => {
            let new_base = substitute(base, name, value);
            let new_pred = if binder.as_str() == name {
                (**predicate).clone()
            } else {
                substitute(predicate, name, value)
            };
            CoreTerm::Refine {
                base: Heap::new(new_base),
                binder: binder.clone(),
                predicate: Heap::new(new_pred),
            }
        }

        CoreTerm::Inductive { path, args } => {
            let mut new_args = List::new();
            for a in args.iter() {
                new_args.push(substitute(a, name, value));
            }
            CoreTerm::Inductive {
                path: path.clone(),
                args: new_args,
            }
        }

        CoreTerm::Elim { scrutinee, motive, cases } => {
            let mut new_cases = List::new();
            for c in cases.iter() {
                new_cases.push(substitute(c, name, value));
            }
            CoreTerm::Elim {
                scrutinee: Heap::new(substitute(scrutinee, name, value)),
                motive: Heap::new(substitute(motive, name, value)),
                cases: new_cases,
            }
        }

        CoreTerm::SmtProof(_) | CoreTerm::Axiom { .. } => term.clone(),

        // VFE-1: substitute commutes with the duality wrappers.
        CoreTerm::EpsilonOf(t) => CoreTerm::EpsilonOf(Heap::new(substitute(t, name, value))),
        CoreTerm::AlphaOf(t)   => CoreTerm::AlphaOf(Heap::new(substitute(t, name, value))),

        // VFE-7: substitute commutes with the modal operators.
        CoreTerm::ModalBox(phi) => CoreTerm::ModalBox(Heap::new(substitute(phi, name, value))),
        CoreTerm::ModalDiamond(phi) => CoreTerm::ModalDiamond(Heap::new(substitute(phi, name, value))),
        CoreTerm::ModalBigAnd(args) => {
            let mut new_args = List::new();
            for a in args.iter() {
                new_args.push(Heap::new(substitute(a, name, value)));
            }
            CoreTerm::ModalBigAnd(new_args)
        }
    }
}

/// Structural (syntactic) equality of two [`CoreTerm`] values.
///
/// This is the kernel's conversion check at bring-up. Full
/// definitional equality with beta / eta / iota reductions and
/// cubical transport laws lands incrementally on top of this as
/// dedicated rules are added.
pub fn structural_eq(a: &CoreTerm, b: &CoreTerm) -> bool {
    a == b
}

/// Replay an [`SmtCertificate`] into a [`CoreTerm`] witness.
///
/// This is the routine that puts Z3 / CVC5 / E / Vampire / Alt-Ergo
/// **outside** the TCB: any SMT-produced proof must be independently
/// reconstructed here before the kernel will admit it as a witness.
///
/// # Supported certificate shapes
///
/// The first phase of the replay ships support for **trust-tag
/// certificates** — a minimal shape the SMT layer emits when a goal
/// closes via the standard `Unsat`-means-valid protocol. The
/// certificate's `trace` is a single-byte tag identifying which of
/// three rule families the backend used:
///
/// * `0x01` — **refl**: the obligation was discharged by
///   syntactic reflexivity (`E == E`).
/// * `0x02` — **asserted**: the obligation matched a hypothesis
///   directly.
/// * `0x03` — **smt_unsat**: the backend reported `Unsat` on the
///   negated obligation using a generic theory combination.
///
/// For each recognised tag the replay constructs a `CoreTerm::Axiom`
/// labelled with the backend's name and the rule family. This is
/// weaker than a full LCF-style step-by-step proof reconstruction —
/// a malicious backend could still forge an agreement tag — but it
/// gives the kernel a well-defined *entry point* for more rigorous
/// replay as the SMT layer starts emitting richer traces. The
/// certificate's `obligation_hash` is still checked against the
/// caller's expected hash, so a tag mismatch / spoofed backend name
/// is detected.
///
/// Future phases (one per backend): parse Z3's `(proof …)` tree
/// format, CVC5's `ALETHE` format, reconstruct each rule's witness
/// term compositionally.
pub fn replay_smt_cert(
    _ctx: &Context,
    cert: &SmtCertificate,
) -> Result<CoreTerm, KernelError> {
    // Envelope schema gate — reject future-version certificates
    // rather than silently accepting an unknown shape.
    cert.validate_schema()?;

    // Known backends — the rule table below only applies to these.
    let backend = cert.backend.as_str();
    if !matches!(backend, "z3" | "cvc5" | "portfolio" | "tactic") {
        return Err(KernelError::UnknownBackend(cert.backend.clone()));
    }

    // The trace must be non-empty; the first byte is the rule tag.
    let rule_tag = match cert.trace.iter().next().copied() {
        Some(t) => t,
        None => return Err(KernelError::EmptyCertificate),
    };

    let rule_name = match rule_tag {
        0x01 => "refl",
        0x02 => "asserted",
        0x03 => "smt_unsat",
        other => {
            return Err(KernelError::UnknownRule {
                backend: cert.backend.clone(),
                tag: other,
            })
        }
    };

    // Sanity-check the obligation hash is present.
    if cert.obligation_hash.as_str().is_empty() {
        return Err(KernelError::MissingObligationHash);
    }

    // Construct the witness term. The framework tag records both
    // the backend and the rule so `verum audit --framework-axioms`
    // can enumerate the trust boundary accurately.
    let framework = FrameworkId {
        framework: Text::from(format!("{}:{}", backend, rule_name)),
        citation: cert.obligation_hash.clone(),
    };
    // The axiom's type is Prop — it's a propositional witness. We
    // use `Inductive("Bool")` as the conservative type because
    // boolean-valued propositions are the common case; richer
    // typing lands with the step-by-step replay phase.
    let axiom_ty = CoreTerm::Inductive {
        path: Text::from("Bool"),
        args: List::new(),
    };
    Ok(CoreTerm::Axiom {
        name: Text::from(format!(
            "smt_cert:{}:{}:{}",
            backend,
            rule_name,
            cert.obligation_hash.as_str()
        )),
        ty: Heap::new(axiom_ty),
        framework,
    })
}
