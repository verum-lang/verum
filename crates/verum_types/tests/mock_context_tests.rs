#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! @mock directive drift guard (#62).
//!
//! `vcs/runner/vtest/src/mock.rs` implements the MockRegistry used to inject
//! context mocks during test runs.
//! `vcs/runner/vtest/src/directive.rs` wires the `@mock:` directive.
//!
//! This drift guard pins:
//!   1. mock.rs defines `MockRegistry` struct.
//!   2. MockRegistry has `register` method.
//!   3. MockRegistry has `get` method.
//!   4. MockRegistry has `is_empty` and `len` methods.
//!   5. mock.rs defines `MockEntry` struct.
//!   6. MockEntry has `value`, `noop`, and `forbidden` constructors.
//!   7. MockEntry has `panic_on_access: bool` field.
//!   8. directive.rs documents `@mock:` directive.
//!   9. TestDirectives has `mocks: List<Text>` field.
//!  10. Parser handles `@mock:` prefix.
//!  11. The VCS spec uses `@mock: Database` and `@mock: Logger`.

const MOCK_RS: &str = include_str!("../../../vcs/runner/vtest/src/mock.rs");
const DIRECTIVE_RS: &str = include_str!("../../../vcs/runner/vtest/src/directive.rs");
const MOCK_SPEC: &str = include_str!(
    "../../../vcs/specs/L2-standard/testing/mock_context_injection.vr"
);

// ── 1. MockRegistry struct ────────────────────────────────────────────────────

#[test]
fn mock_registry_struct_exists() {
    assert!(
        MOCK_RS.contains("pub struct MockRegistry"),
        "mock.rs must define 'pub struct MockRegistry'"
    );
}

// ── 2. register method ────────────────────────────────────────────────────────

#[test]
fn mock_registry_has_register_method() {
    assert!(
        MOCK_RS.contains("pub fn register"),
        "MockRegistry must have 'pub fn register' method"
    );
}

// ── 3. get method ─────────────────────────────────────────────────────────────

#[test]
fn mock_registry_has_get_method() {
    assert!(
        MOCK_RS.contains("pub fn get"),
        "MockRegistry must have 'pub fn get' method"
    );
}

// ── 4. is_empty and len ───────────────────────────────────────────────────────

#[test]
fn mock_registry_has_is_empty() {
    assert!(
        MOCK_RS.contains("pub fn is_empty"),
        "MockRegistry must have 'pub fn is_empty' method"
    );
}

#[test]
fn mock_registry_has_len() {
    assert!(
        MOCK_RS.contains("pub fn len"),
        "MockRegistry must have 'pub fn len' method"
    );
}

// ── 5. MockEntry struct ───────────────────────────────────────────────────────

#[test]
fn mock_entry_struct_exists() {
    assert!(
        MOCK_RS.contains("pub struct MockEntry"),
        "mock.rs must define 'pub struct MockEntry'"
    );
}

// ── 6. MockEntry constructors ─────────────────────────────────────────────────

#[test]
fn mock_entry_value_constructor_exists() {
    assert!(
        MOCK_RS.contains("pub fn value"),
        "MockEntry must have 'pub fn value' constructor"
    );
}

#[test]
fn mock_entry_noop_constructor_exists() {
    assert!(
        MOCK_RS.contains("pub fn noop"),
        "MockEntry must have 'pub fn noop' constructor"
    );
}

#[test]
fn mock_entry_forbidden_constructor_exists() {
    assert!(
        MOCK_RS.contains("pub fn forbidden"),
        "MockEntry must have 'pub fn forbidden' constructor"
    );
}

// ── 7. panic_on_access field ──────────────────────────────────────────────────

#[test]
fn mock_entry_has_panic_on_access_field() {
    assert!(
        MOCK_RS.contains("panic_on_access: bool"),
        "MockEntry must have 'panic_on_access: bool' field"
    );
}

// ── 8. directive.rs documents @mock: ─────────────────────────────────────────

#[test]
fn directive_module_doc_lists_mock() {
    assert!(
        DIRECTIVE_RS.contains("@mock:"),
        "directive.rs module doc must document '@mock: <ContextType>'"
    );
}

// ── 9. TestDirectives has mocks field ────────────────────────────────────────

#[test]
fn test_directives_has_mocks_list_field() {
    assert!(
        DIRECTIVE_RS.contains("pub mocks: List<Text>"),
        "TestDirectives must have 'pub mocks: List<Text>' field"
    );
}

// ── 10. Parser handles @mock: prefix ─────────────────────────────────────────

#[test]
fn parser_handles_mock_prefix() {
    assert!(
        DIRECTIVE_RS.contains("strip_prefix(\"@mock:\")"),
        "directive.rs parser must handle '@mock:' via strip_prefix"
    );
}

#[test]
fn parser_pushes_to_directives_mocks() {
    assert!(
        DIRECTIVE_RS.contains("directives.mocks"),
        "directive.rs must push to 'directives.mocks' when @mock: is found"
    );
}

// ── 11. VCS spec uses @mock: Database and @mock: Logger ──────────────────────

#[test]
fn mock_spec_uses_database_mock() {
    assert!(
        MOCK_SPEC.contains("@mock: Database"),
        "mock_context_injection.vr must use '@mock: Database'"
    );
}

#[test]
fn mock_spec_uses_logger_mock() {
    assert!(
        MOCK_SPEC.contains("@mock: Logger"),
        "mock_context_injection.vr must use '@mock: Logger'"
    );
}

#[test]
fn mock_spec_is_typecheck_pass() {
    assert!(
        MOCK_SPEC.contains("@test: typecheck-pass"),
        "mock_context_injection.vr must be '@test: typecheck-pass'"
    );
}
