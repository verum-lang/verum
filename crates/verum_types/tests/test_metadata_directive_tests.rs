#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! @flaky / @slow / @hardware / @deprecated test metadata drift guard (#65).
//!
//! `vcs/runner/vtest/src/directive.rs` wires four test-metadata directives into
//! `TestDirectives`.
//!
//! This drift guard pins:
//!   1. directive.rs documents `@flaky:`.
//!   2. directive.rs documents `@slow:`.
//!   3. directive.rs documents `@hardware:`.
//!   4. directive.rs documents `@deprecated:`.
//!   5. TestDirectives has `flaky: Option<Text>`.
//!   6. TestDirectives has `slow_threshold_ms: Option<u64>`.
//!   7. TestDirectives has `hardware: Option<Text>`.
//!   8. TestDirectives has `deprecated: Option<Text>`.
//!   9. Parser handles `@flaky:` prefix.
//!  10. Parser handles `@slow:` prefix.
//!  11. Parser handles `@hardware:` prefix.
//!  12. Parser handles `@deprecated:` prefix.
//!  13. VCS spec uses all four directives.

const DIRECTIVE_RS: &str = include_str!("../../../vcs/runner/vtest/src/directive.rs");
const META_SPEC: &str = include_str!(
    "../../../vcs/specs/L2-standard/testing/test_metadata_directives.vr"
);

// ── 1-4. doc entries ──────────────────────────────────────────────────────────

#[test]
fn directive_doc_lists_flaky() {
    assert!(
        DIRECTIVE_RS.contains("@flaky:"),
        "directive.rs must document '@flaky:'"
    );
}

#[test]
fn directive_doc_lists_slow() {
    assert!(
        DIRECTIVE_RS.contains("@slow:"),
        "directive.rs must document '@slow:'"
    );
}

#[test]
fn directive_doc_lists_hardware() {
    assert!(
        DIRECTIVE_RS.contains("@hardware:"),
        "directive.rs must document '@hardware:'"
    );
}

#[test]
fn directive_doc_lists_deprecated() {
    assert!(
        DIRECTIVE_RS.contains("@deprecated:"),
        "directive.rs must document '@deprecated:'"
    );
}

// ── 5-8. TestDirectives fields ────────────────────────────────────────────────

#[test]
fn test_directives_has_flaky_option_text() {
    assert!(
        DIRECTIVE_RS.contains("pub flaky: Option<Text>"),
        "TestDirectives must have 'pub flaky: Option<Text>' field"
    );
}

#[test]
fn test_directives_has_slow_threshold_ms() {
    assert!(
        DIRECTIVE_RS.contains("pub slow_threshold_ms: Option<u64>"),
        "TestDirectives must have 'pub slow_threshold_ms: Option<u64>' field"
    );
}

#[test]
fn test_directives_has_hardware_option_text() {
    assert!(
        DIRECTIVE_RS.contains("pub hardware: Option<Text>"),
        "TestDirectives must have 'pub hardware: Option<Text>' field"
    );
}

#[test]
fn test_directives_has_deprecated_option_text() {
    assert!(
        DIRECTIVE_RS.contains("pub deprecated: Option<Text>"),
        "TestDirectives must have 'pub deprecated: Option<Text>' field"
    );
}

// ── 9-12. parser branches ─────────────────────────────────────────────────────

#[test]
fn parser_handles_flaky_prefix() {
    assert!(
        DIRECTIVE_RS.contains("strip_prefix(\"@flaky:\")"),
        "directive.rs parser must handle '@flaky:' via strip_prefix"
    );
}

#[test]
fn parser_handles_slow_prefix() {
    assert!(
        DIRECTIVE_RS.contains("strip_prefix(\"@slow:\")"),
        "directive.rs parser must handle '@slow:' via strip_prefix"
    );
}

#[test]
fn parser_handles_hardware_prefix() {
    assert!(
        DIRECTIVE_RS.contains("strip_prefix(\"@hardware:\")"),
        "directive.rs parser must handle '@hardware:' via strip_prefix"
    );
}

#[test]
fn parser_handles_deprecated_prefix() {
    assert!(
        DIRECTIVE_RS.contains("strip_prefix(\"@deprecated:\")"),
        "directive.rs parser must handle '@deprecated:' via strip_prefix"
    );
}

// ── 13. VCS spec ──────────────────────────────────────────────────────────────

#[test]
fn meta_spec_uses_flaky() {
    assert!(
        META_SPEC.contains("@flaky:"),
        "test_metadata_directives.vr must use '@flaky:'"
    );
}

#[test]
fn meta_spec_uses_slow() {
    assert!(
        META_SPEC.contains("@slow:"),
        "test_metadata_directives.vr must use '@slow:'"
    );
}

#[test]
fn meta_spec_uses_hardware() {
    assert!(
        META_SPEC.contains("@hardware:"),
        "test_metadata_directives.vr must use '@hardware:'"
    );
}

#[test]
fn meta_spec_uses_deprecated() {
    assert!(
        META_SPEC.contains("@deprecated:"),
        "test_metadata_directives.vr must use '@deprecated:'"
    );
}

#[test]
fn meta_spec_is_typecheck_pass() {
    assert!(
        META_SPEC.contains("@test: typecheck-pass"),
        "test_metadata_directives.vr must be '@test: typecheck-pass'"
    );
}
