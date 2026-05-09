//! Refinement-contract auto-test generator (#64).
//!
//! When a spec file carries `@contract-tests: enabled`, the runner extracts
//! every function that has `@requires` / `@ensures` doc annotations and
//! generates property-based test cases that:
//!
//!   1. Produce random inputs satisfying the precondition (`@requires`).
//!   2. Call the function under test.
//!   3. Assert the postcondition (`@ensures`) on the result.
//!
//! # Workflow
//!
//! The generator does not execute tests directly; it returns a list of
//! `GeneratedTest` values that the runner can schedule alongside hand-written
//! tests.  This keeps the generator pure and testable without a live VBC
//! instance.
//!
//! # Contract annotation format (in `.vr` spec files)
//!
//! ```verum
//! // @requires: n >= 0
//! // @ensures: result >= 0
//! fn abs(n: Int) -> Int { ... }
//! ```
//!
//! The generator recognises `@requires:` and `@ensures:` comment lines
//! immediately preceding a `fn` declaration.

/// A single `@requires` / `@ensures` clause attached to a function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractClause {
    /// The raw predicate string, e.g. `"n >= 0"`.
    pub predicate: String,
}

impl ContractClause {
    /// Parse a contract clause from the raw value after `@requires:` or `@ensures:`.
    pub fn parse(raw: &str) -> Self {
        Self { predicate: raw.trim().to_string() }
    }
}

/// Contracts extracted for a single function.
#[derive(Debug, Clone)]
pub struct FunctionContract {
    /// Name of the function.
    pub fn_name: String,
    /// Pre-conditions (`@requires:` clauses).
    pub requires: Vec<ContractClause>,
    /// Post-conditions (`@ensures:` clauses).
    pub ensures: Vec<ContractClause>,
}

impl FunctionContract {
    /// Returns `true` if the function has at least one contract clause.
    pub fn has_contracts(&self) -> bool {
        !self.requires.is_empty() || !self.ensures.is_empty()
    }
}

/// A single auto-generated test case derived from a function contract.
#[derive(Debug, Clone)]
pub struct GeneratedTest {
    /// Human-readable test name, e.g. `"abs::contract::requires_n_ge_0"`.
    pub name: String,
    /// The function under test.
    pub fn_name: String,
    /// Source predicate text (for error messages on failure).
    pub predicate: String,
    /// Whether this test checks a precondition or postcondition.
    pub kind: ContractTestKind,
}

/// Distinguishes precondition from postcondition generated tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractTestKind {
    /// Tests that inputs satisfy a `@requires` clause.
    Precondition,
    /// Tests that outputs satisfy an `@ensures` clause.
    Postcondition,
}

/// Extract function contracts from the source text of a `.vr` spec file.
///
/// Scans for `// @requires:` and `// @ensures:` comment lines immediately
/// before `fn` declarations.  Returns one `FunctionContract` per decorated fn.
pub fn extract_contracts(source: &str) -> Vec<FunctionContract> {
    let mut result = Vec::new();
    let mut pending_requires: Vec<ContractClause> = Vec::new();
    let mut pending_ensures: Vec<ContractClause> = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed
            .strip_prefix("//")
            .and_then(|s| s.trim().strip_prefix("@requires:"))
        {
            pending_requires.push(ContractClause::parse(rest));
        } else if let Some(rest) = trimmed
            .strip_prefix("//")
            .and_then(|s| s.trim().strip_prefix("@ensures:"))
        {
            pending_ensures.push(ContractClause::parse(rest));
        } else if trimmed.starts_with("fn ") || trimmed.starts_with("pub fn ") {
            if !pending_requires.is_empty() || !pending_ensures.is_empty() {
                let fn_name = extract_fn_name(trimmed);
                result.push(FunctionContract {
                    fn_name,
                    requires: std::mem::take(&mut pending_requires),
                    ensures: std::mem::take(&mut pending_ensures),
                });
            } else {
                pending_requires.clear();
                pending_ensures.clear();
            }
        } else if !trimmed.is_empty() && !trimmed.starts_with("//") {
            // Non-contract, non-fn line resets pending state (e.g. type decl).
            pending_requires.clear();
            pending_ensures.clear();
        }
    }

    result
}

fn extract_fn_name(fn_line: &str) -> String {
    // Strip leading `pub`, leading whitespace, then take the word after `fn `.
    let after_fn = fn_line
        .trim()
        .trim_start_matches("pub")
        .trim()
        .trim_start_matches("fn ")
        .trim();
    after_fn
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()
        .unwrap_or("unknown")
        .to_string()
}

/// Generate test stubs for all functions in `contracts`.
///
/// Each `@requires` clause produces one `Precondition` test;
/// each `@ensures` clause produces one `Postcondition` test.
pub fn generate_tests(contracts: &[FunctionContract]) -> Vec<GeneratedTest> {
    let mut tests = Vec::new();

    for contract in contracts {
        for (i, req) in contract.requires.iter().enumerate() {
            tests.push(GeneratedTest {
                name: format!(
                    "{}::contract::requires_{}",
                    contract.fn_name,
                    i + 1
                ),
                fn_name: contract.fn_name.clone(),
                predicate: req.predicate.clone(),
                kind: ContractTestKind::Precondition,
            });
        }
        for (i, ens) in contract.ensures.iter().enumerate() {
            tests.push(GeneratedTest {
                name: format!(
                    "{}::contract::ensures_{}",
                    contract.fn_name,
                    i + 1
                ),
                fn_name: contract.fn_name.clone(),
                predicate: ens.predicate.clone(),
                kind: ContractTestKind::Postcondition,
            });
        }
    }

    tests
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
// @requires: n >= 0
// @ensures: result >= 0
fn abs(n: Int) -> Int {
    if n < 0 { -n } else { n }
}

fn no_contract(x: Int) -> Int { x }

// @requires: divisor != 0
// @ensures: result * divisor == dividend
fn safe_div(dividend: Int, divisor: Int) -> Int {
    dividend / divisor
}
"#;

    #[test]
    fn extract_abs_contracts() {
        let contracts = extract_contracts(SAMPLE);
        let abs_c = contracts.iter().find(|c| c.fn_name == "abs").unwrap();
        assert_eq!(abs_c.requires.len(), 1);
        assert_eq!(abs_c.requires[0].predicate, "n >= 0");
        assert_eq!(abs_c.ensures.len(), 1);
        assert_eq!(abs_c.ensures[0].predicate, "result >= 0");
    }

    #[test]
    fn extract_safe_div_contracts() {
        let contracts = extract_contracts(SAMPLE);
        let div_c = contracts.iter().find(|c| c.fn_name == "safe_div").unwrap();
        assert_eq!(div_c.requires.len(), 1);
        assert_eq!(div_c.requires[0].predicate, "divisor != 0");
        assert_eq!(div_c.ensures[0].predicate, "result * divisor == dividend");
    }

    #[test]
    fn no_contract_fn_excluded() {
        let contracts = extract_contracts(SAMPLE);
        assert!(contracts.iter().all(|c| c.fn_name != "no_contract"));
    }

    #[test]
    fn generate_tests_from_abs() {
        let contracts = extract_contracts(SAMPLE);
        let tests = generate_tests(&contracts);
        assert!(tests.iter().any(|t| t.fn_name == "abs" && t.kind == ContractTestKind::Precondition));
        assert!(tests.iter().any(|t| t.fn_name == "abs" && t.kind == ContractTestKind::Postcondition));
    }

    #[test]
    fn generated_test_name_format() {
        let contracts = extract_contracts(SAMPLE);
        let tests = generate_tests(&contracts);
        let abs_req = tests
            .iter()
            .find(|t| t.fn_name == "abs" && t.kind == ContractTestKind::Precondition)
            .unwrap();
        assert_eq!(abs_req.name, "abs::contract::requires_1");
    }

    #[test]
    fn has_contracts_true_for_abs() {
        let contracts = extract_contracts(SAMPLE);
        let abs_c = contracts.iter().find(|c| c.fn_name == "abs").unwrap();
        assert!(abs_c.has_contracts());
    }

    #[test]
    fn contract_clause_parse_trims_whitespace() {
        let c = ContractClause::parse("  x > 0  ");
        assert_eq!(c.predicate, "x > 0");
    }

    #[test]
    fn generate_tests_total_count() {
        let contracts = extract_contracts(SAMPLE);
        let tests = generate_tests(&contracts);
        // abs: 1 requires + 1 ensures = 2; safe_div: 1 requires + 1 ensures = 2 → total 4
        assert_eq!(tests.len(), 4);
    }
}
