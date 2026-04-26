//! Inline kernel tests — moved out of `crates/verum_kernel/src/lib.rs` per
//! #198 V8. These tests use only the public crate API
//! (`verum_kernel::CoreTerm` etc.) and are equivalent to the original
//! inline `#[cfg(test)] mod tests { use super::*; ... }` block at
//! the bottom of `lib.rs` — moving them to integration suites
//! eliminates the last bloat from the kernel `lib.rs` (1394 → 145
//! LOC), which now contains only crate-level docs + the module-map
//! re-exports + per-module sub-mod declarations.

use verum_common::{Heap, List, Maybe, Text};
use verum_kernel::*;


fn unit_ty() -> CoreTerm {
    CoreTerm::Inductive {
        path: Text::from("Unit"),
        args: List::new(),
    }
}

#[test]
fn empty_context_depth_zero() {
    assert_eq!(Context::new().depth(), 0);
}

#[test]
fn extend_then_lookup_finds_binding() {
    let ctx = Context::new().extend(Text::from("x"), unit_ty());
    assert!(matches!(ctx.lookup("x"), Maybe::Some(_)));
    assert_eq!(ctx.depth(), 1);
}

#[test]
fn shadow_returns_innermost() {
    let ctx = Context::new()
        .extend(Text::from("x"), unit_ty())
        .extend(
            Text::from("x"),
            CoreTerm::Universe(UniverseLevel::Concrete(0)),
        );
    match ctx.lookup("x") {
        Maybe::Some(ty) => assert!(matches!(
            ty,
            CoreTerm::Universe(UniverseLevel::Concrete(0))
        )),
        Maybe::None => panic!("expected shadowed binding"),
    }
}

#[test]
fn unbound_variable_is_an_error() {
    let ctx = Context::new();
    let ax = AxiomRegistry::new();
    let err = check(&ctx, &CoreTerm::Var(Text::from("y")), &ax).unwrap_err();
    assert!(matches!(err, KernelError::UnboundVariable(_)));
}

#[test]
fn universe_checks_to_universe() {
    // `Type(0) : Type(1)` — under predicative universes, every level
    // inhabits its strict successor, so `check` (shape-head projection
    // over `infer`) reports the successor level, not the input.
    let ctx = Context::new();
    let ax = AxiomRegistry::new();
    let ty = check(
        &ctx,
        &CoreTerm::Universe(UniverseLevel::Concrete(0)),
        &ax,
    )
    .unwrap();
    assert_eq!(ty, CoreType::Universe(UniverseLevel::Concrete(1)));
}

#[test]
fn axiom_registry_refuses_duplicate_name() {
    let mut reg = AxiomRegistry::new();
    let fw = FrameworkId {
        framework: Text::from("lurie_htt"),
        citation: Text::from("HTT 6.2.2.7"),
    };
    reg.register(Text::from("sheafification"), unit_ty(), fw.clone())
        .unwrap();
    let err = reg
        .register(Text::from("sheafification"), unit_ty(), fw)
        .unwrap_err();
    assert!(matches!(err, KernelError::DuplicateAxiom(_)));
}

#[test]
fn axiom_known_to_registry_is_checkable() {
    let mut reg = AxiomRegistry::new();
    let fw = FrameworkId {
        framework: Text::from("connes_reconstruction"),
        citation: Text::from("Connes 2008 axiom (vii)"),
    };
    reg.register(Text::from("first_order_condition"), unit_ty(), fw.clone())
        .unwrap();
    let term = CoreTerm::Axiom {
        name: Text::from("first_order_condition"),
        ty: Heap::new(unit_ty()),
        framework: fw,
    };
    let ctx = Context::new();
    let head = check(&ctx, &term, &reg).unwrap();
    // The registered axiom has `Unit` as its type; `infer` returns
    // that type verbatim and `shape_of` projects it to the
    // `Inductive(_)` head.
    assert_eq!(head, CoreType::Inductive(Text::from("Unit")));
}

#[test]
fn smt_replay_rejects_empty_trace() {
    let cert = SmtCertificate::new(
        Text::from("z3"),
        Text::from("4.13.0"),
        List::new(),
        Text::from("sha256:0"),
    );
    let ctx = Context::new();
    let err = replay_smt_cert(&ctx, &cert).unwrap_err();
    assert!(matches!(err, KernelError::EmptyCertificate));
}

#[test]
fn smt_replay_rejects_unknown_backend() {
    let mut trace = List::new();
    trace.push(0x01);
    let cert = SmtCertificate::new(
        Text::from("unknown_solver"),
        Text::from("1.0.0"),
        trace,
        Text::from("sha256:0"),
    );
    let ctx = Context::new();
    let err = replay_smt_cert(&ctx, &cert).unwrap_err();
    assert!(matches!(err, KernelError::UnknownBackend(_)));
}

#[test]
fn smt_replay_rejects_unknown_rule_tag() {
    let mut trace = List::new();
    trace.push(0xFF);
    let cert = SmtCertificate::new(
        Text::from("z3"),
        Text::from("4.13.0"),
        trace,
        Text::from("sha256:a"),
    );
    let ctx = Context::new();
    let err = replay_smt_cert(&ctx, &cert).unwrap_err();
    assert!(matches!(err, KernelError::UnknownRule { tag: 0xFF, .. }));
}

#[test]
fn smt_replay_rejects_missing_obligation_hash() {
    let mut trace = List::new();
    trace.push(0x03);
    let cert = SmtCertificate::new(
        Text::from("z3"),
        Text::from("4.13.0"),
        trace,
        Text::from(""),
    );
    let ctx = Context::new();
    let err = replay_smt_cert(&ctx, &cert).unwrap_err();
    assert!(matches!(err, KernelError::MissingObligationHash));
}

/// V8 — `replay_smt_cert_with_obligation` rejects certificates
/// whose `obligation_hash` doesn't match the caller-supplied
/// expected hash. Closes the doc/code mismatch where pre-V8
/// `replay_smt_cert` claimed (in its docstring) to perform this
/// comparison but actually only checked non-emptiness.
#[test]
fn smt_replay_with_obligation_rejects_mismatched_hash() {
    use verum_kernel::replay_smt_cert_with_obligation;
    let mut trace = List::new();
    trace.push(0x03);
    let cert = SmtCertificate::new(
        Text::from("z3"),
        Text::from("4.13.0"),
        trace,
        Text::from("sha256:cafebabe"),
    );
    let ctx = Context::new();
    let err = replay_smt_cert_with_obligation(&ctx, &cert, "sha256:deadbeef")
        .expect_err("hash mismatch must reject");
    match err {
        KernelError::ObligationHashMismatch { expected, actual } => {
            assert_eq!(expected.as_str(), "sha256:deadbeef");
            assert_eq!(actual.as_str(), "sha256:cafebabe");
        }
        other => panic!("expected ObligationHashMismatch, got {:?}", other),
    }
}

#[test]
fn smt_replay_with_obligation_accepts_matched_hash() {
    use verum_kernel::replay_smt_cert_with_obligation;
    let mut trace = List::new();
    trace.push(0x03);
    let cert = SmtCertificate::new(
        Text::from("z3"),
        Text::from("4.13.0"),
        trace,
        Text::from("sha256:abc123"),
    );
    let ctx = Context::new();
    let witness = replay_smt_cert_with_obligation(&ctx, &cert, "sha256:abc123")
        .expect("matched hash must succeed");
    // Witness shape unchanged from the non-comparison primitive.
    assert!(matches!(witness, CoreTerm::Axiom { .. }));
}

/// V8 — sanity: the original `replay_smt_cert` is still callable
/// without an expected hash for kernel-internal consumers (e.g.,
/// the `infer` arm for `SmtProof` doesn't yet have the goal at
/// type-inference time).
#[test]
fn smt_replay_no_obligation_still_callable() {
    let mut trace = List::new();
    trace.push(0x03);
    let cert = SmtCertificate::new(
        Text::from("z3"),
        Text::from("4.13.0"),
        trace,
        Text::from("sha256:foo"),
    );
    let ctx = Context::new();
    let witness = replay_smt_cert(&ctx, &cert).expect("non-comparison path must work");
    assert!(matches!(witness, CoreTerm::Axiom { .. }));
}

#[test]
fn smt_replay_refl_tag_produces_axiom_witness() {
    let mut trace = List::new();
    trace.push(0x01);
    let cert = SmtCertificate::new(
        Text::from("z3"),
        Text::from("4.13.0"),
        trace,
        Text::from("sha256:deadbeef"),
    );
    let ctx = Context::new();
    let term = replay_smt_cert(&ctx, &cert).unwrap();
    match term {
        CoreTerm::Axiom { name, framework, .. } => {
            assert!(name.as_str().starts_with("smt_cert:z3:refl:"));
            assert_eq!(framework.framework.as_str(), "z3:refl");
            assert_eq!(framework.citation.as_str(), "sha256:deadbeef");
        }
        other => panic!("expected Axiom, got {:?}", other),
    }
}

#[test]
fn smt_replay_accepts_cvc5_smt_unsat_tag() {
    let mut trace = List::new();
    trace.push(0x03);
    let cert = SmtCertificate::new(
        Text::from("cvc5"),
        Text::from("1.2.0"),
        trace,
        Text::from("sha256:feed"),
    );
    let ctx = Context::new();
    let term = replay_smt_cert(&ctx, &cert).unwrap();
    match term {
        CoreTerm::Axiom { framework, .. } => {
            assert_eq!(framework.framework.as_str(), "cvc5:smt_unsat");
        }
        other => panic!("expected Axiom, got {:?}", other),
    }
}

// -----------------------------------------------------------------
// SmtProof constructor tests (task #62)
// -----------------------------------------------------------------

#[test]
fn smtproof_infer_replays_certificate_and_returns_bool_type() {
    let mut trace = List::new();
    trace.push(0x03); // smt_unsat tag
    let cert = SmtCertificate::new(
        Text::from("z3"),
        Text::from("4.13.0"),
        trace,
        Text::from("sha256:deadbeef"),
    );
    let term = CoreTerm::SmtProof(cert);
    let ctx = Context::new();
    let reg = AxiomRegistry::new();
    let ty = infer(&ctx, &term, &reg).unwrap();
    assert_eq!(
        ty,
        CoreTerm::Inductive {
            path: Text::from("Bool"),
            args: List::new(),
        }
    );
}

#[test]
fn smtproof_infer_rejects_malformed_certificate() {
    // Empty trace → replay fails → infer surfaces EmptyCertificate.
    let cert = SmtCertificate::new(
        Text::from("z3"),
        Text::from("4.13.0"),
        List::new(),
        Text::from("sha256:x"),
    );
    let term = CoreTerm::SmtProof(cert);
    let ctx = Context::new();
    let reg = AxiomRegistry::new();
    let err = infer(&ctx, &term, &reg).unwrap_err();
    assert!(matches!(err, KernelError::EmptyCertificate));
}

#[test]
fn smtproof_infer_rejects_future_schema() {
    let mut trace = List::new();
    trace.push(0x01);
    let mut cert = SmtCertificate::new(
        Text::from("cvc5"),
        Text::from("1.2.0"),
        trace,
        Text::from("sha256:1"),
    );
    cert.schema_version = CERTIFICATE_SCHEMA_VERSION + 5;
    let term = CoreTerm::SmtProof(cert);
    let ctx = Context::new();
    let reg = AxiomRegistry::new();
    let err = infer(&ctx, &term, &reg).unwrap_err();
    assert!(matches!(
        err,
        KernelError::UnsupportedCertificateSchema { .. }
    ));
}

#[test]
fn smtproof_check_returns_bool_shape() {
    let mut trace = List::new();
    trace.push(0x01);
    let cert = SmtCertificate::new(
        Text::from("z3"),
        Text::from("4.13.0"),
        trace,
        Text::from("sha256:feed"),
    );
    let term = CoreTerm::SmtProof(cert);
    let ctx = Context::new();
    let reg = AxiomRegistry::new();
    let head = check(&ctx, &term, &reg).unwrap();
    assert_eq!(head, CoreType::Inductive(Text::from("Bool")));
}

// -----------------------------------------------------------------
// Envelope schema + metadata tests (task #75)
// -----------------------------------------------------------------

#[test]
fn new_certificate_stamps_current_schema_version() {
    let cert = SmtCertificate::new(
        Text::from("z3"),
        Text::from("4.13.0"),
        List::new(),
        Text::from("sha256:0"),
    );
    assert_eq!(cert.schema_version, CERTIFICATE_SCHEMA_VERSION);
    assert_eq!(cert.verum_version.as_str(), env!("CARGO_PKG_VERSION"));
    assert!(cert.metadata.is_empty());
    assert!(cert.created_at.as_str().is_empty());
}

#[test]
fn legacy_unversioned_certificate_still_validates() {
    let mut cert = SmtCertificate::new(
        Text::from("z3"),
        Text::from("4.13.0"),
        List::new(),
        Text::from("sha256:0"),
    );
    cert.schema_version = 0;
    assert!(cert.validate_schema().is_ok());
}

#[test]
fn future_schema_version_is_rejected() {
    let mut cert = SmtCertificate::new(
        Text::from("z3"),
        Text::from("4.13.0"),
        List::new(),
        Text::from("sha256:0"),
    );
    cert.schema_version = CERTIFICATE_SCHEMA_VERSION + 100;
    let err = cert.validate_schema().unwrap_err();
    assert!(matches!(
        err,
        KernelError::UnsupportedCertificateSchema { .. }
    ));
}

#[test]
fn replay_rejects_future_schema_version() {
    let mut trace = List::new();
    trace.push(0x01);
    let mut cert = SmtCertificate::new(
        Text::from("z3"),
        Text::from("4.13.0"),
        trace,
        Text::from("sha256:deadbeef"),
    );
    cert.schema_version = CERTIFICATE_SCHEMA_VERSION + 1;
    let err = replay_smt_cert(&Context::new(), &cert).unwrap_err();
    assert!(matches!(
        err,
        KernelError::UnsupportedCertificateSchema { found, max_supported }
            if found == CERTIFICATE_SCHEMA_VERSION + 1
                && max_supported == CERTIFICATE_SCHEMA_VERSION
    ));
}

#[test]
fn with_metadata_appends_keys_in_order() {
    let cert = SmtCertificate::new(
        Text::from("z3"),
        Text::from("4.13.0"),
        List::new(),
        Text::from("sha256:0"),
    )
    .with_metadata(Text::from("tactic"), Text::from("omega"))
    .with_metadata(Text::from("duration_ms"), Text::from("42"))
    .with_created_at(Text::from("2026-04-23T12:34:56Z"));
    assert_eq!(cert.metadata.len(), 2);
    assert_eq!(cert.metadata[0].0.as_str(), "tactic");
    assert_eq!(cert.metadata[0].1.as_str(), "omega");
    assert_eq!(cert.metadata[1].0.as_str(), "duration_ms");
    assert_eq!(cert.created_at.as_str(), "2026-04-23T12:34:56Z");
}

#[test]
fn serde_roundtrip_preserves_all_envelope_fields() {
    let cert = SmtCertificate::new(
        Text::from("cvc5"),
        Text::from("1.2.0"),
        {
            let mut t = List::new();
            t.push(0x03);
            t
        },
        Text::from("sha256:feed"),
    )
    .with_metadata(Text::from("solver_opts"), Text::from("--produce-proofs"))
    .with_created_at(Text::from("2026-04-23T00:00:00Z"));
    let json = serde_json::to_string(&cert).unwrap();
    let rehydrated: SmtCertificate = serde_json::from_str(&json).unwrap();
    assert_eq!(rehydrated, cert);
}

// -----------------------------------------------------------------
// Dependent-type rules — Pi / Lam / App + substitution
// -----------------------------------------------------------------

/// `Type(0) : Type(1)`.
#[test]
fn universe_inhabits_successor() {
    let ctx = Context::new();
    let ax = AxiomRegistry::new();
    let ty = infer(
        &ctx,
        &CoreTerm::Universe(UniverseLevel::Concrete(0)),
        &ax,
    )
    .unwrap();
    assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(1)));
}

/// Polymorphic identity: `λ (A : Type) (x : A). x : (A : Type) → A → A`.
///
/// This is the canonical smoke test for Π-introduction + App-elim
/// with capture-avoiding substitution. `infer` must return the
/// exact Π-type (not a shape-head abstraction) so App can destructure.
#[test]
fn polymorphic_identity_types_correctly() {
    let ax = AxiomRegistry::new();
    let id_lam = CoreTerm::Lam {
        binder: Text::from("A"),
        domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
        body: Heap::new(CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(CoreTerm::Var(Text::from("A"))),
            body: Heap::new(CoreTerm::Var(Text::from("x"))),
        }),
    };
    let ty = infer(&Context::new(), &id_lam, &ax).unwrap();
    // Expect: Pi (A : Type(0)) (Pi (x : A) A)
    assert!(matches!(
        ty,
        CoreTerm::Pi { ref binder, .. } if binder.as_str() == "A"
    ));
    let outer_codom = match ty {
        CoreTerm::Pi { codomain, .. } => codomain,
        _ => unreachable!(),
    };
    assert!(matches!(
        *outer_codom,
        CoreTerm::Pi { ref binder, .. } if binder.as_str() == "x"
    ));
}

/// `(λ (x : Unit). x) tt : Unit`  — App + beta-style substitution.
#[test]
fn application_of_identity_substitutes_argument() {
    let ax = AxiomRegistry::new();
    let tt = CoreTerm::Axiom {
        name: Text::from("tt"),
        ty: Heap::new(unit_ty()),
        framework: FrameworkId {
            framework: Text::from("builtin"),
            citation: Text::from("unit-introduction"),
        },
    };
    let mut reg = AxiomRegistry::new();
    reg.register(
        Text::from("tt"),
        unit_ty(),
        FrameworkId {
            framework: Text::from("builtin"),
            citation: Text::from("unit-introduction"),
        },
    )
    .unwrap();
    let _ = ax; // keep ax handle for compile cleanliness
    let id_lam = CoreTerm::Lam {
        binder: Text::from("x"),
        domain: Heap::new(unit_ty()),
        body: Heap::new(CoreTerm::Var(Text::from("x"))),
    };
    let applied = CoreTerm::App(Heap::new(id_lam), Heap::new(tt));
    let ty = infer(&Context::new(), &applied, &reg).unwrap();
    assert_eq!(ty, unit_ty());
}

/// App with domain mismatch is rejected.
#[test]
fn application_with_type_mismatch_rejects() {
    let mut reg = AxiomRegistry::new();
    // Register `zero` of type Nat, `tt` of type Unit.
    let nat_ty = CoreTerm::Inductive {
        path: Text::from("Nat"),
        args: List::new(),
    };
    reg.register(
        Text::from("zero"),
        nat_ty.clone(),
        FrameworkId {
            framework: Text::from("builtin"),
            citation: Text::from("nat-introduction"),
        },
    )
    .unwrap();
    reg.register(
        Text::from("tt"),
        unit_ty(),
        FrameworkId {
            framework: Text::from("builtin"),
            citation: Text::from("unit-introduction"),
        },
    )
    .unwrap();

    // λ (x : Nat). x — identity over Nat
    let id_nat = CoreTerm::Lam {
        binder: Text::from("x"),
        domain: Heap::new(nat_ty),
        body: Heap::new(CoreTerm::Var(Text::from("x"))),
    };
    let tt = CoreTerm::Axiom {
        name: Text::from("tt"),
        ty: Heap::new(unit_ty()),
        framework: FrameworkId {
            framework: Text::from("builtin"),
            citation: Text::from("unit-introduction"),
        },
    };
    // (λ (x : Nat). x) tt  — tt is Unit, not Nat → must error.
    let applied = CoreTerm::App(Heap::new(id_nat), Heap::new(tt));
    let err = infer(&Context::new(), &applied, &reg).unwrap_err();
    assert!(matches!(err, KernelError::TypeMismatch { .. }));
}

/// Applying a non-function term produces NotAFunction.
#[test]
fn application_of_non_function_rejects() {
    let mut reg = AxiomRegistry::new();
    reg.register(
        Text::from("tt"),
        unit_ty(),
        FrameworkId {
            framework: Text::from("builtin"),
            citation: Text::from("unit-introduction"),
        },
    )
    .unwrap();
    let tt = CoreTerm::Axiom {
        name: Text::from("tt"),
        ty: Heap::new(unit_ty()),
        framework: FrameworkId {
            framework: Text::from("builtin"),
            citation: Text::from("unit-introduction"),
        },
    };
    let applied = CoreTerm::App(Heap::new(tt.clone()), Heap::new(tt));
    let err = infer(&Context::new(), &applied, &reg).unwrap_err();
    assert!(matches!(err, KernelError::NotAFunction(_)));
}

/// Substitution does not cross a shadowing binder.
#[test]
fn substitute_stops_at_shadowing_binder() {
    let inner = CoreTerm::Lam {
        binder: Text::from("x"),
        domain: Heap::new(unit_ty()),
        body: Heap::new(CoreTerm::Var(Text::from("x"))),
    };
    let replaced = substitute(&inner, "x", &unit_ty());
    // The bound `x` inside the lambda must NOT be replaced.
    match &replaced {
        CoreTerm::Lam { body, .. } => {
            assert_eq!(**body, CoreTerm::Var(Text::from("x")));
        }
        _ => panic!("expected a lambda"),
    }
}

/// Substitution replaces free occurrences.
#[test]
fn substitute_replaces_free_occurrences() {
    let term = CoreTerm::App(
        Heap::new(CoreTerm::Var(Text::from("f"))),
        Heap::new(CoreTerm::Var(Text::from("a"))),
    );
    let replaced = substitute(&term, "a", &unit_ty());
    match replaced {
        CoreTerm::App(f, a) => {
            assert_eq!(*f, CoreTerm::Var(Text::from("f")));
            assert_eq!(*a, unit_ty());
        }
        _ => panic!("expected App"),
    }
}

/// Full-structural verify: identity lambda has the exact Π type.
#[test]
fn verify_full_accepts_matching_type() {
    let ax = AxiomRegistry::new();
    let id_unit = CoreTerm::Lam {
        binder: Text::from("x"),
        domain: Heap::new(unit_ty()),
        body: Heap::new(CoreTerm::Var(Text::from("x"))),
    };
    let expected = CoreTerm::Pi {
        binder: Text::from("x"),
        domain: Heap::new(unit_ty()),
        codomain: Heap::new(unit_ty()),
    };
    verify_full(&Context::new(), &id_unit, &expected, &ax).unwrap();
}

#[test]
fn verify_full_rejects_mismatched_type() {
    let ax = AxiomRegistry::new();
    let id_unit = CoreTerm::Lam {
        binder: Text::from("x"),
        domain: Heap::new(unit_ty()),
        body: Heap::new(CoreTerm::Var(Text::from("x"))),
    };
    let wrong = CoreTerm::Universe(UniverseLevel::Concrete(0));
    let err =
        verify_full(&Context::new(), &id_unit, &wrong, &ax).unwrap_err();
    assert!(matches!(err, KernelError::TypeMismatch { .. }));
}

// -----------------------------------------------------------------
// Σ-type rules — Sigma / Pair / Fst / Snd
// -----------------------------------------------------------------

fn tt_axiom(reg: &mut AxiomRegistry) -> CoreTerm {
    let fw = FrameworkId {
        framework: Text::from("builtin"),
        citation: Text::from("unit-introduction"),
    };
    let _ = reg.register(Text::from("tt"), unit_ty(), fw.clone());
    CoreTerm::Axiom {
        name: Text::from("tt"),
        ty: Heap::new(unit_ty()),
        framework: fw,
    }
}

/// Σ-formation: `Sigma (x : Unit) Unit` is a type.
#[test]
fn sigma_formation_returns_universe() {
    let ax = AxiomRegistry::new();
    let sigma = CoreTerm::Sigma {
        binder: Text::from("x"),
        fst_ty: Heap::new(unit_ty()),
        snd_ty: Heap::new(unit_ty()),
    };
    let ty = infer(&Context::new(), &sigma, &ax).unwrap();
    assert!(matches!(ty, CoreTerm::Universe(_)));
}

/// Non-dependent Pair: (tt, tt) : Sigma (_:Unit) Unit.
#[test]
fn pair_introduction_builds_sigma() {
    let mut reg = AxiomRegistry::new();
    let tt = tt_axiom(&mut reg);
    let pair = CoreTerm::Pair(Heap::new(tt.clone()), Heap::new(tt));
    let ty = infer(&Context::new(), &pair, &reg).unwrap();
    match ty {
        CoreTerm::Sigma { fst_ty, snd_ty, .. } => {
            assert_eq!(*fst_ty, unit_ty());
            assert_eq!(*snd_ty, unit_ty());
        }
        _ => panic!("expected Sigma"),
    }
}

/// `fst((tt, tt)) : Unit`.
#[test]
fn fst_projection_types_to_first_component() {
    let mut reg = AxiomRegistry::new();
    let tt = tt_axiom(&mut reg);
    let pair = CoreTerm::Pair(Heap::new(tt.clone()), Heap::new(tt));
    let fst = CoreTerm::Fst(Heap::new(pair));
    let ty = infer(&Context::new(), &fst, &reg).unwrap();
    assert_eq!(ty, unit_ty());
}

/// `snd((tt, tt)) : Unit` — since the Σ-type is non-dependent,
/// substitution doesn't change anything and we still get Unit.
#[test]
fn snd_projection_types_to_second_component() {
    let mut reg = AxiomRegistry::new();
    let tt = tt_axiom(&mut reg);
    let pair = CoreTerm::Pair(Heap::new(tt.clone()), Heap::new(tt));
    let snd = CoreTerm::Snd(Heap::new(pair));
    let ty = infer(&Context::new(), &snd, &reg).unwrap();
    assert_eq!(ty, unit_ty());
}

/// `fst(tt)` — tt : Unit is not a pair — rejected.
#[test]
fn fst_of_non_pair_rejects() {
    let mut reg = AxiomRegistry::new();
    let tt = tt_axiom(&mut reg);
    let wrong = CoreTerm::Fst(Heap::new(tt));
    let err = infer(&Context::new(), &wrong, &reg).unwrap_err();
    assert!(matches!(err, KernelError::NotAPair(_)));
}

// -----------------------------------------------------------------
// Cubical Path-type rules — PathTy / Refl
// -----------------------------------------------------------------

/// `Path<Unit>(tt, tt) : Type(0)`.
#[test]
fn path_formation_returns_carrier_universe() {
    let mut reg = AxiomRegistry::new();
    let tt = tt_axiom(&mut reg);
    let path = CoreTerm::PathTy {
        carrier: Heap::new(unit_ty()),
        lhs: Heap::new(tt.clone()),
        rhs: Heap::new(tt),
    };
    let ty = infer(&Context::new(), &path, &reg).unwrap();
    assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(0)));
}

/// `refl(tt) : Path<Unit>(tt, tt)`.
#[test]
fn refl_produces_path_type_with_identical_endpoints() {
    let mut reg = AxiomRegistry::new();
    let tt = tt_axiom(&mut reg);
    let refl = CoreTerm::Refl(Heap::new(tt.clone()));
    let ty = infer(&Context::new(), &refl, &reg).unwrap();
    match ty {
        CoreTerm::PathTy { carrier, lhs, rhs } => {
            assert_eq!(*carrier, unit_ty());
            assert_eq!(*lhs, tt);
            assert_eq!(*rhs, tt);
        }
        _ => panic!("expected PathTy"),
    }
}

// -----------------------------------------------------------------
// Refinement / cubical / Elim — bring-up rules
// -----------------------------------------------------------------

/// `{x : Unit | tt} : Type(0)` — refinement formation.
#[test]
fn refine_formation_returns_base_universe() {
    let mut reg = AxiomRegistry::new();
    let tt = tt_axiom(&mut reg);
    let refined = CoreTerm::Refine {
        base: Heap::new(unit_ty()),
        binder: Text::from("x"),
        predicate: Heap::new(tt),
    };
    let ty = infer(&Context::new(), &refined, &reg).unwrap();
    assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(0)));
}

/// `hcomp φ walls tt : Unit`. The strengthened rule requires
/// phi/walls to be well-typed; bind them in the context so
/// inference succeeds.
#[test]
fn hcomp_infers_base_type() {
    let mut reg = AxiomRegistry::new();
    let tt = tt_axiom(&mut reg);
    let ctx = Context::new()
        .extend(Text::from("phi"), unit_ty())
        .extend(Text::from("walls"), unit_ty());
    let hc = CoreTerm::HComp {
        phi: Heap::new(CoreTerm::Var(Text::from("phi"))),
        walls: Heap::new(CoreTerm::Var(Text::from("walls"))),
        base: Heap::new(tt),
    };
    let ty = infer(&ctx, &hc, &reg).unwrap();
    assert_eq!(ty, unit_ty());
}

/// HComp with an ill-typed `phi` is rejected — the strengthened
/// rule no longer swallows subterm errors.
#[test]
fn hcomp_rejects_unbound_phi() {
    let mut reg = AxiomRegistry::new();
    let tt = tt_axiom(&mut reg);
    let hc = CoreTerm::HComp {
        phi: Heap::new(CoreTerm::Var(Text::from("phi_unbound"))),
        walls: Heap::new(tt.clone()),
        base: Heap::new(tt),
    };
    let err = infer(&Context::new(), &hc, &reg).unwrap_err();
    assert!(matches!(err, KernelError::UnboundVariable(_)));
}

/// `transp path r tt` — returns the path's right endpoint type.
#[test]
fn transp_returns_path_rhs_type() {
    let mut reg = AxiomRegistry::new();
    let tt = tt_axiom(&mut reg);
    let ctx = Context::new().extend(Text::from("r"), unit_ty());
    let path = CoreTerm::PathTy {
        carrier: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
        lhs: Heap::new(unit_ty()),
        rhs: Heap::new(unit_ty()),
    };
    let tr = CoreTerm::Transp {
        path: Heap::new(path),
        regular: Heap::new(CoreTerm::Var(Text::from("r"))),
        value: Heap::new(tt),
    };
    let ty = infer(&ctx, &tr, &reg).unwrap();
    assert_eq!(ty, unit_ty());
}

/// Transp with an unbound regular endpoint is rejected.
#[test]
fn transp_rejects_unbound_regular() {
    let mut reg = AxiomRegistry::new();
    let tt = tt_axiom(&mut reg);
    let path = CoreTerm::PathTy {
        carrier: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
        lhs: Heap::new(unit_ty()),
        rhs: Heap::new(unit_ty()),
    };
    let tr = CoreTerm::Transp {
        path: Heap::new(path),
        regular: Heap::new(CoreTerm::Var(Text::from("r_unbound"))),
        value: Heap::new(tt),
    };
    let err = infer(&Context::new(), &tr, &reg).unwrap_err();
    assert!(matches!(err, KernelError::UnboundVariable(_)));
}

/// `Glue<Unit>(…)` inhabits the universe of its carrier.
#[test]
fn glue_returns_carrier_universe() {
    let ax = AxiomRegistry::new();
    let ctx = Context::new()
        .extend(Text::from("phi"), unit_ty())
        .extend(Text::from("fiber"), unit_ty())
        .extend(Text::from("equiv"), unit_ty());
    let g = CoreTerm::Glue {
        carrier: Heap::new(unit_ty()),
        phi: Heap::new(CoreTerm::Var(Text::from("phi"))),
        fiber: Heap::new(CoreTerm::Var(Text::from("fiber"))),
        equiv: Heap::new(CoreTerm::Var(Text::from("equiv"))),
    };
    let ty = infer(&ctx, &g, &ax).unwrap();
    assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(0)));
}

/// Glue whose equiv is ill-typed — rejected.
#[test]
fn glue_rejects_unbound_equiv() {
    let ax = AxiomRegistry::new();
    let ctx = Context::new()
        .extend(Text::from("phi"), unit_ty())
        .extend(Text::from("fiber"), unit_ty());
    let g = CoreTerm::Glue {
        carrier: Heap::new(unit_ty()),
        phi: Heap::new(CoreTerm::Var(Text::from("phi"))),
        fiber: Heap::new(CoreTerm::Var(Text::from("fiber"))),
        equiv: Heap::new(CoreTerm::Var(Text::from("equiv_unbound"))),
    };
    let err = infer(&ctx, &g, &ax).unwrap_err();
    assert!(matches!(err, KernelError::UnboundVariable(_)));
}

/// Glue whose carrier is not in a universe is rejected.
#[test]
fn glue_rejects_non_universe_carrier() {
    let mut reg = AxiomRegistry::new();
    // tt is of type Unit, which is not a universe.
    let tt = tt_axiom(&mut reg);
    let ctx = Context::new()
        .extend(Text::from("phi"), unit_ty())
        .extend(Text::from("fiber"), unit_ty())
        .extend(Text::from("equiv"), unit_ty());
    let g = CoreTerm::Glue {
        // Glue expects carrier to inhabit a universe. `tt : Unit`
        // inhabits Unit (a type), not a universe, so
        // `universe_level` rejects it.
        carrier: Heap::new(tt),
        phi: Heap::new(CoreTerm::Var(Text::from("phi"))),
        fiber: Heap::new(CoreTerm::Var(Text::from("fiber"))),
        equiv: Heap::new(CoreTerm::Var(Text::from("equiv"))),
    };
    // This should fail because tt's type (Unit) is not a universe.
    let res = infer(&ctx, &g, &reg);
    assert!(res.is_err());
}

/// `elim e motive cases : motive e` — shape-level Elim rule.
///
/// V1 (#207-adjacent, this file's commit): Elim now requires
/// motive's TYPE be a Π and scrutinee's type to match the Π's
/// domain. Result remains the syntactic `App(motive, scrutinee)`
/// (β-reduction is downstream's job; returning the codomain would
/// give the type's TYPE — i.e., a universe — instead).
#[test]
fn elim_types_to_motive_applied_to_scrutinee() {
    let mut reg = AxiomRegistry::new();
    let tt = tt_axiom(&mut reg);
    // motive : Unit → Type(0) — represented as λ(u:Unit). Unit
    let motive = CoreTerm::Lam {
        binder: Text::from("u"),
        domain: Heap::new(unit_ty()),
        body: Heap::new(unit_ty()),
    };
    let e = CoreTerm::Elim {
        scrutinee: Heap::new(tt.clone()),
        motive: Heap::new(motive.clone()),
        cases: List::new(),
    };
    let ty = infer(&Context::new(), &e, &reg).unwrap();
    match ty {
        CoreTerm::App(f, a) => {
            assert_eq!(*f, motive);
            assert_eq!(*a, tt);
        }
        _ => panic!("expected App (motive scrutinee)"),
    }
}

/// V1 — Elim with a non-Π-typed motive must reject. Pre-V1 the
/// kernel happily returned `App(42, scrutinee)` syntactically.
#[test]
fn elim_rejects_non_function_motive() {
    let mut reg = AxiomRegistry::new();
    let tt = tt_axiom(&mut reg);
    // Non-function motive: a bare `tt` (type Unit, not a Π).
    let bad_motive = tt.clone();
    let e = CoreTerm::Elim {
        scrutinee: Heap::new(tt),
        motive: Heap::new(bad_motive),
        cases: List::new(),
    };
    let res = infer(&Context::new(), &e, &reg);
    assert!(matches!(
        res,
        Err(verum_kernel::KernelError::NotAFunction(_))
    ));
}

/// V1 — Elim where the scrutinee's type doesn't match the
/// motive's domain must reject. Pre-V1 silently accepted.
#[test]
fn elim_rejects_scrutinee_domain_mismatch() {
    use verum_kernel::{CoreTerm, UniverseLevel};
    let mut reg = AxiomRegistry::new();
    let tt = tt_axiom(&mut reg);
    // motive : Type(0) → Type(0) — but scrutinee is `tt : Unit`.
    let motive = CoreTerm::Lam {
        binder: Text::from("X"),
        domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
        body: Heap::new(CoreTerm::Var(Text::from("X"))),
    };
    let e = CoreTerm::Elim {
        scrutinee: Heap::new(tt),
        motive: Heap::new(motive),
        cases: List::new(),
    };
    let res = infer(&Context::new(), &e, &reg);
    assert!(matches!(
        res,
        Err(verum_kernel::KernelError::TypeMismatch { .. })
    ));
}

/// `Path<Unit>(tt, someBool)` — endpoint type mismatch — rejected.
/// Demonstrates that the kernel checks endpoint types against the
/// declared carrier, not just shape.
#[test]
fn path_rejects_endpoint_type_mismatch() {
    let mut reg = AxiomRegistry::new();
    let tt = tt_axiom(&mut reg);
    // Register an axiom of a *different* type.
    let bool_ty = CoreTerm::Inductive {
        path: Text::from("Bool"),
        args: List::new(),
    };
    reg.register(
        Text::from("true_val"),
        bool_ty.clone(),
        FrameworkId {
            framework: Text::from("builtin"),
            citation: Text::from("bool-introduction"),
        },
    )
    .unwrap();
    let true_val = CoreTerm::Axiom {
        name: Text::from("true_val"),
        ty: Heap::new(bool_ty),
        framework: FrameworkId {
            framework: Text::from("builtin"),
            citation: Text::from("bool-introduction"),
        },
    };
    let bogus = CoreTerm::PathTy {
        carrier: Heap::new(unit_ty()),
        lhs: Heap::new(tt),
        rhs: Heap::new(true_val),
    };
    let err = infer(&Context::new(), &bogus, &reg).unwrap_err();
    assert!(matches!(err, KernelError::TypeMismatch { .. }));
}

// -----------------------------------------------------------------
// FrameworkAttr → AxiomRegistry loader
// -----------------------------------------------------------------

/// Helper: build a parsed module with one `@framework(id, "cite")`
/// axiom declaration.
fn module_with_axiom(
    framework_name: &str,
    citation: &str,
    axiom_name: &str,
) -> verum_ast::Module {
    use verum_ast::attr::Attribute;
    use verum_ast::decl::{AxiomDecl, Visibility};
    use verum_ast::expr::{Expr, ExprKind};
    use verum_ast::literal::{Literal, LiteralKind, StringLit};
    use verum_ast::span::Span;
    use verum_ast::{Ident, Item, ItemKind};

    let span = Span::default();
    let name_expr = Expr::ident(Ident::new(Text::from(framework_name), span));
    let cite_lit = Literal::new(
        LiteralKind::Text(StringLit::Regular(Text::from(citation))),
        span,
    );
    let cite_expr = Expr::literal(cite_lit);

    let mut args: List<Expr> = List::new();
    args.push(name_expr);
    args.push(cite_expr);
    let framework_attr =
        Attribute::new(Text::from("framework"), Maybe::Some(args), span);

    let mut attrs: List<Attribute> = List::new();
    attrs.push(framework_attr);

    // Minimal AxiomDecl — body and clauses stay empty; the
    // loader only inspects name + attributes.
    let axiom_ident = Ident::new(Text::from(axiom_name), span);
    let proposition = Expr::literal(Literal::new(
        LiteralKind::Bool(true),
        span,
    ));
    let decl = AxiomDecl::new(axiom_ident, proposition, span);
    let mut decl = decl;
    decl.visibility = Visibility::Public;
    decl.attributes = attrs.clone();

    let item = Item {
        kind: ItemKind::Axiom(decl),
        attributes: List::new(),
        span,
    };

    let mut items: List<Item> = List::new();
    items.push(item);
    verum_ast::Module {
        items,
        span,
        file_id: verum_ast::span::FileId::new(0),
        attributes: List::new(),
    }
}

#[test]
fn load_framework_axioms_registers_single_marker() {
    let module = module_with_axiom(
        "lurie_htt",
        "HTT 6.2.2.7",
        "sheafification_is_topos",
    );
    let mut reg = AxiomRegistry::new();
    let report = load_framework_axioms(&module, &mut reg);

    assert!(report.is_clean(), "expected clean load, got {:?}", report);
    assert_eq!(report.registered.len(), 1);
    assert_eq!(
        report.registered.get(0).map(|t| t.as_str()),
        Some("sheafification_is_topos")
    );

    match reg.get("sheafification_is_topos") {
        Maybe::Some(entry) => {
            assert_eq!(entry.framework.framework.as_str(), "lurie_htt");
            assert_eq!(entry.framework.citation.as_str(), "HTT 6.2.2.7");
        }
        Maybe::None => panic!("axiom not registered"),
    }
}

#[test]
fn load_framework_axioms_detects_duplicate() {
    let m1 = module_with_axiom(
        "lurie_htt",
        "HTT 6.2.2.7",
        "sheafification_is_topos",
    );
    let m2 = module_with_axiom(
        "schreiber_dcct",
        "DCCT §3.9",
        "sheafification_is_topos", // same name — collision
    );
    let mut reg = AxiomRegistry::new();
    let r1 = load_framework_axioms(&m1, &mut reg);
    assert!(r1.is_clean());

    let r2 = load_framework_axioms(&m2, &mut reg);
    assert_eq!(r2.duplicates.len(), 1);
    assert_eq!(
        r2.duplicates.get(0).map(|t| t.as_str()),
        Some("sheafification_is_topos")
    );
    assert!(r2.registered.is_empty());
}

#[test]
fn load_framework_axioms_skips_non_axiom_items() {
    // A theorem with @framework is NOT auto-registered — the
    // loader only consumes axioms. (Theorems are consumers, not
    // postulates, so their elaborator path handles registration
    // when a proof term is submitted.)
    use verum_ast::attr::Attribute;
    use verum_ast::decl::{TheoremDecl, Visibility};
    use verum_ast::expr::{Expr, ExprKind};
    use verum_ast::literal::{Literal, LiteralKind, StringLit};
    use verum_ast::span::Span;
    use verum_ast::{Ident, Item, ItemKind};

    let span = Span::default();

    let framework_attr = {
        let name_expr = Expr::ident(Ident::new(Text::from("lurie_htt"), span));
        let cite_lit = Literal::new(
            LiteralKind::Text(StringLit::Regular(Text::from("HTT 6.2.2.7"))),
            span,
        );
        let cite_expr = Expr::literal(cite_lit);
        let mut args: List<Expr> = List::new();
        args.push(name_expr);
        args.push(cite_expr);
        Attribute::new(Text::from("framework"), Maybe::Some(args), span)
    };
    let mut attrs: List<Attribute> = List::new();
    attrs.push(framework_attr);

    let theorem_ident = Ident::new(Text::from("some_theorem"), span);
    let mut thm = TheoremDecl::new(
        theorem_ident,
        Expr::literal(Literal::new(LiteralKind::Bool(true), span)),
        span,
    );
    thm.visibility = Visibility::Public;
    thm.attributes = attrs;

    let item = Item {
        kind: ItemKind::Theorem(thm),
        attributes: List::new(),
        span,
    };

    let mut items: List<Item> = List::new();
    items.push(item);
    let module = verum_ast::Module {
        items,
        span,
        file_id: verum_ast::span::FileId::new(0),
        attributes: List::new(),
    };

    let mut reg = AxiomRegistry::new();
    let report = load_framework_axioms(&module, &mut reg);
    assert!(report.is_clean());
    assert!(report.registered.is_empty());
    assert_eq!(reg.all().len(), 0);
}

#[test]
fn axiom_after_load_is_checkable_by_infer() {
    // End-to-end: load a framework axiom, then successfully
    // check a CoreTerm::Axiom that references it.
    let module = module_with_axiom(
        "connes_reconstruction",
        "Connes 2008 axiom (vii)",
        "first_order_condition",
    );
    let mut reg = AxiomRegistry::new();
    let report = load_framework_axioms(&module, &mut reg);
    assert!(report.is_clean());

    let term = CoreTerm::Axiom {
        name: Text::from("first_order_condition"),
        ty: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
        framework: FrameworkId {
            framework: Text::from("connes_reconstruction"),
            citation: Text::from("Connes 2008 axiom (vii)"),
        },
    };
    let ty = infer(&Context::new(), &term, &reg).unwrap();
    assert!(matches!(ty, CoreTerm::Universe(_)));
}

// -------------------------------------------------------------
// Rule 10: UIP-Free — axioms reducing to UIP are rejected.
// -------------------------------------------------------------

/// Build the direct UIP statement:
/// `Π A. Π a. Π b. Π p. Π q. PathTy(PathTy(A, a, b), p, q)`.
fn uip_statement() -> CoreTerm {
    fn var(n: &str) -> CoreTerm {
        CoreTerm::Var(Text::from(n))
    }
    let path_a_a_b = CoreTerm::PathTy {
        carrier: Heap::new(var("A")),
        lhs: Heap::new(var("a")),
        rhs: Heap::new(var("b")),
    };
    let path_of_paths = CoreTerm::PathTy {
        carrier: Heap::new(path_a_a_b.clone()),
        lhs: Heap::new(var("p")),
        rhs: Heap::new(var("q")),
    };
    let pi_q = CoreTerm::Pi {
        binder: Text::from("q"),
        domain: Heap::new(path_a_a_b.clone()),
        codomain: Heap::new(path_of_paths),
    };
    let pi_p = CoreTerm::Pi {
        binder: Text::from("p"),
        domain: Heap::new(path_a_a_b),
        codomain: Heap::new(pi_q),
    };
    let pi_b = CoreTerm::Pi {
        binder: Text::from("b"),
        domain: Heap::new(var("A")),
        codomain: Heap::new(pi_p),
    };
    let pi_a_val = CoreTerm::Pi {
        binder: Text::from("a"),
        domain: Heap::new(var("A")),
        codomain: Heap::new(pi_b),
    };
    CoreTerm::Pi {
        binder: Text::from("A"),
        domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
        codomain: Heap::new(pi_a_val),
    }
}

#[test]
fn uip_axiom_is_rejected_by_register() {
    let mut reg = AxiomRegistry::new();
    let result = reg.register(
        Text::from("uip"),
        uip_statement(),
        FrameworkId {
            framework: Text::from("set_level"),
            citation: Text::from("attempted UIP postulate"),
        },
    );
    match result {
        Err(KernelError::UipForbidden(name)) => {
            assert_eq!(name.as_str(), "uip");
        }
        other => panic!("expected UipForbidden, got {:?}", other),
    }
    assert_eq!(reg.all().len(), 0);
}

#[test]
fn non_uip_axiom_is_accepted() {
    // A plain axiom claiming a proposition about a universe
    // is not UIP and must not be rejected by rule 10.
    let mut reg = AxiomRegistry::new();
    let result = reg.register(
        Text::from("some_axiom"),
        CoreTerm::Universe(UniverseLevel::Concrete(0)),
        FrameworkId {
            framework: Text::from("test"),
            citation: Text::from("test"),
        },
    );
    assert!(result.is_ok());
    assert_eq!(reg.all().len(), 1);
}

#[test]
fn single_pi_over_path_is_not_uip() {
    // A single Π over a path type is NOT UIP — guard should not
    // false-positive on partial shapes.
    fn var(n: &str) -> CoreTerm {
        CoreTerm::Var(Text::from(n))
    }
    let almost = CoreTerm::Pi {
        binder: Text::from("A"),
        domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
        codomain: Heap::new(CoreTerm::PathTy {
            carrier: Heap::new(var("A")),
            lhs: Heap::new(var("a")),
            rhs: Heap::new(var("b")),
        }),
    };
    let mut reg = AxiomRegistry::new();
    let result = reg.register(
        Text::from("path_forall"),
        almost,
        FrameworkId {
            framework: Text::from("test"),
            citation: Text::from("test"),
        },
    );
    assert!(result.is_ok(), "partial shape must not trigger UIP guard");
}
