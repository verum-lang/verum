//! Refinement type generator for fuzzing
//!
//! Generates programs with refinement types to test the verification system.
//! Includes:
//! - Simple refinements (x > 0, len(s) < 100)
//! - Complex predicates with quantifiers
//! - Dependent types
//! - Pre/post conditions
//! - Loop invariants

use super::{Generate, GeneratorConfig, indent, random_primitive_type};
use rand::prelude::*;

/// Generator for programs with refinement types
pub struct RefinementGenerator {
    config: GeneratorConfig,
    current_depth: usize,
    variables: Vec<(String, String, Option<String>)>, // (name, type, refinement)
}

impl RefinementGenerator {
    /// Create a new refinement generator
    pub fn new(config: GeneratorConfig) -> Self {
        Self {
            config,
            current_depth: 0,
            variables: Vec::new(),
        }
    }

    /// Reset state
    fn reset(&mut self) {
        self.current_depth = 0;
        self.variables.clear();
    }

    /// Generate a complete program with refinements
    fn generate_program<R: Rng>(&mut self, rng: &mut R) -> String {
        self.reset();
        let mut output = String::new();

        output.push_str("use verum_std::core::{List, Text, Map, Maybe}\n\n");

        // Generate refined type aliases
        let num_types = rng.random_range(1..=3);
        for _ in 0..num_types {
            output.push_str(&self.generate_refined_type_alias(rng));
            output.push_str("\n\n");
        }

        // Generate functions with contracts
        let num_functions = rng.random_range(2..=5);
        for i in 0..num_functions {
            output.push_str(&self.generate_contracted_function(rng, &format!("func_{}", i)));
            output.push_str("\n\n");
        }

        // Generate main with verification
        output.push_str(&self.generate_verified_main(rng));

        output
    }

    /// Generate a refined type alias
    fn generate_refined_type_alias<R: Rng>(&self, rng: &mut R) -> String {
        let name = format!("Refined{}", rng.random_range(0..100));
        let base_type = random_primitive_type(rng);
        let refinement = self.generate_simple_refinement(rng, "x", base_type);

        format!("type {} = {{ x: {} | {} }}", name, base_type, refinement)
    }

    /// Generate a simple refinement predicate
    fn generate_simple_refinement<R: Rng>(&self, rng: &mut R, var: &str, ty: &str) -> String {
        match ty {
            "Int" => self.generate_int_refinement(rng, var),
            "Float" => self.generate_float_refinement(rng, var),
            "Text" => self.generate_text_refinement(rng, var),
            "Bool" => format!("{} == true", var),
            _ => format!("{} != ()", var),
        }
    }

    /// Generate an integer refinement
    fn generate_int_refinement<R: Rng>(&self, rng: &mut R, var: &str) -> String {
        match rng.random_range(0..12) {
            0 => format!("{} > 0", var),
            1 => format!("{} >= 0", var),
            2 => format!("{} < 0", var),
            3 => format!("{} <= 0", var),
            4 => format!("{} != 0", var),
            5 => format!("{} >= 0 && {} <= 100", var, var),
            6 => format!("{} > -100 && {} < 100", var, var),
            7 => format!("{} % 2 == 0", var), // Even
            8 => format!("{} % 2 == 1", var), // Odd
            9 => format!("{} >= 1 && {} <= 10", var, var),
            10 => format!("abs({}) <= 1000", var),
            _ => format!("{} == {}", var, rng.random_range(-10..10)),
        }
    }

    /// Generate a float refinement
    fn generate_float_refinement<R: Rng>(&self, rng: &mut R, var: &str) -> String {
        match rng.random_range(0..8) {
            0 => format!("{} > 0.0", var),
            1 => format!("{} >= 0.0", var),
            2 => format!("{} < 1.0", var),
            3 => format!("{} >= 0.0 && {} <= 1.0", var, var), // Unit interval
            4 => format!("{} > -1.0 && {} < 1.0", var, var),
            5 => format!("abs({}) < 100.0", var),
            6 => format!("{} != 0.0", var),
            _ => format!("{} >= -1000.0 && {} <= 1000.0", var, var),
        }
    }

    /// Generate a text refinement
    fn generate_text_refinement<R: Rng>(&self, rng: &mut R, var: &str) -> String {
        match rng.random_range(0..8) {
            0 => format!("len({}) > 0", var),
            1 => format!("len({}) < 100", var),
            2 => format!("len({}) >= 1 && len({}) <= 50", var, var),
            3 => format!("len({}) == 0", var), // Empty
            4 => format!("len({}) > 0 && len({}) < 256", var, var),
            5 => format!("!is_empty({})", var),
            6 => format!("len({}) <= 1024", var),
            _ => format!("len({}) < 10000", var),
        }
    }

    /// Generate a function with contracts
    fn generate_contracted_function<R: Rng>(&mut self, rng: &mut R, name: &str) -> String {
        self.variables.clear();

        // Generate parameters with refinements
        let num_params = rng.random_range(1..=3);
        let mut params = Vec::new();
        let mut requires = Vec::new();

        for i in 0..num_params {
            let param_name = format!("p{}", i);
            let param_type = random_primitive_type(rng);
            let refinement = if rng.random_bool(0.7) {
                let ref_str = self.generate_simple_refinement(rng, &param_name, param_type);
                requires.push(ref_str.clone());
                Some(ref_str)
            } else {
                None
            };

            self.variables
                .push((param_name.clone(), param_type.to_string(), refinement));
            params.push(format!("{}: {}", param_name, param_type));
        }

        // Return type with optional refinement
        let return_type = random_primitive_type(rng);
        let return_refinement = if rng.random_bool(0.6) {
            Some(self.generate_return_refinement(rng, return_type))
        } else {
            None
        };

        // Build contract
        let mut contract = String::new();
        if !requires.is_empty() || return_refinement.is_some() {
            contract.push_str("    contract#\"\n");
            for req in &requires {
                contract.push_str(&format!("        requires {};\n", req));
            }
            if let Some(ref ensures) = return_refinement {
                contract.push_str(&format!("        ensures {};\n", ensures));
            }
            contract.push_str("    \"\n");
        }

        // Generate body
        let body = self.generate_verified_body(rng, return_type);

        // Optional verification attribute
        let verify_attr = if rng.random_bool(0.5) {
            "@verify(proof)\n"
        } else if rng.random_bool(0.5) {
            "@verify(check)\n"
        } else {
            ""
        };

        format!(
            "{}fn {}({}) -> {} {{\n{}{}\n}}",
            verify_attr,
            name,
            params.join(", "),
            return_type,
            contract,
            body
        )
    }

    /// Generate a return refinement
    fn generate_return_refinement<R: Rng>(&self, rng: &mut R, ty: &str) -> String {
        let result_var = "result";

        match ty {
            "Int" => {
                // Generate refinement that relates to parameters
                if !self.variables.is_empty() && rng.random_bool(0.5) {
                    let (param_name, _, _) = &self.variables[0];
                    match rng.random_range(0..4) {
                        0 => format!("{} >= {}", result_var, param_name),
                        1 => format!("{} <= {}", result_var, param_name),
                        2 => format!("abs({}) <= abs({})", result_var, param_name),
                        _ => format!("{} >= 0", result_var),
                    }
                } else {
                    self.generate_int_refinement(rng, result_var)
                }
            }
            "Bool" => format!("{} == true || {} == false", result_var, result_var),
            _ => self.generate_simple_refinement(rng, result_var, ty),
        }
    }

    /// Generate a verified function body
    fn generate_verified_body<R: Rng>(&mut self, rng: &mut R, return_type: &str) -> String {
        let mut body = String::new();
        let indent_level = 1;

        // Add some verified statements
        let num_stmts = rng.random_range(1..=5);
        for i in 0..num_stmts {
            match rng.random_range(0..5) {
                0 => {
                    // Let with assert
                    let var = format!("x{}", i);
                    body.push_str(&format!(
                        "{}let {} = {};\n",
                        indent(indent_level),
                        var,
                        rng.random_range(0..100)
                    ));
                    body.push_str(&format!("{}assert {} >= 0;\n", indent(indent_level), var));
                }
                1 => {
                    // If with invariant
                    body.push_str(&format!(
                        "{}if {} > 0 {{\n",
                        indent(indent_level),
                        rng.random_range(1..10)
                    ));
                    body.push_str(&format!("{}assert true;\n", indent(indent_level + 1)));
                    body.push_str(&format!("{}}}\n", indent(indent_level)));
                }
                2 => {
                    // Loop with invariant
                    body.push_str(&format!("{}let mut i = 0;\n", indent(indent_level)));
                    body.push_str(&format!(
                        "{}while i < {} {{\n",
                        indent(indent_level),
                        rng.random_range(1..10)
                    ));
                    body.push_str(&format!("{}invariant i >= 0;\n", indent(indent_level + 1)));
                    body.push_str(&format!("{}i = i + 1;\n", indent(indent_level + 1)));
                    body.push_str(&format!("{}}}\n", indent(indent_level)));
                }
                3 => {
                    // Assume (for verification)
                    body.push_str(&format!(
                        "{}assume {} > 0;\n",
                        indent(indent_level),
                        rng.random_range(1..10)
                    ));
                }
                _ => {
                    // Simple let
                    body.push_str(&format!(
                        "{}let val_{} = {};\n",
                        indent(indent_level),
                        i,
                        rng.random_range(-10..10)
                    ));
                }
            }
        }

        // Return expression
        let return_expr = match return_type {
            "Int" => format!("{}", rng.random_range(0..100)),
            "Float" => format!("{:.2}", rng.random_range(0.0..100.0)),
            "Bool" => if rng.random_bool(0.5) {
                "true"
            } else {
                "false"
            }
            .to_string(),
            "Text" => "\"result\"".to_string(),
            _ => "()".to_string(),
        };
        body.push_str(&format!("{}{}", indent(indent_level), return_expr));

        body
    }

    /// Generate main with verification examples
    fn generate_verified_main<R: Rng>(&mut self, rng: &mut R) -> String {
        let mut body = String::new();
        let indent_level = 1;

        // Declare some refined variables
        body.push_str(&format!(
            "{}// Refined variable declarations\n",
            indent(indent_level)
        ));

        // Positive integer
        body.push_str(&format!(
            "{}let positive: {{ x: Int | x > 0 }} = {};\n",
            indent(indent_level),
            rng.random_range(1..100)
        ));

        // Bounded integer
        body.push_str(&format!(
            "{}let bounded: {{ x: Int | x >= 0 && x <= 100 }} = {};\n",
            indent(indent_level),
            rng.random_range(0..=100)
        ));

        // Non-empty string
        body.push_str(&format!(
            "{}let non_empty: {{ s: Text | len(s) > 0 }} = \"hello\";\n",
            indent(indent_level)
        ));

        // Add assertions
        body.push_str(&format!("\n{}// Assertions\n", indent(indent_level)));
        body.push_str(&format!("{}assert positive > 0;\n", indent(indent_level)));
        body.push_str(&format!(
            "{}assert bounded >= 0 && bounded <= 100;\n",
            indent(indent_level)
        ));
        body.push_str(&format!(
            "{}assert len(non_empty) > 0;\n",
            indent(indent_level)
        ));

        // Add some arithmetic with refinement preservation
        body.push_str(&format!(
            "\n{}// Refinement-preserving operations\n",
            indent(indent_level)
        ));
        body.push_str(&format!(
            "{}let sum = positive + bounded;\n",
            indent(indent_level)
        ));
        body.push_str(&format!("{}assert sum > 0;\n", indent(indent_level)));

        // Conditional with refinement
        body.push_str(&format!(
            "\n{}// Conditional refinement\n",
            indent(indent_level)
        ));
        body.push_str(&format!(
            "{}let result = if positive > 50 {{ positive }} else {{ bounded }};\n",
            indent(indent_level)
        ));
        body.push_str(&format!("{}assert result >= 0;\n", indent(indent_level)));

        format!("fn main() {{\n{}}}\n", body)
    }

    /// Generate a complex predicate with quantifiers
    #[allow(dead_code)]
    fn generate_quantified_predicate<R: Rng>(&self, rng: &mut R) -> String {
        match rng.random_range(0..6) {
            0 => "forall i: Int. 0 <= i && i < len(arr) => arr[i] >= 0".to_string(),
            1 => "exists i: Int. 0 <= i && i < len(arr) && arr[i] == target".to_string(),
            2 => {
                "forall i, j: Int. 0 <= i && i < j && j < len(arr) => arr[i] <= arr[j]".to_string()
            }
            3 => "forall x: Int. member(x, set) => x > 0".to_string(),
            4 => "exists k: Int. k >= 0 && arr[k] == max(arr)".to_string(),
            _ => "forall i: Int. 0 <= i && i < len(arr) => arr[i] < 1000".to_string(),
        }
    }
}

impl Generate for RefinementGenerator {
    fn generate<R: Rng>(&mut self, rng: &mut R) -> String {
        self.generate_program(rng)
    }

    fn name(&self) -> &'static str {
        "RefinementGenerator"
    }

    fn description(&self) -> &'static str {
        "Generates programs with refinement types and verification conditions"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_refinement_generator() {
        let config = GeneratorConfig {
            include_refinements: true,
            ..Default::default()
        };
        let mut generator = RefinementGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let program = generator.generate(&mut rng);
        assert!(!program.is_empty());
        assert!(program.contains("fn main()"));
    }

    #[test]
    fn test_generates_refinements() {
        let config = GeneratorConfig {
            include_refinements: true,
            ..Default::default()
        };
        let mut generator = RefinementGenerator::new(config);

        // Check that refinements are generated across multiple seeds
        let mut found_refinement = false;
        for seed in 0..20 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let program = generator.generate(&mut rng);
            if program.contains("| x >")
                || program.contains("| len(")
                || program.contains("requires")
            {
                found_refinement = true;
                break;
            }
        }

        assert!(found_refinement, "Should generate refinement predicates");
    }

    #[test]
    fn test_generates_contracts() {
        let config = GeneratorConfig {
            include_refinements: true,
            max_functions: 5,
            ..Default::default()
        };
        let mut generator = RefinementGenerator::new(config);

        let mut found_requires = false;
        let mut found_ensures = false;

        for seed in 0..50 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let program = generator.generate(&mut rng);

            if program.contains("requires") {
                found_requires = true;
            }
            if program.contains("ensures") {
                found_ensures = true;
            }

            if found_requires && found_ensures {
                break;
            }
        }

        assert!(found_requires, "Should generate requires clauses");
        assert!(found_ensures, "Should generate ensures clauses");
    }
}
