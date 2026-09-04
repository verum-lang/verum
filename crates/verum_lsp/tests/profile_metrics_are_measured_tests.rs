//! `verum/getProfile` must report measurements, never inventions (T1137).
//!
//! Before this suite existed the handler built its payload like this:
//!
//! ```ignore
//! let has_refinement = name.contains("verify") || name.contains("check");
//! let verification_time = if has_refinement { 5000 } else { 100 };
//! let type_check_time = 50u64;                      // per function
//! let parse_time = (doc.text.len() / 100) as u64;
//! let codegen_time = doc.symbols.len() as u64 * 20;
//! "runtimeMetrics": { "total": 1000, "businessLogic": 900, .. }
//! ```
//!
//! Every assertion below names what it would have read under that code,
//! so the suite carries its own positive pole: none of these tests can
//! pass by accident on a payload that went back to guessing.

use serde_json::Value;
use tower_lsp::lsp_types::*;
use tower_lsp::{LanguageServer, LspService};
use verum_lsp::Backend;

/// Two documents differing ONLY in their function names.
///
/// `verify_` and `check_` are the exact substrings the old code keyed
/// on, so under it these two would report verification 10000 and 200
/// respectively. A profile that still reads names cannot pass this.
const NAMED_LIKE_VERIFICATION: &str = "\
fn verify_total(a: Int) -> Int { a + 1 }
fn check_total(b: Int) -> Int { b + 2 }
";

const NAMED_NEUTRALLY: &str = "\
fn alpha_total(a: Int) -> Int { a + 1 }
fn gamma_total(b: Int) -> Int { b + 2 }
";

async fn profile_of(uri: &str, source: &str) -> Value {
    let (service, _socket) = LspService::new(Backend::new);
    let backend = service.inner();

    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: Url::parse(uri).expect("test uri parses"),
                language_id: "verum".to_string(),
                version: 1,
                text: source.to_string(),
            },
        })
        .await;

    backend
        .handle_get_profile(serde_json::json!({
            "textDocument": { "uri": uri }
        }))
        .await
        .expect("getProfile answers")
}

fn compilation(profile: &Value, field: &str) -> u64 {
    profile["compilationMetrics"][field]
        .as_u64()
        .unwrap_or_else(|| panic!("compilationMetrics.{field} is a number"))
}

#[tokio::test]
async fn renaming_a_function_does_not_change_any_reported_metric() {
    let named = profile_of("file:///named.vr", NAMED_LIKE_VERIFICATION).await;
    let neutral = profile_of("file:///neutral.vr", NAMED_NEUTRALLY).await;

    // Old code: 10000 vs 200. The two sources are the same length and
    // the same shape, so any difference here is a name being read.
    assert_eq!(
        compilation(&named, "verification"),
        compilation(&neutral, "verification"),
        "verification time differs between two documents that differ \
         only in function names — the handler is reading names again"
    );

    // TYPE-CHECK TIME IS NOT ASSERTED EQUAL, and the reason is the
    // point of this whole suite.
    //
    // This assertion was first written as `assert_eq!` — "identical
    // under the old code (50 per function), so a future change that
    // makes it name-dependent cannot slip through" — and it FAILED on
    // the first run against the fix:
    //
    //     left: 7313    right: 2761
    //
    // Those are microseconds. Two nearly identical documents type-check
    // in different times because the first one pays cold-start costs.
    // A real clock cannot be asserted equal to another real clock, and
    // an assertion that requires it is measuring the wrong thing.
    //
    // The failure is also the strongest evidence in this file that the
    // fix landed: under the old code both figures were `50 * 2 = 100`,
    // identical because they were a formula over function count. Two
    // different plausible durations are what a clock looks like.
    //
    // What remains assertable is that the clock RAN for both.
    assert!(
        compilation(&named, "typeChecking") > 0 && compilation(&neutral, "typeChecking") > 0,
        "type-check time is zero for a document that type-checks — nothing timed it"
    );
}

#[tokio::test]
async fn phases_the_lsp_does_not_perform_report_zero() {
    let profile = profile_of("file:///phases.vr", NAMED_NEUTRALLY).await;

    // The LSP parses, builds a symbol table and type-checks. It does
    // not verify and does not generate code, so these are not small
    // numbers — they are absent work.
    // Old code: verification 200, codegen 40.
    assert_eq!(
        compilation(&profile, "verification"),
        0,
        "the LSP performs no verification; any non-zero figure is invented"
    );
    assert_eq!(
        compilation(&profile, "codegen"),
        0,
        "the LSP performs no codegen; any non-zero figure is invented"
    );

    // The unit was previously unstated, which let a formula's output
    // pass for a duration.
    assert_eq!(
        profile["compilationMetrics"]["unit"].as_str(),
        Some("microseconds"),
        "compilation metrics must declare their unit"
    );
}

#[tokio::test]
async fn runtime_metrics_are_marked_unmeasured_rather_than_asserted() {
    let profile = profile_of("file:///runtime.vr", NAMED_NEUTRALLY).await;

    // Old code: a literal 1000 / 900 / 100 for EVERY document.
    assert_eq!(
        profile["runtimeMetrics"]["measured"].as_bool(),
        Some(false),
        "the LSP never executes the program; the payload must say so"
    );
    assert_eq!(
        profile["runtimeMetrics"]["total"].as_u64(),
        Some(0),
        "a runtime total the LSP never observed must not be reported"
    );
    assert_eq!(
        profile["runtimeMetrics"]["businessLogic"].as_u64(),
        Some(0),
        "a business-logic time the LSP never observed must not be reported"
    );
}

#[tokio::test]
async fn a_document_that_fails_to_parse_reports_no_type_check_time() {
    // The type-check pass does not run when parsing fails. Reporting
    // the previous round's figure would be a stale duration presented
    // as the current one.
    let profile = profile_of("file:///broken.vr", "fn broken( { { {").await;

    assert_eq!(
        compilation(&profile, "typeChecking"),
        0,
        "no type-check ran, so no type-check time exists to report"
    );
}

#[tokio::test]
async fn a_real_parse_puts_a_non_zero_duration_on_the_clock() {
    // The positive pole for the whole suite: the assertions above are
    // all "must be zero", and a handler that returned zeros for
    // everything would satisfy every one of them. This one requires a
    // figure that only a clock can produce.
    //
    // Deliberately NOT written as "parse time != text.len()/100": at
    // 20 KB that formula answers 200us and a real parse of the same
    // document lands in the same neighbourhood, so the comparison
    // would fail on timing luck rather than on behaviour.
    let mut source = String::new();
    while source.len() < 20_000 {
        let n = source.len();
        source.push_str(&format!("fn f{n}(a: Int) -> Int {{ a + 1 }}\n"));
    }

    let profile = profile_of("file:///sized.vr", &source).await;

    // 20 KB of Verum cannot lex and parse in under one microsecond, so
    // this is safe against a fast machine while still failing outright
    // if the field is hardcoded, dropped, or never written.
    assert!(
        compilation(&profile, "parsing") > 0,
        "a 20 KB document reported a parse time of zero — nothing timed it"
    );

    // And the total must account for the phases that did run, so a
    // future edit cannot leave `total` behind as its own invention.
    assert_eq!(
        compilation(&profile, "total"),
        compilation(&profile, "parsing")
            + compilation(&profile, "typeChecking")
            + compilation(&profile, "verification")
            + compilation(&profile, "codegen"),
        "total must be the sum of the reported phases"
    );
}
