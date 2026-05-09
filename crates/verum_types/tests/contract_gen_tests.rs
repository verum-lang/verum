#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! Refinement-contract auto-test generator drift guard (#64).
//!
//! `vcs/runner/vtest/src/contract_gen.rs` implements contract extraction and
//! test generation from `@requires` / `@ensures` annotations.
//! `vcs/runner/vtest/src/directive.rs` wires the `@contract-tests:` directive.
//!
//! This drift guard pins:
//!   1. contract_gen.rs defines `FunctionContract` struct.
//!   2. FunctionContract has `fn_name`, `requires`, `ensures` fields.
//!   3. FunctionContract has `has_contracts` method.
//!   4. contract_gen.rs defines `ContractClause` struct with `predicate` field.
//!   5. contract_gen.rs exports `extract_contracts` function.
//!   6. contract_gen.rs exports `generate_tests` function.
//!   7. contract_gen.rs defines `GeneratedTest` struct.
//!   8. contract_gen.rs defines `ContractTestKind` enum (Precondition/Postcondition).
//!   9. directive.rs documents `@contract-tests:`.
//!  10. TestDirectives has `contract_tests: bool` field.
//!  11. Parser handles `@contract-tests:` prefix.
//!  12. VCS spec uses `@contract-tests: enabled`.

const CONTRACT_GEN_RS: &str =
    include_str!("../../../vcs/runner/vtest/src/contract_gen.rs");
const DIRECTIVE_RS: &str =
    include_str!("../../../vcs/runner/vtest/src/directive.rs");
const CONTRACT_SPEC: &str = include_str!(
    "../../../vcs/specs/L2-standard/testing/refinement_contract_autotest.vr"
);

// ── 1. FunctionContract struct ────────────────────────────────────────────────

#[test]
fn function_contract_struct_exists() {
    assert!(
        CONTRACT_GEN_RS.contains("pub struct FunctionContract"),
        "contract_gen.rs must define 'pub struct FunctionContract'"
    );
}

// ── 2. FunctionContract fields ────────────────────────────────────────────────

#[test]
fn function_contract_has_fn_name() {
    assert!(
        CONTRACT_GEN_RS.contains("fn_name:"),
        "FunctionContract must have 'fn_name' field"
    );
}

#[test]
fn function_contract_has_requires() {
    assert!(
        CONTRACT_GEN_RS.contains("requires:"),
        "FunctionContract must have 'requires' field"
    );
}

#[test]
fn function_contract_has_ensures() {
    assert!(
        CONTRACT_GEN_RS.contains("ensures:"),
        "FunctionContract must have 'ensures' field"
    );
}

// ── 3. has_contracts method ───────────────────────────────────────────────────

#[test]
fn function_contract_has_contracts_method() {
    assert!(
        CONTRACT_GEN_RS.contains("pub fn has_contracts"),
        "FunctionContract must have 'pub fn has_contracts' method"
    );
}

// ── 4. ContractClause with predicate field ────────────────────────────────────

#[test]
fn contract_clause_struct_exists() {
    assert!(
        CONTRACT_GEN_RS.contains("pub struct ContractClause"),
        "contract_gen.rs must define 'pub struct ContractClause'"
    );
}

#[test]
fn contract_clause_has_predicate_field() {
    assert!(
        CONTRACT_GEN_RS.contains("pub predicate: String"),
        "ContractClause must have 'pub predicate: String' field"
    );
}

// ── 5. extract_contracts function ────────────────────────────────────────────

#[test]
fn extract_contracts_fn_exists() {
    assert!(
        CONTRACT_GEN_RS.contains("pub fn extract_contracts"),
        "contract_gen.rs must export 'pub fn extract_contracts'"
    );
}

// ── 6. generate_tests function ───────────────────────────────────────────────

#[test]
fn generate_tests_fn_exists() {
    assert!(
        CONTRACT_GEN_RS.contains("pub fn generate_tests"),
        "contract_gen.rs must export 'pub fn generate_tests'"
    );
}

// ── 7. GeneratedTest struct ───────────────────────────────────────────────────

#[test]
fn generated_test_struct_exists() {
    assert!(
        CONTRACT_GEN_RS.contains("pub struct GeneratedTest"),
        "contract_gen.rs must define 'pub struct GeneratedTest'"
    );
}

// ── 8. ContractTestKind enum ──────────────────────────────────────────────────

#[test]
fn contract_test_kind_enum_exists() {
    assert!(
        CONTRACT_GEN_RS.contains("pub enum ContractTestKind"),
        "contract_gen.rs must define 'pub enum ContractTestKind'"
    );
}

#[test]
fn contract_test_kind_precondition_variant_exists() {
    assert!(
        CONTRACT_GEN_RS.contains("Precondition,") || CONTRACT_GEN_RS.contains("Precondition\n"),
        "ContractTestKind must have Precondition variant"
    );
}

#[test]
fn contract_test_kind_postcondition_variant_exists() {
    assert!(
        CONTRACT_GEN_RS.contains("Postcondition,") || CONTRACT_GEN_RS.contains("Postcondition\n"),
        "ContractTestKind must have Postcondition variant"
    );
}

// ── 9. directive.rs documents @contract-tests: ───────────────────────────────

#[test]
fn directive_doc_lists_contract_tests() {
    assert!(
        DIRECTIVE_RS.contains("@contract-tests:"),
        "directive.rs module doc must document '@contract-tests:'"
    );
}

// ── 10. TestDirectives has contract_tests: bool ───────────────────────────────

#[test]
fn test_directives_has_contract_tests_bool() {
    assert!(
        DIRECTIVE_RS.contains("pub contract_tests: bool"),
        "TestDirectives must have 'pub contract_tests: bool' field"
    );
}

// ── 11. Parser handles @contract-tests: prefix ───────────────────────────────

#[test]
fn parser_handles_contract_tests_prefix() {
    assert!(
        DIRECTIVE_RS.contains("strip_prefix(\"@contract-tests:\")"),
        "directive.rs parser must handle '@contract-tests:' via strip_prefix"
    );
}

#[test]
fn parser_assigns_contract_tests_bool() {
    assert!(
        DIRECTIVE_RS.contains("directives.contract_tests"),
        "directive.rs must assign 'directives.contract_tests' when @contract-tests: is found"
    );
}

// ── 12. VCS spec uses @contract-tests: enabled ───────────────────────────────

#[test]
fn contract_spec_uses_contract_tests_enabled() {
    assert!(
        CONTRACT_SPEC.contains("@contract-tests: enabled"),
        "refinement_contract_autotest.vr must use '@contract-tests: enabled'"
    );
}

#[test]
fn contract_spec_is_typecheck_pass() {
    assert!(
        CONTRACT_SPEC.contains("@test: typecheck-pass"),
        "refinement_contract_autotest.vr must be '@test: typecheck-pass'"
    );
}
