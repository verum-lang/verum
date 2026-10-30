use super::*;

fn params() -> ValidateRefinementParams {
    ValidateRefinementParams {
        text_document: TextDocumentIdentifier {
            uri: Url::parse("file:///draft.vr").unwrap(),
        },
        position: Position::new(0, 3),
        mode: ValidationMode::Quick,
    }
}

#[test]
fn inconclusive_and_absent_obligations_never_report_valid() {
    let validator = RefinementValidator::new();
    for (result, expected) in [
        (ValidationResult::Unknown, RefinementStatus::Unknown),
        (
            ValidationResult::NotApplicable,
            RefinementStatus::NotApplicable,
        ),
    ] {
        let response = validator.result_to_response(result, Duration::ZERO);
        assert!(!response.valid);
        assert_eq!(response.status, expected);
    }
    let response = validator.result_to_response(ValidationResult::Valid, Duration::ZERO);
    assert!(response.valid);
    assert_eq!(response.status, RefinementStatus::Valid);
}

#[tokio::test]
async fn cache_never_reuses_another_source_at_the_same_uri_and_position() {
    let validator = RefinementValidator::new();
    let first = validator
        .validate_source(params(), "fn main() {}")
        .await
        .unwrap();
    assert_eq!(first.status, RefinementStatus::NotApplicable);
    let changed = validator.validate_source(params(), "fn main(").await;
    assert!(
        changed.is_err(),
        "a cached result must not mask a broken revision"
    );
    let restored = validator
        .validate_source(params(), "fn main() {}")
        .await
        .unwrap();
    assert_eq!(restored.status, RefinementStatus::NotApplicable);
}

#[tokio::test]
async fn unsupported_solver_is_inconclusive_and_not_a_relabelled_z3_result() {
    let validator = RefinementValidator::new();
    validator.config.write().smt_solver = SmtSolver::CVC5;
    let source = "fn test(x: Int{it > 0}) {}";
    let mut request = params();
    request.position.character = 13;
    let response = validator.validate_source(request, source).await.unwrap();
    assert_eq!(response.status, RefinementStatus::Unknown);
    assert!(!response.valid);
}

#[test]
fn refinement_ranges_share_standard_utf16_mapping() {
    let source = "// 😀\r\nfn main() {}";
    let start = source.find("main").unwrap() as u32;
    assert_eq!(
        span_to_range(
            source,
            verum_ast::Span::new(start, start + 4, FileId::new(1))
        ),
        Range::new(Position::new(1, 3), Position::new(1, 7))
    );
    assert_eq!(
        span_to_range(source, verum_ast::Span::new(3, 7, FileId::new(1))),
        Range::new(Position::new(0, 3), Position::new(0, 5))
    );
}

#[tokio::test]
async fn cursor_selection_and_changed_values_are_validated_independently() {
    let validator = RefinementValidator::new();
    let mut request = params();
    request.mode = ValidationMode::Thorough;
    let good = "fn main() { let x: Int{it > 0} = 1; }";
    let bad = "fn main() { let x: Int{it > 0} = 0; }";
    request.position.character = 20;
    let valid = validator
        .validate_source(request.clone(), good)
        .await
        .unwrap();
    assert_eq!(valid.status, RefinementStatus::Valid);
    let invalid = validator
        .validate_source(request.clone(), bad)
        .await
        .unwrap();
    assert_eq!(invalid.status, RefinementStatus::Invalid);
    assert!(!invalid.valid);
    request.position.character = 3;
    let absent = validator.validate_source(request, bad).await.unwrap();
    assert_eq!(
        absent.status,
        RefinementStatus::NotApplicable,
        "a refinement elsewhere must not be attributed to the cursor"
    );
}
