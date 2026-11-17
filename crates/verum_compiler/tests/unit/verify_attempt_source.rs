//! T2306: the source policy imports its real level and agrees with compiler verdicts.
use super::*;
use verum_vbc::{
    codegen::{CodegenConfig, CodegenErrorKind, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
    types::TypeRef,
};
use verum_verification::level::{ProofAttempt, VerificationLevel, VerificationOutcome};

const ATTEMPT: &str = include_str!("../../../../core/verify/attempt.vr");
const LEVEL: &str = include_str!("../../../../core/verify/level.vr");
const LEVELS: [(&str, &str); 12] = [
    ("Runtime", "runtime"), ("Static", "static"), ("Fast", "fast"),
    ("Formal", "formal"), ("Proof", "proof"), ("Thorough", "thorough"),
    ("Reliable", "reliable"), ("Certified", "certified"),
    ("CoherentStatic", "coherent_static"), ("CoherentRuntime", "coherent_runtime"),
    ("Coherent", "coherent"), ("Synthesize", "synthesize"),
];

fn policy_provider() -> Module {
    // The operation depends on these exact source declarations. Formatting,
    // parsing and ordinal rendering have separate bodies and are not replaced
    // by stubs in this executable slice. The complete source is checked below.
    let mut module = Parser::new(LEVEL).parse_module().unwrap();
    let header = module.file_header_index().unwrap();
    let mut index = 0;
    module.items.retain(|item| {
        let keep = index == header || match &item.kind {
            ItemKind::Type(decl) => decl.name.name == "VerificationLevel",
            ItemKind::Function(decl) => decl.name.name == "allows_runtime_fallback",
            _ => false,
        };
        index += 1;
        keep
    });
    assert_eq!(module.items.len(), 3);
    let ItemKind::Type(decl) = &module.items[1].kind else { panic!("actual level declaration") };
    let verum_ast::TypeDeclBody::Variant(variants) = &decl.body else { panic!("actual level variants") };
    let actual: List<_> = variants.iter().map(|variant| variant.name.name.as_str()).collect();
    let expected: List<_> = LEVELS.iter().map(|(name, _)| *name).collect();
    assert_eq!(actual, expected, "every actual strategy needs a policy control");
    module
}

#[test]
fn complete_verification_sources_resolve_in_the_release_graph() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    for owner in ["core.verify.level", "core.verify.attempt"] {
        let module = registry.read().get_by_path(owner).unwrap().ast.clone();
        check_source_module(&mut phase.configured_checker(), owner, &module, &registry)
            .unwrap_or_else(|errors| panic!("{owner}: {errors:#?}"));
    }
    let client = Parser::new(r#"
        module policy_client;
        mount core.verify.{VerificationLevel, ProofAttempt, VerificationOutcome,
            evaluate_attempt, requires_runtime_check, is_hard_failure, is_warning};
        fn proof_failure()->VerificationOutcome {
            evaluate_attempt(VerificationLevel.Fast, ProofAttempt.Unattempted)
        }
        fn strict()->Bool { is_hard_failure(proof_failure()) }
        fn warning()->Bool { is_warning(evaluate_attempt(VerificationLevel.Static, ProofAttempt.Unattempted)) }
        fn dynamic()->Bool { requires_runtime_check(evaluate_attempt(VerificationLevel.Runtime, ProofAttempt.Proven)) }
    "#).parse_module().unwrap();
    check_source_module(&mut phase.configured_checker(), "policy_client", &client, &registry)
        .unwrap_or_else(|errors| panic!("public verification API: {errors:#?}"));
    for helper in ["evaluate_lax", "evaluate_strict"] {
        let denied = Parser::new(&format!(
            "module policy_client; mount core.verify.attempt.{helper}; fn probe() {{}}"
        )).parse_module().unwrap();
        let errors = check_source_module(&mut phase.configured_checker(), "policy_client", &denied, &registry)
            .expect_err("private policy helper became public");
        assert!(errors.iter().any(|error|
            matches!(error.code(), Some("E401" | "E202")) && error.message().contains(helper)),
            "{helper}: {errors:#?}");
    }
}

#[test]
fn a_global_same_named_level_does_not_replace_the_source_mount() {
    let provider = policy_provider();
    let mut attempt = Parser::new(ATTEMPT).parse_module().unwrap();
    let count = attempt.items.len();
    attempt.items.retain(|item| !matches!(&item.kind, ItemKind::Mount(_)));
    assert_eq!(count - attempt.items.len(), 1, "remove only the actual level mount");
    let stranger = Parser::new("module unrelated; public type VerificationLevel is Counterfeit;")
        .parse_module().unwrap();
    for files in [[&provider, &stranger, &attempt], [&attempt, &stranger, &provider]] {
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("policy_probe").with_validation());
        let error = codegen.collect_unit_declarations(&files).expect_err("unmounted sibling type became lexical authority");
        assert!(matches!(&error.kind, CodegenErrorKind::InvalidSourceSignature(message)
            if message.contains("VerificationLevel") && message.contains("core.verify.attempt")), "{error}");
    }
}

fn expected_outcome(level: &str, attempt: usize) -> (i64, Text) {
    let level = VerificationLevel::from_annotation(level).expect("actual compiler strategy projection");
    let attempt = match attempt {
        0 => ProofAttempt::Proven,
        1 => ProofAttempt::Failed("counterexample #73".into()),
        2 => ProofAttempt::Unattempted,
        _ => unreachable!(),
    };
    match level.evaluate_attempt(attempt) {
        VerificationOutcome::ElideCheck => (0, Text::new()),
        VerificationOutcome::EmitRuntimeCheck => (1, Text::new()),
        VerificationOutcome::FallbackWithWarning(reason) => (2, reason),
        VerificationOutcome::HardFail(reason) => (3, reason),
    }
}

#[test]
fn source_and_wire_policy_match_every_compiler_strategy_and_attempt() {
    let provider = policy_provider();
    let mut source = Text::from("module policy.client;
mount core.verify.level.VerificationLevel;
mount core.verify.attempt.{ProofAttempt, VerificationOutcome, evaluate_attempt, requires_runtime_check, is_hard_failure, is_warning};
");
    let mut expectations = List::new();
    for (variant, annotation) in LEVELS {
        for (index, attempt) in ["ProofAttempt.Proven", "ProofAttempt.Failed(\"counterexample #73\")", "ProofAttempt.Unattempted"]
            .into_iter().enumerate()
        {
            let (expected, reason) = expected_outcome(annotation, index);
            let reason = serde_json::to_string(reason.as_str()).unwrap();
            let name = format!("policy_{variant}_{index}");
            let runtime = expected == 1 || expected == 2;
            let hard = expected == 3;
            let warning = expected == 2;
            source.push_str(&format!(r#"
fn {name}()->Int {{
    if requires_runtime_check(evaluate_attempt(VerificationLevel.{variant}, {attempt})) != {runtime} {{ return -21; }}
    if is_hard_failure(evaluate_attempt(VerificationLevel.{variant}, {attempt})) != {hard} {{ return -22; }}
    if is_warning(evaluate_attempt(VerificationLevel.{variant}, {attempt})) != {warning} {{ return -23; }}
    let outcome = evaluate_attempt(VerificationLevel.{variant}, {attempt});
    match outcome {{
        VerificationOutcome.ElideCheck => 0,
        VerificationOutcome.EmitRuntimeCheck => 1,
        VerificationOutcome.FallbackWithWarning(reason) => if reason == {reason} {{2}} else {{-12}},
        VerificationOutcome.HardFail(reason) => if reason == {reason} {{3}} else {{-13}},
    }}
}}
"#));
            expectations.push((format!("policy.client.{name}"), expected));
        }
    }
    let attempt = Parser::new(ATTEMPT).parse_module().unwrap();
    let client = Parser::new(&source).parse_module().unwrap();
    let files = [&client, &attempt, &provider];
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("policy_probe").with_validation());
    codegen.collect_unit_declarations(&files).unwrap();
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(&files, ItemFailurePolicy::Strict).unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    for (route, module) in [("source", module), ("wire", decoded)] {
        let evaluate = module.get_function(module.find_function_by_name("core.verify.attempt.evaluate_attempt").unwrap()).unwrap();
        let TypeRef::Concrete(level) = evaluate.params[0].type_ref else { panic!("level signature lost nominal identity") };
        let owner = module.get_type(level).unwrap().origin_module.unwrap();
        assert_eq!(module.get_string(owner), Some("core.verify.level"));
        let entries: List<_> = expectations.iter().map(|(name, expected)|
            (name, *expected, module.find_function_by_name(name).expect("exact policy probe"))).collect();
        let mut vm = Interpreter::try_new_validated(module.into()).expect("strict source/wire validation");
        for (name, expected, entry) in entries {
            let result = vm.execute_function(entry).unwrap_or_else(|error| panic!("{route} {name}: {error}"));
            assert_eq!(result.try_as_i64(), Some(expected), "{route} {name}");
        }
    }
}
