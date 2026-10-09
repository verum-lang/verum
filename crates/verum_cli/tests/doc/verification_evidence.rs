//! T1640: documentation reports source intent separately from proof evidence.
//! The actual generator runs in a child process so its project cwd is isolated.

use super::{execute, extract_functions_from_ast};
use crate::config::Manifest;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

const CASE_ENV: &str = "VERUM_T1640_DOC_CASE";
const CHILD_TEST: &str = "commands::doc::verification_evidence::isolated_generator_case";

const ANNOTATED: &str = r#"
@verify(runtime)
public fn runtime_only() ensures false {}

@verify(static)
public fn static_only() ensures false {}

@verify(formal)
public fn false_postcondition() ensures false {}

@verify(proof)
public fn proof_requested() ensures true {}

@verify(certified)
public fn certificate_requested() ensures true {}

@verify(runtime, formal)
public fn multiple_strategies() {}

@verify([runtime, formal])
public fn strategy_list() {}

@verify(thorugh)
public fn unknown_strategy() {}

@proven("<script>forged receipt</script>")
public fn asserted_proof() ensures false {}

public fn unannotated() {}
"#;

const FALLBACK: &str = r#"
/// Verified: the author's unsupported claim.
pub fn prose_claim() {}

/// @verify(certified)
pub fn comment_strategy() {}

// An incomplete declaration forces the existing source fallback.
pub fn unfinished(
"#;

#[test]
fn annotations_describe_intent_without_claiming_a_proof() {
    child_case("annotations");
}

#[test]
fn fallback_comments_cannot_create_a_proven_badge() {
    child_case("fallback");
}

#[test]
fn generated_index_explains_the_evidence_boundary() {
    child_case("index");
}

fn child_case(case: &str) {
    let output = Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", CHILD_TEST, "--nocapture"])
        .env(CASE_ENV, case)
        .output()
        .expect("spawn isolated generator test");
    assert!(
        output.status.success(),
        "case {case} failed:\nstdout:\n{}\nstderr:\n{}",
        std::str::from_utf8(&output.stdout).unwrap_or("non-UTF-8 stdout"),
        std::str::from_utf8(&output.stderr).unwrap_or("non-UTF-8 stderr"),
    );
}

#[test]
fn isolated_generator_case() {
    let Ok(case) = std::env::var(CASE_ENV) else {
        return;
    };
    let project = TempDir::new().expect("temporary documentation project");
    std::env::set_current_dir(project.path()).unwrap();
    std::fs::create_dir("src").unwrap();
    std::fs::write(
        Manifest::MANIFEST_FILENAME,
        "[cog]\nname = \"evidence-docs\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let source = if case == "fallback" {
        FALLBACK
    } else {
        ANNOTATED
    };
    let source_path = Path::new("src/sample.vr");
    let parsed = extract_functions_from_ast(source, false, source_path);
    if case == "fallback" {
        assert!(parsed.is_err(), "the fallback control must fail parsing");
    } else {
        assert_eq!(parsed.expect("valid annotated functions").len(), 10);
    }
    std::fs::write(source_path, source).unwrap();
    execute(false, false, true, "html").expect("generate actual project documentation");
    let html = std::fs::read_to_string("target/doc/sample.html").unwrap();
    let index = std::fs::read_to_string("target/doc/index.html").unwrap();

    match case.as_str() {
        "annotations" => {
            for (name, annotation) in [
                ("runtime_only", "@verify(runtime)"),
                ("static_only", "@verify(static)"),
                ("false_postcondition", "@verify(formal)"),
                ("proof_requested", "@verify(proof)"),
                ("certificate_requested", "@verify(certified)"),
                ("multiple_strategies", "@verify(runtime, formal)"),
                ("strategy_list", "@verify([runtime, formal])"),
                ("unknown_strategy", "@verify(thorugh)"),
                (
                    "asserted_proof",
                    "@proven(&quot;&lt;script&gt;forged receipt&lt;/script&gt;&quot;)",
                ),
            ] {
                let section = function_section(&html, name);
                assert!(
                    section.contains(&format!("<code>{annotation}</code>")),
                    "{name} must retain the declared annotation, including its strategy: {section}",
                );
                assert!(section.contains("Declared verification:"));
                assert_unverified(section);
            }
            let ordinary = function_section(&html, "unannotated");
            assert_unverified(ordinary);
            assert!(!ordinary.contains("Declared verification:"));
            assert!(
                !html.contains("<script>"),
                "annotations must remain escaped"
            );
        }
        "fallback" => {
            assert_unverified(function_section(&html, "prose_claim"));
            assert_unverified(function_section(&html, "comment_strategy"));
            assert!(html.contains("the author&#x27;s unsupported claim."));
            assert!(html.contains("@verify(certified)"));
        }
        "index" => {
            assert!(index.contains("Source annotations describe requested checks"));
            assert!(index.contains("Proof evidence is not evaluated"));
            assert!(!index.contains("Proven (0ns)"));
            assert!(!index.contains("Runtime checked"));
        }
        _ => panic!("unknown generator case: {case}"),
    }
}

fn function_section<'a>(html: &'a str, name: &str) -> &'a str {
    let heading = format!("<h2 id=\"{name}\">{name}</h2>");
    html.split("<div class=\"function-doc\">")
        .find(|section| section.contains(&heading))
        .unwrap_or_else(|| panic!("generated function section absent: {name}"))
}

fn assert_unverified(section: &str) {
    assert!(
        section.contains("<span class=\"badge badge-unverified\">Unverified</span>"),
        "source-only documentation cannot claim a checked result: {section}",
    );
    assert!(!section.contains("badge-proven"));
    assert!(section.contains("Proof evidence was not evaluated for this documentation."));
}
