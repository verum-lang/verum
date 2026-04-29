//! End-to-end integration tests for `verum tactic`.
//!
//! Spawns the actual `verum` binary and validates the chain:
//!
//!   `verum tactic {list,explain,laws}` (clap) →
//!     `commands::tactic::run_*` →
//!       `verum_verification::tactic_combinator::DefaultTacticCatalog` →
//!         ranked plain / JSON output
//!
//! Together with the 17 trait-level tests in
//! `verum_verification::tactic_combinator::tests` and the 17 handler
//! unit tests in `commands::tactic::tests`, this proves the
//! `TacticCatalog` trait surface is consumable from a shell — closing
//! the integration gap #76 was opened to address.

use std::path::PathBuf;
use std::process::Command;

fn verum_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_verum"))
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(verum_bin())
        .args(args)
        .output()
        .expect("CLI invocation must succeed")
}

// ─────────────────────────────────────────────────────────────────────
// list
// ─────────────────────────────────────────────────────────────────────

#[test]
fn tactic_list_plain_lists_all_fifteen() {
    let out = run(&["tactic", "list"]);
    assert!(
        out.status.success(),
        "verum tactic list exited non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Every canonical combinator name MUST appear in the listing.
    for name in [
        "skip",
        "fail",
        "seq",
        "orelse",
        "repeat",
        "repeat_n",
        "try",
        "solve",
        "first_of",
        "all_goals",
        "index_focus",
        "named_focus",
        "per_goal_split",
        "have",
        "apply_with",
    ] {
        assert!(
            stdout.contains(name),
            "list output missing combinator `{}`: {stdout}",
            name
        );
    }
    assert!(
        stdout.contains("Total: 15"),
        "list output missing total count line: {stdout}"
    );
}

#[test]
fn tactic_list_json_well_formed_with_fifteen_entries() {
    let out = run(&["tactic", "list", "--format", "json"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("output must be valid JSON: {e}\nstdout={stdout}"));

    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["count"], 15);
    let entries = parsed["entries"]
        .as_array()
        .expect("entries must be an array");
    assert_eq!(entries.len(), 15);

    for e in entries {
        assert!(e["name"].is_string());
        assert!(e["category"].is_string());
        assert!(e["signature"].is_string());
        assert!(e["semantics"].is_string());
        assert!(e["example"].is_string());
        assert!(e["doc_anchor"].is_string());
        assert!(e["laws"].is_array());
    }
}

#[test]
fn tactic_list_category_filter_restricts_entries() {
    // 5 categories: identity, composition, control, focus, forward.
    // Sum of all category-filtered counts must equal 15.
    let mut total = 0;
    for cat in ["identity", "composition", "control", "focus", "forward"] {
        let out = run(&["tactic", "list", "--category", cat, "--format", "json"]);
        assert!(out.status.success(), "category {} failed", cat);
        let parsed: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
        let n = parsed["count"].as_u64().unwrap();
        assert!(n > 0, "category {} has zero combinators", cat);
        total += n;

        let entries = parsed["entries"].as_array().unwrap();
        for e in entries {
            assert_eq!(e["category"], cat, "category mixing in {} filter", cat);
        }
    }
    assert_eq!(total, 15, "sum of category counts must equal 15");
}

#[test]
fn tactic_list_rejects_unknown_category() {
    let out = run(&["tactic", "list", "--category", "nonsense"]);
    assert!(
        !out.status.success(),
        "unknown --category must produce non-zero exit"
    );
}

#[test]
fn tactic_list_rejects_unknown_format() {
    let out = run(&["tactic", "list", "--format", "yaml"]);
    assert!(!out.status.success());
}

// ─────────────────────────────────────────────────────────────────────
// explain
// ─────────────────────────────────────────────────────────────────────

#[test]
fn tactic_explain_resolves_every_canonical_combinator() {
    for name in [
        "skip",
        "fail",
        "seq",
        "orelse",
        "repeat",
        "repeat_n",
        "try",
        "solve",
        "first_of",
        "all_goals",
        "index_focus",
        "named_focus",
        "per_goal_split",
        "have",
        "apply_with",
    ] {
        let out = run(&["tactic", "explain", name, "--format", "json"]);
        assert!(
            out.status.success(),
            "explain {} exited non-zero: stderr={}",
            name,
            String::from_utf8_lossy(&out.stderr)
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&out.stdout))
                .unwrap_or_else(|e| panic!("explain {}: invalid JSON: {e}", name));
        assert_eq!(parsed["name"], name);
        assert!(parsed["signature"].is_string());
        assert!(parsed["semantics"].is_string());
        assert!(parsed["example"].is_string());
        assert!(parsed["category"].is_string());
        assert!(parsed["doc_anchor"].is_string());
    }
}

#[test]
fn tactic_explain_solve_carries_law_resolution() {
    // `solve` participates in `solve-of-skip-fails-when-open`. Pin
    // the contract that the explain endpoint resolves the law name
    // back to its full record (lhs / rhs / rationale).
    let out = run(&["tactic", "explain", "solve", "--format", "json"]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let laws = parsed["laws"].as_array().unwrap();
    assert!(!laws.is_empty(), "solve must have at least one law");

    let law = laws
        .iter()
        .find(|l| l["name"] == "solve-of-skip-fails-when-open")
        .expect("solve-of-skip-fails-when-open must be present");
    assert!(law["lhs"].is_string());
    assert!(law["rhs"].is_string());
    assert!(law["rationale"].is_string());
}

#[test]
fn tactic_explain_seq_lists_three_laws() {
    // `seq` participates in left-identity / right-identity /
    // associativity — three laws.  Pins the simplifier-relevant
    // count.
    let out = run(&["tactic", "explain", "seq", "--format", "json"]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let laws = parsed["laws"].as_array().unwrap();
    assert_eq!(
        laws.len(),
        3,
        "seq must have 3 laws (left-id / right-id / assoc); got {}",
        laws.len()
    );
}

#[test]
fn tactic_explain_rejects_unknown_name() {
    let out = run(&["tactic", "explain", "nonsense"]);
    assert!(
        !out.status.success(),
        "unknown combinator must produce non-zero exit"
    );
}

#[test]
fn tactic_explain_rejects_unknown_format() {
    let out = run(&["tactic", "explain", "solve", "--format", "yaml"]);
    assert!(!out.status.success());
}

// ─────────────────────────────────────────────────────────────────────
// laws
// ─────────────────────────────────────────────────────────────────────

#[test]
fn tactic_laws_plain_lists_all_twelve() {
    let out = run(&["tactic", "laws"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Pin the 12 canonical law names — this is the simplifier's
    // normalisation set.  Adding or removing a law without
    // updating the simplifier surface would break this test.
    for name in [
        "seq-left-identity",
        "seq-right-identity",
        "seq-associative",
        "orelse-left-identity",
        "orelse-right-identity",
        "orelse-associative",
        "repeat-zero-is-skip",
        "repeat-one-is-body",
        "try-equals-orelse-skip",
        "solve-of-skip-fails-when-open",
        "first-of-singleton-collapses",
        "all-goals-of-skip-is-skip",
    ] {
        assert!(
            stdout.contains(name),
            "laws output missing law `{}`: {stdout}",
            name
        );
    }
    assert!(stdout.contains("Total: 12"));
}

#[test]
fn tactic_laws_json_well_formed() {
    let out = run(&["tactic", "laws", "--format", "json"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["count"], 12);
    let laws = parsed["laws"].as_array().unwrap();
    assert_eq!(laws.len(), 12);

    for l in laws {
        assert!(l["name"].is_string());
        assert!(l["lhs"].is_string());
        assert!(l["rhs"].is_string());
        assert!(l["rationale"].is_string());
        assert!(l["participants"].is_array());
        // Every law must name at least one participating combinator.
        assert!(!l["participants"].as_array().unwrap().is_empty());
    }
}

#[test]
fn tactic_laws_rejects_unknown_format() {
    let out = run(&["tactic", "laws", "--format", "yaml"]);
    assert!(!out.status.success());
}

// ─────────────────────────────────────────────────────────────────────
// Cross-endpoint consistency
// ─────────────────────────────────────────────────────────────────────

#[test]
fn list_and_explain_agree_on_signatures() {
    // Pin the contract that `list --format json` and `explain --format json`
    // produce identical signature / semantics strings for every combinator.
    let list_out = run(&["tactic", "list", "--format", "json"]);
    let list = serde_json::from_str::<serde_json::Value>(
        &String::from_utf8_lossy(&list_out.stdout),
    )
    .unwrap();
    let entries = list["entries"].as_array().unwrap();

    for e in entries {
        let name = e["name"].as_str().unwrap();
        let explain_out = run(&["tactic", "explain", name, "--format", "json"]);
        let explain: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&explain_out.stdout)).unwrap();
        assert_eq!(
            e["signature"], explain["signature"],
            "signature mismatch for {}",
            name
        );
        assert_eq!(
            e["semantics"], explain["semantics"],
            "semantics mismatch for {}",
            name
        );
        assert_eq!(
            e["doc_anchor"], explain["doc_anchor"],
            "doc_anchor mismatch for {}",
            name
        );
    }
}

#[test]
fn explain_law_names_subset_of_laws_endpoint() {
    // For every combinator, every law name listed in `explain` MUST
    // resolve to a law in the `laws` endpoint.  Pins the
    // single-source-of-truth contract between catalogue entries and
    // the algebraic-law inventory.
    let laws_out = run(&["tactic", "laws", "--format", "json"]);
    let laws_parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&laws_out.stdout)).unwrap();
    let known_law_names: std::collections::HashSet<String> = laws_parsed["laws"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap().to_string())
        .collect();

    let list_out = run(&["tactic", "list", "--format", "json"]);
    let list: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&list_out.stdout)).unwrap();
    let entries = list["entries"].as_array().unwrap();

    for e in entries {
        let name = e["name"].as_str().unwrap();
        let referenced: Vec<String> = e["laws"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect();
        for law in referenced {
            assert!(
                known_law_names.contains(&law),
                "combinator `{}` references unknown law `{}`",
                name,
                law
            );
        }
    }
}
