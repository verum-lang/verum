//! End-to-end integration tests for `verum doc-render`.
//!
//! Spawns the actual `verum` binary against tempdir-rooted fixtures
//! and validates the chain:
//!
//!   `verum doc-render {render,graph,check-refs}` (clap) →
//!     `commands::doc_render::run_*` →
//!       walks .vr files →
//!       `verum_verification::doc_render::DefaultDocRenderer` →
//!         Markdown / LaTeX / HTML / DOT / JSON
//!
//! Together with the 22 trait-level tests in
//! `verum_verification::doc_render::tests` and the 8 handler tests in
//! `commands::doc_render::tests`, this proves the auto-paper pipeline
//! is consumable end-to-end.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

fn verum_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_verum"))
}

fn create_project(name: &str, main_vr_body: &str) -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("tempdir");
    let dir = temp.path().join(name);
    fs::create_dir_all(&dir).expect("create project dir");
    let manifest = format!(
        r#"[cog]
name = "{name}"
version = "0.1.0"

[language]
profile = "application"

[dependencies]
"#
    );
    fs::write(dir.join("Verum.toml"), manifest).expect("write Verum.toml");
    let src = dir.join("src");
    fs::create_dir_all(&src).expect("create src/");
    fs::write(src.join("main.vr"), main_vr_body).expect("write main.vr");
    (temp, dir)
}

fn run(args: &[&str], cwd: &PathBuf) -> Output {
    Command::new(verum_bin())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn verum CLI")
}

const TWO_THEOREMS: &str = r#"@verify(runtime)
public theorem t_a()
    ensures true
    proof by auto;

@verify(runtime)
public theorem t_b()
    ensures true
    proof by auto;

public fn main() {}
"#;

// ─────────────────────────────────────────────────────────────────────
// render — every format renders without panic
// ─────────────────────────────────────────────────────────────────────

#[test]
fn doc_render_markdown_lists_both_theorems() {
    let (_t, dir) = create_project("dr_md", TWO_THEOREMS);
    let out = run(
        &["doc-render", "render", "--format", "markdown"],
        &dir,
    );
    assert!(
        out.status.success(),
        "doc-render markdown exited non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("# Verum corpus"));
    assert!(stdout.contains("Table of contents"));
    assert!(stdout.contains("t_a"));
    assert!(stdout.contains("t_b"));
}

#[test]
fn doc_render_latex_emits_theorem_environment() {
    let (_t, dir) = create_project("dr_tex", TWO_THEOREMS);
    let out = run(&["doc-render", "render", "--format", "latex"], &dir);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\\section{Verum corpus"));
    assert!(stdout.contains("\\begin{theorem}"));
    assert!(stdout.contains("\\end{theorem}"));
}

#[test]
fn doc_render_html_wraps_in_section() {
    let (_t, dir) = create_project("dr_html", TWO_THEOREMS);
    let out = run(&["doc-render", "render", "--format", "html"], &dir);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("<section class=\"verum-corpus\">"));
    assert!(stdout.contains("<article class=\"verum-item verum-theorem\""));
    assert!(stdout.contains("</section>"));
}

#[test]
fn doc_render_short_alias_md_works() {
    let (_t, dir) = create_project("dr_alias", TWO_THEOREMS);
    let out = run(&["doc-render", "render", "--format", "md"], &dir);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("# Verum corpus"));
}

#[test]
fn doc_render_rejects_unknown_format() {
    let (_t, dir) = create_project("dr_bad", TWO_THEOREMS);
    let out = run(&["doc-render", "render", "--format", "yaml"], &dir);
    assert!(
        !out.status.success(),
        "unknown --format must produce non-zero exit"
    );
}

#[test]
fn doc_render_writes_to_out_path_when_supplied() {
    let (_t, dir) = create_project("dr_out", TWO_THEOREMS);
    let out_path = dir.join("out.md");
    let out = run(
        &[
            "doc-render",
            "render",
            "--format",
            "md",
            "--out",
            out_path.to_str().unwrap(),
        ],
        &dir,
    );
    assert!(out.status.success());
    assert!(out_path.exists(), "out file must be created");
    let body = fs::read_to_string(&out_path).unwrap();
    assert!(body.contains("# Verum corpus"));
    assert!(body.contains("t_a"));
}

// ─────────────────────────────────────────────────────────────────────
// public-only filter
// ─────────────────────────────────────────────────────────────────────

#[test]
fn doc_render_public_filter_hides_private_theorems() {
    let (_t, dir) = create_project(
        "dr_public",
        r#"@verify(runtime)
public theorem t_pub()
    ensures true
    proof by auto;

@verify(runtime)
theorem t_priv()
    ensures true
    proof by auto;

public fn main() {}
"#,
    );
    let out = run(
        &[
            "doc-render",
            "render",
            "--format",
            "json",
            "--public",
        ],
        &dir,
    );
    // JSON wasn't in the doc-render render formats — instead we
    // exercise --public via Markdown and inspect the body.
    let _ = out;
    let out_md = run(
        &["doc-render", "render", "--format", "md", "--public"],
        &dir,
    );
    assert!(out_md.status.success());
    let stdout = String::from_utf8_lossy(&out_md.stdout);
    assert!(stdout.contains("t_pub"), "public theorem must appear");
    assert!(
        !stdout.contains("t_priv"),
        "private theorem must be filtered out: {stdout}"
    );
}

// ─────────────────────────────────────────────────────────────────────
// graph
// ─────────────────────────────────────────────────────────────────────

#[test]
fn doc_render_graph_dot_emits_well_formed_digraph() {
    let (_t, dir) = create_project("dr_graph", TWO_THEOREMS);
    let out = run(&["doc-render", "graph", "--format", "dot"], &dir);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("digraph corpus_citations"));
    assert!(stdout.contains("\"t_a\""));
    assert!(stdout.contains("\"t_b\""));
}

#[test]
fn doc_render_graph_json_well_formed() {
    let (_t, dir) = create_project("dr_graph_json", TWO_THEOREMS);
    let out = run(&["doc-render", "graph", "--format", "json"], &dir);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["item_count"], 2);
    let edges = parsed["edges"].as_array().unwrap();
    // No proof citations in this fixture → 0 edges.
    assert_eq!(edges.len(), 0);
}

#[test]
fn doc_render_graph_rejects_unknown_format() {
    let (_t, dir) = create_project("dr_graph_bad", TWO_THEOREMS);
    let out = run(&["doc-render", "graph", "--format", "svg"], &dir);
    assert!(
        !out.status.success(),
        "unknown graph --format must produce non-zero exit"
    );
}

// ─────────────────────────────────────────────────────────────────────
// check-refs — clean / broken / json
// ─────────────────────────────────────────────────────────────────────

#[test]
fn doc_render_check_refs_clean_corpus_succeeds() {
    let (_t, dir) = create_project("dr_refs_clean", TWO_THEOREMS);
    let out = run(&["doc-render", "check-refs"], &dir);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("✓") || stdout.contains("All "));
}

#[test]
fn doc_render_check_refs_json_well_formed() {
    let (_t, dir) = create_project("dr_refs_json", TWO_THEOREMS);
    let out = run(&["doc-render", "check-refs", "--format", "json"], &dir);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["broken_count"], 0);
    assert!(parsed["broken"].as_array().unwrap().is_empty());
}

#[test]
fn doc_render_check_refs_rejects_unknown_format() {
    let (_t, dir) = create_project("dr_refs_bad", TWO_THEOREMS);
    let out = run(
        &["doc-render", "check-refs", "--format", "yaml"],
        &dir,
    );
    assert!(!out.status.success());
}

// ─────────────────────────────────────────────────────────────────────
// Empty project — every subcommand runs cleanly
// ─────────────────────────────────────────────────────────────────────

#[test]
fn doc_render_empty_project_render_succeeds() {
    let (_t, dir) = create_project(
        "dr_empty",
        r#"public fn main() {}
"#,
    );
    let out = run(&["doc-render", "render"], &dir);
    assert!(out.status.success());
}

#[test]
fn doc_render_empty_project_graph_succeeds() {
    let (_t, dir) = create_project(
        "dr_empty_g",
        r#"public fn main() {}
"#,
    );
    let out = run(&["doc-render", "graph"], &dir);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("digraph"));
}

#[test]
fn doc_render_empty_project_check_refs_succeeds() {
    let (_t, dir) = create_project(
        "dr_empty_r",
        r#"public fn main() {}
"#,
    );
    let out = run(&["doc-render", "check-refs"], &dir);
    assert!(out.status.success());
}
