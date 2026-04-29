//! End-to-end integration tests for `verum proof-repair`.
//!
//! Spawns the actual `verum` binary and validates the full chain:
//!
//!   `verum proof-repair` (CLI clap) →
//!     `commands::proof_repair::run_proof_repair` →
//!       `verum_diagnostics::proof_repair::DefaultRepairEngine` →
//!         ranked output (plain or JSON)
//!
//! Together with the 17 handler unit tests in
//! `commands::proof_repair::tests` and the 10 trait-level tests in
//! `verum_diagnostics::proof_repair::tests`, this proves the
//! `RepairEngine` trait surface is consumable from a shell — closing
//! the integration gap #87 was opened to address.

use std::path::PathBuf;
use std::process::Command;

fn verum_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_verum"))
}

// ─────────────────────────────────────────────────────────────────────
// Plain output (default)
// ─────────────────────────────────────────────────────────────────────

#[test]
fn proof_repair_unbound_name_plain_output() {
    let out = Command::new(verum_bin())
        .args([
            "proof-repair",
            "--kind",
            "unbound-name",
            "--field",
            "name=foo_lemma",
            "--format",
            "plain",
        ])
        .output()
        .expect("CLI invocation must succeed");
    assert!(
        out.status.success(),
        "verum proof-repair exited non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Headline + suggestion block must be present.
    assert!(
        stdout.contains("Failure kind: unbound-name"),
        "missing headline: {stdout}"
    );
    assert!(
        stdout.contains("Suggestions"),
        "missing suggestions header: {stdout}"
    );
    // Top suggestion for unbound-name should propose a `mount`.
    assert!(
        stdout.contains("mount"),
        "expected `mount` suggestion in output: {stdout}"
    );
    // Doc-link must surface (📖 marker).
    assert!(
        stdout.contains("https://docs.verum.lang/"),
        "doc-link must appear: {stdout}"
    );
}

#[test]
fn proof_repair_refine_depth_plain_output_includes_doc_link() {
    let out = Command::new(verum_bin())
        .args([
            "proof-repair",
            "--kind",
            "refine-depth",
            "--field",
            "refined_type=CategoricalLevel",
            "--field",
            "predicate_depth=omega-2",
        ])
        .output()
        .expect("CLI invocation must succeed");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // The K-Refine rule is the headline doc-link.
    assert!(
        stdout.contains("k-refine"),
        "expected k-refine doc-link: {stdout}"
    );
}

// ─────────────────────────────────────────────────────────────────────
// JSON output
// ─────────────────────────────────────────────────────────────────────

#[test]
fn proof_repair_json_output_is_well_formed() {
    let out = Command::new(verum_bin())
        .args([
            "proof-repair",
            "--kind",
            "unbound-name",
            "--field",
            "name=foo",
            "--max",
            "3",
            "--format",
            "json",
        ])
        .output()
        .expect("CLI invocation must succeed");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Round-trip through serde_json — proves output is valid JSON.
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("output must be valid JSON: {e}\nstdout={stdout}"));

    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["kind"], "unbound-name");
    let suggestions = parsed["suggestions"]
        .as_array()
        .expect("suggestions must be an array");
    assert!(!suggestions.is_empty(), "should produce at least one suggestion");

    // Each suggestion must carry required fields.
    for s in suggestions {
        assert!(s["snippet"].is_string());
        assert!(s["rationale"].is_string());
        assert!(s["applicability"].is_string());
        assert!(s["score"].is_number());
        let score = s["score"].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&score), "score out of [0,1]");
        // doc_link is either a string or null.
        assert!(s["doc_link"].is_string() || s["doc_link"].is_null());
    }
}

#[test]
fn proof_repair_json_records_score_descending() {
    let out = Command::new(verum_bin())
        .args([
            "proof-repair",
            "--kind",
            "refine-depth",
            "--field",
            "refined_type=X",
            "--field",
            "predicate_depth=Y",
            "--max",
            "5",
            "--format",
            "json",
        ])
        .output()
        .expect("CLI invocation must succeed");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let suggestions = parsed["suggestions"].as_array().unwrap();

    // Pin the engine's ranking contract: scores are monotonically
    // non-increasing through the list.
    let scores: Vec<f64> = suggestions
        .iter()
        .map(|s| s["score"].as_f64().unwrap())
        .collect();
    for w in scores.windows(2) {
        assert!(
            w[0] >= w[1],
            "suggestions must be score-descending: {:?}",
            scores
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// All 9 ProofFailureKind variants reachable from the CLI
// ─────────────────────────────────────────────────────────────────────

#[test]
fn proof_repair_every_kind_produces_at_least_one_suggestion() {
    // End-to-end equivalent of the in-handler
    // engine_returns_at_least_one_for_every_kind_via_handler_path
    // unit test. This is the strongest contract on the integration:
    // every typed failure kind we expose at the CLI surface MUST
    // return a non-empty suggestion list when invoked via `Command`.
    let cases: &[(&str, &[(&str, &str)])] = &[
        ("refine-depth", &[("refined_type", "X"), ("predicate_depth", "ω")]),
        (
            "positivity",
            &[
                ("type_name", "Bad"),
                ("constructor", "Wrap"),
                ("position", "left of arrow"),
            ],
        ),
        (
            "universe",
            &[("source_universe", "Type_1"), ("expected_universe", "Type_0")],
        ),
        ("fwax-not-prop", &[("axiom_name", "ax"), ("body_sort", "Type")]),
        ("adjunction", &[("side", "unit")]),
        ("type-mismatch", &[("expected", "Int"), ("actual", "Bool")]),
        ("unbound-name", &[("name", "foo")]),
        (
            "apply-mismatch",
            &[
                ("lemma_name", "f"),
                ("actual_conclusion", "A"),
                ("goal", "B"),
            ],
        ),
        ("tactic-open", &[("tactic", "lia"), ("reason", "non-trivial")]),
    ];

    for (kind, fields) in cases {
        let mut args: Vec<String> = vec![
            "proof-repair".into(),
            "--kind".into(),
            (*kind).into(),
            "--format".into(),
            "json".into(),
        ];
        for (k, v) in *fields {
            args.push("--field".into());
            args.push(format!("{}={}", k, v));
        }
        let out = Command::new(verum_bin())
            .args(&args)
            .output()
            .unwrap_or_else(|_| panic!("CLI must spawn for kind {}", kind));
        assert!(
            out.status.success(),
            "kind {} exited non-zero: stderr={}",
            kind,
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        let parsed: serde_json::Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("kind {}: invalid JSON: {e}", kind));
        let n = parsed["suggestions"].as_array().unwrap().len();
        assert!(
            n >= 1,
            "kind {} produced empty suggestion list (V0 catalogue gap)",
            kind
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// Validation contracts
// ─────────────────────────────────────────────────────────────────────

#[test]
fn proof_repair_rejects_unknown_kind() {
    let out = Command::new(verum_bin())
        .args(["proof-repair", "--kind", "garbage", "--field", "x=1"])
        .output()
        .expect("CLI invocation must succeed");
    assert!(
        !out.status.success(),
        "unknown --kind must produce non-zero exit"
    );
}

#[test]
fn proof_repair_rejects_zero_max() {
    let out = Command::new(verum_bin())
        .args([
            "proof-repair",
            "--kind",
            "unbound-name",
            "--field",
            "name=foo",
            "--max",
            "0",
        ])
        .output()
        .expect("CLI invocation must succeed");
    assert!(!out.status.success(), "--max 0 must produce non-zero exit");
}

#[test]
fn proof_repair_rejects_unknown_format() {
    let out = Command::new(verum_bin())
        .args([
            "proof-repair",
            "--kind",
            "unbound-name",
            "--field",
            "name=foo",
            "--format",
            "yaml",
        ])
        .output()
        .expect("CLI invocation must succeed");
    assert!(
        !out.status.success(),
        "unknown --format must produce non-zero exit"
    );
}

#[test]
fn proof_repair_rejects_missing_required_field() {
    // `refine-depth` requires both refined_type and predicate_depth;
    // omit one and verify the CLI surfaces a descriptive error.
    let out = Command::new(verum_bin())
        .args([
            "proof-repair",
            "--kind",
            "refine-depth",
            "--field",
            "refined_type=X",
            // missing predicate_depth
        ])
        .output()
        .expect("CLI invocation must succeed");
    assert!(
        !out.status.success(),
        "missing required --field must produce non-zero exit"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("predicate_depth"),
        "stderr should name the missing field: {stderr}"
    );
}

#[test]
fn proof_repair_rejects_malformed_field_flag() {
    let out = Command::new(verum_bin())
        .args([
            "proof-repair",
            "--kind",
            "unbound-name",
            "--field",
            "no-equals",
        ])
        .output()
        .expect("CLI invocation must succeed");
    assert!(
        !out.status.success(),
        "--field without `=` must produce non-zero exit"
    );
}

#[test]
fn proof_repair_adjunction_rejects_invalid_side() {
    let out = Command::new(verum_bin())
        .args([
            "proof-repair",
            "--kind",
            "adjunction",
            "--field",
            "side=garbage",
        ])
        .output()
        .expect("CLI invocation must succeed");
    assert!(
        !out.status.success(),
        "adjunction with invalid side must produce non-zero exit"
    );
}

#[test]
fn proof_repair_max_truncates_suggestion_list() {
    // refine-depth catalogue produces 2 suggestions; --max 1
    // must truncate to a single entry.
    let out = Command::new(verum_bin())
        .args([
            "proof-repair",
            "--kind",
            "refine-depth",
            "--field",
            "refined_type=X",
            "--field",
            "predicate_depth=Y",
            "--max",
            "1",
            "--format",
            "json",
        ])
        .output()
        .expect("CLI invocation must succeed");
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let suggestions = parsed["suggestions"].as_array().unwrap();
    assert_eq!(suggestions.len(), 1);
}
