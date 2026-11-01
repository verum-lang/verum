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

#[test]
fn proof_locations_are_exact_utf8_ranges_owned_by_captured_declarations() {
    let input = SourceRequest {
        profile: SourceProfile::Finite.as_str().into(),
        source: "// λ🙂\nfn helper(x:Bool)->Bool requires x ensures result { x }\nfn t(x:Bool)->Bool ensures result == x { if x { helper(x) } else { false } }".into(),
        declaration: "t".into(),
    };
    let report = replay(&serde_json::to_vec(&input).unwrap());
    assert_eq!(report.validate_for(&input), Ok(SourceOutcome::Verified));
    let original = serde_json::to_value(report).unwrap();
    let call_index = original["implementation"]["obligations"]
        .as_array()
        .unwrap()
        .iter()
        .position(|o| o["kind"] == "body_call")
        .unwrap();
    for mutation in [
        "source_span",
        "source_hash",
        "dependency_span",
        "dependency_hash",
        "callee",
        "call_hash",
        "call_owner",
        "missing_obligations",
        "semantics",
    ] {
        let mut value = original.clone();
        match mutation {
            "source_span" => value["source"]["span"]["end"] = json!(u32::MAX),
            "source_hash" => value["source"]["span"]["sha256"] = json!("stale"),
            "dependency_span" => {
                value["implementation"]["dependencies"][0]["span"]["start"] = json!(4)
            }
            "dependency_hash" => {
                value["implementation"]["dependencies"][0]["sha256"] = json!("stale")
            }
            "callee" => {
                value["implementation"]["obligations"][call_index]["callee"]["declaration"] =
                    json!("other")
            }
            "call_hash" => {
                value["implementation"]["obligations"][call_index]["call"]["sha256"] =
                    json!("stale")
            }
            "call_owner" => {
                value["implementation"]["obligations"][call_index]["owner"] =
                    value["implementation"]["obligations"][call_index]["callee"].clone()
            }
            "missing_obligations" => value["implementation"]["obligations"] = json!([]),
            "semantics" => value["implementation"]["semantics"] = json!("different"),
            _ => unreachable!(),
        }
        let changed: SourceReport = serde_json::from_value(value).unwrap();
        assert!(changed.validate_for(&input).is_err(), "{mutation}");
    }
    use verum_source_proof::{SourceSpan, sha256};
    for (start, end, text, valid) in [
        (3, 5, "λ", true),
        (5, 9, "🙂", true),
        (4, 5, "λ", false),
        (5, 8, "🙂", false),
        (9, 9, "", true),
        (9, 8, "", false),
    ] {
        let span = SourceSpan {
            start,
            end,
            sha256: sha256(text.as_bytes()),
        };
        assert_eq!(span.matches_source(&input.source), valid);
    }
    let mut rejected = SourceRequest {
        source: "fn t(x:Bool)->Bool ensures result == x { !x }".into(),
        ..input
    };
    let mut report = replay(&serde_json::to_vec(&rejected).unwrap());
    assert_eq!(report.validate_for(&rejected), Ok(SourceOutcome::Rejected));
    report.diagnostic_span.as_mut().unwrap().sha256 = "stale".into();
    assert!(report.validate_for(&rejected).is_err());
    rejected.source = "fn t(".into();
    let report = replay(&serde_json::to_vec(&rejected).unwrap());
    assert_eq!(report.validate_for(&rejected), Ok(SourceOutcome::Rejected));
}
