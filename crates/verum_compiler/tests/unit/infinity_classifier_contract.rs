//! Classifier applications retain the actual ambient topos requirement.
use super::*;

#[test]
fn actual_classifier_reports_its_missing_ambient_argument() {
    // This pins declaration arity, not full module admission. The complete
    // positive consumer still hits independent generic-dyn/carrier obligations
    // in infinity_topos (T2687/T2252); its expected-success source is retained
    // with that investigation. Require the specific missing-X diagnostic here,
    // never merely the presence of an unrelated import error.
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    for (body, rejected) in [
        (
            "fn use_classifier<C: InfSubobjectClassifier>() -> Int { 0 }",
            "missing type argument for `X`",
        ),
        (
            // The removed example was never a declared classifier witness.
            // Keep its missing ambient parameter and original methods as a
            // negative control against the actual source protocol.
            "type TrivialInfSubobjectClassifier is { marker: Int }; \
             implement InfSubobjectClassifier for TrivialInfSubobjectClassifier { \
                 fn omega(&self) -> Int { 1 } \
                 fn classify(m: &Int) -> Int { *m } \
                 fn pullback_of_true_is_m(m: &Int) -> Bool { true } \
             }",
            "missing type argument for `X`",
        ),
    ] {
        let source = format!(
            "module classifier_consumer; \
             mount core.math.infinity_topos.{{InfinityTopos, InfSubobjectClassifier}}; \
             {body}"
        );
        let module = Parser::new(&source).parse_module().unwrap();
        let result = check_source_module(
            &mut phase.configured_checker(),
            "classifier_consumer",
            &module,
            &registry,
        );
        let errors = result.expect_err("invalid classifier application accepted");
        assert!(
            errors
                .iter()
                .any(|error| error.code() == Some("E405") && error.message().contains(rejected)),
            "{source}: {:?}",
            errors
                .iter()
                .map(|error| (error.code(), error.message()))
                .collect::<List<_>>()
        );
    }
}

#[test]
fn revised_classifier_sources_parse_with_the_canonical_grammar() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for relative in [
        "core/math/examples.vr",
        "vcs/specs/L1-core/dependent/050_infinity_topos_module_loads.vr",
        "vcs/specs/L1-core/dependent/070_phase_c_full_coverage_omnibus.vr",
    ] {
        let source = std::fs::read_to_string(root.join(relative)).unwrap();
        Parser::new(&source)
            .parse_module()
            .unwrap_or_else(|errors| panic!("{relative}: {errors:#?}"));
    }
}
