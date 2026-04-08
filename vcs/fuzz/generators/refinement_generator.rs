//! Refinement type generator for Verum
//!
//! This module generates programs that use Verum's refinement type system,
//! which allows expressing constraints on values that are verified at compile time
//! using SMT solving (Z3).
//!
//! # Refinement Types in Verum
//!
//! Refinement types extend base types with logical predicates:
//! - `Int where x > 0` - positive integers
//! - `List<Int> where len(self) <= 10` - bounded lists
//! - `Float where 0.0 <= x && x <= 1.0` - normalized floats
//!
//! # Generated Programs
//!
//! This generator creates programs with:
//! - Function preconditions (`requires`)
//! - Function postconditions (`ensures`)
//! - Loop invariants (`invariant`)
//! - Assertions with refinement proofs

use rand::Rng;
use rand::seq::IndexedRandom;

/// Represents a refinement predicate
#[derive(Debug, Clone)]
pub enum RefinementPredicate {
    /// Variable comparison: x op value
    Comparison {
        var: String,
        op: CompOp,
        value: RefinementValue,
    },
    /// Range constraint: low <= x && x <= high
    Range {
        var: String,
        low: RefinementValue,
        high: RefinementValue,
    },
    /// Logical conjunction
    And(Box<RefinementPredicate>, Box<RefinementPredicate>),
    /// Logical disjunction
    Or(Box<RefinementPredicate>, Box<RefinementPredicate>),
    /// Logical negation
    Not(Box<RefinementPredicate>),
    /// Array/List length constraint
    LengthConstraint {
        var: String,
        op: CompOp,
        value: RefinementValue,
    },
    /// Implication: p1 => p2
    Implies(Box<RefinementPredicate>, Box<RefinementPredicate>),
    /// Quantified: forall i in range. predicate
    Forall {
        var: String,
        range: (i64, i64),
        body: Box<RefinementPredicate>,
    },
    /// Simple boolean
    Bool(bool),
}

/// Comparison operators
#[derive(Debug, Clone, Copy)]
pub enum CompOp {
    Lt, // <
    Le, // <=
    Gt, // >
    Ge, // >=
    Eq, // ==
    Ne, // !=
}

impl CompOp {
    fn to_string(&self) -> &'static str {
        match self {
            CompOp::Lt => "<",
            CompOp::Le => "<=",
            CompOp::Gt => ">",
            CompOp::Ge => ">=",
            CompOp::Eq => "==",
            CompOp::Ne => "!=",
        }
    }

    fn all() -> &'static [CompOp] {
        &[
            CompOp::Lt,
            CompOp::Le,
            CompOp::Gt,
            CompOp::Ge,
            CompOp::Eq,
            CompOp::Ne,
        ]
    }
}

/// Values used in refinement predicates
#[derive(Debug, Clone)]
pub enum RefinementValue {
    Literal(i64),
    Variable(String),
    Length(String), // len(var)
    Add(Box<RefinementValue>, Box<RefinementValue>),
    Sub(Box<RefinementValue>, Box<RefinementValue>),
    Mul(Box<RefinementValue>, Box<RefinementValue>),
}

impl RefinementValue {
    fn to_syntax(&self) -> String {
        match self {
            RefinementValue::Literal(n) => n.to_string(),
            RefinementValue::Variable(v) => v.clone(),
            RefinementValue::Length(v) => format!("len({})", v),
            RefinementValue::Add(l, r) => format!("({} + {})", l.to_syntax(), r.to_syntax()),
            RefinementValue::Sub(l, r) => format!("({} - {})", l.to_syntax(), r.to_syntax()),
            RefinementValue::Mul(l, r) => format!("({} * {})", l.to_syntax(), r.to_syntax()),
        }
    }
}

impl RefinementPredicate {
    /// Convert predicate to Verum syntax
    pub fn to_syntax(&self) -> String {
        match self {
            RefinementPredicate::Comparison { var, op, value } => {
                format!("{} {} {}", var, op.to_string(), value.to_syntax())
            }
            RefinementPredicate::Range { var, low, high } => {
                format!(
                    "{} <= {} && {} <= {}",
                    low.to_syntax(),
                    var,
                    var,
                    high.to_syntax()
                )
            }
            RefinementPredicate::And(l, r) => {
                format!("({} && {})", l.to_syntax(), r.to_syntax())
            }
            RefinementPredicate::Or(l, r) => {
                format!("({} || {})", l.to_syntax(), r.to_syntax())
            }
            RefinementPredicate::Not(p) => {
                format!("!({})", p.to_syntax())
            }
            RefinementPredicate::LengthConstraint { var, op, value } => {
                format!("len({}) {} {}", var, op.to_string(), value.to_syntax())
            }
            RefinementPredicate::Implies(p, q) => {
                format!("({} => {})", p.to_syntax(), q.to_syntax())
            }
            RefinementPredicate::Forall { var, range, body } => {
                format!(
                    "forall {} in {}..{}. {}",
                    var,
                    range.0,
                    range.1,
                    body.to_syntax()
                )
            }
            RefinementPredicate::Bool(b) => b.to_string(),
        }
    }
}

/// Configuration for the refinement generator
#[derive(Debug, Clone)]
pub struct RefinementConfig {
    /// Maximum predicate nesting depth
    pub max_predicate_depth: usize,
    /// Maximum number of conjuncts in a predicate
    pub max_conjuncts: usize,
    /// Whether to generate quantified predicates
    pub enable_quantifiers: bool,
    /// Whether to generate implication predicates
    pub enable_implications: bool,
    /// Maximum number of functions
    pub max_functions: usize,
    /// Maximum number of loop iterations
    pub max_loop_iterations: usize,
    /// Probability of adding preconditions
    pub precondition_probability: f64,
    /// Probability of adding postconditions
    pub postcondition_probability: f64,
    /// Probability of adding loop invariants
    pub invariant_probability: f64,
}

impl Default for RefinementConfig {
    fn default() -> Self {
        Self {
            max_predicate_depth: 3,
            max_conjuncts: 4,
            enable_quantifiers: true,
            enable_implications: true,
            max_functions: 5,
            max_loop_iterations: 20,
            precondition_probability: 0.8,
            postcondition_probability: 0.7,
            invariant_probability: 0.6,
        }
    }
}

/// Tracks variable information for generating valid refinements
#[derive(Debug, Clone)]
struct VariableInfo {
    name: String,
    base_type: BaseType,
    refinement: Option<RefinementPredicate>,
}

/// Base types that can have refinements
#[derive(Debug, Clone)]
enum BaseType {
    Int,
    Float,
    List(Box<BaseType>),
    Array(Box<BaseType>, usize),
}

impl BaseType {
    fn to_syntax(&self) -> String {
        match self {
            BaseType::Int => "Int".to_string(),
            BaseType::Float => "Float".to_string(),
            BaseType::List(inner) => format!("List<{}>", inner.to_syntax()),
            BaseType::Array(inner, size) => format!("[{}; {}]", inner.to_syntax(), size),
        }
    }
}

/// Generator for programs with refinement types
pub struct RefinementGenerator {
    config: RefinementConfig,
}

impl RefinementGenerator {
    /// Create a new refinement generator
    pub fn new(config: RefinementConfig) -> Self {
        Self { config }
    }

    /// Generate a complete program with refinement types
    pub fn generate_program<R: Rng>(&self, rng: &mut R) -> String {
        let mut program = String::new();

        program.push_str("// Refinement types test program\n");
        program.push_str("// All predicates should be verifiable by SMT solver\n\n");
        program.push_str("use verum_std::core::{List, Text, Maybe}\n\n");

        // Generate refined type aliases
        let type_aliases = self.generate_type_aliases(rng);
        program.push_str(&type_aliases);
        program.push('\n');

        // Generate functions with pre/post conditions
        let num_funcs = rng.random_range(2..=self.config.max_functions);
        for i in 0..num_funcs {
            let func = self.generate_refined_function(rng, i);
            program.push_str(&func);
            program.push('\n');
        }

        // Generate main with assertions
        program.push_str(&self.generate_main_with_assertions(rng));

        program
    }

    /// Generate type aliases with refinements
    fn generate_type_aliases<R: Rng>(&self, rng: &mut R) -> String {
        let mut result = String::new();

        // Positive integer
        result.push_str("type Positive = Int where self > 0\n");

        // Non-negative integer
        result.push_str("type Natural = Int where self >= 0\n");

        // Bounded integer
        let bound = rng.random_range(10..1000);
        result.push_str(&format!(
            "type Bounded = Int where 0 <= self && self < {}\n",
            bound
        ));

        // Percentage (0-100)
        result.push_str("type Percentage = Int where 0 <= self && self <= 100\n");

        // Unit interval float
        result.push_str("type UnitFloat = Float where 0.0 <= self && self <= 1.0\n");

        // Non-empty list
        result.push_str("type NonEmptyList<T> = List<T> where len(self) > 0\n");

        // Bounded list
        let max_len = rng.random_range(5..50);
        result.push_str(&format!(
            "type BoundedList<T> = List<T> where len(self) <= {}\n",
            max_len
        ));

        result
    }

    /// Generate a function with refinement annotations
    fn generate_refined_function<R: Rng>(&self, rng: &mut R, idx: usize) -> String {
        let mut result = String::new();
        let func_name = format!("refined_func_{}", idx);

        // Choose function pattern
        match rng.random_range(0..5) {
            0 => self.generate_arithmetic_function(rng, &func_name, &mut result),
            1 => self.generate_list_function(rng, &func_name, &mut result),
            2 => self.generate_search_function(rng, &func_name, &mut result),
            3 => self.generate_bounded_function(rng, &func_name, &mut result),
            _ => self.generate_loop_function(rng, &func_name, &mut result),
        }

        result
    }

    /// Generate an arithmetic function with refinements
    fn generate_arithmetic_function<R: Rng>(&self, rng: &mut R, name: &str, result: &mut String) {
        let patterns = [
            ("safe_div", "a: Int, b: Int where b != 0", "Int", "a / b"),
            (
                "abs",
                "x: Int",
                "Int where result >= 0",
                "if x >= 0 { x } else { -x }",
            ),
            (
                "clamp",
                "x: Int, low: Int, high: Int where low <= high",
                "Int where low <= result && result <= high",
                "if x < low { low } else if x > high { high } else { x }",
            ),
            (
                "safe_sqrt_int",
                "x: Int where x >= 0",
                "Int where result >= 0 && result * result <= x",
                "{\n        let mut r: Int = 0;\n        while (r + 1) * (r + 1) <= x {\n            r = r + 1;\n        }\n        r\n    }",
            ),
        ];

        let (base_name, params, ret, body) = patterns.choose(rng).unwrap();
        let actual_name = format!("{}_{}", name, base_name);

        // Add doc comment
        result.push_str(&format!("/// {} with refinement types\n", base_name));

        // Preconditions
        if rng.random_bool(self.config.precondition_probability) && params.contains("where") {
            result.push_str("// Precondition encoded in parameter type\n");
        }

        result.push_str(&format!("fn {}({}) -> {} {{\n", actual_name, params, ret));
        result.push_str(&format!("    {}\n", body));
        result.push_str("}\n");
    }

    /// Generate a list function with refinements
    fn generate_list_function<R: Rng>(&self, rng: &mut R, name: &str, result: &mut String) {
        let patterns = [
            (
                "first",
                "list: List<Int> where len(list) > 0",
                "Int",
                "list[0]",
            ),
            (
                "last",
                "list: List<Int> where len(list) > 0",
                "Int",
                "list[len(list) - 1]",
            ),
            (
                "sum_bounded",
                "list: List<Int> where len(list) <= 100",
                "Int",
                "{\n        let mut total: Int = 0;\n        for x in list {\n            total = total + x;\n        }\n        total\n    }",
            ),
            (
                "get_safe",
                "list: List<Int>, idx: Int where 0 <= idx && idx < len(list)",
                "Int",
                "list[idx]",
            ),
        ];

        let (base_name, params, ret, body) = patterns.choose(rng).unwrap();
        let actual_name = format!("{}_{}", name, base_name);

        result.push_str(&format!("/// List operation: {}\n", base_name));
        result.push_str(&format!("fn {}({}) -> {} {{\n", actual_name, params, ret));
        result.push_str(&format!("    {}\n", body));
        result.push_str("}\n");
    }

    /// Generate a search/find function with refinements
    fn generate_search_function<R: Rng>(&self, _rng: &mut R, name: &str, result: &mut String) {
        result.push_str(&format!("/// Binary search with correctness proof\n"));
        result.push_str(&format!(
            "fn {}_binary_search(\n    arr: List<Int>,\n    target: Int\n) -> Maybe<Int where 0 <= self && self < len(arr)>\n",
            name
        ));
        result.push_str("    requires is_sorted(arr)\n");
        result.push_str("    ensures match result {\n");
        result.push_str("        Some(idx) => arr[idx] == target,\n");
        result.push_str("        None => forall i in 0..len(arr). arr[i] != target\n");
        result.push_str("    }\n");
        result.push_str("{\n");
        result.push_str("    let mut low: Int where 0 <= self = 0;\n");
        result.push_str("    let mut high: Int = len(arr);\n");
        result.push_str("    \n");
        result.push_str("    while low < high\n");
        result.push_str("        invariant 0 <= low && low <= high && high <= len(arr)\n");
        result.push_str("        invariant forall i in 0..low. arr[i] < target\n");
        result.push_str("        invariant forall i in high..len(arr). arr[i] > target\n");
        result.push_str("    {\n");
        result.push_str(
            "        let mid: Int where low <= self && self < high = low + (high - low) / 2;\n",
        );
        result.push_str("        if arr[mid] == target {\n");
        result.push_str("            return Some(mid);\n");
        result.push_str("        } else if arr[mid] < target {\n");
        result.push_str("            low = mid + 1;\n");
        result.push_str("        } else {\n");
        result.push_str("            high = mid;\n");
        result.push_str("        }\n");
        result.push_str("    }\n");
        result.push_str("    None\n");
        result.push_str("}\n");
    }

    /// Generate a function with bounded iterations
    fn generate_bounded_function<R: Rng>(&self, rng: &mut R, name: &str, result: &mut String) {
        let max_iter = rng.random_range(10..100);

        result.push_str(&format!("/// Bounded iteration function\n"));
        result.push_str(&format!(
            "fn {}_bounded_loop(n: Int where 0 <= n && n <= {}) -> Int where result >= 0 {{\n",
            name, max_iter
        ));
        result.push_str("    let mut result: Int where result >= 0 = 0;\n");
        result.push_str("    let mut i: Int where 0 <= i = 0;\n");
        result.push_str("    \n");
        result.push_str("    while i < n\n");
        result.push_str("        invariant 0 <= i && i <= n\n");
        result.push_str("        invariant result >= 0\n");
        result.push_str(&format!("        decreases n - i\n"));
        result.push_str("    {\n");
        result.push_str("        result = result + i;\n");
        result.push_str("        i = i + 1;\n");
        result.push_str("    }\n");
        result.push_str("    result\n");
        result.push_str("}\n");
    }

    /// Generate a loop function with invariants
    fn generate_loop_function<R: Rng>(&self, rng: &mut R, name: &str, result: &mut String) {
        let operations = [
            ("accumulate", "+", "0"),
            ("product", "*", "1"),
            ("max_find", "max", "Int::MIN"),
        ];
        let (op_name, op, init) = operations.choose(rng).unwrap();

        result.push_str(&format!("/// {} with loop invariant\n", op_name));
        result.push_str(&format!(
            "fn {}_{}_loop(list: List<Int> where len(list) > 0) -> Int {{\n",
            name, op_name
        ));
        result.push_str(&format!("    let mut acc: Int = {};\n", init));
        result.push_str("    let mut idx: Int where 0 <= idx = 0;\n");
        result.push_str("    \n");
        result.push_str("    while idx < len(list)\n");
        result.push_str("        invariant 0 <= idx && idx <= len(list)\n");

        // Add operation-specific invariant
        match *op_name {
            "accumulate" => {
                result.push_str("        invariant acc == sum(list[0..idx])\n");
            }
            "product" => {
                result.push_str("        invariant acc == product(list[0..idx])\n");
            }
            "max_find" => {
                result.push_str("        invariant forall i in 0..idx. acc >= list[i]\n");
            }
            _ => {}
        }

        result.push_str("    {\n");

        match *op {
            "+" => result.push_str("        acc = acc + list[idx];\n"),
            "*" => result.push_str("        acc = acc * list[idx];\n"),
            "max" => result.push_str("        if list[idx] > acc { acc = list[idx]; }\n"),
            _ => {}
        }

        result.push_str("        idx = idx + 1;\n");
        result.push_str("    }\n");
        result.push_str("    acc\n");
        result.push_str("}\n");
    }

    /// Generate main function with assertions
    fn generate_main_with_assertions<R: Rng>(&self, rng: &mut R) -> String {
        let mut result = String::from("fn main() {\n");

        // Generate test cases with assertions
        result.push_str("    // Test refined arithmetic\n");
        result.push_str("    let pos: Positive = 42;\n");
        result.push_str("    assert pos > 0;\n\n");

        result.push_str("    let bounded: Bounded = 50;\n");
        result.push_str("    assert 0 <= bounded;\n\n");

        // List tests
        result.push_str("    // Test refined lists\n");
        result.push_str("    let non_empty: NonEmptyList<Int> = [1, 2, 3];\n");
        result.push_str("    assert len(non_empty) > 0;\n\n");

        // Arithmetic operations
        let divisor = rng.random_range(1..100);
        result.push_str(&format!(
            "    let safe_result = refined_func_0_safe_div(100, {});\n",
            divisor
        ));
        result.push_str("    assert safe_result >= 0 || safe_result < 0; // Always true\n\n");

        // Generate additional random assertions
        let num_assertions = rng.random_range(3..8);
        for i in 0..num_assertions {
            let assertion = self.generate_random_assertion(rng, i);
            result.push_str(&format!("    {}\n", assertion));
        }

        result.push_str("}\n");
        result
    }

    fn generate_random_assertion<R: Rng>(&self, rng: &mut R, idx: usize) -> String {
        let patterns = [
            // Simple arithmetic properties
            "let x_{idx}: Int = {val}; assert x_{idx} + 0 == x_{idx};",
            "let y_{idx}: Int = {val}; assert y_{idx} * 1 == y_{idx};",
            "let z_{idx}: Positive = {pos_val}; assert z_{idx} > 0;",
            // List properties
            "let list_{idx}: List<Int> = [{vals}]; assert len(list_{idx}) == {len};",
            // Range properties
            "let bounded_{idx}: Int where 0 <= self && self <= 100 = {bounded_val}; assert bounded_{idx} <= 100;",
        ];

        let pattern = patterns.choose(rng).unwrap();
        let val = rng.random_range(-100..100);
        let pos_val = rng.random_range(1..100);
        let bounded_val = rng.random_range(0..=100);
        let len = rng.random_range(1..5);
        let vals: Vec<String> = (0..len)
            .map(|_| rng.random_range(0..100).to_string())
            .collect();

        pattern
            .replace("{idx}", &idx.to_string())
            .replace("{val}", &val.to_string())
            .replace("{pos_val}", &pos_val.to_string())
            .replace("{bounded_val}", &bounded_val.to_string())
            .replace("{vals}", &vals.join(", "))
            .replace("{len}", &len.to_string())
    }

    /// Generate a predicate of specified complexity
    pub fn generate_predicate<R: Rng>(
        &self,
        rng: &mut R,
        var: &str,
        depth: usize,
    ) -> RefinementPredicate {
        if depth >= self.config.max_predicate_depth {
            return self.generate_simple_predicate(rng, var);
        }

        match rng.random_range(0..10) {
            0..=3 => self.generate_simple_predicate(rng, var),
            4..=5 => {
                // Conjunction
                let left = self.generate_predicate(rng, var, depth + 1);
                let right = self.generate_predicate(rng, var, depth + 1);
                RefinementPredicate::And(Box::new(left), Box::new(right))
            }
            6 => {
                // Disjunction
                let left = self.generate_predicate(rng, var, depth + 1);
                let right = self.generate_predicate(rng, var, depth + 1);
                RefinementPredicate::Or(Box::new(left), Box::new(right))
            }
            7 if self.config.enable_implications => {
                // Implication
                let left = self.generate_predicate(rng, var, depth + 1);
                let right = self.generate_predicate(rng, var, depth + 1);
                RefinementPredicate::Implies(Box::new(left), Box::new(right))
            }
            8 if self.config.enable_quantifiers => {
                // Forall
                let quant_var = format!("i_{}", depth);
                let upper = rng.random_range(5..20);
                let body = self.generate_simple_predicate(rng, &quant_var);
                RefinementPredicate::Forall {
                    var: quant_var,
                    range: (0, upper),
                    body: Box::new(body),
                }
            }
            _ => self.generate_simple_predicate(rng, var),
        }
    }

    fn generate_simple_predicate<R: Rng>(&self, rng: &mut R, var: &str) -> RefinementPredicate {
        match rng.random_range(0..4) {
            0 => {
                // Simple comparison
                let op = *CompOp::all().choose(rng).unwrap();
                let value = RefinementValue::Literal(rng.random_range(-100..100));
                RefinementPredicate::Comparison {
                    var: var.to_string(),
                    op,
                    value,
                }
            }
            1 => {
                // Range
                let low = rng.random_range(-50..0);
                let high = rng.random_range(0..50);
                RefinementPredicate::Range {
                    var: var.to_string(),
                    low: RefinementValue::Literal(low),
                    high: RefinementValue::Literal(high),
                }
            }
            2 => {
                // Length constraint (for lists)
                let op = *[CompOp::Lt, CompOp::Le, CompOp::Gt, CompOp::Ge]
                    .choose(rng)
                    .unwrap();
                let value = RefinementValue::Literal(rng.random_range(0..20));
                RefinementPredicate::LengthConstraint {
                    var: var.to_string(),
                    op,
                    value,
                }
            }
            _ => {
                // Boolean literal
                RefinementPredicate::Bool(rng.random_bool(0.5))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_refinement_generator() {
        let config = RefinementConfig::default();
        let generator = RefinementGenerator::new(config);

        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let program = generator.generate_program(&mut rng);
        assert!(!program.is_empty());
        assert!(program.contains("where")); // Should have refinement annotations
        assert!(program.contains("fn main()"));
    }

    #[test]
    fn test_predicate_generation() {
        let config = RefinementConfig::default();
        let generator = RefinementGenerator::new(config);

        let mut rng = ChaCha8Rng::seed_from_u64(123);

        for _ in 0..20 {
            let pred = generator.generate_predicate(&mut rng, "x", 0);
            let syntax = pred.to_syntax();
            assert!(!syntax.is_empty());
        }
    }

    #[test]
    fn test_predicate_syntax() {
        let pred = RefinementPredicate::Range {
            var: "x".to_string(),
            low: RefinementValue::Literal(0),
            high: RefinementValue::Literal(100),
        };
        assert_eq!(pred.to_syntax(), "0 <= x && x <= 100");

        let pred = RefinementPredicate::And(
            Box::new(RefinementPredicate::Comparison {
                var: "x".to_string(),
                op: CompOp::Gt,
                value: RefinementValue::Literal(0),
            }),
            Box::new(RefinementPredicate::Comparison {
                var: "x".to_string(),
                op: CompOp::Lt,
                value: RefinementValue::Literal(10),
            }),
        );
        assert_eq!(pred.to_syntax(), "(x > 0 && x < 10)");
    }
}
