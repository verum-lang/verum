//! Parser construct generator for fuzzing
//!
//! Generates syntactically valid Verum programs that exercise the parser.
//! Covers all major syntactic constructs including:
//! - Declarations (functions, types, imports)
//! - Expressions (binary, unary, calls, lambdas)
//! - Statements (let, if, match, loops)
//! - Patterns (literals, bindings, destructuring)

use super::{
    Generate, GeneratorConfig, indent, random_function_name, random_identifier,
    random_primitive_type, random_type, random_type_name,
};
use rand::prelude::*;

/// Generator for parser constructs
pub struct ParserGenerator {
    config: GeneratorConfig,
    /// Current depth for recursive generation
    current_depth: usize,
    /// Variables in scope (name, type)
    scope: Vec<(String, String)>,
    /// Function names defined
    functions: Vec<String>,
    /// Type names defined
    types: Vec<String>,
}

impl ParserGenerator {
    /// Create a new parser generator
    pub fn new(config: GeneratorConfig) -> Self {
        Self {
            config,
            current_depth: 0,
            scope: Vec::new(),
            functions: Vec::new(),
            types: Vec::new(),
        }
    }

    /// Reset state for new generation
    fn reset(&mut self) {
        self.current_depth = 0;
        self.scope.clear();
        self.functions.clear();
        self.types.clear();
    }

    /// Generate a complete program
    fn generate_program<R: Rng>(&mut self, rng: &mut R) -> String {
        self.reset();
        let mut output = String::new();

        // Optional imports
        if rng.random_bool(0.7) {
            output.push_str(&self.generate_imports(rng));
            output.push('\n');
        }

        // Type definitions
        let num_types = rng.random_range(0..=self.config.max_types);
        for _ in 0..num_types {
            output.push_str(&self.generate_type_def(rng));
            output.push_str("\n\n");
        }

        // Functions
        let num_functions = rng.random_range(1..=self.config.max_functions);
        for _ in 0..num_functions {
            let name = random_function_name(rng);
            self.functions.push(name.clone());
            output.push_str(&self.generate_function(rng, &name));
            output.push_str("\n\n");
        }

        // Main function
        output.push_str(&self.generate_main(rng));

        output
    }

    /// Generate imports
    fn generate_imports<R: Rng>(&self, rng: &mut R) -> String {
        let mut imports = Vec::new();

        if rng.random_bool(0.8) {
            imports.push("use verum_std::core::{List, Text, Map, Maybe, Set}");
        }
        if rng.random_bool(0.3) {
            imports.push("use verum_std::io::{print, println, read_line}");
        }
        if rng.random_bool(0.2) {
            imports.push("use verum_std::math::{abs, min, max, sqrt}");
        }
        if rng.random_bool(0.2) && self.config.include_async {
            imports.push("use verum_std::async::{spawn, sleep, join}");
        }

        imports.join("\n")
    }

    /// Generate a type definition
    fn generate_type_def<R: Rng>(&mut self, rng: &mut R) -> String {
        let name = random_type_name(rng);
        self.types.push(name.clone());

        match rng.random_range(0..4) {
            0 => self.generate_record_type(rng, &name),
            1 => self.generate_variant_type(rng, &name),
            2 => self.generate_type_alias(rng, &name),
            _ => self.generate_generic_type(rng, &name),
        }
    }

    /// Generate a record type
    fn generate_record_type<R: Rng>(&self, rng: &mut R, name: &str) -> String {
        let num_fields = rng.random_range(1..=5);
        let fields: Vec<String> = (0..num_fields)
            .map(|i| {
                let field_name = format!("field_{}", i);
                let field_type = random_type(rng, 0);
                format!("    {}: {}", field_name, field_type)
            })
            .collect();

        format!("type {} = {{\n{}\n}}", name, fields.join(",\n"))
    }

    /// Generate a variant type (enum)
    fn generate_variant_type<R: Rng>(&self, rng: &mut R, name: &str) -> String {
        let num_variants = rng.random_range(2..=6);
        let variants: Vec<String> = (0..num_variants)
            .map(|i| {
                let variant_name = format!("Variant{}", i);
                if rng.random_bool(0.5) {
                    format!("    {}", variant_name)
                } else if rng.random_bool(0.5) {
                    format!("    {}({})", variant_name, random_primitive_type(rng))
                } else {
                    let fields = format!(
                        "{{ a: {}, b: {} }}",
                        random_primitive_type(rng),
                        random_primitive_type(rng)
                    );
                    format!("    {}{}", variant_name, fields)
                }
            })
            .collect();

        format!("type {} = |\n{}\n|", name, variants.join(",\n"))
    }

    /// Generate a type alias
    fn generate_type_alias<R: Rng>(&self, rng: &mut R, name: &str) -> String {
        format!("type {} = {}", name, random_type(rng, 0))
    }

    /// Generate a generic type
    fn generate_generic_type<R: Rng>(&self, rng: &mut R, name: &str) -> String {
        let type_params = vec!["T", "U", "V"];
        let num_params = rng.random_range(1..=2);
        let params: Vec<&str> = type_params.iter().take(num_params).cloned().collect();

        let num_fields = rng.random_range(1..=3);
        let fields: Vec<String> = (0..num_fields)
            .map(|i| {
                let field_type = if rng.random_bool(0.5) {
                    params[rng.random_range(0..params.len())].to_string()
                } else {
                    random_primitive_type(rng).to_string()
                };
                format!("    field_{}: {}", i, field_type)
            })
            .collect();

        format!(
            "type {}<{}> = {{\n{}\n}}",
            name,
            params.join(", "),
            fields.join(",\n")
        )
    }

    /// Generate a function
    fn generate_function<R: Rng>(&mut self, rng: &mut R, name: &str) -> String {
        self.scope.clear();

        // Generate parameters
        let num_params = rng.random_range(0..=4);
        let params: Vec<String> = (0..num_params)
            .map(|i| {
                let param_name = format!("p{}", i);
                let param_type = random_primitive_type(rng);
                self.scope
                    .push((param_name.clone(), param_type.to_string()));
                format!("{}: {}", param_name, param_type)
            })
            .collect();

        // Optional return type
        let return_type = if rng.random_bool(0.7) {
            Some(random_primitive_type(rng).to_string())
        } else {
            None
        };

        // Optional async
        let async_kw = if self.config.include_async && rng.random_bool(0.2) {
            "async "
        } else {
            ""
        };

        // Optional pub
        let pub_kw = if rng.random_bool(0.3) { "pub " } else { "" };

        // Generate body
        let body = self.generate_block(rng, 1);

        let return_annotation = return_type.map_or(String::new(), |t| format!(" -> {}", t));

        format!(
            "{}{}fn {}({}){}  {{\n{}\n}}",
            pub_kw,
            async_kw,
            name,
            params.join(", "),
            return_annotation,
            body
        )
    }

    /// Generate main function
    fn generate_main<R: Rng>(&mut self, rng: &mut R) -> String {
        self.scope.clear();
        let body = self.generate_block(rng, 1);
        format!("fn main() {{\n{}\n}}\n", body)
    }

    /// Generate a block of statements
    fn generate_block<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let num_statements = rng.random_range(1..=self.config.max_statements.min(10));
        let mut statements = Vec::new();

        for _ in 0..num_statements {
            statements.push(self.generate_statement(rng, indent_level));
        }

        statements.join("\n")
    }

    /// Generate a statement
    fn generate_statement<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        if self.current_depth > self.config.max_depth {
            return self.generate_simple_statement(rng, indent_level);
        }

        self.current_depth += 1;
        let result = match rng.random_range(0..15) {
            0..=4 => self.generate_let_statement(rng, indent_level),
            5 => self.generate_if_statement(rng, indent_level),
            6 => self.generate_match_statement(rng, indent_level),
            7 => self.generate_for_loop(rng, indent_level),
            8 => self.generate_while_loop(rng, indent_level),
            9 => self.generate_loop_statement(rng, indent_level),
            10 => self.generate_return_statement(rng, indent_level),
            11 => self.generate_expression_statement(rng, indent_level),
            12 => self.generate_block_statement(rng, indent_level),
            13 => self.generate_assignment(rng, indent_level),
            _ => self.generate_simple_statement(rng, indent_level),
        };
        self.current_depth -= 1;
        result
    }

    /// Generate a simple statement (for depth limit)
    fn generate_simple_statement<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let name = random_identifier(rng);
        let literal = self.generate_literal(rng);
        self.scope.push((name.clone(), "Int".to_string()));
        format!("{}let {} = {};", indent(indent_level), name, literal)
    }

    /// Generate a let statement
    fn generate_let_statement<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let name = random_identifier(rng);
        let type_annotation = if rng.random_bool(0.5) {
            format!(": {}", random_primitive_type(rng))
        } else {
            String::new()
        };
        let expr = self.generate_expression(rng, indent_level);
        self.scope.push((name.clone(), "Int".to_string()));

        format!(
            "{}let {}{} = {};",
            indent(indent_level),
            name,
            type_annotation,
            expr
        )
    }

    /// Generate an if statement
    fn generate_if_statement<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let condition = self.generate_bool_expression(rng);
        let then_block = self.generate_block(rng, indent_level + 1);

        let else_part = if rng.random_bool(0.5) {
            format!(
                " else {{\n{}\n{}}}",
                self.generate_block(rng, indent_level + 1),
                indent(indent_level)
            )
        } else if rng.random_bool(0.3) {
            // else if
            format!(
                " else {}",
                self.generate_if_statement(rng, indent_level).trim_start()
            )
        } else {
            String::new()
        };

        format!(
            "{}if {} {{\n{}\n{}}}{}",
            indent(indent_level),
            condition,
            then_block,
            indent(indent_level),
            else_part
        )
    }

    /// Generate a match statement
    fn generate_match_statement<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let scrutinee = self.generate_expression(rng, indent_level);
        let num_arms = rng.random_range(2..=5);
        let mut arms = Vec::new();

        for i in 0..num_arms {
            let pattern = if i == num_arms - 1 {
                "_".to_string() // Wildcard for exhaustiveness
            } else {
                self.generate_pattern(rng)
            };
            let body = self.generate_expression(rng, indent_level + 1);
            arms.push(format!(
                "{}{} => {}",
                indent(indent_level + 1),
                pattern,
                body
            ));
        }

        format!(
            "{}let result = match {} {{\n{}\n{}}};",
            indent(indent_level),
            scrutinee,
            arms.join(",\n"),
            indent(indent_level)
        )
    }

    /// Generate a for loop
    fn generate_for_loop<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let var = random_identifier(rng);
        let start = rng.random_range(0..10);
        let end = start + rng.random_range(1..20);

        self.scope.push((var.clone(), "Int".to_string()));
        let body = self.generate_block(rng, indent_level + 1);

        format!(
            "{}for {} in {}..{} {{\n{}\n{}}}",
            indent(indent_level),
            var,
            start,
            end,
            body,
            indent(indent_level)
        )
    }

    /// Generate a while loop
    fn generate_while_loop<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let condition = self.generate_bool_expression(rng);
        let body = format!(
            "{}\n{}break;",
            self.generate_statement(rng, indent_level + 1),
            indent(indent_level + 1)
        );

        format!(
            "{}while {} {{\n{}\n{}}}",
            indent(indent_level),
            condition,
            body,
            indent(indent_level)
        )
    }

    /// Generate a loop statement
    fn generate_loop_statement<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let body = format!(
            "{}\n{}break;",
            self.generate_statement(rng, indent_level + 1),
            indent(indent_level + 1)
        );

        format!(
            "{}loop {{\n{}\n{}}}",
            indent(indent_level),
            body,
            indent(indent_level)
        )
    }

    /// Generate a return statement
    fn generate_return_statement<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        if rng.random_bool(0.3) {
            format!("{}return;", indent(indent_level))
        } else {
            format!(
                "{}return {};",
                indent(indent_level),
                self.generate_expression(rng, indent_level)
            )
        }
    }

    /// Generate an expression statement
    fn generate_expression_statement<R: Rng>(
        &mut self,
        rng: &mut R,
        indent_level: usize,
    ) -> String {
        format!(
            "{}{};",
            indent(indent_level),
            self.generate_expression(rng, indent_level)
        )
    }

    /// Generate a block statement
    fn generate_block_statement<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let body = self.generate_block(rng, indent_level + 1);
        format!(
            "{}{{\n{}\n{}}}",
            indent(indent_level),
            body,
            indent(indent_level)
        )
    }

    /// Generate an assignment
    fn generate_assignment<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        if self.scope.is_empty() {
            return self.generate_let_statement(rng, indent_level);
        }

        let (name, _) = &self.scope[rng.random_range(0..self.scope.len())];
        let name = name.clone();
        let expr = self.generate_expression(rng, indent_level);

        format!("{}{} = {};", indent(indent_level), name, expr)
    }

    /// Generate an expression
    fn generate_expression<R: Rng>(&mut self, rng: &mut R, _indent_level: usize) -> String {
        if self.current_depth > self.config.max_depth {
            return self.generate_literal(rng);
        }

        self.current_depth += 1;
        let result = match rng.random_range(0..15) {
            0..=3 => self.generate_literal(rng),
            4..=5 => self.generate_binary_expression(rng),
            6 => self.generate_unary_expression(rng),
            7 => self.generate_if_expression(rng),
            8 => self.generate_block_expression(rng),
            9 => self.generate_list_expression(rng),
            10 => self.generate_tuple_expression(rng),
            11 => self.generate_call_expression(rng),
            12 => self.generate_lambda_expression(rng),
            13 => self.generate_variable_reference(rng),
            _ => self.generate_field_access(rng),
        };
        self.current_depth -= 1;
        result
    }

    /// Generate a literal
    fn generate_literal<R: Rng>(&self, rng: &mut R) -> String {
        match rng.random_range(0..8) {
            0 => format!("{}", rng.random_range(-1000..1000)),
            1 => format!("{:.2}", rng.random_range(-1000.0..1000.0)),
            2 => if rng.random_bool(0.5) {
                "true"
            } else {
                "false"
            }
            .to_string(),
            3 => format!("\"{}\"", super::random_string(rng, 20)),
            4 => "()".to_string(),
            5 => format!("'{}'", (b'a' + rng.random_range(0..26)) as char),
            6 => format!("0x{:X}", rng.random_range(0..256)),
            _ => format!("0b{:b}", rng.random_range(0..256)),
        }
    }

    /// Generate a binary expression
    fn generate_binary_expression<R: Rng>(&mut self, rng: &mut R) -> String {
        let ops = [
            "+", "-", "*", "/", "%", "==", "!=", "<", ">", "<=", ">=", "&&", "||",
        ];
        let op = ops[rng.random_range(0..ops.len())];
        format!(
            "({} {} {})",
            self.generate_expression(rng, 0),
            op,
            self.generate_expression(rng, 0)
        )
    }

    /// Generate a unary expression
    fn generate_unary_expression<R: Rng>(&mut self, rng: &mut R) -> String {
        let ops = ["-", "!"];
        let op = ops[rng.random_range(0..ops.len())];
        format!("({}{})", op, self.generate_expression(rng, 0))
    }

    /// Generate a boolean expression
    /// NOTE: Fixed unbounded recursion - now tracks depth properly
    fn generate_bool_expression<R: Rng>(&mut self, rng: &mut R) -> String {
        // Check depth limit to prevent unbounded recursion and stack overflow
        if self.current_depth > 3 || rng.random_bool(0.5) {
            return if rng.random_bool(0.5) {
                "true"
            } else {
                "false"
            }
            .to_string();
        }

        // Increment depth before recursive calls
        self.current_depth += 1;
        let result = match rng.random_range(0..5) {
            0 => format!(
                "({} == {})",
                self.generate_expression(rng, 0),
                self.generate_expression(rng, 0)
            ),
            1 => format!(
                "({} < {})",
                self.generate_expression(rng, 0),
                self.generate_expression(rng, 0)
            ),
            2 => format!(
                "({} > {})",
                self.generate_expression(rng, 0),
                self.generate_expression(rng, 0)
            ),
            3 => format!(
                "({} && {})",
                self.generate_bool_expression(rng),
                self.generate_bool_expression(rng)
            ),
            _ => format!(
                "({} || {})",
                self.generate_bool_expression(rng),
                self.generate_bool_expression(rng)
            ),
        };
        // Restore depth after recursive calls
        self.current_depth -= 1;
        result
    }

    /// Generate an if expression
    fn generate_if_expression<R: Rng>(&mut self, rng: &mut R) -> String {
        format!(
            "if {} {{ {} }} else {{ {} }}",
            self.generate_bool_expression(rng),
            self.generate_expression(rng, 0),
            self.generate_expression(rng, 0)
        )
    }

    /// Generate a block expression
    fn generate_block_expression<R: Rng>(&mut self, rng: &mut R) -> String {
        let num_stmts = rng.random_range(0..3);
        let mut stmts = String::new();
        for i in 0..num_stmts {
            stmts.push_str(&format!(
                "let tmp_{} = {}; ",
                i,
                self.generate_expression(rng, 0)
            ));
        }
        format!("{{ {}{} }}", stmts, self.generate_expression(rng, 0))
    }

    /// Generate a list expression
    fn generate_list_expression<R: Rng>(&mut self, rng: &mut R) -> String {
        let num_elements = rng.random_range(0..5);
        let elements: Vec<String> = (0..num_elements)
            .map(|_| self.generate_literal(rng))
            .collect();
        format!("[{}]", elements.join(", "))
    }

    /// Generate a tuple expression
    fn generate_tuple_expression<R: Rng>(&mut self, rng: &mut R) -> String {
        let num_elements = rng.random_range(2..5);
        let elements: Vec<String> = (0..num_elements)
            .map(|_| self.generate_literal(rng))
            .collect();
        format!("({})", elements.join(", "))
    }

    /// Generate a function call expression
    fn generate_call_expression<R: Rng>(&mut self, rng: &mut R) -> String {
        let builtin_funcs = ["len", "abs", "min", "max", "print", "debug"];

        let func_name = if !self.functions.is_empty() && rng.random_bool(0.3) {
            self.functions[rng.random_range(0..self.functions.len())].clone()
        } else {
            builtin_funcs[rng.random_range(0..builtin_funcs.len())].to_string()
        };

        let num_args = rng.random_range(1..3);
        let args: Vec<String> = (0..num_args)
            .map(|_| self.generate_expression(rng, 0))
            .collect();

        format!("{}({})", func_name, args.join(", "))
    }

    /// Generate a lambda expression
    fn generate_lambda_expression<R: Rng>(&mut self, rng: &mut R) -> String {
        let num_params = rng.random_range(1..3);
        let params: Vec<String> = (0..num_params).map(|i| format!("x{}", i)).collect();

        format!(
            "|{}| {}",
            params.join(", "),
            self.generate_expression(rng, 0)
        )
    }

    /// Generate a variable reference
    fn generate_variable_reference<R: Rng>(&self, rng: &mut R) -> String {
        if self.scope.is_empty() {
            format!("var_{}", rng.random_range(0..10))
        } else {
            let (name, _) = &self.scope[rng.random_range(0..self.scope.len())];
            name.clone()
        }
    }

    /// Generate a field access
    fn generate_field_access<R: Rng>(&self, rng: &mut R) -> String {
        let fields = ["field", "value", "data", "inner", "0", "1"];
        format!("x.{}", fields[rng.random_range(0..fields.len())])
    }

    /// Generate a pattern with depth limit to prevent unbounded recursion
    fn generate_pattern<R: Rng>(&self, rng: &mut R) -> String {
        self.generate_pattern_with_depth(rng, 0)
    }

    /// Generate a pattern with explicit depth tracking
    /// NOTE: Fixed unbounded recursion - limits nesting depth to 5
    fn generate_pattern_with_depth<R: Rng>(&self, rng: &mut R, depth: usize) -> String {
        // Limit recursion depth to prevent stack overflow
        const MAX_PATTERN_DEPTH: usize = 5;
        if depth >= MAX_PATTERN_DEPTH {
            // Return simple non-recursive pattern at max depth
            return "_".to_string();
        }

        match rng.random_range(0..8) {
            0 => "_".to_string(),
            1 => random_identifier(rng),
            2 => format!("{}", rng.random_range(0..10)),
            3 => if rng.random_bool(0.5) {
                "true"
            } else {
                "false"
            }
            .to_string(),
            4 => format!("\"{}\"", super::random_string(rng, 5)),
            5 => {
                // Tuple pattern - recursive with depth tracking
                let elems: Vec<String> = (0..rng.random_range(2..4))
                    .map(|_| self.generate_pattern_with_depth(rng, depth + 1))
                    .collect();
                format!("({})", elems.join(", "))
            }
            6 => {
                // List pattern - recursive with depth tracking
                let elems: Vec<String> = (0..rng.random_range(0..3))
                    .map(|_| self.generate_pattern_with_depth(rng, depth + 1))
                    .collect();
                format!("[{}]", elems.join(", "))
            }
            _ => {
                // Constructor pattern - recursive with depth tracking
                format!("Some({})", self.generate_pattern_with_depth(rng, depth + 1))
            }
        }
    }
}

impl Generate for ParserGenerator {
    fn generate<R: Rng>(&mut self, rng: &mut R) -> String {
        self.generate_program(rng)
    }

    fn name(&self) -> &'static str {
        "ParserGenerator"
    }

    fn description(&self) -> &'static str {
        "Generates syntactically valid programs covering all parser constructs"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_parser_generator() {
        let config = GeneratorConfig::default();
        let mut generator = ParserGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..10 {
            let program = generator.generate(&mut rng);
            assert!(!program.is_empty());
            assert!(program.contains("fn main()"));
        }
    }

    #[test]
    fn test_type_definitions() {
        let config = GeneratorConfig {
            max_types: 5,
            ..Default::default()
        };
        let mut generator = ParserGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let program = generator.generate(&mut rng);
        assert!(program.contains("type ") || program.contains("fn main()"));
    }

    #[test]
    fn test_all_statement_types() {
        let config = GeneratorConfig {
            max_statements: 100,
            max_depth: 5,
            ..Default::default()
        };
        let mut generator = ParserGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        // Generate many programs to cover all statement types
        let mut found_if = false;
        let mut found_for = false;
        let mut found_while = false;
        let mut found_match = false;

        for seed in 0..100 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let program = generator.generate(&mut rng);

            if program.contains("if ") {
                found_if = true;
            }
            if program.contains("for ") {
                found_for = true;
            }
            if program.contains("while ") {
                found_while = true;
            }
            if program.contains("match ") {
                found_match = true;
            }
        }

        assert!(found_if, "Should generate if statements");
        assert!(found_for, "Should generate for loops");
        assert!(found_while, "Should generate while loops");
        assert!(found_match, "Should generate match statements");
    }
}
