use serde_json::json;
use verum_source_proof::{SourceOutcome, SourceProfile, SourceReport, SourceRequest, replay};

fn request(profile: SourceProfile) -> SourceRequest {
    SourceRequest {
        profile: profile.as_str().into(),
        source: match profile {
            SourceProfile::Logical => "theorem t(A:Type,x:A):A proof=x;",
            SourceProfile::Finite => "fn t(x:Bool)->Bool ensures result == x { x }",
        }
        .into(),
        declaration: "t".into(),
    }
}

#[test]
fn reports_bind_exact_profiles_declarations_sources_and_checker_evidence() {
    for profile in SourceProfile::ALL {
        let input = request(profile);
        let report = replay(&serde_json::to_vec(&input).unwrap());
        assert_eq!(report.validate_for(&input), Ok(SourceOutcome::Verified));
        let original = serde_json::to_value(report).unwrap();
        for (path, value) in [
            (vec!["profile"], json!("different-profile")),
            (vec!["source", "declaration"], json!("another")),
            (vec!["source", "sha256"], json!("stale")),
            (vec!["elaborator"], json!("stale")),
            (vec!["native", "checker"], json!("stale")),
            (vec!["native", "accepted"], json!(false)),
            (vec!["native", "profile"], json!("different-foundation")),
            (vec!["native", "limits", "max_nodes"], json!(0)),
            (vec!["judgment"], json!(null)),
            (vec!["judgment", "statement"], json!({"Var": 0})),
            (vec!["status"], json!("rejected")),
            (vec!["status"], json!("success")),
        ] {
            let mut changed = original.clone();
            let mut target = &mut changed;
            for field in &path[..path.len() - 1] {
                target = &mut target[*field];
            }
            target[path[path.len() - 1]] = value;
            let changed: SourceReport = serde_json::from_value(changed).unwrap();
            assert!(
                changed.validate_for(&input).is_err(),
                "{profile:?} {path:?}"
            );
        }
        let mut changed: SourceReport = serde_json::from_value(original).unwrap();
        if profile == SourceProfile::Finite {
            changed.implementation = None;
            assert!(changed.validate_for(&input).is_err());
        }
    }
}

#[test]
fn semantic_refusals_are_not_changed_into_success_or_resource_results() {
    let mut input = request(SourceProfile::Finite);
    input.source = "fn t(x:Bool)->Bool ensures result == x { !x }".into();
    let mut report = replay(&serde_json::to_vec(&input).unwrap());
    assert_eq!(report.validate_for(&input), Ok(SourceOutcome::Rejected));
    report.status = "verified".into();
    assert!(report.validate_for(&input).is_err());
    report.status = "inconclusive".into();
    assert!(report.validate_for(&input).is_err());
    input.source = "fn t(x:Int)->Bool ensures result { true }".into();
    let report = replay(&serde_json::to_vec(&input).unwrap());
    assert_eq!(report.validate_for(&input), Ok(SourceOutcome::Unsupported));
}
