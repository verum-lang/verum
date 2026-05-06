#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! # CLI framework smoke tests
//!
//! Validates that every `core/cli/*.vr` file parses cleanly. The primary
//! gate is the existing `core_stdlib_validation_test` — this file adds
//! finer-grained per-module assertions that flag *which* file is at
//! fault when the umbrella test fails.
//!
//! Run with:
//!
//!     cargo test -p verum_compiler --test cli_framework_smoke
//!
//! Spec: internal/specs/cli-framework.md

use std::fs;
use std::path::{Path, PathBuf};

use verum_ast::span::FileId;
use verum_fast_parser::FastParser;

fn project_core_cli() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("core")
        .join("cli")
}

fn parse_one(path: &Path) -> Result<(), String> {
    let src = fs::read_to_string(path).map_err(|e| format!("read {path:?}: {e}"))?;
    let parser = FastParser::new();
    parser
        .parse_module_str(&src, FileId::new(0))
        .map(|_| ())
        .map_err(|e| format!("parse {path:?}: {e:?}"))
}

#[test]
fn cli_mod_parses() {
    parse_one(&project_core_cli().join("mod.vr")).expect("mod.vr should parse");
}

#[test]
fn cli_spec_parses() {
    parse_one(&project_core_cli().join("spec.vr")).expect("spec.vr should parse");
}

#[test]
fn cli_types_parses() {
    parse_one(&project_core_cli().join("types.vr")).expect("types.vr should parse");
}

#[test]
fn cli_error_parses() {
    parse_one(&project_core_cli().join("error.vr")).expect("error.vr should parse");
}

#[test]
fn cli_parser_parses() {
    parse_one(&project_core_cli().join("parser.vr")).expect("parser.vr should parse");
}

#[test]
fn cli_help_parses() {
    parse_one(&project_core_cli().join("help.vr")).expect("help.vr should parse");
}

#[test]
fn cli_builder_parses() {
    parse_one(&project_core_cli().join("builder.vr")).expect("builder.vr should parse");
}

#[test]
fn cli_runtime_parses() {
    parse_one(&project_core_cli().join("runtime.vr")).expect("runtime.vr should parse");
}

#[test]
fn cli_derive_parses() {
    parse_one(&project_core_cli().join("derive.vr")).expect("derive.vr should parse");
}

#[test]
fn cli_refinement_parses() {
    parse_one(&project_core_cli().join("refinement.vr"))
        .expect("refinement.vr should parse");
}

#[test]
fn cli_completion_parses() {
    parse_one(&project_core_cli().join("completion.vr"))
        .expect("completion.vr should parse");
}

#[test]
fn cli_manpage_parses() {
    parse_one(&project_core_cli().join("manpage.vr")).expect("manpage.vr should parse");
}

#[test]
fn cli_config_parses() {
    parse_one(&project_core_cli().join("config.vr")).expect("config.vr should parse");
}

#[test]
fn cli_testing_parses() {
    parse_one(&project_core_cli().join("testing.vr")).expect("testing.vr should parse");
}

#[test]
fn cli_example_parses() {
    parse_one(&project_core_cli().join("example.vr")).expect("example.vr should parse");
}

#[test]
fn every_cli_vr_parses() {
    let dir = project_core_cli();
    let mut failures = Vec::new();
    for entry in fs::read_dir(&dir).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().and_then(|s| s.to_str()) != Some("vr") {
            continue;
        }
        if let Err(msg) = parse_one(&p) {
            failures.push(msg);
        }
    }
    assert!(
        failures.is_empty(),
        "core/cli/*.vr parse failures:\n{}",
        failures.join("\n")
    );
}
