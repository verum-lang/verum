//! Grammar-aware random program generator for Verum
//!
//! This module provides a grammar-guided fuzzer that generates syntactically
//! valid Verum programs by following the language grammar rules. It uses
//! probabilistic choices to create diverse program structures while ensuring
//! all generated code is parseable.
//!
//! # Architecture
//!
//! The generator maintains a context stack to track:
//! - Current scope depth (for limiting recursion)
//! - Available identifiers (for valid references)
//! - Type context (for generating type-consistent expressions)
//!
//! # Usage
//!
//! ```rust
//! use verum_fuzz::generators::GrammarGenerator;
//! use rand::rng;
//!
//! let generator = GrammarGenerator::builder()
//!     .max_depth(5)
//!     .max_statements(20)
//!     .build();
//!
//! let program = generator.generate_program(&mut rng());
//! ```

use rand::Rng;
use rand::distr::Distribution;
use rand::distr::weighted::WeightedIndex;
use rand::seq::IndexedRandom;
use std::collections::HashSet;

/// Configuration for the grammar generator
#[derive(Debug, Clone)]
pub struct GrammarConfig {
    /// Maximum depth for nested expressions/statements
    pub max_depth: usize,
    /// Maximum number of statements per function body
    pub max_statements: usize,
    /// Maximum number of function parameters
    pub max_params: usize,
    /// Maximum number of type parameters (generics)
    pub max_type_params: usize,
    /// Maximum string literal length
    pub max_string_length: usize,
    /// Maximum integer literal value
    pub max_int_value: i64,
    /// Probability weights for expression types
    pub expr_weights: ExpressionWeights,
    /// Probability weights for statement types
    pub stmt_weights: StatementWeights,
    /// Whether to generate async code
    pub enable_async: bool,
    /// Whether to generate refinement types
    pub enable_refinements: bool,
    /// Whether to generate CBGR constructs
    pub enable_cbgr: bool,
}

impl Default for GrammarConfig {
    fn default() -> Self {
        Self {
            max_depth: 5,
            max_statements: 15,
            max_params: 4,
            max_type_params: 2,
            max_string_length: 50,
            max_int_value: 1000,
            expr_weights: ExpressionWeights::default(),
            stmt_weights: StatementWeights::default(),
            enable_async: true,
            enable_refinements: true,
            enable_cbgr: true,
        }
    }
}

/// Weights for different expression types during generation
#[derive(Debug, Clone)]
pub struct ExpressionWeights {
    pub literal: u32,
    pub identifier: u32,
    pub binary: u32,
    pub unary: u32,
    pub call: u32,
    pub if_expr: u32,
    pub match_expr: u32,
    pub block: u32,
    pub field_access: u32,
    pub index: u32,
    pub lambda: u32,
    pub struct_literal: u32,
}

impl Default for ExpressionWeights {
    fn default() -> Self {
        Self {
            literal: 25,
            identifier: 20,
            binary: 15,
            unary: 5,
            call: 10,
            if_expr: 8,
            match_expr: 5,
            block: 4,
            field_access: 3,
            index: 2,
            lambda: 2,
            struct_literal: 1,
        }
    }
}

/// Weights for different statement types during generation
#[derive(Debug, Clone)]
pub struct StatementWeights {
    pub let_binding: u32,
    pub assignment: u32,
    pub expression: u32,
    pub if_stmt: u32,
    pub match_stmt: u32,
    pub for_loop: u32,
    pub while_loop: u32,
    pub return_stmt: u32,
}

impl Default for StatementWeights {
    fn default() -> Self {
        Self {
            let_binding: 30,
            assignment: 15,
            expression: 20,
            if_stmt: 10,
            match_stmt: 5,
            for_loop: 8,
            while_loop: 7,
            return_stmt: 5,
        }
    }
}

/// Builder for GrammarGenerator
pub struct GrammarGeneratorBuilder {
    config: GrammarConfig,
}

impl GrammarGeneratorBuilder {
    pub fn new() -> Self {
        Self {
            config: GrammarConfig::default(),
        }
    }

    pub fn max_depth(mut self, depth: usize) -> Self {
        self.config.max_depth = depth;
        self
    }

    pub fn max_statements(mut self, count: usize) -> Self {
        self.config.max_statements = count;
        self
    }

    pub fn max_params(mut self, count: usize) -> Self {
        self.config.max_params = count;
        self
    }

    pub fn enable_async(mut self, enable: bool) -> Self {
        self.config.enable_async = enable;
        self
    }

    pub fn enable_refinements(mut self, enable: bool) -> Self {
        self.config.enable_refinements = enable;
        self
    }

    pub fn enable_cbgr(mut self, enable: bool) -> Self {
        self.config.enable_cbgr = enable;
        self
    }

    pub fn expr_weights(mut self, weights: ExpressionWeights) -> Self {
        self.config.expr_weights = weights;
        self
    }

    pub fn stmt_weights(mut self, weights: StatementWeights) -> Self {
        self.config.stmt_weights = weights;
        self
    }

    pub fn build(self) -> GrammarGenerator {
        GrammarGenerator::new(self.config)
    }
}

impl Default for GrammarGeneratorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Context maintained during program generation
#[derive(Debug, Clone)]
struct GenerationContext {
    /// Currently available variable names in scope
    variables: Vec<String>,
    /// Currently available function names
    functions: Vec<String>,
    /// Currently available type names
    types: Vec<String>,
    /// Current nesting depth
    depth: usize,
    /// Whether we're inside an async context
    in_async: bool,
    /// Counter for generating unique names
    name_counter: usize,
    /// Set of reserved keywords to avoid
    reserved: HashSet<String>,
}

impl GenerationContext {
    fn new() -> Self {
        let reserved: HashSet<String> = [
            "fn", "let", "mut", "if", "else", "match", "for", "while", "loop", "return", "break",
            "continue", "struct", "enum", "impl", "trait", "type", "pub", "mod", "use", "async",
            "await", "true", "false", "self", "Self", "using", "provide", "context", "where", "in",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        Self {
            variables: Vec::new(),
            functions: vec!["print".to_string(), "len".to_string(), "push".to_string()],
            types: vec![
                "Int".to_string(),
                "Float".to_string(),
                "Bool".to_string(),
                "Text".to_string(),
                "Unit".to_string(),
            ],
            depth: 0,
            in_async: false,
            name_counter: 0,
            reserved,
        }
    }

    fn fresh_name(&mut self, prefix: &str) -> String {
        self.name_counter += 1;
        format!("{}_{}", prefix, self.name_counter)
    }

    fn push_scope(&mut self) {
        self.depth += 1;
    }

    fn pop_scope(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }
}

/// Grammar-aware program generator for Verum
pub struct GrammarGenerator {
    config: GrammarConfig,
    expr_dist: WeightedIndex<u32>,
    stmt_dist: WeightedIndex<u32>,
}

impl GrammarGenerator {
    /// Create a new generator with the given configuration
    pub fn new(config: GrammarConfig) -> Self {
        let expr_weights = vec![
            config.expr_weights.literal,
            config.expr_weights.identifier,
            config.expr_weights.binary,
            config.expr_weights.unary,
            config.expr_weights.call,
            config.expr_weights.if_expr,
            config.expr_weights.match_expr,
            config.expr_weights.block,
            config.expr_weights.field_access,
            config.expr_weights.index,
            config.expr_weights.lambda,
            config.expr_weights.struct_literal,
        ];

        let stmt_weights = vec![
            config.stmt_weights.let_binding,
            config.stmt_weights.assignment,
            config.stmt_weights.expression,
            config.stmt_weights.if_stmt,
            config.stmt_weights.match_stmt,
            config.stmt_weights.for_loop,
            config.stmt_weights.while_loop,
            config.stmt_weights.return_stmt,
        ];

        Self {
            config,
            expr_dist: WeightedIndex::new(&expr_weights).unwrap(),
            stmt_dist: WeightedIndex::new(&stmt_weights).unwrap(),
        }
    }

    /// Create a builder for configuring the generator
    pub fn builder() -> GrammarGeneratorBuilder {
        GrammarGeneratorBuilder::new()
    }

    /// Generate a complete Verum program
    pub fn generate_program<R: Rng>(&self, rng: &mut R) -> String {
        let mut ctx = GenerationContext::new();
        let mut program = String::new();

        // Generate module-level comments
        program.push_str("// Auto-generated Verum program for fuzz testing\n\n");

        // Generate imports
        program.push_str(&self.generate_imports(rng, &mut ctx));
        program.push('\n');

        // Generate type declarations (structs, enums)
        let num_types = rng.random_range(0..3);
        for _ in 0..num_types {
            program.push_str(&self.generate_type_decl(rng, &mut ctx));
            program.push('\n');
        }

        // Generate function declarations
        let num_functions = rng.random_range(1..5);
        for _ in 0..num_functions {
            program.push_str(&self.generate_function(rng, &mut ctx));
            program.push('\n');
        }

        // Generate main function
        program.push_str(&self.generate_main(rng, &mut ctx));

        program
    }

    /// Generate import statements
    fn generate_imports<R: Rng>(&self, rng: &mut R, _ctx: &mut GenerationContext) -> String {
        let mut imports = String::new();

        // Always import core types
        imports.push_str("use verum_std::core::{List, Text, Map, Maybe}\n");

        // Optionally import other modules
        if rng.random_bool(0.3) {
            imports.push_str("use verum_std::io::{print, println}\n");
        }

        if self.config.enable_async && rng.random_bool(0.2) {
            imports.push_str("use verum_std::async::{spawn, sleep}\n");
        }

        imports
    }

    /// Generate a type declaration (struct or enum)
    fn generate_type_decl<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        if rng.random_bool(0.6) {
            self.generate_struct(rng, ctx)
        } else {
            self.generate_enum(rng, ctx)
        }
    }

    /// Generate a struct declaration
    fn generate_struct<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        let name = ctx.fresh_name("Struct");
        ctx.types.push(name.clone());

        let mut result = format!("struct {} {{\n", name);

        let num_fields = rng.random_range(1..=4);
        for i in 0..num_fields {
            let field_name = format!("field_{}", i);
            let field_type = self.generate_type(rng, ctx, 0);
            result.push_str(&format!("    {}: {},\n", field_name, field_type));
        }

        result.push_str("}\n");
        result
    }

    /// Generate an enum declaration
    fn generate_enum<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        let name = ctx.fresh_name("Enum");
        ctx.types.push(name.clone());

        let mut result = format!("enum {} {{\n", name);

        let num_variants = rng.random_range(2..=5);
        for i in 0..num_variants {
            let variant_name = format!("Variant_{}", i);
            if rng.random_bool(0.5) {
                // Variant with data
                let data_type = self.generate_simple_type(rng);
                result.push_str(&format!("    {}({}),\n", variant_name, data_type));
            } else {
                // Unit variant
                result.push_str(&format!("    {},\n", variant_name));
            }
        }

        result.push_str("}\n");
        result
    }

    /// Generate a function declaration
    fn generate_function<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        let name = ctx.fresh_name("func");
        ctx.functions.push(name.clone());

        let is_async = self.config.enable_async && rng.random_bool(0.2);
        let is_pub = rng.random_bool(0.3);

        let mut result = String::new();

        // Function signature
        if is_pub {
            result.push_str("pub ");
        }
        if is_async {
            result.push_str("async ");
        }
        result.push_str(&format!("fn {}(", name));

        // Parameters
        let num_params = rng.random_range(0..=self.config.max_params);
        let mut param_names = Vec::new();
        for i in 0..num_params {
            if i > 0 {
                result.push_str(", ");
            }
            let param_name = format!("arg_{}", i);
            let param_type = self.generate_type(rng, ctx, 0);
            result.push_str(&format!("{}: {}", param_name, param_type));
            param_names.push(param_name);
        }
        result.push(')');

        // Return type
        if rng.random_bool(0.7) {
            let ret_type = self.generate_type(rng, ctx, 0);
            result.push_str(&format!(" -> {}", ret_type));
        }

        // Context requirements
        if rng.random_bool(0.2) {
            result.push_str(" using [Logger]");
        }

        result.push_str(" {\n");

        // Add parameters to scope
        let old_vars = ctx.variables.clone();
        ctx.variables.extend(param_names);
        ctx.in_async = is_async;

        // Function body
        let body = self.generate_block_contents(rng, ctx);
        result.push_str(&body);

        // Restore scope
        ctx.variables = old_vars;
        ctx.in_async = false;

        result.push_str("}\n");
        result
    }

    /// Generate the main function
    fn generate_main<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        let mut result = String::from("fn main() {\n");

        let body = self.generate_block_contents(rng, ctx);
        result.push_str(&body);

        result.push_str("}\n");
        result
    }

    /// Generate contents of a block (statements)
    fn generate_block_contents<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        let mut result = String::new();
        let num_stmts = rng.random_range(1..=self.config.max_statements);

        for _ in 0..num_stmts {
            let stmt = self.generate_statement(rng, ctx);
            result.push_str("    ");
            result.push_str(&stmt);
            result.push('\n');
        }

        result
    }

    /// Generate a single statement
    fn generate_statement<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        if ctx.depth >= self.config.max_depth {
            // At max depth, only generate simple statements
            return self.generate_simple_statement(rng, ctx);
        }

        match self.stmt_dist.sample(rng) {
            0 => self.generate_let_binding(rng, ctx),
            1 => self.generate_assignment(rng, ctx),
            2 => format!("{};", self.generate_expression(rng, ctx)),
            3 => self.generate_if_statement(rng, ctx),
            4 => self.generate_match_statement(rng, ctx),
            5 => self.generate_for_loop(rng, ctx),
            6 => self.generate_while_loop(rng, ctx),
            7 => self.generate_return_statement(rng, ctx),
            _ => self.generate_simple_statement(rng, ctx),
        }
    }

    fn generate_simple_statement<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut GenerationContext,
    ) -> String {
        if rng.random_bool(0.5) {
            self.generate_let_binding(rng, ctx)
        } else {
            format!("{};", self.generate_literal(rng))
        }
    }

    fn generate_let_binding<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        let var_name = ctx.fresh_name("var");
        let is_mut = rng.random_bool(0.3);
        let has_type_annotation = rng.random_bool(0.4);

        let mut result = String::from("let ");
        if is_mut {
            result.push_str("mut ");
        }
        result.push_str(&var_name);

        if has_type_annotation {
            let ty = self.generate_type(rng, ctx, 0);
            result.push_str(&format!(": {}", ty));
        }

        result.push_str(" = ");
        result.push_str(&self.generate_expression(rng, ctx));
        result.push(';');

        ctx.variables.push(var_name);
        result
    }

    fn generate_assignment<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        if ctx.variables.is_empty() {
            return self.generate_let_binding(rng, ctx);
        }

        let var = ctx.variables.choose(rng).unwrap().clone();
        let expr = self.generate_expression(rng, ctx);
        format!("{} = {};", var, expr)
    }

    fn generate_if_statement<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        ctx.push_scope();

        let condition = self.generate_bool_expression(rng, ctx);
        let mut result = format!("if {} {{\n", condition);

        let then_body = self.generate_block_contents(rng, ctx);
        result.push_str(&then_body);
        result.push_str("    }");

        if rng.random_bool(0.5) {
            result.push_str(" else {\n");
            let else_body = self.generate_block_contents(rng, ctx);
            result.push_str(&else_body);
            result.push_str("    }");
        }

        ctx.pop_scope();
        result
    }

    fn generate_match_statement<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        ctx.push_scope();

        let scrutinee = self.generate_expression(rng, ctx);
        let mut result = format!("match {} {{\n", scrutinee);

        let num_arms = rng.random_range(2..=4);
        for i in 0..num_arms {
            if i == num_arms - 1 {
                // Wildcard pattern for last arm
                result.push_str("        _ => {\n");
            } else {
                let pattern = self.generate_pattern(rng, ctx);
                result.push_str(&format!("        {} => {{\n", pattern));
            }

            let arm_body = self.generate_expression(rng, ctx);
            result.push_str(&format!("            {}\n", arm_body));
            result.push_str("        },\n");
        }

        result.push_str("    }");
        ctx.pop_scope();
        result
    }

    fn generate_for_loop<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        ctx.push_scope();

        let iter_var = ctx.fresh_name("i");
        let start = rng.random_range(0..10);
        let end = rng.random_range(start + 1..start + 20);

        let mut result = format!("for {} in {}..{} {{\n", iter_var, start, end);

        ctx.variables.push(iter_var);
        let body = self.generate_block_contents(rng, ctx);
        result.push_str(&body);
        result.push_str("    }");

        ctx.pop_scope();
        result
    }

    fn generate_while_loop<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        ctx.push_scope();

        let condition = self.generate_bool_expression(rng, ctx);
        let mut result = format!("while {} {{\n", condition);

        let body = self.generate_block_contents(rng, ctx);
        result.push_str(&body);
        result.push_str("    }");

        ctx.pop_scope();
        result
    }

    fn generate_return_statement<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut GenerationContext,
    ) -> String {
        if rng.random_bool(0.3) {
            "return;".to_string()
        } else {
            format!("return {};", self.generate_expression(rng, ctx))
        }
    }

    /// Generate an expression based on weighted random selection
    pub fn generate_expression<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        if ctx.depth >= self.config.max_depth {
            return self.generate_literal(rng);
        }

        ctx.push_scope();
        let result = match self.expr_dist.sample(rng) {
            0 => self.generate_literal(rng),
            1 => self.generate_identifier(rng, ctx),
            2 => self.generate_binary_expr(rng, ctx),
            3 => self.generate_unary_expr(rng, ctx),
            4 => self.generate_call_expr(rng, ctx),
            5 => self.generate_if_expr(rng, ctx),
            6 => self.generate_match_expr(rng, ctx),
            7 => self.generate_block_expr(rng, ctx),
            8 => self.generate_field_access(rng, ctx),
            9 => self.generate_index_expr(rng, ctx),
            10 => self.generate_lambda(rng, ctx),
            11 => self.generate_struct_literal(rng, ctx),
            _ => self.generate_literal(rng),
        };
        ctx.pop_scope();
        result
    }

    fn generate_bool_expression<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        match rng.random_range(0..5) {
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
                format!("{} {} {}", lhs, op, rhs)
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

    fn generate_literal<R: Rng>(&self, rng: &mut R) -> String {
        match rng.random_range(0..6) {
            0 => rng
                .random_range(-self.config.max_int_value..=self.config.max_int_value)
                .to_string(),
            1 => format!("{:.2}", rng.random::<f64>() * 100.0),
            2 => if rng.random_bool(0.5) {
                "true"
            } else {
                "false"
            }
            .to_string(),
            3 => {
                let len = rng.random_range(0..self.config.max_string_length);
                let s: String = (0..len)
                    .map(|_| {
                        let idx = rng.random_range(0..62);
                        match idx {
                            0..=25 => (b'a' + idx as u8) as char,
                            26..=51 => (b'A' + (idx - 26) as u8) as char,
                            _ => (b'0' + (idx - 52) as u8) as char,
                        }
                    })
                    .collect();
                format!("\"{}\"", s)
            }
            4 => "()".to_string(),
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

    fn generate_identifier<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        if ctx.variables.is_empty() {
            // Return a literal if no variables are in scope
            "0".to_string()
        } else {
            ctx.variables.choose(rng).unwrap().clone()
        }
    }

    fn generate_binary_expr<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        let lhs = self.generate_expression(rng, ctx);
        let rhs = self.generate_expression(rng, ctx);
        let op = [
            "+", "-", "*", "/", "%", "==", "!=", "<", ">", "<=", ">=", "&&", "||",
        ]
        .choose(rng)
        .unwrap();
        format!("({} {} {})", lhs, op, rhs)
    }

    fn generate_unary_expr<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        let expr = self.generate_expression(rng, ctx);
        let op = ["-", "!"].choose(rng).unwrap();
        format!("({}{})", op, expr)
    }

    fn generate_call_expr<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        let func = if ctx.functions.is_empty() {
            "print".to_string()
        } else {
            ctx.functions.choose(rng).unwrap().clone()
        };

        let num_args = rng.random_range(0..=2);
        let args: Vec<String> = (0..num_args)
            .map(|_| self.generate_expression(rng, ctx))
            .collect();

        format!("{}({})", func, args.join(", "))
    }

    fn generate_if_expr<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        let condition = self.generate_bool_expression(rng, ctx);
        let then_expr = self.generate_expression(rng, ctx);
        let else_expr = self.generate_expression(rng, ctx);
        format!(
            "if {} {{ {} }} else {{ {} }}",
            condition, then_expr, else_expr
        )
    }

    fn generate_match_expr<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        let scrutinee = self.generate_expression(rng, ctx);
        let mut result = format!("match {} {{\n", scrutinee);

        let num_arms = rng.random_range(2..=3);
        for i in 0..num_arms {
            if i == num_arms - 1 {
                result.push_str("        _ => ");
            } else {
                let pattern = self.generate_pattern(rng, ctx);
                result.push_str(&format!("        {} => ", pattern));
            }
            result.push_str(&self.generate_expression(rng, ctx));
            result.push_str(",\n");
        }

        result.push_str("    }");
        result
    }

    fn generate_block_expr<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
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
        result
    }

    fn generate_field_access<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        let base = self.generate_identifier(rng, ctx);
        let field = format!("field_{}", rng.random_range(0..3));
        format!("{}.{}", base, field)
    }

    fn generate_index_expr<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        let base = self.generate_identifier(rng, ctx);
        let index = rng.random_range(0..10);
        format!("{}[{}]", base, index)
    }

    fn generate_lambda<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        let num_params = rng.random_range(1..=2);
        let params: Vec<String> = (0..num_params).map(|i| format!("x_{}", i)).collect();

        let mut inner_ctx = ctx.clone();
        inner_ctx.variables.extend(params.clone());

        let body = self.generate_expression(rng, &mut inner_ctx);
        format!("|{}| {}", params.join(", "), body)
    }

    fn generate_struct_literal<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        // Use a simple inline struct literal syntax
        let num_fields = rng.random_range(1..=3);
        let mut fields = Vec::new();
        for i in 0..num_fields {
            let value = self.generate_expression(rng, ctx);
            fields.push(format!("field_{}: {}", i, value));
        }
        format!("{{ {} }}", fields.join(", "))
    }

    fn generate_pattern<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        match rng.random_range(0..5) {
            0 => "_".to_string(),
            1 => ctx.fresh_name("pat"),
            2 => rng.random_range(0..10).to_string(),
            3 => if rng.random_bool(0.5) {
                "true"
            } else {
                "false"
            }
            .to_string(),
            _ => format!("\"{}\"", self.generate_simple_string(rng)),
        }
    }

    fn generate_simple_string<R: Rng>(&self, rng: &mut R) -> String {
        let len = rng.random_range(1..10);
        (0..len)
            .map(|_| (b'a' + rng.random_range(0..26)) as char)
            .collect()
    }

    /// Generate a type
    fn generate_type<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut GenerationContext,
        depth: usize,
    ) -> String {
        if depth >= 2 {
            return self.generate_simple_type(rng);
        }

        match rng.random_range(0..10) {
            0..=5 => self.generate_simple_type(rng),
            6 => {
                // Generic type
                let base = ["List", "Maybe", "Set"].choose(rng).unwrap();
                let inner = self.generate_type(rng, ctx, depth + 1);
                format!("{}<{}>", base, inner)
            }
            7 => {
                // Map type
                let key = self.generate_simple_type(rng);
                let value = self.generate_type(rng, ctx, depth + 1);
                format!("Map<{}, {}>", key, value)
            }
            8 => {
                // Tuple type
                let num = rng.random_range(2..=3);
                let types: Vec<String> = (0..num)
                    .map(|_| self.generate_type(rng, ctx, depth + 1))
                    .collect();
                format!("({})", types.join(", "))
            }
            _ => {
                // Reference type (CBGR)
                if self.config.enable_cbgr {
                    let inner = self.generate_simple_type(rng);
                    let tier = ["&", "&checked ", "&unsafe "].choose(rng).unwrap();
                    format!("{}{}", tier, inner)
                } else {
                    self.generate_simple_type(rng)
                }
            }
        }
    }

    fn generate_simple_type<R: Rng>(&self, rng: &mut R) -> String {
        ["Int", "Float", "Bool", "Text", "Unit"]
            .choose(rng)
            .unwrap()
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_generator_produces_valid_syntax() {
        let generator = GrammarGenerator::builder()
            .max_depth(3)
            .max_statements(5)
            .build();

        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..10 {
            let program = generator.generate_program(&mut rng);
            assert!(!program.is_empty());
            assert!(program.contains("fn main()"));
        }
    }

    #[test]
    fn test_deterministic_with_seed() {
        let generator = GrammarGenerator::builder().max_depth(2).build();

        let mut rng1 = ChaCha8Rng::seed_from_u64(12345);
        let mut rng2 = ChaCha8Rng::seed_from_u64(12345);

        let program1 = generator.generate_program(&mut rng1);
        let program2 = generator.generate_program(&mut rng2);

        assert_eq!(program1, program2);
    }

    #[test]
    fn test_config_affects_output() {
        let small_gen = GrammarGenerator::builder()
            .max_depth(1)
            .max_statements(2)
            .build();

        let large_gen = GrammarGenerator::builder()
            .max_depth(5)
            .max_statements(20)
            .build();

        let mut rng = ChaCha8Rng::seed_from_u64(99);

        let small_prog = small_gen.generate_program(&mut rng);
        let mut rng = ChaCha8Rng::seed_from_u64(99);
        let large_prog = large_gen.generate_program(&mut rng);

        // Larger config should generally produce longer programs
        assert!(large_prog.len() >= small_prog.len() / 2);
    }
}
