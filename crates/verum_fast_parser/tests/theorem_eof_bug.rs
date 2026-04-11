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

//! Regression: generic theorem with `proof by auto;` at EOF fails
//! to parse with "expected tactic expression" error at the imaginary
//! line past EOF. This file isolates the pattern as parser unit tests.

use verum_common::span::FileId;
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;

fn try_parse(source: &str) -> Result<(), String> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser
        .parse_module(lexer, file_id)
        .map(|_| ())
        .map_err(|e| format!("{:?}", e))
}

#[test]
fn theorem_no_generic_proof_by_auto_semi_at_eof_with_newline() {
    let source = "theorem foo() proof by auto;\n";
    let r = try_parse(source);
    eprintln!("no-generic-with-newline: {:?}", r);
    assert!(r.is_ok(), "no-generic theorem with trailing `;\\n` at EOF should parse");
}

#[test]
fn theorem_no_generic_proof_by_auto_semi_at_eof_no_newline() {
    let source = "theorem foo() proof by auto;";
    let r = try_parse(source);
    eprintln!("no-generic-no-newline: {:?}", r);
    assert!(r.is_ok(), "no-generic theorem with trailing `;` (no newline) at EOF should parse");
}

#[test]
fn theorem_multiline_proof_body_at_eof() {
    // Critical: with the proof body on its OWN line, does it parse?
    let source = "theorem foo()\n    proof by auto;\n";
    let r = try_parse(source);
    eprintln!("multiline: {:?}", r);
    assert!(r.is_ok(), "multiline theorem with trailing `;\\n` should parse");
}

#[test]
fn theorem_multiline_proof_body_no_trailing_newline() {
    let source = "theorem foo()\n    proof by auto;";
    let r = try_parse(source);
    eprintln!("multiline-no-trail: {:?}", r);
    assert!(r.is_ok(), "multiline theorem with trailing `;` (no trailing \\n) should parse");
}

#[test]
fn theorem_with_generic_proof_by_auto_no_semi_at_eof() {
    let source = "theorem foo<T>() proof by auto";
    let r = try_parse(source);
    eprintln!("generic-no-semi: {:?}", r);
    assert!(r.is_ok(), "generic theorem without trailing `;` at EOF should parse");
}

#[test]
fn theorem_with_generic_proof_by_auto_semi_at_eof_REPRO() {
    let source = "theorem foo<T>() proof by auto;";
    let r = try_parse(source);
    eprintln!("REPRO: {:?}", r);
    // This reproduces the bug. Expected to PASS; if it fails, bug is confirmed.
    assert!(
        r.is_ok(),
        "generic theorem with trailing `;` at EOF should parse: {:?}",
        r
    );
}

#[test]
fn theorem_with_generic_proof_by_auto_semi_followed_by_item() {
    let source = "theorem foo<T>() proof by auto;\nfn main() { }";
    let r = try_parse(source);
    eprintln!("generic-semi-with-trailing-fn: {:?}", r);
    assert!(r.is_ok(), "generic theorem with trailing `;` and following item should parse");
}
