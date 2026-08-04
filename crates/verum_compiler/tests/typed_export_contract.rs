//! Contract tests for the typed-IR export seam (T0675, with the
//! reproducibility and capability-visibility acceptance folded in from
//! T0677 / T0676 — see `docs/architecture/deterministic-profile-and-typed-export.md`).
//!
//! NOTE: CI currently runs `cargo test --workspace --lib --bins`, which
//! excludes integration tests — this gate is inert on PRs until CI adds
//! `--tests` (tracked as T0709). It still runs locally and documents the
//! contract.

use verum_compiler::typed_export::{
    build_typed_ir, to_canonical_bytes, TYPED_IR_SCHEMA, TYPED_IR_VERSION,
};

fn parse(source: &str) -> verum_ast::Module {
    verum_fast_parser::FastParser::new()
        .parse_module_str(source, verum_common::FileId::new(0))
        .expect("fixture must parse")
}

fn export_bytes(source: &str) -> Vec<u8> {
    let module = parse(source);
    to_canonical_bytes(&build_typed_ir(&module))
}

const FIXTURE: &str = r#"
type Temperature is { celsius: Float };

type Reading is Empty | Value(Int);

fn clamp(x: Int) -> Int {
    if x > 100 { 100 } else { x }
}

fn count_down(n: Int) -> Int {
    let mut i = n;
    let mut acc = 0;
    while i > 0 decreases i {
        acc = acc + i;
        i = i - 1;
    }
    acc
}
"#;

/// §5 (T0677 fold): two independent parse+convert+serialize runs over the
/// same source produce byte-identical artefacts. Any map-iteration-order,
/// environment or timestamp leakage breaks this equality.
#[test]
fn double_run_bytes_identical() {
    let first = export_bytes(FIXTURE);
    let second = export_bytes(FIXTURE);
    assert!(!first.is_empty());
    assert_eq!(
        first, second,
        "typed-IR export must be byte-identical across runs"
    );
}

/// The artefact self-describes: schema id + independent semver, and the
/// canonical form ends with exactly one trailing newline (a stable target
/// for line-oriented diff/hash tooling).
#[test]
fn schema_header_pinned() {
    let bytes = export_bytes(FIXTURE);
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).expect("artefact must be valid JSON");
    assert_eq!(value["schema"], TYPED_IR_SCHEMA);
    assert_eq!(value["version"], TYPED_IR_VERSION);
    assert!(bytes.ends_with(b"\n"));
    assert!(!bytes.ends_with(b"\n\n"));
}

/// §4: types and functions arrive sorted by name (canonical ordering is a
/// schema property, not an accident of declaration order).
#[test]
fn items_sorted_canonically() {
    let bytes = export_bytes(FIXTURE);
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let fn_names: Vec<&str> = value["module"]["functions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    let mut sorted = fn_names.clone();
    sorted.sort();
    assert_eq!(fn_names, sorted, "functions must be name-sorted");
    let ty_names: Vec<&str> = value["module"]["types"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    let mut ty_sorted = ty_names.clone();
    ty_sorted.sort();
    assert_eq!(ty_names, ty_sorted, "types must be name-sorted");
}

/// §4 + §6 (T0676 fold): declared DI contexts (`using [...]`), refinement
/// predicates and `decreases` loop metadata are all visible to a §4
/// consumer — the capability/effect surface a downstream backend keys on.
#[test]
fn contexts_refinements_and_bounds_visible() {
    let source = r#"
fn log_reading(level: Int) -> Int using [Logger] {
    level
}

fn safe_div(a: Int, b: Int{ it != 0 }) -> Int {
    a / b
}

fn walk(n: Int) -> Int {
    let mut i = n;
    while i > 0 decreases i {
        i = i - 1;
    }
    i
}
"#;
    let bytes = export_bytes(source);
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let functions = value["module"]["functions"].as_array().unwrap();

    let log_fn = functions
        .iter()
        .find(|f| f["name"] == "log_reading")
        .expect("log_reading exported");
    let contexts: Vec<&str> = log_fn["contexts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(contexts, ["Logger"], "declared `using` contexts travel");

    // The refinement arrives STRUCTURED: param `b` is a `refined` type
    // whose predicate tree mentions the binder `it` — richer than a
    // source-text echo, and exactly what a downstream verifier consumes.
    let div_fn = functions
        .iter()
        .find(|f| f["name"] == "safe_div")
        .expect("safe_div exported");
    let b_param = div_fn["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "b")
        .expect("param b exported");
    assert_eq!(
        b_param["ty"]["kind"], "refined",
        "refined parameter type must export as kind=refined"
    );
    let predicate_raw = b_param["ty"]["predicate"].to_string();
    assert!(
        predicate_raw.contains("it"),
        "refinement predicate tree must reference the binder, got {predicate_raw}"
    );

    let walk_fn = functions
        .iter()
        .find(|f| f["name"] == "walk")
        .expect("walk exported");
    let loops = walk_fn["loops"].as_array().unwrap();
    assert!(!loops.is_empty(), "loop metadata must be exported");
    let measures = loops[0]["measures"].as_array().unwrap();
    assert!(
        measures.iter().any(|d| d.as_str() == Some("i")),
        "the `decreases` measure must be recorded, got {measures:?}"
    );
    assert!(
        loops[0]["bound"]["class"].is_string(),
        "the §3 bound classification must be carried"
    );
}

/// Absolute-path hygiene: nothing in the artefact may leak the build
/// machine's filesystem layout.
#[test]
fn no_path_leakage() {
    let bytes = export_bytes(FIXTURE);
    let raw = String::from_utf8(bytes).unwrap();
    let cwd = std::env::current_dir().unwrap();
    let cwd_str = cwd.to_string_lossy();
    assert!(
        !raw.contains(cwd_str.as_ref()),
        "artefact must not embed the working directory"
    );
    assert!(!raw.contains("/Users/"), "artefact must not embed home paths");
    assert!(!raw.contains("C:\\"), "artefact must not embed Windows paths");
}
