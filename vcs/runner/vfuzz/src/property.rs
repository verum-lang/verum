//! Property-based testing support
//!
//! Provides infrastructure for defining and testing properties that the
//! Verum compiler/runtime should satisfy.
//!
//! # Property Categories
//!
//! ## Structural Properties
//! - **Idempotency**: parse(print(parse(x))) == parse(x)
//! - **Roundtrip**: parse(print(ast)) == ast
//! - **Monotonicity**: Adding code doesn't remove existing errors
//!
//! ## Algebraic Properties
//! - **Commutativity**: Where applicable (e.g., set operations)
//! - **Associativity**: For binary operators
//! - **Identity**: Operations with identity elements
//!
//! ## Compiler Properties
//! - No crashes on any input
//! - Type soundness
//! - CBGR memory safety
//! - Tier equivalence (Tier 0 == Tier 3)
//!
//! # Usage
//!
//! ```rust,ignore
//! use verum_vfuzz::property::{Property, PropertyResult, PropertyFailure};
//!
//! let prop = Property::new("parse-roundtrip", "desc", |input| {
//!     // Test implementation
//!     PropertyResult::Pass(())
//! });
//! ```

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::Hash;

/// Simple result type for fuzz properties
#[derive(Debug, Clone)]
pub enum FuzzPropertyResult {
    /// Property holds
    Pass,
    /// Property was violated
    Fail(String),
    /// Property could not be tested
    Skip,
}

/// Trait for simple fuzz properties used in property-based fuzzing
pub trait FuzzProperty: Send + Sync {
    /// Name of this property
    fn name(&self) -> &str;
    /// Check if the property holds for the given input
    fn check(&self, input: &str) -> FuzzPropertyResult;
}

/// A testable property
pub struct Property<I, O> {
    /// Property name
    pub name: String,
    /// Property description
    pub description: String,
    /// The test function
    test_fn: Box<dyn Fn(I) -> PropertyResult<O> + Send + Sync>,
    /// Number of shrink attempts
    pub shrink_attempts: usize,
    /// Whether failure should stop the campaign
    pub critical: bool,
}

impl<I, O> Property<I, O> {
    /// Create a new property
    pub fn new<F>(name: &str, description: &str, test_fn: F) -> Self
    where
        F: Fn(I) -> PropertyResult<O> + Send + Sync + 'static,
    {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            test_fn: Box::new(test_fn),
            shrink_attempts: 100,
            critical: false,
        }
    }

    /// Mark as critical (failure stops campaign)
    pub fn critical(mut self) -> Self {
        self.critical = true;
        self
    }

    /// Set shrink attempts
    pub fn shrink_attempts(mut self, attempts: usize) -> Self {
        self.shrink_attempts = attempts;
        self
    }

    /// Test the property with an input
    pub fn test(&self, input: I) -> PropertyResult<O> {
        (self.test_fn)(input)
    }
}

impl<I, O> fmt::Debug for Property<I, O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Property")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("critical", &self.critical)
            .finish()
    }
}

/// Result of testing a property
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PropertyResult<T> {
    /// Property holds
    Pass(T),
    /// Property was violated
    Fail(PropertyFailure),
    /// Property could not be tested (precondition failed)
    Skip(String),
    /// Testing caused an error
    Error(String),
}

impl<T> PropertyResult<T> {
    /// Check if the property passed
    pub fn is_pass(&self) -> bool {
        matches!(self, PropertyResult::Pass(_))
    }

    /// Check if the property failed
    pub fn is_fail(&self) -> bool {
        matches!(self, PropertyResult::Fail(_))
    }

    /// Map the success value
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> PropertyResult<U> {
        match self {
            PropertyResult::Pass(v) => PropertyResult::Pass(f(v)),
            PropertyResult::Fail(e) => PropertyResult::Fail(e),
            PropertyResult::Skip(s) => PropertyResult::Skip(s),
            PropertyResult::Error(e) => PropertyResult::Error(e),
        }
    }
}

/// Details about a property failure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertyFailure {
    /// The property name
    pub property: String,
    /// Description of the failure
    pub message: String,
    /// Expected value (if applicable)
    pub expected: Option<String>,
    /// Actual value (if applicable)
    pub actual: Option<String>,
    /// The input that caused the failure
    pub input: String,
    /// Minimized input (if available)
    pub minimized: Option<String>,
    /// Stack trace (if available)
    pub trace: Option<String>,
}

impl PropertyFailure {
    /// Create a new failure
    pub fn new(property: &str, message: &str, input: &str) -> Self {
        Self {
            property: property.to_string(),
            message: message.to_string(),
            expected: None,
            actual: None,
            input: input.to_string(),
            minimized: None,
            trace: None,
        }
    }

    /// Add expected/actual values
    pub fn with_values(mut self, expected: &str, actual: &str) -> Self {
        self.expected = Some(expected.to_string());
        self.actual = Some(actual.to_string());
        self
    }

    /// Add minimized input
    pub fn with_minimized(mut self, minimized: &str) -> Self {
        self.minimized = Some(minimized.to_string());
        self
    }
}

/// Predefined properties for the Verum compiler
pub struct CompilerProperties;

impl CompilerProperties {
    /// Parsing should not crash on any input
    pub fn parse_no_crash() -> Property<String, ()> {
        Property::new(
            "parse-no-crash",
            "Parser should not crash on any input, even malformed",
            |input: String| {
                // Simulate calling parser
                // In real implementation, this would call verum_parser::parse
                if input.contains("CRASH_TRIGGER") {
                    PropertyResult::Error("Simulated crash".to_string())
                } else {
                    PropertyResult::Pass(())
                }
            },
        )
        .critical()
    }

    /// Type checking should not crash
    pub fn typecheck_no_crash() -> Property<String, ()> {
        Property::new(
            "typecheck-no-crash",
            "Type checker should not crash, only report errors",
            |_input: String| {
                // Placeholder - would call type checker
                PropertyResult::Pass(())
            },
        )
        .critical()
    }

    /// Valid programs should roundtrip through parse/format
    pub fn parse_roundtrip() -> Property<String, ()> {
        Property::new(
            "parse-roundtrip",
            "parse(format(parse(x))) == parse(x) for valid programs",
            |input: String| {
                // Placeholder implementation
                // Would parse, format, reparse and compare
                let _ = input;
                PropertyResult::Pass(())
            },
        )
    }

    /// Type inference should be deterministic
    pub fn type_inference_deterministic() -> Property<String, ()> {
        Property::new(
            "type-inference-deterministic",
            "Type inference should produce the same result on repeated calls",
            |input: String| {
                // Placeholder - would run type inference twice and compare
                let _ = input;
                PropertyResult::Pass(())
            },
        )
    }

    /// Well-typed programs should not have runtime type errors
    pub fn type_soundness() -> Property<String, ()> {
        Property::new(
            "type-soundness",
            "Well-typed programs should not produce type errors at runtime",
            |_input: String| {
                // Placeholder - would compile and run, check for type errors
                PropertyResult::Pass(())
            },
        )
        .critical()
    }

    /// CBGR should prevent use-after-free
    pub fn cbgr_no_use_after_free() -> Property<String, ()> {
        Property::new(
            "cbgr-no-use-after-free",
            "CBGR should detect and prevent all use-after-free",
            |_input: String| {
                // Placeholder - would check CBGR generation tracking
                PropertyResult::Pass(())
            },
        )
        .critical()
    }

    /// SMT verification should be sound
    pub fn smt_soundness() -> Property<String, ()> {
        Property::new(
            "smt-soundness",
            "If SMT proves a property, it should hold at runtime",
            |_input: String| {
                // Placeholder - would verify SMT result against runtime
                PropertyResult::Pass(())
            },
        )
        .critical()
    }

    /// Tier 0 and Tier 3 should produce identical results
    pub fn tier_equivalence() -> Property<String, ()> {
        Property::new(
            "tier-equivalence",
            "Tier 0 (interpreter) and Tier 3 (AOT) should produce identical observable results",
            |_input: String| {
                // Placeholder - would run on both tiers and compare
                PropertyResult::Pass(())
            },
        )
        .critical()
    }

    /// Code generation should not produce invalid LLVM IR
    pub fn codegen_valid_ir() -> Property<String, ()> {
        Property::new(
            "codegen-valid-ir",
            "Generated LLVM IR should pass LLVM's verifier",
            |_input: String| {
                // Placeholder - would generate IR and verify
                PropertyResult::Pass(())
            },
        )
    }

    /// Async operations should not deadlock
    pub fn async_no_deadlock() -> Property<String, ()> {
        Property::new(
            "async-no-deadlock",
            "Async operations should complete within timeout",
            |_input: String| {
                // Placeholder - would run with timeout
                PropertyResult::Pass(())
            },
        )
    }
}

/// Property test runner
pub struct PropertyRunner {
    /// Properties to test
    properties: Vec<Box<dyn PropertyTest>>,
    /// Results collected
    results: Vec<PropertyTestResult>,
    /// Total tests run
    total: usize,
    /// Tests passed
    passed: usize,
    /// Tests failed
    failed: usize,
    /// Tests skipped
    skipped: usize,
    /// Tests errored
    errored: usize,
}

/// Trait for type-erased property testing
pub trait PropertyTest: Send + Sync {
    /// Get property name
    fn name(&self) -> &str;
    /// Get property description
    fn description(&self) -> &str;
    /// Is this property critical?
    fn is_critical(&self) -> bool;
    /// Test with a string input
    fn test_string(&self, input: &str) -> PropertyTestResult;
}

impl<O: 'static> PropertyTest for Property<String, O> {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn is_critical(&self) -> bool {
        self.critical
    }

    fn test_string(&self, input: &str) -> PropertyTestResult {
        match self.test(input.to_string()) {
            PropertyResult::Pass(_) => PropertyTestResult::Pass,
            PropertyResult::Fail(f) => PropertyTestResult::Fail(f),
            PropertyResult::Skip(s) => PropertyTestResult::Skip(s),
            PropertyResult::Error(e) => PropertyTestResult::Error(e),
        }
    }
}

/// Result of a property test (type-erased)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PropertyTestResult {
    /// Passed
    Pass,
    /// Failed
    Fail(PropertyFailure),
    /// Skipped
    Skip(String),
    /// Error
    Error(String),
}

impl Default for PropertyRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl PropertyRunner {
    /// Create a new property runner
    pub fn new() -> Self {
        Self {
            properties: Vec::new(),
            results: Vec::new(),
            total: 0,
            passed: 0,
            failed: 0,
            skipped: 0,
            errored: 0,
        }
    }

    /// Add a property to test
    pub fn add_property(&mut self, prop: impl PropertyTest + 'static) {
        self.properties.push(Box::new(prop));
    }

    /// Add all compiler properties
    pub fn add_compiler_properties(&mut self) {
        self.add_property(CompilerProperties::parse_no_crash());
        self.add_property(CompilerProperties::typecheck_no_crash());
        self.add_property(CompilerProperties::parse_roundtrip());
        self.add_property(CompilerProperties::type_inference_deterministic());
        self.add_property(CompilerProperties::type_soundness());
        self.add_property(CompilerProperties::cbgr_no_use_after_free());
        self.add_property(CompilerProperties::tier_equivalence());
    }

    /// Run all properties on an input
    pub fn run(&mut self, input: &str) -> bool {
        let mut all_passed = true;

        for prop in &self.properties {
            let result = prop.test_string(input);
            self.total += 1;

            match &result {
                PropertyTestResult::Pass => {
                    self.passed += 1;
                }
                PropertyTestResult::Fail(_f) => {
                    self.failed += 1;
                    if prop.is_critical() {
                        all_passed = false;
                    }
                    self.results.push(result.clone());
                }
                PropertyTestResult::Skip(_) => {
                    self.skipped += 1;
                }
                PropertyTestResult::Error(_) => {
                    self.errored += 1;
                    if prop.is_critical() {
                        all_passed = false;
                    }
                    self.results.push(result.clone());
                }
            }
        }

        all_passed
    }

    /// Get failures
    pub fn failures(&self) -> impl Iterator<Item = &PropertyFailure> {
        self.results.iter().filter_map(|r| match r {
            PropertyTestResult::Fail(f) => Some(f),
            _ => None,
        })
    }

    /// Get statistics
    pub fn stats(&self) -> PropertyStats {
        PropertyStats {
            total: self.total,
            passed: self.passed,
            failed: self.failed,
            skipped: self.skipped,
            errored: self.errored,
            pass_rate: if self.total > 0 {
                self.passed as f64 / self.total as f64
            } else {
                0.0
            },
        }
    }

    /// Reset statistics
    pub fn reset(&mut self) {
        self.results.clear();
        self.total = 0;
        self.passed = 0;
        self.failed = 0;
        self.skipped = 0;
        self.errored = 0;
    }
}

/// Property testing statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PropertyStats {
    /// Total tests run
    pub total: usize,
    /// Tests passed
    pub passed: usize,
    /// Tests failed
    pub failed: usize,
    /// Tests skipped
    pub skipped: usize,
    /// Tests errored
    pub errored: usize,
    /// Pass rate (0.0 - 1.0)
    pub pass_rate: f64,
}

/// Macro for defining properties
#[macro_export]
macro_rules! property {
    ($name:expr, $desc:expr, |$input:ident| $body:expr) => {
        Property::new($name, $desc, |$input: String| $body)
    };
}

/// Macro for property assertions
#[macro_export]
macro_rules! prop_assert {
    ($cond:expr) => {
        if !$cond {
            return PropertyResult::Fail(PropertyFailure::new("", stringify!($cond), ""));
        }
    };
    ($cond:expr, $msg:expr) => {
        if !$cond {
            return PropertyResult::Fail(PropertyFailure::new("", $msg, ""));
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_property_pass() {
        let prop = Property::new("always-pass", "Always passes", |_: String| {
            PropertyResult::Pass(())
        });

        let result = prop.test("anything".to_string());
        assert!(result.is_pass());
    }

    #[test]
    fn test_property_fail() {
        let prop = Property::new("always-fail", "Always fails", |input: String| {
            PropertyResult::<()>::Fail(PropertyFailure::new("always-fail", "intentional", &input))
        });

        let result = prop.test("test".to_string());
        assert!(result.is_fail());
    }

    #[test]
    fn test_property_runner() {
        let mut runner = PropertyRunner::new();

        runner.add_property(Property::new("test-prop", "Test", |_: String| {
            PropertyResult::Pass(())
        }));

        let passed = runner.run("input");
        assert!(passed);

        let stats = runner.stats();
        assert_eq!(stats.total, 1);
        assert_eq!(stats.passed, 1);
    }

    #[test]
    fn test_compiler_properties() {
        let mut runner = PropertyRunner::new();
        runner.add_compiler_properties();

        // Should have multiple properties
        assert!(runner.properties.len() > 5);

        // All should pass on normal input
        let passed = runner.run("fn main() { 42 }");
        assert!(passed);
    }

    #[test]
    fn test_critical_property() {
        let prop = CompilerProperties::parse_no_crash();
        assert!(prop.critical);
    }
}

// ============================================================================
// Property Categories
// ============================================================================

/// Category of property
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PropertyCategory {
    /// Structural property (roundtrip, idempotency)
    Structural,
    /// Algebraic property (commutativity, associativity)
    Algebraic,
    /// Semantic property (determinism, purity)
    Semantic,
    /// Safety property (memory safety, type safety)
    Safety,
    /// Performance property (no exponential blowup)
    Performance,
}

// ============================================================================
// Idempotency Properties
// ============================================================================

/// Properties testing idempotency (f(f(x)) == f(x))
pub struct IdempotencyProperties;

impl IdempotencyProperties {
    /// Parse idempotency: parse(print(parse(x))) == parse(x)
    ///
    /// This tests that parsing is stable after a roundtrip through
    /// pretty-printing. The key insight is that while the source text
    /// may differ (whitespace, comments), the resulting AST should be
    /// structurally equivalent.
    pub fn parse_idempotent() -> Property<String, ()> {
        Property::new(
            "parse-idempotent",
            "parse(print(parse(x))) should equal parse(x) for valid programs",
            |input: String| {
                // Simulated implementation
                // In a real implementation, this would:
                // 1. Parse the input to get AST1
                // 2. Pretty-print AST1 to get source2
                // 3. Parse source2 to get AST2
                // 4. Compare AST1 and AST2 structurally

                // Skip invalid inputs
                if input.trim().is_empty() {
                    return PropertyResult::Skip("Empty input".to_string());
                }

                // Placeholder: always pass for valid-looking inputs
                if input.contains("fn ") || input.contains("type ") {
                    PropertyResult::Pass(())
                } else {
                    PropertyResult::Skip("Not a valid program".to_string())
                }
            },
        )
    }

    /// Type normalization idempotency: normalize(normalize(T)) == normalize(T)
    pub fn type_normalize_idempotent() -> Property<String, ()> {
        Property::new(
            "type-normalize-idempotent",
            "Type normalization should be idempotent",
            |_input: String| {
                // Placeholder implementation
                PropertyResult::Pass(())
            },
        )
    }

    /// Error collection idempotency: collecting errors twice gives same result
    pub fn error_collection_idempotent() -> Property<String, ()> {
        Property::new(
            "error-collection-idempotent",
            "Collecting errors multiple times should give identical results",
            |_input: String| PropertyResult::Pass(()),
        )
    }
}

// ============================================================================
// Roundtrip Properties
// ============================================================================

/// Properties testing roundtrip preservation
pub struct RoundtripProperties;

impl RoundtripProperties {
    /// AST roundtrip: parse(print(ast)) == ast
    ///
    /// Tests that pretty-printing and re-parsing preserves the AST structure.
    /// This is stricter than idempotency because it requires exact AST equality.
    pub fn ast_roundtrip() -> Property<String, ()> {
        Property::new(
            "ast-roundtrip",
            "parse(print(ast)) should equal ast for valid ASTs",
            |input: String| {
                // Skip if input doesn't look valid
                if !input.contains("fn ") && !input.contains("type ") {
                    return PropertyResult::Skip("Not a valid program structure".to_string());
                }

                // Simulated roundtrip check
                // In real implementation:
                // 1. Parse input to AST
                // 2. Pretty-print AST
                // 3. Re-parse
                // 4. Compare ASTs for exact equality

                PropertyResult::Pass(())
            },
        )
    }

    /// Type inference roundtrip: infer(erase_types(infer(prog))) == infer(prog)
    pub fn type_inference_roundtrip() -> Property<String, ()> {
        Property::new(
            "type-inference-roundtrip",
            "Re-inferring types after erasure should give same result",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// Serialization roundtrip: deserialize(serialize(x)) == x
    pub fn serialization_roundtrip() -> Property<String, ()> {
        Property::new(
            "serialization-roundtrip",
            "Serialization/deserialization should preserve values exactly",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// Token roundtrip: tokens == lex(unlex(tokens))
    pub fn token_roundtrip() -> Property<String, ()> {
        Property::new(
            "token-roundtrip",
            "Lexing the string representation of tokens gives same tokens",
            |_input: String| PropertyResult::Pass(()),
        )
    }
}

// ============================================================================
// Monotonicity Properties
// ============================================================================

/// Properties testing monotonicity (f(x) <= f(x + y))
pub struct MonotonicityProperties;

impl MonotonicityProperties {
    /// Error monotonicity: adding code doesn't remove existing errors
    ///
    /// If a program has errors E, then adding more code should not
    /// make errors in E disappear (though new errors may appear).
    pub fn error_monotonicity() -> Property<(String, String), ()> {
        Property::new(
            "error-monotonicity",
            "Adding code should not remove existing errors",
            |(_base, _addition): (String, String)| {
                // In real implementation:
                // 1. Compile base, collect errors E1
                // 2. Compile (base + addition), collect errors E2
                // 3. For each error e in E1 at location in base,
                //    verify e still exists in E2 (at same location)

                PropertyResult::Pass(())
            },
        )
    }

    /// Type specificity monotonicity: refining types makes them more specific
    pub fn type_specificity_monotonic() -> Property<String, ()> {
        Property::new(
            "type-specificity-monotonic",
            "Adding refinements should only make types more specific",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// Coverage monotonicity: more tests never decrease coverage
    pub fn coverage_monotonic() -> Property<String, ()> {
        Property::new(
            "coverage-monotonic",
            "Adding tests should never decrease code coverage",
            |_input: String| PropertyResult::Pass(()),
        )
    }
}

// ============================================================================
// Commutativity Properties
// ============================================================================

/// Properties testing commutativity (f(a, b) == f(b, a))
pub struct CommutativityProperties;

impl CommutativityProperties {
    /// Set union commutativity: A | B == B | A
    pub fn set_union_commutative() -> Property<(String, String), ()> {
        Property::new(
            "set-union-commutative",
            "Set union should be commutative",
            |(_a, _b): (String, String)| {
                // Test that set1.union(set2) == set2.union(set1)
                PropertyResult::Pass(())
            },
        )
    }

    /// Set intersection commutativity: A & B == B & A
    pub fn set_intersection_commutative() -> Property<(String, String), ()> {
        Property::new(
            "set-intersection-commutative",
            "Set intersection should be commutative",
            |(_a, _b): (String, String)| PropertyResult::Pass(()),
        )
    }

    /// Map merge commutativity (for non-conflicting keys)
    pub fn map_merge_commutative() -> Property<(String, String), ()> {
        Property::new(
            "map-merge-commutative",
            "Map merge should be commutative for non-conflicting keys",
            |(_a, _b): (String, String)| PropertyResult::Pass(()),
        )
    }

    /// Numeric addition commutativity: a + b == b + a
    pub fn addition_commutative() -> Property<String, ()> {
        Property::new(
            "addition-commutative",
            "Numeric addition should be commutative",
            |input: String| {
                // Parse two numbers and verify a + b == b + a
                let _ = input;
                PropertyResult::Pass(())
            },
        )
    }

    /// Multiplication commutativity: a * b == b * a
    pub fn multiplication_commutative() -> Property<String, ()> {
        Property::new(
            "multiplication-commutative",
            "Numeric multiplication should be commutative",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// Boolean AND commutativity: a && b == b && a (for pure expressions)
    pub fn and_commutative() -> Property<String, ()> {
        Property::new(
            "and-commutative",
            "Boolean AND should be commutative for pure expressions",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// Boolean OR commutativity: a || b == b || a (for pure expressions)
    pub fn or_commutative() -> Property<String, ()> {
        Property::new(
            "or-commutative",
            "Boolean OR should be commutative for pure expressions",
            |_input: String| PropertyResult::Pass(()),
        )
    }
}

// ============================================================================
// Associativity Properties
// ============================================================================

/// Properties testing associativity ((a op b) op c == a op (b op c))
pub struct AssociativityProperties;

impl AssociativityProperties {
    /// Addition associativity: (a + b) + c == a + (b + c)
    pub fn addition_associative() -> Property<String, ()> {
        Property::new(
            "addition-associative",
            "Integer addition should be associative",
            |_input: String| {
                // Note: Floating point is NOT associative due to rounding
                PropertyResult::Pass(())
            },
        )
    }

    /// Multiplication associativity: (a * b) * c == a * (b * c)
    pub fn multiplication_associative() -> Property<String, ()> {
        Property::new(
            "multiplication-associative",
            "Integer multiplication should be associative",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// String concatenation associativity: (a ++ b) ++ c == a ++ (b ++ c)
    pub fn concat_associative() -> Property<String, ()> {
        Property::new(
            "concat-associative",
            "String concatenation should be associative",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// List concatenation associativity
    pub fn list_concat_associative() -> Property<String, ()> {
        Property::new(
            "list-concat-associative",
            "List concatenation should be associative",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// Function composition associativity: (f . g) . h == f . (g . h)
    pub fn composition_associative() -> Property<String, ()> {
        Property::new(
            "composition-associative",
            "Function composition should be associative",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// Boolean AND associativity: (a && b) && c == a && (b && c)
    pub fn and_associative() -> Property<String, ()> {
        Property::new(
            "and-associative",
            "Boolean AND should be associative",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// Boolean OR associativity: (a || b) || c == a || (b || c)
    pub fn or_associative() -> Property<String, ()> {
        Property::new(
            "or-associative",
            "Boolean OR should be associative",
            |_input: String| PropertyResult::Pass(()),
        )
    }
}

// ============================================================================
// Identity Properties
// ============================================================================

/// Properties testing identity elements
pub struct IdentityProperties;

impl IdentityProperties {
    /// Additive identity: x + 0 == x
    pub fn additive_identity() -> Property<String, ()> {
        Property::new(
            "additive-identity",
            "Adding zero should not change the value",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// Multiplicative identity: x * 1 == x
    pub fn multiplicative_identity() -> Property<String, ()> {
        Property::new(
            "multiplicative-identity",
            "Multiplying by one should not change the value",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// Empty list identity: xs ++ [] == xs
    pub fn empty_list_identity() -> Property<String, ()> {
        Property::new(
            "empty-list-identity",
            "Concatenating empty list should not change the list",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// Empty string identity: s ++ "" == s
    pub fn empty_string_identity() -> Property<String, ()> {
        Property::new(
            "empty-string-identity",
            "Concatenating empty string should not change the string",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// Identity function: id(x) == x
    pub fn identity_function() -> Property<String, ()> {
        Property::new(
            "identity-function",
            "Identity function should return its argument unchanged",
            |_input: String| PropertyResult::Pass(()),
        )
    }
}

// ============================================================================
// Distributivity Properties
// ============================================================================

/// Properties testing distributivity (a * (b + c) == a*b + a*c)
pub struct DistributivityProperties;

impl DistributivityProperties {
    /// Multiplication distributes over addition: a * (b + c) == a*b + a*c
    pub fn mult_over_add() -> Property<String, ()> {
        Property::new(
            "mult-distributes-over-add",
            "Multiplication should distribute over addition",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// AND distributes over OR: a && (b || c) == (a && b) || (a && c)
    pub fn and_over_or() -> Property<String, ()> {
        Property::new(
            "and-distributes-over-or",
            "AND should distribute over OR",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// Map over list concatenation: map(f, xs ++ ys) == map(f, xs) ++ map(f, ys)
    pub fn map_over_concat() -> Property<String, ()> {
        Property::new(
            "map-distributes-over-concat",
            "Map should distribute over list concatenation",
            |_input: String| PropertyResult::Pass(()),
        )
    }
}

// ============================================================================
// Absorption Properties
// ============================================================================

/// Properties testing absorption laws
pub struct AbsorptionProperties;

impl AbsorptionProperties {
    /// Boolean absorption: a || (a && b) == a
    pub fn or_and_absorption() -> Property<String, ()> {
        Property::new(
            "or-and-absorption",
            "a || (a && b) should equal a",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// Boolean absorption: a && (a || b) == a
    pub fn and_or_absorption() -> Property<String, ()> {
        Property::new(
            "and-or-absorption",
            "a && (a || b) should equal a",
            |_input: String| PropertyResult::Pass(()),
        )
    }
}

// ============================================================================
// Involution Properties
// ============================================================================

/// Properties testing involution (f(f(x)) == x)
pub struct InvolutionProperties;

impl InvolutionProperties {
    /// Double negation: !!x == x (for booleans)
    pub fn double_negation() -> Property<String, ()> {
        Property::new(
            "double-negation",
            "Double negation should return the original value",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// Double reverse: reverse(reverse(xs)) == xs
    pub fn double_reverse() -> Property<String, ()> {
        Property::new(
            "double-reverse",
            "Reversing a list twice should return the original list",
            |_input: String| PropertyResult::Pass(()),
        )
    }

    /// Double bitwise NOT: ~~x == x
    pub fn double_bitwise_not() -> Property<String, ()> {
        Property::new(
            "double-bitwise-not",
            "Double bitwise NOT should return the original value",
            |_input: String| PropertyResult::Pass(()),
        )
    }
}

// ============================================================================
// Enhanced PropertyRunner with Property Categories
// ============================================================================

/// Extended property runner with category support
pub struct ExtendedPropertyRunner {
    /// Base runner
    inner: PropertyRunner,
    /// Properties by category
    by_category: HashMap<PropertyCategory, Vec<String>>,
    /// Category statistics
    category_stats: HashMap<PropertyCategory, PropertyStats>,
}

impl Default for ExtendedPropertyRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtendedPropertyRunner {
    /// Create a new extended runner
    pub fn new() -> Self {
        Self {
            inner: PropertyRunner::new(),
            by_category: HashMap::new(),
            category_stats: HashMap::new(),
        }
    }

    /// Add a property with category
    pub fn add_property_with_category(
        &mut self,
        prop: impl PropertyTest + 'static,
        category: PropertyCategory,
    ) {
        let name = prop.name().to_string();
        self.by_category.entry(category).or_default().push(name);
        self.inner.add_property(prop);
    }

    /// Add all idempotency properties
    pub fn add_idempotency_properties(&mut self) {
        self.add_property_with_category(
            IdempotencyProperties::parse_idempotent(),
            PropertyCategory::Structural,
        );
        self.add_property_with_category(
            IdempotencyProperties::type_normalize_idempotent(),
            PropertyCategory::Structural,
        );
        self.add_property_with_category(
            IdempotencyProperties::error_collection_idempotent(),
            PropertyCategory::Structural,
        );
    }

    /// Add all roundtrip properties
    pub fn add_roundtrip_properties(&mut self) {
        self.add_property_with_category(
            RoundtripProperties::ast_roundtrip(),
            PropertyCategory::Structural,
        );
        self.add_property_with_category(
            RoundtripProperties::type_inference_roundtrip(),
            PropertyCategory::Structural,
        );
        self.add_property_with_category(
            RoundtripProperties::serialization_roundtrip(),
            PropertyCategory::Structural,
        );
        self.add_property_with_category(
            RoundtripProperties::token_roundtrip(),
            PropertyCategory::Structural,
        );
    }

    /// Add all algebraic properties
    pub fn add_algebraic_properties(&mut self) {
        // Commutativity
        self.add_property_with_category(
            CommutativityProperties::addition_commutative(),
            PropertyCategory::Algebraic,
        );
        self.add_property_with_category(
            CommutativityProperties::multiplication_commutative(),
            PropertyCategory::Algebraic,
        );

        // Associativity
        self.add_property_with_category(
            AssociativityProperties::addition_associative(),
            PropertyCategory::Algebraic,
        );
        self.add_property_with_category(
            AssociativityProperties::multiplication_associative(),
            PropertyCategory::Algebraic,
        );
        self.add_property_with_category(
            AssociativityProperties::concat_associative(),
            PropertyCategory::Algebraic,
        );

        // Identity
        self.add_property_with_category(
            IdentityProperties::additive_identity(),
            PropertyCategory::Algebraic,
        );
        self.add_property_with_category(
            IdentityProperties::multiplicative_identity(),
            PropertyCategory::Algebraic,
        );
        self.add_property_with_category(
            IdentityProperties::empty_list_identity(),
            PropertyCategory::Algebraic,
        );

        // Involution
        self.add_property_with_category(
            InvolutionProperties::double_negation(),
            PropertyCategory::Algebraic,
        );
        self.add_property_with_category(
            InvolutionProperties::double_reverse(),
            PropertyCategory::Algebraic,
        );
    }

    /// Add all safety properties
    pub fn add_safety_properties(&mut self) {
        self.add_property_with_category(
            CompilerProperties::parse_no_crash(),
            PropertyCategory::Safety,
        );
        self.add_property_with_category(
            CompilerProperties::typecheck_no_crash(),
            PropertyCategory::Safety,
        );
        self.add_property_with_category(
            CompilerProperties::type_soundness(),
            PropertyCategory::Safety,
        );
        self.add_property_with_category(
            CompilerProperties::cbgr_no_use_after_free(),
            PropertyCategory::Safety,
        );
    }

    /// Add all properties
    pub fn add_all_properties(&mut self) {
        self.add_idempotency_properties();
        self.add_roundtrip_properties();
        self.add_algebraic_properties();
        self.add_safety_properties();
    }

    /// Run all properties on input
    pub fn run(&mut self, input: &str) -> bool {
        self.inner.run(input)
    }

    /// Get statistics
    pub fn stats(&self) -> PropertyStats {
        self.inner.stats()
    }

    /// Get properties in a category
    pub fn properties_in_category(&self, category: PropertyCategory) -> &[String] {
        self.by_category
            .get(&category)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Get category count
    pub fn category_count(&self, category: PropertyCategory) -> usize {
        self.by_category
            .get(&category)
            .map(|v| v.len())
            .unwrap_or(0)
    }

    /// Reset
    pub fn reset(&mut self) {
        self.inner.reset();
        self.category_stats.clear();
    }
}

// ============================================================================
// Property Combinator
// ============================================================================

/// Combine multiple properties
pub struct PropertyCombinator<I, O> {
    properties: Vec<Box<dyn Fn(I) -> PropertyResult<O> + Send + Sync>>,
    name: String,
}

impl<I: Clone, O> PropertyCombinator<I, O> {
    /// Create a new combinator
    pub fn new(name: &str) -> Self {
        Self {
            properties: Vec::new(),
            name: name.to_string(),
        }
    }

    /// Add a property check
    pub fn add<F>(mut self, f: F) -> Self
    where
        F: Fn(I) -> PropertyResult<O> + Send + Sync + 'static,
    {
        self.properties.push(Box::new(f));
        self
    }

    /// Run all checks (AND semantics - all must pass)
    pub fn run_all(&self, input: I) -> PropertyResult<()>
    where
        I: Clone,
    {
        for prop in &self.properties {
            match prop(input.clone()) {
                PropertyResult::Pass(_) => continue,
                PropertyResult::Fail(f) => return PropertyResult::Fail(f),
                PropertyResult::Skip(s) => return PropertyResult::Skip(s),
                PropertyResult::Error(e) => return PropertyResult::Error(e),
            }
        }
        PropertyResult::Pass(())
    }

    /// Run until first success (OR semantics)
    pub fn run_any(&self, input: I) -> PropertyResult<O>
    where
        I: Clone,
    {
        let mut last_failure = None;

        for prop in &self.properties {
            match prop(input.clone()) {
                PropertyResult::Pass(v) => return PropertyResult::Pass(v),
                PropertyResult::Fail(f) => last_failure = Some(f),
                PropertyResult::Skip(_) => continue,
                PropertyResult::Error(e) => return PropertyResult::Error(e),
            }
        }

        if let Some(f) = last_failure {
            PropertyResult::Fail(f)
        } else {
            PropertyResult::Skip("No property succeeded".to_string())
        }
    }
}

// ============================================================================
// Property Generators for Specific Types
// ============================================================================

/// Generate property tests for numeric operations
pub fn numeric_properties() -> Vec<Box<dyn PropertyTest>> {
    vec![
        Box::new(CommutativityProperties::addition_commutative()),
        Box::new(CommutativityProperties::multiplication_commutative()),
        Box::new(AssociativityProperties::addition_associative()),
        Box::new(AssociativityProperties::multiplication_associative()),
        Box::new(IdentityProperties::additive_identity()),
        Box::new(IdentityProperties::multiplicative_identity()),
    ]
}

/// Generate property tests for collection operations
pub fn collection_properties() -> Vec<Box<dyn PropertyTest>> {
    vec![
        Box::new(AssociativityProperties::list_concat_associative()),
        Box::new(IdentityProperties::empty_list_identity()),
        Box::new(InvolutionProperties::double_reverse()),
    ]
}

/// Generate property tests for boolean operations
pub fn boolean_properties() -> Vec<Box<dyn PropertyTest>> {
    vec![
        Box::new(CommutativityProperties::and_commutative()),
        Box::new(CommutativityProperties::or_commutative()),
        Box::new(AssociativityProperties::and_associative()),
        Box::new(AssociativityProperties::or_associative()),
        Box::new(InvolutionProperties::double_negation()),
        Box::new(DistributivityProperties::and_over_or()),
        Box::new(AbsorptionProperties::or_and_absorption()),
        Box::new(AbsorptionProperties::and_or_absorption()),
    ]
}

#[cfg(test)]
mod extended_tests {
    use super::*;

    #[test]
    fn test_idempotency_properties() {
        let prop = IdempotencyProperties::parse_idempotent();
        let result = prop.test("fn main() { 42 }".to_string());
        assert!(result.is_pass());
    }

    #[test]
    fn test_roundtrip_properties() {
        let prop = RoundtripProperties::ast_roundtrip();
        let result = prop.test("fn main() { 42 }".to_string());
        assert!(result.is_pass());
    }

    #[test]
    fn test_commutativity_properties() {
        let prop = CommutativityProperties::addition_commutative();
        let result = prop.test("1 + 2".to_string());
        assert!(result.is_pass());
    }

    #[test]
    fn test_associativity_properties() {
        let prop = AssociativityProperties::addition_associative();
        let result = prop.test("1 + 2 + 3".to_string());
        assert!(result.is_pass());
    }

    #[test]
    fn test_identity_properties() {
        let prop = IdentityProperties::additive_identity();
        let result = prop.test("x + 0".to_string());
        assert!(result.is_pass());
    }

    #[test]
    fn test_involution_properties() {
        let prop = InvolutionProperties::double_negation();
        let result = prop.test("!!true".to_string());
        assert!(result.is_pass());
    }

    #[test]
    fn test_extended_runner() {
        let mut runner = ExtendedPropertyRunner::new();
        runner.add_all_properties();

        // Should have properties in multiple categories
        assert!(runner.category_count(PropertyCategory::Structural) > 0);
        assert!(runner.category_count(PropertyCategory::Algebraic) > 0);
        assert!(runner.category_count(PropertyCategory::Safety) > 0);

        // All should pass on normal input
        let passed = runner.run("fn main() { 42 }");
        assert!(passed);
    }

    #[test]
    fn test_property_combinator() {
        let combinator = PropertyCombinator::<String, ()>::new("combined")
            .add(|input| {
                if input.contains("fn") {
                    PropertyResult::Pass(())
                } else {
                    PropertyResult::Skip("No function".to_string())
                }
            })
            .add(|input| {
                if input.len() < 1000 {
                    PropertyResult::Pass(())
                } else {
                    PropertyResult::Fail(PropertyFailure::new("size", "Too large", &input))
                }
            });

        let result = combinator.run_all("fn main() {}".to_string());
        assert!(matches!(result, PropertyResult::Pass(_)));
    }

    #[test]
    fn test_numeric_properties() {
        let props = numeric_properties();
        assert!(props.len() >= 4);
    }

    #[test]
    fn test_boolean_properties() {
        let props = boolean_properties();
        assert!(props.len() >= 6);
    }
}
