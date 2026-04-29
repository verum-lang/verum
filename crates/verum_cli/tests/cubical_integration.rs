//! End-to-end integration tests for `verum cubical`.

use std::path::PathBuf;
use std::process::{Command, Output};

fn verum_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_verum"))
}

fn run(args: &[&str]) -> Output {
    Command::new(verum_bin())
        .args(args)
        .output()
        .expect("CLI invocation must succeed")
}

// ─────────────────────────────────────────────────────────────────────
// primitives
// ─────────────────────────────────────────────────────────────────────

#[test]
fn primitives_lists_seventeen_canonical_entries() {
    let out = run(&["cubical", "primitives", "--output", "json"]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["count"], 17);
    let entries = parsed["entries"].as_array().unwrap();
    let names: std::collections::BTreeSet<&str> = entries
        .iter()
        .map(|e| e["primitive"].as_str().unwrap())
        .collect();
    for required in [
        "path",
        "refl",
        "j_rule",
        "transp",
        "hcomp",
        "comp",
        "glue",
        "equiv",
        "univalence",
    ] {
        assert!(names.contains(required), "missing primitive {}", required);
    }
}

#[test]
fn primitives_with_category_filter_restricts_count() {
    let mut total = 0;
    for cat in [
        "identity",
        "path_ops",
        "induction",
        "transport",
        "composition",
        "glue",
        "universe",
    ] {
        let out = run(&[
            "cubical",
            "primitives",
            "--category",
            cat,
            "--output",
            "json",
        ]);
        assert!(out.status.success(), "category {} failed", cat);
        let parsed: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
        let n = parsed["count"].as_u64().unwrap();
        total += n;
        let entries = parsed["entries"].as_array().unwrap();
        for e in entries {
            assert_eq!(e["category"], cat);
        }
    }
    assert_eq!(total, 17, "category counts must sum to 17");
}

#[test]
fn primitives_rejects_unknown_category() {
    let out = run(&["cubical", "primitives", "--category", "garbage"]);
    assert!(!out.status.success());
}

#[test]
fn primitives_markdown_table() {
    let out = run(&["cubical", "primitives", "--output", "markdown"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("# Cubical primitive catalogue"));
    assert!(stdout.contains("| Name | Category | Semantics |"));
}

// ─────────────────────────────────────────────────────────────────────
// explain
// ─────────────────────────────────────────────────────────────────────

#[test]
fn explain_resolves_every_canonical_primitive() {
    for name in [
        "path",
        "path_over",
        "refl",
        "sym",
        "trans",
        "ap",
        "apd",
        "j_rule",
        "transp",
        "coe",
        "subst",
        "hcomp",
        "comp",
        "glue",
        "unglue",
        "equiv",
        "univalence",
    ] {
        let out = run(&["cubical", "explain", name, "--output", "json"]);
        assert!(out.status.success(), "explain {} failed", name);
        let parsed: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
        assert_eq!(parsed["primitive"], name);
    }
}

#[test]
fn explain_aliases_resolve() {
    // `transport` → `transp`, `ua` → `univalence`, `J` → `j_rule`.
    for (alias, _canonical) in [("transport", "transp"), ("ua", "univalence"), ("j", "j_rule")] {
        let out = run(&["cubical", "explain", alias, "--output", "json"]);
        assert!(out.status.success(), "alias {} failed", alias);
    }
}

#[test]
fn explain_rejects_unknown() {
    let out = run(&["cubical", "explain", "garbage"]);
    assert!(!out.status.success());
}

#[test]
fn explain_carries_computation_rules() {
    // `hcomp` participates in two reduction rules.
    let out = run(&["cubical", "explain", "hcomp", "--output", "json"]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let rules = parsed["computation_rules"].as_array().unwrap();
    assert!(!rules.is_empty(), "hcomp must reference computation rules");
}

#[test]
fn explain_markdown_format() {
    let out = run(&["cubical", "explain", "univalence", "--output", "markdown"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("# `univalence`"));
    assert!(stdout.contains("**Signature:**"));
    assert!(stdout.contains("**Semantics:**"));
}

// ─────────────────────────────────────────────────────────────────────
// rules
// ─────────────────────────────────────────────────────────────────────

#[test]
fn rules_lists_substantive_inventory() {
    let out = run(&["cubical", "rules", "--output", "json"]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let count = parsed["count"].as_u64().unwrap();
    assert!(count >= 25, "V0 rule inventory must be substantive: got {}", count);
    let rules = parsed["rules"].as_array().unwrap();
    let names: std::collections::BTreeSet<&str> = rules
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    // Pin every reduction rule called out in #78.
    for required in [
        "path-J",
        "transp-fill",
        "coe-uncurry",
        "hcomp-id-when-empty-system",
        "ua-id",
        "ua-trans",
        "ua-unique",
    ] {
        assert!(
            names.contains(required),
            "computation rule `{}` missing — required by #78 acceptance",
            required
        );
    }
}

#[test]
fn rules_markdown_table() {
    let out = run(&["cubical", "rules", "--output", "markdown"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("# Cubical computation rules"));
    assert!(stdout.contains("| Name | LHS ↪ RHS | Rationale |"));
}

// ─────────────────────────────────────────────────────────────────────
// face
// ─────────────────────────────────────────────────────────────────────

#[test]
fn face_parses_canonical_grammar() {
    for s in [
        "1",
        "0",
        "i = 0",
        "j = 1",
        "i = 0 ∧ j = 1",
        "i = 0 ∨ j = 1",
        "(i = 0 ∨ j = 1) ∧ k = 1",
    ] {
        let out = run(&["cubical", "face", s, "--output", "json"]);
        assert!(out.status.success(), "face `{}` failed", s);
    }
}

#[test]
fn face_parses_ascii_alternatives() {
    for s in [
        "i = 0 /\\ j = 1",
        "i = 0 \\/ j = 1",
        "i = 0 and j = 1",
        "i = 0 or j = 1",
    ] {
        let out = run(&["cubical", "face", s]);
        assert!(out.status.success(), "ASCII face `{}` failed", s);
    }
}

#[test]
fn face_records_free_variables() {
    let out = run(&[
        "cubical",
        "face",
        "i = 0 ∧ (j = 1 ∨ k = 0)",
        "--output",
        "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let vars = parsed["free_variables"].as_array().unwrap();
    let names: std::collections::BTreeSet<&str> =
        vars.iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(names.len(), 3);
    for required in ["i", "j", "k"] {
        assert!(names.contains(required), "free var `{}` missing", required);
    }
}

#[test]
fn face_canonical_round_trip() {
    let out = run(&[
        "cubical",
        "face",
        "i = 0 ∧ j = 1",
        "--output",
        "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let canonical = parsed["canonical"].as_str().unwrap();
    // Re-parse the canonical form — must succeed.
    let out2 = run(&["cubical", "face", canonical]);
    assert!(out2.status.success(), "canonical `{}` did not re-parse", canonical);
}

#[test]
fn face_rejects_malformed() {
    for s in ["i =", "i = 2", "(i = 0", "garbage @ 0"] {
        let out = run(&["cubical", "face", s]);
        assert!(!out.status.success(), "malformed `{}` should reject", s);
    }
}

#[test]
fn face_rejects_empty() {
    let out = run(&["cubical", "face", ""]);
    assert!(!out.status.success());
}

// ─────────────────────────────────────────────────────────────────────
// Acceptance pin
// ─────────────────────────────────────────────────────────────────────

#[test]
fn task_78_every_acceptance_bullet_reachable_via_cli() {
    // §1: HComp / §2: Transp / §3: Glue / §4: ua / Univalence — all
    // reachable through `cubical explain`.
    for primitive in ["hcomp", "transp", "glue", "univalence"] {
        let out = run(&["cubical", "explain", primitive, "--output", "json"]);
        assert!(
            out.status.success(),
            "primitive `{}` not reachable",
            primitive
        );
    }
    // §5: transport reductions present in `cubical rules` output.
    let out = run(&["cubical", "rules", "--output", "json"]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let names: std::collections::BTreeSet<&str> = parsed["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    for rule in ["transp-fill", "transp-on-refl", "coe-uncurry", "subst-refl"] {
        assert!(
            names.contains(rule),
            "transport-reduction rule `{}` missing",
            rule
        );
    }
    // §6: HIT support — Path / J / Glue / hcomp + Univalence are
    // the building blocks; pin their presence.
    for primitive in ["path", "j_rule", "glue", "hcomp"] {
        let out = run(&["cubical", "explain", primitive, "--output", "json"]);
        assert!(out.status.success(), "HIT primitive `{}` missing", primitive);
    }
}

#[test]
fn task_78_face_formula_grammar_complete_via_cli() {
    // CCHM face-formula grammar must accept the full canonical
    // grammar.  Pin every production via the CLI.
    for s in [
        "1",
        "0",
        "⊤",
        "⊥",
        "top",
        "bot",
        "i = 0",
        "i = 1",
        "i = 0 ∧ j = 1",
        "i = 0 ∨ j = 1",
        "(i = 0 ∨ j = 1) ∧ k = 1",
        "i = 0 /\\ j = 1",
        "i = 0 \\/ j = 1",
        "i = 0 and j = 1",
        "i = 0 or j = 1",
    ] {
        let out = run(&["cubical", "face", s]);
        assert!(out.status.success(), "face grammar production `{}` rejected", s);
    }
}
