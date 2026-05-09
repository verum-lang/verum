#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! @setup / @teardown fixture directive drift guard (#58).
//!
//! `vcs/runner/vtest/src/directive.rs` parses test metadata from VCS spec
//! headers.  Task #58 adds two new directives:
//!
//!   `@setup: <fn_name>`    — names a fn() called before each test in the file.
//!   `@teardown: <fn_name>` — names a fn() called after each test in the file.
//!
//! This drift guard pins:
//!   1. The directive.rs module-level docstring lists `@setup:` and `@teardown:`.
//!   2. `TestDirectives` struct has `setup_fn: Option<Text>` and
//!      `teardown_fn: Option<Text>` fields.
//!   3. The parser handles `@setup:` and `@teardown:` prefixes.
//!   4. The VCS spec file `setup_teardown_fixtures.vr` uses both directives.
//!   5. `@setup`/`@teardown` do NOT appear in the DOC_ONLY exemption list
//!      (they are real parsed directives, not doc-only metadata).

const DIRECTIVE_RS: &str = include_str!("../../../vcs/runner/vtest/src/directive.rs");
const FIXTURE_SPEC: &str = include_str!(
    "../../../vcs/specs/L2-standard/testing/setup_teardown_fixtures.vr"
);

// ── 1. Documentation lists @setup and @teardown ───────────────────────────────

#[test]
fn directive_module_doc_lists_setup() {
    assert!(
        DIRECTIVE_RS.contains("@setup: <fn_name>"),
        "directive.rs module doc must document '@setup: <fn_name>'"
    );
}

#[test]
fn directive_module_doc_lists_teardown() {
    assert!(
        DIRECTIVE_RS.contains("@teardown: <fn_name>"),
        "directive.rs module doc must document '@teardown: <fn_name>'"
    );
}

// ── 2. TestDirectives struct fields ───────────────────────────────────────────

#[test]
fn test_directives_struct_has_setup_fn_field() {
    assert!(
        DIRECTIVE_RS.contains("pub setup_fn: Option<Text>"),
        "TestDirectives must have 'pub setup_fn: Option<Text>' field"
    );
}

#[test]
fn test_directives_struct_has_teardown_fn_field() {
    assert!(
        DIRECTIVE_RS.contains("pub teardown_fn: Option<Text>"),
        "TestDirectives must have 'pub teardown_fn: Option<Text>' field"
    );
}

// ── 3. Parser handles @setup: and @teardown: prefixes ─────────────────────────

#[test]
fn parser_handles_setup_prefix() {
    assert!(
        DIRECTIVE_RS.contains("strip_prefix(\"@setup:\")"),
        "directive.rs parser must handle '@setup:' prefix via strip_prefix"
    );
}

#[test]
fn parser_handles_teardown_prefix() {
    assert!(
        DIRECTIVE_RS.contains("strip_prefix(\"@teardown:\")"),
        "directive.rs parser must handle '@teardown:' prefix via strip_prefix"
    );
}

#[test]
fn parser_assigns_setup_fn_to_directives() {
    assert!(
        DIRECTIVE_RS.contains("directives.setup_fn"),
        "directive.rs must assign 'directives.setup_fn' when @setup: is found"
    );
}

#[test]
fn parser_assigns_teardown_fn_to_directives() {
    assert!(
        DIRECTIVE_RS.contains("directives.teardown_fn"),
        "directive.rs must assign 'directives.teardown_fn' when @teardown: is found"
    );
}

// ── 4. VCS spec uses both directives ──────────────────────────────────────────

#[test]
fn fixture_spec_declares_setup_directive() {
    assert!(
        FIXTURE_SPEC.contains("@setup:"),
        "setup_teardown_fixtures.vr must use the '@setup:' directive"
    );
}

#[test]
fn fixture_spec_declares_teardown_directive() {
    assert!(
        FIXTURE_SPEC.contains("@teardown:"),
        "setup_teardown_fixtures.vr must use the '@teardown:' directive"
    );
}

#[test]
fn fixture_spec_names_setup_function() {
    assert!(
        FIXTURE_SPEC.contains("fn test_setup()"),
        "setup_teardown_fixtures.vr must define 'fn test_setup()'"
    );
}

#[test]
fn fixture_spec_names_teardown_function() {
    assert!(
        FIXTURE_SPEC.contains("fn test_teardown()"),
        "setup_teardown_fixtures.vr must define 'fn test_teardown()'"
    );
}

// ── 5. @setup / @teardown are NOT in DOC_ONLY list ───────────────────────────

#[test]
fn setup_is_not_doc_only() {
    // The DOC_ONLY array holds directives that produce no parse warning when
    // unrecognised.  @setup is a real parsed directive, not doc-only.
    if let Some(doc_only_start) = DIRECTIVE_RS.find("const DOC_ONLY: &[&str]") {
        let snippet = &DIRECTIVE_RS[doc_only_start..doc_only_start + 200];
        assert!(
            !snippet.contains("\"@setup\""),
            "@setup must not appear in DOC_ONLY — it is a real parsed directive"
        );
    }
}

#[test]
fn teardown_is_not_doc_only() {
    if let Some(doc_only_start) = DIRECTIVE_RS.find("const DOC_ONLY: &[&str]") {
        let snippet = &DIRECTIVE_RS[doc_only_start..doc_only_start + 200];
        assert!(
            !snippet.contains("\"@teardown\""),
            "@teardown must not appear in DOC_ONLY — it is a real parsed directive"
        );
    }
}
