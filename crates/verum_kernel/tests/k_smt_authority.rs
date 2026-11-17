//! Solver provenance is not proof authority. These are executable admission
//! controls, not a theorem about all Rust inference or SMT reconstruction.
use std::{fs, process::Command};

use verum_common::{List, Text};
use verum_kernel::soundness::{
    SoundnessExporter, agda::AgdaBackend, coq::CoqBackend, isabelle::IsabelleBackend,
    lean::LeanBackend,
};
use verum_kernel::{
    AxiomRegistry, Context, CoreTerm, KernelError, SmtCertificate, infer, replay_smt_cert,
    replay_smt_cert_with_obligation,
};
use verum_proof::proof_checker::{Certificate, Term};

#[test]
fn tree_diagnostics_preserve_malformed_and_resource_refusals() {
    use verum_kernel::proof_tree::{ProofNode, parse_sexpr, replay_aletha_tree, replay_z3_tree};
    use verum_proof::limits::AdmissionLimits;
    let limits = AdmissionLimits::DEFAULT;
    let nested = parse_sexpr("(mp (refl x) (fabricated x))").unwrap();
    assert!(matches!(
        replay_z3_tree(&nested),
        Err(KernelError::UnknownRule { .. })
    ));
    let nested = parse_sexpr("(resolution (step t1 :rule fabricated))").unwrap();
    assert!(matches!(
        replay_aletha_tree(&nested),
        Err(KernelError::UnknownRule { .. })
    ));
    for tree in [
        ProofNode::Atom(Text::from("refl")),
        ProofNode::List(List::new()),
    ] {
        assert!(matches!(
            replay_z3_tree(&tree),
            Err(KernelError::SmtReplayFailed { .. })
        ));
        assert!(matches!(
            replay_aletha_tree(&tree),
            Err(KernelError::SmtReplayFailed { .. })
        ));
    }
    let mut deep = ProofNode::Atom(Text::from("x"));
    for _ in 0..limits.max_depth {
        deep = ProofNode::List(
            [ProofNode::Atom(Text::from("refl")), deep]
                .into_iter()
                .collect(),
        );
    }
    assert!(matches!(
        replay_z3_tree(&deep),
        Err(KernelError::UnsupportedSmtProofTree { .. })
    ));
    let deep = ProofNode::List(
        [ProofNode::Atom(Text::from("refl")), deep]
            .into_iter()
            .collect(),
    );
    assert!(
        matches!(replay_z3_tree(&deep), Err(KernelError::SmtProofResourceLimit { resource: "depth", limit }) if limit == limits.max_depth)
    );
    let wide = ProofNode::List(
        std::iter::once(ProofNode::Atom(Text::from("refl")))
            .chain((0..limits.max_nodes).map(|_| ProofNode::Atom(Text::from("x"))))
            .collect(),
    );
    assert!(
        matches!(replay_z3_tree(&wide), Err(KernelError::SmtProofResourceLimit { resource: "nodes", limit }) if limit == limits.max_nodes)
    );
    // A missing :rule must not cause an unbounded scan before the size check.
    let oversized_step = ProofNode::List(
        std::iter::once(ProofNode::Atom(Text::from("step")))
            .chain((0..limits.max_nodes).map(|_| ProofNode::Atom(Text::from("x"))))
            .collect(),
    );
    assert!(matches!(
        replay_aletha_tree(&oversized_step),
        Err(KernelError::SmtProofResourceLimit {
            resource: "nodes",
            ..
        })
    ));
    let payload = ProofNode::List(
        [
            ProofNode::Atom(Text::from("step")),
            ProofNode::Atom(Text::from("t1")),
            ProofNode::Atom(Text::from(":rule")),
            ProofNode::Atom(Text::from("refl")),
            ProofNode::List(
                std::iter::once(ProofNode::Atom(Text::from(
                    "x".repeat(limits.max_bytes + 1),
                )))
                .collect(),
            ),
        ]
        .into_iter()
        .collect(),
    );
    assert!(
        matches!(replay_aletha_tree(&payload), Err(KernelError::SmtProofResourceLimit { resource: "bytes", limit }) if limit == limits.max_bytes)
    );
}

#[test]
fn recognized_solver_envelopes_never_manufacture_proof_or_bool() {
    let ctx = Context::new();
    let axioms = AxiomRegistry::new();
    for backend in ["z3", "cvc5", "portfolio", "tactic"] {
        for tag in 1..=3 {
            for suffix in [&b""[..], &b"unsat"[..], &b"forged-proof"[..]] {
                let trace: List<_> = std::iter::once(tag).chain(suffix.iter().copied()).collect();
                let cert = SmtCertificate::new(
                    Text::from(backend),
                    Text::from("untrusted"),
                    trace,
                    Text::from("sha256:chosen-by-producer"),
                );
                for result in [
                    replay_smt_cert(&ctx, &cert),
                    replay_smt_cert_with_obligation(&ctx, &cert, "sha256:chosen-by-producer"),
                    infer(&ctx, &CoreTerm::SmtProof(cert.clone()), &axioms),
                ] {
                    assert!(
                        matches!(result,
                        Err(KernelError::UnsupportedSmtCertificate { backend: ref found, tag: found_tag })
                        if found.as_str() == backend && found_tag == tag),
                        "{backend}/{tag}: {result:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn unsupported_rule_is_absent_from_all_foreign_admission_schemas() {
    let exporter = SoundnessExporter::new();
    assert_eq!(exporter.unsupported_count(), 5);
    assert_eq!(exporter.model_unavailable_count(), 5);
    assert_eq!(exporter.unverified_framework_reference_count(), 7);
    assert!(!exporter.has_complete_schema_coverage());
    let report = exporter.render_trust_extension_report();
    let smt = report
        .entries
        .iter()
        .find(|r| r.rule_name == "K_Smt")
        .unwrap();
    assert_eq!(smt.status_kind, "unsupported");
    assert!(
        smt.trust_note
            .as_ref()
            .is_some_and(|note| note.contains("refused"))
    );
    for output in [
        exporter.emit(&LeanBackend),
        exporter.emit(&CoqBackend),
        exporter.emit(&IsabelleBackend),
        exporter.emit(&AgdaBackend),
    ] {
        assert!(output.contains("UNSUPPORTED K_Smt"));
        for forbidden in [
            "K_Smt_sound",
            "K-Smt-sound",
            "t_smt",
            "T_smt",
            "K_Eps_Mu_sound",
            "K_Round_Trip_sound",
            "t_eps_mu",
            "t_round_trip",
            "T_eps_mu",
            "T_round_trip",
            "K_FwAx_sound",
            "t_fwax",
            "T_fwax",
            "K_Pos_sound",
            "K_Inductive_sound",
            "K_Elim_sound",
            "K_HComp_sound",
            "K_Glue_sound",
            "t_hcomp",
            "T_glue",
            "K_Refine_Omega_sound",
            "t_refine_omega",
        ] {
            assert!(
                !output.contains(forbidden),
                "unsupported admission remains: {forbidden}"
            );
        }
    }
}

#[test]
fn checking_a_dependent_pair_requires_the_whole_expected_family_to_be_formed() {
    use verum_kernel::{substitute, verify_full};
    let axioms = AxiomRegistry::new();
    let u0 = CoreTerm::Universe(verum_kernel::UniverseLevel::Concrete(0));
    let var = |name: &str| CoreTerm::Var(name.into());
    let pi = |name: &str, domain: CoreTerm, codomain: CoreTerm| CoreTerm::Pi {
        binder: name.into(),
        domain: domain.into(),
        codomain: codomain.into(),
    };
    let first = pi("y", u0.clone(), u0.clone());
    let index_sort = infer(&Context::new(), &first, &axioms).unwrap();
    let sigma = |family: CoreTerm| CoreTerm::Sigma {
        binder: "X".into(),
        fst_ty: index_sort.clone().into(),
        snd_ty: family.into(),
    };
    let identity = CoreTerm::Lam {
        binder: "y".into(),
        domain: u0.clone().into(),
        body: var("y").into(),
    };
    let valid = CoreTerm::Pair(first.clone().into(), identity.into());
    verify_full(&Context::new(), &valid, &sigma(var("X")), &axioms).unwrap();

    // The fiber (g : X) -> g arg is a type at the particular function type
    // 'first'. It fails at a product type in the same universe and at a variable index.
    let ctx = Context::new().extend("arg".into(), u0.clone());
    let family = pi(
        "g",
        var("X"),
        CoreTerm::App(var("g").into(), var("arg").into()),
    );
    let good_fiber = substitute(&family, "X", &first);
    let other = CoreTerm::Sigma {
        binder: "y".into(),
        fst_ty: u0.clone().into(),
        snd_ty: u0.into(),
    };
    verify_full(&ctx, &other, &index_sort, &axioms).unwrap();
    let bad_fiber = substitute(&family, "X", &other);
    assert!(infer(&ctx, &good_fiber, &axioms).is_ok());
    assert!(matches!(
        infer(&ctx, &bad_fiber, &axioms),
        Err(KernelError::NotAFunction(_))
    ));
    let ctx = ctx.extend("value".into(), good_fiber.clone());
    verify_full(&ctx, &first, &index_sort, &axioms).unwrap();
    verify_full(&ctx, &var("value"), &good_fiber, &axioms).unwrap();
    let pair = CoreTerm::Pair(first.into(), var("value").into());
    assert!(matches!(
        verify_full(&ctx, &pair, &sigma(family), &axioms),
        Err(KernelError::NotAFunction(_))
    ));
}

#[test]
fn native_modal_inference_checks_every_operand_before_returning_prop() {
    use verum_kernel::UniverseLevel;
    let ctx = Context::new();
    let axioms = AxiomRegistry::new();
    let well_formed = CoreTerm::Universe(UniverseLevel::Concrete(0));
    let missing = CoreTerm::Var("missing".into());
    for term in [
        CoreTerm::ModalBox(well_formed.clone().into()),
        CoreTerm::ModalDiamond(well_formed.clone().into()),
        CoreTerm::ModalBigAnd(List::new()),
        CoreTerm::ModalBigAnd(
            [well_formed.clone().into(), well_formed.clone().into()]
                .into_iter()
                .collect(),
        ),
    ] {
        assert_eq!(
            infer(&ctx, &term, &axioms).unwrap(),
            CoreTerm::Universe(UniverseLevel::Prop)
        );
    }
    for term in [
        CoreTerm::ModalBox(missing.clone().into()),
        CoreTerm::ModalDiamond(missing.clone().into()),
        CoreTerm::ModalBigAnd([well_formed.into(), missing.into()].into_iter().collect()),
    ] {
        assert!(matches!(
            infer(&ctx, &term, &axioms),
            Err(KernelError::UnboundVariable(_))
        ));
    }
}

fn identity_certificate() -> Certificate {
    Certificate {
        term: Term::lam(Term::universe(0), Term::var(0)),
        claimed_type: Term::pi(Term::universe(0), Term::universe(0)),
        metadata: Default::default(),
    }
}

#[test]
fn actual_native_judgments_still_require_exact_checked_claims() {
    let valid = identity_certificate();
    let mut forged = valid.clone();
    forged.claimed_type = Term::pi(Term::universe(0), Term::universe(1));
    for result in [
        valid.verify(),
        verum_proof::proof_checker_nbe::verify_certificate(&valid),
    ] {
        assert_eq!(result, Ok(()));
    }
    for result in [
        forged.verify(),
        verum_proof::proof_checker_nbe::verify_certificate(&forged),
    ] {
        assert!(matches!(
            result,
            Err(verum_proof::proof_checker::CheckError::TypeMismatch { .. })
        ));
    }
}

#[test]
#[ignore = "requires Lean and VERUM_LEAN_REPLAY for independent executable admission controls"]
fn independent_lean_cannot_use_solver_tags_as_native_proof() {
    let replay = std::env::var_os("VERUM_LEAN_REPLAY").expect("build the independent Lean checker");
    let lean = std::env::var_os("VERUM_LEAN").unwrap_or_else(|| "lean".into());
    let dir = std::env::var_os("VERUM_SMT_AUTHORITY_OUTPUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("verum-smt-authority-{}", std::process::id()))
        });
    fs::create_dir_all(&dir).unwrap();
    let valid = identity_certificate();
    let mut forged = valid.clone();
    forged.claimed_type = Term::pi(Term::universe(0), Term::universe(1));
    let battery = serde_json::json!({"schema_version":1,"certificates":[
        {"id":"checked-identity","term":valid.term,"claimed_type":valid.claimed_type},
        {"id":"forged-claim","term":forged.term,"claimed_type":forged.claimed_type}
    ]});
    let path = dir.join("native-controls.json");
    fs::write(&path, serde_json::to_vec_pretty(&battery).unwrap()).unwrap();
    let result = Command::new(&replay).arg(&path).output().unwrap();
    fs::write(dir.join("native-controls.stdout"), &result.stdout).unwrap();
    fs::write(dir.join("native-controls.stderr"), &result.stderr).unwrap();
    assert!(
        result.status.success(),
        "{}",
        std::str::from_utf8(&result.stderr).unwrap_or("non-UTF8 output")
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["verdicts"].as_array().unwrap().len(), 2);
    assert_eq!(report["verdicts"][0]["id"], "checked-identity");
    assert_eq!(report["verdicts"][0]["ok"], true);
    assert_eq!(report["verdicts"][1]["id"], "forged-claim");
    assert_eq!(report["verdicts"][1]["ok"], false);
    assert_eq!(report["verdicts"][1]["error"], "TypeMismatch");

    // The identical opaque constructor must be rejected by both native wire
    // decoders, before either checker can regard it as a theorem witness.
    for tag in 1..=3 {
        let envelope = SmtCertificate::new(
            Text::from("z3"),
            Text::from("untrusted"),
            std::iter::once(tag).collect(),
            Text::from("matching-is-not-proof"),
        );
        let term = serde_json::to_value(CoreTerm::SmtProof(envelope)).unwrap();
        assert!(serde_json::from_value::<Term>(term.clone()).is_err());
        let malformed = serde_json::json!({"schema_version":1,"certificates":[
            {"id":"opaque-solver-tag","term":term,"claimed_type":valid.claimed_type}
        ]});
        let path = dir.join(format!("opaque-tag-{tag}.json"));
        fs::write(&path, serde_json::to_vec_pretty(&malformed).unwrap()).unwrap();
        let result = Command::new(&replay).arg(&path).output().unwrap();
        fs::write(dir.join(format!("opaque-tag-{tag}.stdout")), &result.stdout).unwrap();
        fs::write(dir.join(format!("opaque-tag-{tag}.stderr")), &result.stderr).unwrap();
        assert_eq!(
            result.status.code(),
            Some(4),
            "opaque tag {tag} was admitted"
        );
    }

    let schema = SoundnessExporter::new().emit(&LeanBackend);
    for (name, command, expected) in [
        ("SupportedSchemas.lean", SCHEMA_CONTROLS, true),
        (
            "NoSmtConstructor.lean",
            "#check KernelSoundness.Typing.t_smt",
            false,
        ),
        (
            "NoSmtTheorem.lean",
            "#check KernelSoundness.K_Smt_sound",
            false,
        ),
    ] {
        fs::write(dir.join(name), format!("{schema}\n{command}\n")).unwrap();
        let result = Command::new(&lean)
            .arg(name)
            .current_dir(&dir)
            .output()
            .unwrap();
        if expected {
            let trace = String::from_utf8_lossy(&result.stdout);
            for rule in SoundnessExporter::new()
                .rules()
                .iter()
                .filter(|r| r.status.is_unverified_framework_reference())
            {
                assert!(
                    trace.contains(&format!("'KernelSoundness.{}'", rule.lemma_name)),
                    "missing axiom report for {}: {trace}",
                    rule.rule_name
                );
            }
            assert!(trace.contains("'KernelSoundness.kernel_soundness'"));
            for (_, dependencies) in trace
                .lines()
                .filter_map(|line| line.split_once("depends on axioms: ["))
            {
                let axioms = dependencies.strip_suffix(']').expect("Lean axiom list");
                for axiom in axioms.split(", ") {
                    assert!(
                        matches!(axiom, "propext" | "Quot.sound"),
                        "unexpected axiom: {axiom}"
                    );
                }
            }
            assert!(
                !trace.contains("sorryAx"),
                "placeholder dependency: {trace}"
            );
        } else {
            let trace = String::from_utf8_lossy(&result.stdout);
            assert!(
                trace.contains("Unknown identifier") || trace.contains("Unknown constant"),
                "{name} must fail on the missing declaration: {trace}"
            );
        }
        fs::write(dir.join(format!("{name}.stdout")), &result.stdout).unwrap();
        fs::write(dir.join(format!("{name}.stderr")), &result.stderr).unwrap();
        assert_eq!(
            result.status.success(),
            expected,
            "{name}: {}",
            std::str::from_utf8(&result.stdout).unwrap_or("non-UTF8 output")
        );
    }
}

const SCHEMA_CONTROLS: &str = r#"
namespace KernelSoundness
open CoreTerm

section
variable {Γ : Ctx} {x : String} {A B a b f p : CoreTerm} {i : Nat}
example (dom : Typing Γ A (Universe i)) (cod : Typing ((x,A)::Γ) B (Universe i)) :
    Typing Γ (Pi x A B) (Universe i) := by
  fail_if_success exact K_Pi_Form_sound dom
  exact K_Pi_Form_sound dom cod
example (dom : Typing Γ A (Universe i)) (body : Typing ((x,A)::Γ) b B) :
    Typing Γ (Lam x A b) (Pi x A B) := by
  fail_if_success exact K_Lam_Intro_sound dom
  exact K_Lam_Intro_sound dom body
example (fnTy : Typing Γ f (Pi x A B)) (arg : Typing Γ a A) :
    Typing Γ (App f a) (subst x a B) := by
  fail_if_success exact K_App_Elim_sound fnTy
  exact K_App_Elim_sound fnTy arg
example (dom : Typing Γ A (Universe i)) (cod : Typing ((x,A)::Γ) B (Universe i)) :
    Typing Γ (Sigma x A B) (Universe i) := by
  fail_if_success exact K_Sigma_Form_sound dom
  exact K_Sigma_Form_sound dom cod
example (formed : Typing Γ (Sigma x A B) (Universe i)) (first : Typing Γ a A)
    (second : Typing Γ b (subst x a B)) : Typing Γ (Pair a b) (Sigma x A B) := by
  fail_if_success exact K_Pair_Intro_sound first second
  fail_if_success exact K_Pair_Intro_sound formed first
  exact K_Pair_Intro_sound formed first second
example (pairTy : Typing Γ p (Sigma x A B)) : Typing Γ (Fst p) A := by
  fail_if_success exact K_Fst_Elim_sound
  exact K_Fst_Elim_sound pairTy
example (pairTy : Typing Γ p (Sigma x A B)) : Typing Γ (Snd p) (subst x (Fst p) B) := by
  fail_if_success exact K_Snd_Elim_sound
  exact K_Snd_Elim_sound pairTy

example (operand : Typing Γ a A) : Typing Γ (ModalBox a) PropSort := by
  fail_if_success exact K_Modal_Box_sound
  exact K_Modal_Box_sound operand
example (operand : Typing Γ a A) : Typing Γ (ModalDiamond a) PropSort :=
  K_Modal_Diamond_sound operand
example {components : List CoreTerm} {typeOf : CoreTerm → CoreTerm}
    (all : ∀ term ∈ components, Typing Γ term (typeOf term)) :
    Typing Γ (ModalBigAnd components) PropSort := by
  fail_if_success exact K_Modal_Big_And_sound
  exact K_Modal_Big_And_sound all
example : lookup [("x", PropSort), ("x", Universe 7)] "x" = some PropSort := rfl
end

#print axioms K_Pi_Form_sound
#print axioms K_Lam_Intro_sound
#print axioms K_App_Elim_sound
#print axioms K_Sigma_Form_sound
#print axioms K_Pair_Intro_sound
#print axioms K_Fst_Elim_sound
#print axioms K_Snd_Elim_sound
#print axioms kernel_soundness
end KernelSoundness
"#;
