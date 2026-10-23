use verum_source_proof::{PROFILE, SourceReport, SourceRequest, replay, sha256};
fn check(source: &str) -> SourceReport {
    replay(
        &serde_json::to_vec(&SourceRequest {
            profile: PROFILE.into(),
            source: source.into(),
            declaration: "t".into(),
        })
        .unwrap(),
    )
}
fn range<'a>(source: &'a str, result: &SourceReport) -> &'a str {
    assert_ne!(result.status, "verified");
    assert!(result.judgment.is_none());
    assert_eq!(result.source.sha256, sha256(source.as_bytes()));
    let span = result
        .diagnostic_span
        .as_ref()
        .expect("source diagnostic location");
    let text = &source[span.start as usize..span.end as usize];
    assert_eq!(span.sha256, sha256(text.as_bytes()));
    text
}
#[test]
fn errors_target_the_inner_identifier_not_a_matching_word_in_a_comment() {
    for prefix in ["// missing\n", "// Ω 🙂 missing\r\n", "/* 中文\r */", ""] {
        let source = format!("{prefix}theorem t(A:Type,x:A):A proof=(missing);");
        let report = check(&source);
        assert_eq!(report.status, "rejected");
        assert_eq!(range(&source, &report), "missing");
        assert_eq!(
            report.diagnostic_span.as_ref().unwrap().start as usize,
            source.rfind("missing").unwrap()
        );
    }
}
#[test]
fn type_and_final_proof_mismatch_have_source_owned_ranges() {
    let source = "// 👩🏽‍🔬\r\ntheorem t(A:Type,x:Unknown):A proof=x;";
    assert_eq!(range(source, &check(source)), "Unknown");
    let source = "theorem t(A:Type,x:A):A proof=A;";
    assert_eq!(range(source, &check(source)), "A");
    assert_eq!(
        check(source).diagnostic_span.unwrap().start as usize,
        source.rfind('A').unwrap()
    );
}
#[test]
fn production_parser_reports_a_valid_location_including_end_of_input() {
    for source in [
        "// Ω🙂\r\ntheorem t(A:Type,x:A):A proof=;",
        "theorem t(A:Type,x:A):A proof=(",
        "theorem t(A:Type,",
    ] {
        let report = check(source);
        assert_eq!(
            report.diagnostic.as_ref().unwrap().code,
            "source_parse_error",
            "{report:?}"
        );
        range(source, &report);
    }
}
#[test]
fn success_and_legacy_reports_have_no_diagnostic_location() {
    let report = check("theorem t(A:Type,x:A):A proof=x;");
    assert_eq!(report.status, "verified");
    assert!(report.diagnostic_span.is_none());
    let wire = serde_json::to_value(report).unwrap();
    assert!(wire.get("diagnostic_span").is_none());
    let restored: SourceReport = serde_json::from_value(wire).unwrap();
    assert!(restored.diagnostic_span.is_none());
}

#[test]
fn grouping_preserves_judgments_and_rejections_through_both_kernels() {
    let plain = check("theorem t(A:Type,x:A):A proof=x;");
    for proof in ["(x)", "((x))", "(((x)))"] {
        let grouped = check(&format!("theorem t(A:Type,x:A):(A) proof={proof};"));
        assert_eq!(grouped.status, "verified", "{grouped:?}");
        assert_eq!(
            serde_json::to_value(grouped.judgment).unwrap(),
            serde_json::to_value(&plain.judgment).unwrap()
        );
    }
    for proof in ["(A)", "((missing))"] {
        let grouped = check(&format!("theorem t(A:Type,x:A):A proof={proof};"));
        assert_eq!(grouped.status, "rejected", "{grouped:?}");
        assert!(grouped.judgment.is_none());
    }
    // The production grammar permits a final proof without a semicolon.
    assert_eq!(check("theorem t(A:Type,x:A):A proof=x").status, "verified");
}
