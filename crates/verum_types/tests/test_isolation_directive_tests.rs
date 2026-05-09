#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! @isolation test directive drift guard (#59).
//!
//! `vcs/runner/vtest/src/isolation.rs` defines `IsolationLevel` with four
//! variants: None, Process (default), Directory, Container.
//!
//! Task #59 adds `@isolation: <level>` as a first-class directive so spec
//! files can request subprocess-per-test execution for stateful tests.
//!
//! This drift guard pins:
//!   1. isolation.rs exports `IsolationLevel` with the four expected variants.
//!   2. directive.rs documents `@isolation: <level>`.
//!   3. TestDirectives has `isolation: Option<Text>` field.
//!   4. Parser handles `@isolation:` prefix.
//!   5. The VCS spec `test_isolation_subprocess.vr` uses `@isolation: process`.
//!   6. IsolationLevel::from_str handles all four valid values.

const ISOLATION_RS: &str = include_str!("../../../vcs/runner/vtest/src/isolation.rs");
const DIRECTIVE_RS: &str = include_str!("../../../vcs/runner/vtest/src/directive.rs");
const ISOLATION_SPEC: &str = include_str!(
    "../../../vcs/specs/L2-standard/testing/test_isolation_subprocess.vr"
);

// ── 1. IsolationLevel variants ────────────────────────────────────────────────

#[test]
fn isolation_level_none_variant_exists() {
    assert!(
        ISOLATION_RS.contains("None,") || ISOLATION_RS.contains("None\n"),
        "IsolationLevel must have a None variant in isolation.rs"
    );
}

#[test]
fn isolation_level_process_variant_exists() {
    assert!(
        ISOLATION_RS.contains("Process,") || ISOLATION_RS.contains("Process\n"),
        "IsolationLevel must have a Process variant in isolation.rs"
    );
}

#[test]
fn isolation_level_directory_variant_exists() {
    assert!(
        ISOLATION_RS.contains("Directory,") || ISOLATION_RS.contains("Directory\n"),
        "IsolationLevel must have a Directory variant in isolation.rs"
    );
}

#[test]
fn isolation_level_container_variant_exists() {
    assert!(
        ISOLATION_RS.contains("Container,") || ISOLATION_RS.contains("Container\n"),
        "IsolationLevel must have a Container variant in isolation.rs"
    );
}

#[test]
fn isolation_level_process_is_default() {
    assert!(
        ISOLATION_RS.contains("#[default]"),
        "IsolationLevel must have a #[default] variant (expected: Process)"
    );
}

#[test]
fn isolation_level_from_str_handles_none() {
    assert!(
        ISOLATION_RS.contains("\"none\" =>"),
        "IsolationLevel::from_str must handle \"none\""
    );
}

#[test]
fn isolation_level_from_str_handles_process() {
    assert!(
        ISOLATION_RS.contains("\"process\" =>"),
        "IsolationLevel::from_str must handle \"process\""
    );
}

#[test]
fn isolation_level_from_str_handles_directory() {
    assert!(
        ISOLATION_RS.contains("\"directory\"") || ISOLATION_RS.contains("\"dir\""),
        "IsolationLevel::from_str must handle \"directory\" or \"dir\""
    );
}

#[test]
fn isolation_level_from_str_handles_container() {
    assert!(
        ISOLATION_RS.contains("\"container\" =>"),
        "IsolationLevel::from_str must handle \"container\""
    );
}

// ── 2. directive.rs documents @isolation ─────────────────────────────────────

#[test]
fn directive_module_doc_lists_isolation() {
    assert!(
        DIRECTIVE_RS.contains("@isolation:"),
        "directive.rs module doc must document '@isolation: <level>'"
    );
}

// ── 3. TestDirectives has isolation field ─────────────────────────────────────

#[test]
fn test_directives_struct_has_isolation_field() {
    assert!(
        DIRECTIVE_RS.contains("pub isolation: Option<Text>"),
        "TestDirectives must have 'pub isolation: Option<Text>' field"
    );
}

// ── 4. Parser handles @isolation: prefix ─────────────────────────────────────

#[test]
fn parser_handles_isolation_prefix() {
    assert!(
        DIRECTIVE_RS.contains("strip_prefix(\"@isolation:\")"),
        "directive.rs parser must handle '@isolation:' via strip_prefix"
    );
}

#[test]
fn parser_assigns_isolation_to_directives() {
    assert!(
        DIRECTIVE_RS.contains("directives.isolation"),
        "directive.rs must assign 'directives.isolation' when @isolation: is found"
    );
}

// ── 5. VCS spec uses @isolation: process ─────────────────────────────────────

#[test]
fn isolation_spec_uses_process_level() {
    assert!(
        ISOLATION_SPEC.contains("@isolation: process"),
        "test_isolation_subprocess.vr must use '@isolation: process'"
    );
}

#[test]
fn isolation_spec_is_typecheck_pass() {
    assert!(
        ISOLATION_SPEC.contains("@test: typecheck-pass"),
        "test_isolation_subprocess.vr must be '@test: typecheck-pass'"
    );
}
