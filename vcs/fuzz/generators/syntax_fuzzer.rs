//! Syntax-aware fuzzer for Verum
//!
//! This module generates random syntactically valid Verum programs by following
//! the language grammar from `grammar/verum.ebnf`. Unlike the grammar_generator
//! which focuses on generating meaningful programs, this fuzzer generates programs
//! that stress-test the parser's edge cases and boundary conditions.
//!
//! # Design Philosophy
//!
//! The syntax fuzzer generates programs that are syntactically valid but may not
//! be semantically correct. This is intentional - we want to test:
//!
//! - Parser robustness with unusual but valid constructs
//! - Edge cases in grammar rules
//! - Operator precedence and associativity
//! - Deeply nested structures
//! - Unusual identifier patterns
//! - Unicode handling
//!
//! # Grammar Coverage
//!
//! The fuzzer aims to cover all productions in the EBNF grammar:
//! - Expressions (binary, unary, postfix, primary)
//! - Statements (let, if, match, for, while, loop)
//! - Declarations (fn, type, impl, context)
//! - Types (primitive, generic, reference, function)
//! - Patterns (literal, identifier, tuple, struct, variant)

use rand::Rng;
use rand::seq::IndexedRandom;
use std::collections::HashSet;

/// Configuration for the syntax fuzzer
#[derive(Debug, Clone)]
pub struct SyntaxFuzzerConfig {
    /// Maximum nesting depth for expressions/statements
    pub max_depth: usize,
    /// Maximum number of statements in a block
    pub max_block_size: usize,
    /// Maximum number of function parameters
    pub max_params: usize,
    /// Maximum length of generated identifiers
    pub max_identifier_length: usize,
    /// Enable Unicode identifiers
    pub unicode_identifiers: bool,
    /// Enable all expression types
    pub enable_all_expressions: bool,
    /// Enable edge case literals (boundary values, special floats)
    pub enable_edge_literals: bool,
    /// Probability of generating complex constructs
    pub complexity_bias: f64,
    /// Enable async constructs
    pub enable_async: bool,
    /// Enable unsafe constructs
    pub enable_unsafe: bool,
    /// Enable CBGR reference types
    pub enable_cbgr: bool,
    /// Enable refinement types
    pub enable_refinements: bool,
}

impl Default for SyntaxFuzzerConfig {
    fn default() -> Self {
        Self {
            max_depth: 10,
            max_block_size: 20,
            max_params: 8,
            max_identifier_length: 32,
            unicode_identifiers: true,
            enable_all_expressions: true,
            enable_edge_literals: true,
            complexity_bias: 0.5,
            enable_async: true,
            enable_unsafe: true,
            enable_cbgr: true,
            enable_refinements: true,
        }
    }
}

/// Context for tracking generation state
#[derive(Debug, Clone)]
struct GenContext {
    depth: usize,
    in_function: bool,
    in_async: bool,
    in_loop: bool,
    in_unsafe: bool,
    variables: Vec<String>,
    functions: Vec<String>,
    types: Vec<String>,
    name_counter: usize,
}

impl GenContext {
    fn new() -> Self {
        Self {
            depth: 0,
            in_function: false,
            in_async: false,
            in_loop: false,
            in_unsafe: false,
            variables: vec!["x".to_string(), "y".to_string(), "z".to_string()],
            functions: vec!["main".to_string()],
            types: vec![
                "Int".to_string(),
                "Float".to_string(),
                "Bool".to_string(),
                "Text".to_string(),
            ],
            name_counter: 0,
        }
    }

    fn fresh_name(&mut self, prefix: &str) -> String {
        self.name_counter += 1;
        format!("{}_{}", prefix, self.name_counter)
    }

    fn enter_scope(&mut self) {
        self.depth += 1;
    }

    fn exit_scope(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn at_max_depth(&self, config: &SyntaxFuzzerConfig) -> bool {
        self.depth >= config.max_depth
    }
}

/// Syntax-aware random program generator
pub struct SyntaxFuzzer {
    config: SyntaxFuzzerConfig,
    reserved_keywords: HashSet<String>,
}

impl SyntaxFuzzer {
    /// Create a new syntax fuzzer with the given configuration
    pub fn new(config: SyntaxFuzzerConfig) -> Self {
        let reserved_keywords: HashSet<String> = [
            "fn",
            "let",
            "is",
            "type",
            "where",
            "using",
            "if",
            "else",
            "match",
            "return",
            "for",
            "while",
            "loop",
            "break",
            "continue",
            "async",
            "await",
            "spawn",
            "defer",
            "try",
            "yield",
            "pub",
            "mut",
            "const",
            "unsafe",
            "ffi",
            "module",
            "import",
            "implement",
            "context",
            "protocol",
            "extends",
            "self",
            "super",
            "crate",
            "static",
            "meta",
            "provide",
            "finally",
            "recover",
            "invariant",
            "decreases",
            "stream",
            "tensor",
            "affine",
            "linear",
            "public",
            "internal",
            "protected",
            "ensures",
            "requires",
            "result",
            "true",
            "false",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        Self {
            config,
            reserved_keywords,
        }
    }

    /// Generate a complete Verum program
    pub fn generate_program<R: Rng>(&self, rng: &mut R) -> String {
        let mut ctx = GenContext::new();
        let mut output = String::new();

        // Header comment
        output.push_str("// Syntax fuzzer generated program\n\n");

        // Optionally generate imports
        if rng.random_bool(0.5) {
            output.push_str(&self.generate_imports(rng));
            output.push('\n');
        }

        // Generate type definitions
        let num_types = rng.random_range(0..3);
        for _ in 0..num_types {
            output.push_str(&self.generate_type_def(rng, &mut ctx));
            output.push('\n');
        }

        // Generate functions
        let num_functions = rng.random_range(1..5);
        for _ in 0..num_functions {
            output.push_str(&self.generate_function(rng, &mut ctx));
            output.push('\n');
        }

        // Generate main function
        output.push_str(&self.generate_main(rng, &mut ctx));

        output
    }

    /// Generate import statements
    fn generate_imports<R: Rng>(&self, rng: &mut R) -> String {
        let mut imports = String::new();

        let import_options = [
            "import verum_core.base.{List, Text, Map, Maybe};",
            "import verum_std.io.{print, println};",
            "import verum_std.collections.{Set, Heap};",
        ];

        let num_imports = rng.random_range(1..=import_options.len());
        for import in import_options.choose_multiple(rng, num_imports) {
            imports.push_str(import);
            imports.push('\n');
        }

        imports
    }

    /// Generate a type definition using Verum `type ... is` syntax
    fn generate_type_def<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        let name = ctx.fresh_name("Type");
        ctx.types.push(name.clone());

        match rng.random_range(0..5) {
            0 => {
                // Record type: type Name is { field: Type, ... };
                let mut result = format!("type {} is {{\n", name);
                let num_fields = rng.random_range(1..=4);
                for i in 0..num_fields {
                    let field_type = self.generate_type_expr(rng, ctx);
                    result.push_str(&format!("    field_{}: {},\n", i, field_type));
                }
                result.push_str("};\n");
                result
            }
            1 => {
                // Sum type: type Name is A | B(T) | C { x: T };
                let mut result = format!("type {} is\n", name);
                let num_variants = rng.random_range(2..=4);
                for i in 0..num_variants {
                    if i > 0 {
                        result.push_str("    | ");
                    } else {
                        result.push_str("    ");
                    }
                    result.push_str(&format!("Variant_{}", i));

                    match rng.random_range(0..3) {
                        0 => {} // Unit variant
                        1 => {
                            // Tuple variant
                            let ty = self.generate_simple_type(rng);
                            result.push_str(&format!("({})", ty));
                        }
                        _ => {
                            // Record variant
                            result.push_str("{ value: Int }");
                        }
                    }
                    result.push('\n');
                }
                result.push_str(";\n");
                result
            }
            2 => {
                // Newtype: type Name is (T);
                let inner = self.generate_simple_type(rng);
                format!("type {} is ({});\n", name, inner)
            }
            3 => {
                // Unit type: type Name is ();
                format!("type {} is ();\n", name)
            }
            _ => {
                // Generic type
                let inner = self.generate_simple_type(rng);
                format!("type {}<T> is {{ inner: T, extra: {} }};\n", name, inner)
            }
        }
    }

    /// Generate a function definition
    fn generate_function<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        let name = ctx.fresh_name("func");
        ctx.functions.push(name.clone());

        let mut result = String::new();

        // Visibility
        if rng.random_bool(0.3) {
            result.push_str("pub ");
        }

        // Async modifier
        let is_async = self.config.enable_async && rng.random_bool(0.2);
        if is_async {
            result.push_str("async ");
        }

        // Unsafe modifier
        let is_unsafe = self.config.enable_unsafe && rng.random_bool(0.1);
        if is_unsafe {
            result.push_str("unsafe ");
        }

        result.push_str(&format!("fn {}(", name));

        // Parameters
        let num_params = rng.random_range(0..=self.config.max_params.min(4));
        for i in 0..num_params {
            if i > 0 {
                result.push_str(", ");
            }
            let param_name = format!("arg_{}", i);
            let param_type = self.generate_type_expr(rng, ctx);
            result.push_str(&format!("{}: {}", param_name, param_type));
        }

        result.push(')');

        // Return type
        if rng.random_bool(0.7) {
            let ret_type = self.generate_type_expr(rng, ctx);
            result.push_str(&format!(" -> {}", ret_type));
        }

        // Context clause
        if rng.random_bool(0.2) {
            result.push_str(" using [Logger]");
        }

        result.push_str(" {\n");

        // Function body
        ctx.in_function = true;
        ctx.in_async = is_async;
        ctx.in_unsafe = is_unsafe;

        let body = self.generate_block_contents(rng, ctx);
        result.push_str(&body);

        ctx.in_function = false;
        ctx.in_async = false;
        ctx.in_unsafe = false;

        result.push_str("}\n");
        result
    }

    /// Generate the main function
    fn generate_main<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        let mut result = String::from("fn main() {\n");

        ctx.in_function = true;
        let body = self.generate_block_contents(rng, ctx);
        result.push_str(&body);
        ctx.in_function = false;

        result.push_str("}\n");
        result
    }

    /// Generate block contents (statements)
    fn generate_block_contents<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        let mut result = String::new();
        let num_stmts = rng.random_range(1..=self.config.max_block_size.min(10));

        for _ in 0..num_stmts {
            let stmt = self.generate_statement(rng, ctx);
            result.push_str("    ");
            result.push_str(&stmt);
            result.push('\n');
        }

        result
    }

    /// Generate a statement
    fn generate_statement<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        if ctx.at_max_depth(&self.config) {
            return self.generate_simple_statement(rng, ctx);
        }

        match rng.random_range(0..12) {
            0 | 1 | 2 => self.generate_let_statement(rng, ctx),
            3 => self.generate_assignment(rng, ctx),
            4 => self.generate_if_statement(rng, ctx),
            5 => self.generate_match_statement(rng, ctx),
            6 => self.generate_for_loop(rng, ctx),
            7 => self.generate_while_loop(rng, ctx),
            8 => self.generate_loop_statement(rng, ctx),
            9 => self.generate_return_statement(rng, ctx),
            10 => self.generate_defer_statement(rng, ctx),
            _ => format!("{};", self.generate_expression(rng, ctx)),
        }
    }

    /// Generate a simple statement (for max depth)
    fn generate_simple_statement<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        if rng.random_bool(0.5) {
            self.generate_let_statement(rng, ctx)
        } else {
            format!("{};", self.generate_literal(rng))
        }
    }

    /// Generate a let statement
    fn generate_let_statement<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        let var_name = ctx.fresh_name("v");
        let is_mut = rng.random_bool(0.3);

        let mut result = String::from("let ");
        if is_mut {
            result.push_str("mut ");
        }
        result.push_str(&var_name);

        // Optional type annotation
        if rng.random_bool(0.4) {
            let ty = self.generate_type_expr(rng, ctx);
            result.push_str(&format!(": {}", ty));
        }

        result.push_str(" = ");
        result.push_str(&self.generate_expression(rng, ctx));
        result.push(';');

        ctx.variables.push(var_name);
        result
    }

    /// Generate an assignment statement
    fn generate_assignment<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        if ctx.variables.is_empty() {
            return self.generate_let_statement(rng, ctx);
        }

        let var = ctx.variables.choose(rng).unwrap().clone();
        let expr = self.generate_expression(rng, ctx);

        let op = ["=", "+=", "-=", "*=", "/="].choose(rng).unwrap();
        format!("{} {} {};", var, op, expr)
    }

    /// Generate an if statement
    fn generate_if_statement<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        ctx.enter_scope();

        let condition = self.generate_bool_expression(rng, ctx);
        let mut result = format!("if {} {{\n", condition);

        let body = self.generate_block_contents(rng, ctx);
        result.push_str(&body);
        result.push_str("    }");

        // Optional else branch
        if rng.random_bool(0.5) {
            if rng.random_bool(0.3) {
                // else if
                result.push_str(" else ");
                result.push_str(&self.generate_if_statement(rng, ctx));
            } else {
                result.push_str(" else {\n");
                let else_body = self.generate_block_contents(rng, ctx);
                result.push_str(&else_body);
                result.push_str("    }");
            }
        }

        ctx.exit_scope();
        result
    }

    /// Generate a match statement
    fn generate_match_statement<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        ctx.enter_scope();

        let scrutinee = self.generate_expression(rng, ctx);
        let mut result = format!("match {} {{\n", scrutinee);

        let num_arms = rng.random_range(2..=5);
        for i in 0..num_arms {
            result.push_str("        ");
            if i == num_arms - 1 {
                result.push_str("_ => ");
            } else {
                let pattern = self.generate_pattern(rng, ctx);
                result.push_str(&format!("{} => ", pattern));
            }
            result.push_str(&self.generate_expression(rng, ctx));
            result.push_str(",\n");
        }

        result.push_str("    }");
        ctx.exit_scope();
        result
    }

    /// Generate a for loop
    fn generate_for_loop<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        ctx.enter_scope();
        ctx.in_loop = true;

        let iter_var = ctx.fresh_name("i");
        let start = rng.random_range(0..10);
        let end = rng.random_range(start + 1..start + 20);

        let mut result = format!("for {} in {}..{} {{\n", iter_var, start, end);

        ctx.variables.push(iter_var);
        let body = self.generate_block_contents(rng, ctx);
        result.push_str(&body);
        result.push_str("    }");

        ctx.in_loop = false;
        ctx.exit_scope();
        result
    }

    /// Generate a while loop
    fn generate_while_loop<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        ctx.enter_scope();
        ctx.in_loop = true;

        let condition = self.generate_bool_expression(rng, ctx);
        let mut result = format!("while {} {{\n", condition);

        let body = self.generate_block_contents(rng, ctx);
        result.push_str(&body);
        result.push_str("    }");

        ctx.in_loop = false;
        ctx.exit_scope();
        result
    }

    /// Generate a loop statement
    fn generate_loop_statement<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        ctx.enter_scope();
        ctx.in_loop = true;

        let mut result = String::from("loop {\n");

        let body = self.generate_block_contents(rng, ctx);
        result.push_str(&body);

        // Always add a break to prevent infinite loops
        result.push_str("        break;\n");
        result.push_str("    }");

        ctx.in_loop = false;
        ctx.exit_scope();
        result
    }

    /// Generate a return statement
    fn generate_return_statement<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        if rng.random_bool(0.3) {
            "return;".to_string()
        } else {
            format!("return {};", self.generate_expression(rng, ctx))
        }
    }

    /// Generate a defer statement
    fn generate_defer_statement<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        if rng.random_bool(0.5) {
            format!("defer {};", self.generate_expression(rng, ctx))
        } else {
            format!("defer {{\n{}    }}", self.generate_block_contents(rng, ctx))
        }
    }

    /// Generate an expression
    fn generate_expression<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        if ctx.at_max_depth(&self.config) {
            return self.generate_literal(rng);
        }

        ctx.enter_scope();
        let result = match rng.random_range(0..15) {
            0 | 1 | 2 => self.generate_literal(rng),
            3 | 4 => self.generate_identifier(rng, ctx),
            5 | 6 => self.generate_binary_expr(rng, ctx),
            7 => self.generate_unary_expr(rng, ctx),
            8 => self.generate_call_expr(rng, ctx),
            9 => self.generate_if_expr(rng, ctx),
            10 => self.generate_block_expr(rng, ctx),
            11 => self.generate_tuple_expr(rng, ctx),
            12 => self.generate_list_expr(rng, ctx),
            13 => self.generate_lambda_expr(rng, ctx),
            _ => self.generate_field_access(rng, ctx),
        };
        ctx.exit_scope();
        result
    }

    /// Generate a boolean expression
    fn generate_bool_expression<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        match rng.random_range(0..6) {
            0 => if rng.random_bool(0.5) {
                "true"
            } else {
                "false"
            }
            .to_string(),
            1 => {
                let lhs = self.generate_expression(rng, ctx);
                let rhs = self.generate_expression(rng, ctx);
                let op = ["==", "!=", "<", ">", "<=", ">="].choose(rng).unwrap();
                format!("({} {} {})", lhs, op, rhs)
            }
            2 => {
                let lhs = self.generate_bool_expression(rng, ctx);
                let rhs = self.generate_bool_expression(rng, ctx);
                let op = ["&&", "||"].choose(rng).unwrap();
                format!("({} {} {})", lhs, op, rhs)
            }
            3 => format!("!{}", self.generate_bool_expression(rng, ctx)),
            _ => {
                if !ctx.variables.is_empty() {
                    ctx.variables.choose(rng).unwrap().clone()
                } else {
                    "true".to_string()
                }
            }
        }
    }

    /// Generate a literal
    fn generate_literal<R: Rng>(&self, rng: &mut R) -> String {
        if self.config.enable_edge_literals && rng.random_bool(0.2) {
            return self.generate_edge_literal(rng);
        }

        match rng.random_range(0..7) {
            0 => rng.random_range(-1000i64..=1000).to_string(),
            1 => format!("{:.4}", rng.random::<f64>() * 100.0),
            2 => if rng.random_bool(0.5) {
                "true"
            } else {
                "false"
            }
            .to_string(),
            3 => {
                let len = rng.random_range(0..20);
                let s: String = (0..len)
                    .map(|_| (b'a' + rng.random_range(0..26)) as char)
                    .collect();
                format!("\"{}\"", s)
            }
            4 => {
                let c = (b'a' + rng.random_range(0..26)) as char;
                format!("'{}'", c)
            }
            5 => "()".to_string(),
            _ => {
                // List literal
                let len = rng.random_range(0..5);
                let elements: Vec<String> = (0..len)
                    .map(|_| rng.random_range(0..100).to_string())
                    .collect();
                format!("[{}]", elements.join(", "))
            }
        }
    }

    /// Generate edge case literals
    fn generate_edge_literal<R: Rng>(&self, rng: &mut R) -> String {
        let edge_cases = [
            "0",
            "-0",
            "1",
            "-1",
            "127",
            "128",
            "-128",
            "255",
            "256",
            "32767",
            "32768",
            "-32768",
            "65535",
            "65536",
            "2147483647",
            "2147483648",
            "-2147483648",
            "9223372036854775807",
            "0.0",
            "-0.0",
            "1e10",
            "1e-10",
            "\"\"",
            "\"\\n\"",
            "\"\\t\"",
            "\"\\0\"",
            "'\\n'",
            "'\\t'",
            "[]",
            "[0]",
        ];
        edge_cases.choose(rng).unwrap().to_string()
    }

    /// Generate an identifier
    fn generate_identifier<R: Rng>(&self, rng: &mut R, ctx: &GenContext) -> String {
        if ctx.variables.is_empty() || rng.random_bool(0.2) {
            "0".to_string()
        } else {
            ctx.variables.choose(rng).unwrap().clone()
        }
    }

    /// Generate a binary expression
    fn generate_binary_expr<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        let lhs = self.generate_expression(rng, ctx);
        let rhs = self.generate_expression(rng, ctx);
        let op = [
            "+", "-", "*", "/", "%", "==", "!=", "<", ">", "<=", ">=", "&&", "||", "&", "|", "^",
            "<<", ">>",
        ]
        .choose(rng)
        .unwrap();
        format!("({} {} {})", lhs, op, rhs)
    }

    /// Generate a unary expression
    fn generate_unary_expr<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        let expr = self.generate_expression(rng, ctx);
        let op = ["-", "!", "~"].choose(rng).unwrap();
        format!("({}{})", op, expr)
    }

    /// Generate a function call expression
    fn generate_call_expr<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        let func = if ctx.functions.is_empty() {
            "print".to_string()
        } else {
            ctx.functions.choose(rng).unwrap().clone()
        };

        let num_args = rng.random_range(0..=3);
        let args: Vec<String> = (0..num_args)
            .map(|_| self.generate_expression(rng, ctx))
            .collect();

        format!("{}({})", func, args.join(", "))
    }

    /// Generate an if expression
    fn generate_if_expr<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        let condition = self.generate_bool_expression(rng, ctx);
        let then_expr = self.generate_expression(rng, ctx);
        let else_expr = self.generate_expression(rng, ctx);
        format!(
            "if {} {{ {} }} else {{ {} }}",
            condition, then_expr, else_expr
        )
    }

    /// Generate a block expression
    fn generate_block_expr<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        ctx.enter_scope();
        let mut result = String::from("{\n");

        let num_stmts = rng.random_range(0..3);
        for _ in 0..num_stmts {
            result.push_str("        ");
            result.push_str(&self.generate_statement(rng, ctx));
            result.push('\n');
        }

        result.push_str("        ");
        result.push_str(&self.generate_expression(rng, ctx));
        result.push_str("\n    }");

        ctx.exit_scope();
        result
    }

    /// Generate a tuple expression
    fn generate_tuple_expr<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        let num_elements = rng.random_range(2..=4);
        let elements: Vec<String> = (0..num_elements)
            .map(|_| self.generate_expression(rng, ctx))
            .collect();
        format!("({})", elements.join(", "))
    }

    /// Generate a list expression
    fn generate_list_expr<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        let num_elements = rng.random_range(0..=5);
        let elements: Vec<String> = (0..num_elements)
            .map(|_| self.generate_expression(rng, ctx))
            .collect();
        format!("[{}]", elements.join(", "))
    }

    /// Generate a lambda expression
    fn generate_lambda_expr<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        let num_params = rng.random_range(1..=3);
        let params: Vec<String> = (0..num_params).map(|i| format!("p_{}", i)).collect();

        let body = self.generate_expression(rng, ctx);
        format!("|{}| {}", params.join(", "), body)
    }

    /// Generate a field access expression
    fn generate_field_access<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        let base = self.generate_identifier(rng, ctx);
        let field = format!("field_{}", rng.random_range(0..3));
        format!("{}.{}", base, field)
    }

    /// Generate a pattern
    fn generate_pattern<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        match rng.random_range(0..8) {
            0 => "_".to_string(),
            1 => ctx.fresh_name("pat"),
            2 => rng.random_range(0..100).to_string(),
            3 => if rng.random_bool(0.5) {
                "true"
            } else {
                "false"
            }
            .to_string(),
            4 => format!("\"{}\"", self.generate_simple_string(rng)),
            5 => {
                // Tuple pattern
                let num = rng.random_range(2..=3);
                let pats: Vec<String> = (0..num).map(|_| self.generate_pattern(rng, ctx)).collect();
                format!("({})", pats.join(", "))
            }
            6 => {
                // Range pattern
                let start = rng.random_range(0..10);
                let end = rng.random_range(start..start + 10);
                format!("{}..{}", start, end)
            }
            _ => {
                // Or pattern
                let p1 = ctx.fresh_name("a");
                let p2 = ctx.fresh_name("b");
                format!("{} | {}", p1, p2)
            }
        }
    }

    /// Generate a type expression
    fn generate_type_expr<R: Rng>(&self, rng: &mut R, ctx: &mut GenContext) -> String {
        if ctx.at_max_depth(&self.config) {
            return self.generate_simple_type(rng);
        }

        match rng.random_range(0..10) {
            0..=4 => self.generate_simple_type(rng),
            5 => {
                // Generic type
                let base = ["List", "Maybe", "Set", "Heap"].choose(rng).unwrap();
                let inner = self.generate_simple_type(rng);
                format!("{}<{}>", base, inner)
            }
            6 => {
                // Map type
                let key = self.generate_simple_type(rng);
                let value = self.generate_simple_type(rng);
                format!("Map<{}, {}>", key, value)
            }
            7 => {
                // Tuple type
                let num = rng.random_range(2..=3);
                let types: Vec<String> = (0..num).map(|_| self.generate_simple_type(rng)).collect();
                format!("({})", types.join(", "))
            }
            8 => {
                // Reference type (CBGR)
                if self.config.enable_cbgr {
                    let inner = self.generate_simple_type(rng);
                    let tier = ["&", "&checked ", "&unsafe "].choose(rng).unwrap();
                    let mutability = if rng.random_bool(0.3) { "mut " } else { "" };
                    format!("{}{}{}", tier, mutability, inner)
                } else {
                    self.generate_simple_type(rng)
                }
            }
            _ => {
                // Function type
                let params: Vec<String> = (0..rng.random_range(0..3))
                    .map(|_| self.generate_simple_type(rng))
                    .collect();
                let ret = self.generate_simple_type(rng);
                format!("fn({}) -> {}", params.join(", "), ret)
            }
        }
    }

    /// Generate a simple type
    fn generate_simple_type<R: Rng>(&self, rng: &mut R) -> String {
        ["Int", "Float", "Bool", "Text", "()"]
            .choose(rng)
            .unwrap()
            .to_string()
    }

    /// Generate a simple string
    fn generate_simple_string<R: Rng>(&self, rng: &mut R) -> String {
        let len = rng.random_range(1..10);
        (0..len)
            .map(|_| (b'a' + rng.random_range(0..26)) as char)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_syntax_fuzzer_generates_valid_programs() {
        let config = SyntaxFuzzerConfig::default();
        let fuzzer = SyntaxFuzzer::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..10 {
            let program = fuzzer.generate_program(&mut rng);
            assert!(!program.is_empty());
            assert!(program.contains("fn main()"));
        }
    }

    #[test]
    fn test_deterministic_generation() {
        let config = SyntaxFuzzerConfig::default();
        let fuzzer = SyntaxFuzzer::new(config);

        let mut rng1 = ChaCha8Rng::seed_from_u64(12345);
        let mut rng2 = ChaCha8Rng::seed_from_u64(12345);

        let program1 = fuzzer.generate_program(&mut rng1);
        let program2 = fuzzer.generate_program(&mut rng2);

        assert_eq!(program1, program2);
    }

    #[test]
    fn test_type_definitions_use_verum_syntax() {
        let config = SyntaxFuzzerConfig::default();
        let fuzzer = SyntaxFuzzer::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..50 {
            let mut ctx = GenContext::new();
            let type_def = fuzzer.generate_type_def(&mut rng, &mut ctx);

            // Should use Verum syntax, not Rust
            assert!(
                type_def.starts_with("type "),
                "Should use 'type' keyword: {}",
                type_def
            );
            // Check for "is" keyword - allow for optional newline after "is"
            assert!(
                type_def.contains(" is ") || type_def.contains(" is\n"),
                "Should use 'is' keyword: {}",
                type_def
            );
            assert!(
                !type_def.contains("struct "),
                "Should not use Rust 'struct': {}",
                type_def
            );
            assert!(
                !type_def.contains("enum "),
                "Should not use Rust 'enum': {}",
                type_def
            );
        }
    }
}
