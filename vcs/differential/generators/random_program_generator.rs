//! Random Program Generator for Differential Testing
//!
//! This module generates syntactically valid random Verum programs for
//! differential testing between Tier 0 (interpreter) and Tier 3 (AOT).
//!
//! Features:
//! - Grammar-aware generation based on verum.ebnf
//! - Configurable complexity and size
//! - Specific characteristic generation (recursion, loops, etc.)
//! - Deterministic generation via seed
//! - Shrinking support for minimizing failing cases

use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rand::prelude::*;
use rand::rngs::StdRng;
use rand::SeedableRng;

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for program generation
#[derive(Debug, Clone)]
pub struct GeneratorConfig {
    /// Random seed for reproducibility
    pub seed: u64,
    /// Maximum expression depth
    pub max_depth: usize,
    /// Maximum number of statements
    pub max_statements: usize,
    /// Maximum number of functions
    pub max_functions: usize,
    /// Maximum loop iterations (to prevent infinite loops)
    pub max_loop_iterations: usize,
    /// Maximum recursion depth
    pub max_recursion_depth: usize,
    /// Enable floating-point operations
    pub enable_floats: bool,
    /// Enable async operations
    pub enable_async: bool,
    /// Enable generics
    pub enable_generics: bool,
    /// Enable closures
    pub enable_closures: bool,
    /// Enable collections
    pub enable_collections: bool,
    /// Enable heap allocation
    pub enable_heap: bool,
    /// Probability of choosing each feature (0.0 - 1.0)
    pub feature_probabilities: FeatureProbabilities,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            max_depth: 5,
            max_statements: 20,
            max_functions: 5,
            max_loop_iterations: 100,
            max_recursion_depth: 10,
            enable_floats: true,
            enable_async: false,
            enable_generics: false,
            enable_closures: true,
            enable_collections: true,
            enable_heap: true,
            feature_probabilities: FeatureProbabilities::default(),
        }
    }
}

/// Probability settings for various features
#[derive(Debug, Clone)]
pub struct FeatureProbabilities {
    /// Probability of using a binary operation
    pub binary_op: f64,
    /// Probability of using a unary operation
    pub unary_op: f64,
    /// Probability of creating a function call
    pub function_call: f64,
    /// Probability of creating an if expression
    pub if_expr: f64,
    /// Probability of creating a match expression
    pub match_expr: f64,
    /// Probability of creating a loop
    pub loop_expr: f64,
    /// Probability of using a variable
    pub variable: f64,
    /// Probability of creating a closure
    pub closure: f64,
    /// Probability of using a collection
    pub collection: f64,
}

impl Default for FeatureProbabilities {
    fn default() -> Self {
        Self {
            binary_op: 0.3,
            unary_op: 0.1,
            function_call: 0.15,
            if_expr: 0.15,
            match_expr: 0.1,
            loop_expr: 0.1,
            variable: 0.4,
            closure: 0.05,
            collection: 0.1,
        }
    }
}

// ============================================================================
// Types
// ============================================================================

/// Verum type representation for generation
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VrType {
    Int,
    Float,
    Bool,
    Char,
    Text,
    Unit,
    List(Box<VrType>),
    Maybe(Box<VrType>),
    Tuple(Vec<VrType>),
    Function(Vec<VrType>, Box<VrType>),
    Custom(String),
}

impl VrType {
    /// Generate type annotation string
    pub fn to_string(&self) -> String {
        match self {
            VrType::Int => "Int".to_string(),
            VrType::Float => "Float".to_string(),
            VrType::Bool => "Bool".to_string(),
            VrType::Char => "Char".to_string(),
            VrType::Text => "Text".to_string(),
            VrType::Unit => "()".to_string(),
            VrType::List(inner) => format!("List<{}>", inner.to_string()),
            VrType::Maybe(inner) => format!("Maybe<{}>", inner.to_string()),
            VrType::Tuple(types) => {
                let parts: Vec<String> = types.iter().map(|t| t.to_string()).collect();
                format!("({})", parts.join(", "))
            }
            VrType::Function(params, ret) => {
                let param_strs: Vec<String> = params.iter().map(|t| t.to_string()).collect();
                format!("fn({}) -> {}", param_strs.join(", "), ret.to_string())
            }
            VrType::Custom(name) => name.clone(),
        }
    }

    /// Check if type supports arithmetic operations
    pub fn is_numeric(&self) -> bool {
        matches!(self, VrType::Int | VrType::Float)
    }

    /// Check if type supports comparison
    pub fn is_comparable(&self) -> bool {
        matches!(self, VrType::Int | VrType::Float | VrType::Bool | VrType::Char | VrType::Text)
    }
}

/// Variable in scope
#[derive(Debug, Clone)]
pub struct Variable {
    pub name: String,
    pub ty: VrType,
    pub mutable: bool,
}

/// Function definition
#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub name: String,
    pub params: Vec<(String, VrType)>,
    pub return_type: VrType,
    pub is_recursive: bool,
}

// ============================================================================
// Generator State
// ============================================================================

/// State maintained during generation
pub struct GeneratorState {
    /// Configuration
    config: GeneratorConfig,
    /// Random number generator
    rng: StdRng,
    /// Variables in current scope
    variables: Vec<HashMap<String, Variable>>,
    /// Available functions
    functions: HashMap<String, FunctionDef>,
    /// Current expression depth
    depth: usize,
    /// Counter for unique names
    name_counter: usize,
    /// Output buffer
    output: String,
    /// Indent level
    indent: usize,
}

impl GeneratorState {
    /// Create new generator state
    pub fn new(config: GeneratorConfig) -> Self {
        let rng = StdRng::seed_from_u64(config.seed);
        Self {
            config,
            rng,
            variables: vec![HashMap::new()],
            functions: HashMap::new(),
            depth: 0,
            name_counter: 0,
            output: String::new(),
            indent: 0,
        }
    }

    /// Generate a unique name
    fn unique_name(&mut self, prefix: &str) -> String {
        self.name_counter += 1;
        format!("{}_{}", prefix, self.name_counter)
    }

    /// Push a new scope
    fn push_scope(&mut self) {
        self.variables.push(HashMap::new());
    }

    /// Pop a scope
    fn pop_scope(&mut self) {
        self.variables.pop();
    }

    /// Add variable to current scope
    fn add_variable(&mut self, var: Variable) {
        if let Some(scope) = self.variables.last_mut() {
            scope.insert(var.name.clone(), var);
        }
    }

    /// Get all visible variables
    fn visible_variables(&self) -> Vec<&Variable> {
        self.variables
            .iter()
            .flat_map(|scope| scope.values())
            .collect()
    }

    /// Get variables of a specific type
    fn variables_of_type(&self, ty: &VrType) -> Vec<&Variable> {
        self.visible_variables()
            .into_iter()
            .filter(|v| &v.ty == ty)
            .collect()
    }

    /// Write to output with current indentation
    fn write(&mut self, s: &str) {
        self.output.push_str(s);
    }

    /// Write a line with current indentation
    fn writeln(&mut self, s: &str) {
        let indent_str = "    ".repeat(self.indent);
        self.output.push_str(&indent_str);
        self.output.push_str(s);
        self.output.push('\n');
    }

    /// Increment indent
    fn indent(&mut self) {
        self.indent += 1;
    }

    /// Decrement indent
    fn dedent(&mut self) {
        if self.indent > 0 {
            self.indent -= 1;
        }
    }

    /// Check if we should stop going deeper
    fn should_stop(&self) -> bool {
        self.depth >= self.config.max_depth
    }

    /// Random bool with probability
    fn chance(&mut self, probability: f64) -> bool {
        self.rng.gen_bool(probability)
    }

    /// Random choice from slice
    fn choose<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            None
        } else {
            Some(&items[self.rng.gen_range(0..items.len())])
        }
    }
}

// ============================================================================
// Random Program Generator
// ============================================================================

/// Main program generator
pub struct RandomProgramGenerator {
    state: GeneratorState,
}

impl RandomProgramGenerator {
    /// Create a new generator with configuration
    pub fn new(config: GeneratorConfig) -> Self {
        Self {
            state: GeneratorState::new(config),
        }
    }

    /// Create with default configuration and given seed
    pub fn with_seed(seed: u64) -> Self {
        let mut config = GeneratorConfig::default();
        config.seed = seed;
        Self::new(config)
    }

    /// Generate a complete random program
    pub fn generate(&mut self) -> String {
        self.state.output.clear();

        // Write header
        self.write_header();

        // Generate helper functions
        let num_functions = self.state.rng.gen_range(0..=self.state.config.max_functions);
        for _ in 0..num_functions {
            self.generate_function();
            self.state.writeln("");
        }

        // Generate main function
        self.generate_main();

        self.state.output.clone()
    }

    /// Write program header with annotations
    fn write_header(&mut self) {
        self.state.writeln("// @test: differential");
        self.state.writeln("// @tier: 0, 3");
        self.state.writeln("// @level: L1");
        self.state
            .writeln(&format!("// @tags: generated, seed_{}", self.state.config.seed));
        self.state.writeln(&format!(
            "// Generated by RandomProgramGenerator, seed: {}",
            self.state.config.seed
        ));
        self.state.writeln("");
    }

    /// Generate a random function
    fn generate_function(&mut self) {
        let name = self.state.unique_name("fn");
        let num_params = self.state.rng.gen_range(0..=3);
        let mut params = Vec::new();

        for i in 0..num_params {
            let param_name = format!("p{}", i);
            let param_type = self.random_simple_type();
            params.push((param_name, param_type));
        }

        let return_type = self.random_simple_type();
        let is_recursive = self.state.chance(0.2);

        // Register function before generating body (for recursion)
        self.state.functions.insert(
            name.clone(),
            FunctionDef {
                name: name.clone(),
                params: params.clone(),
                return_type: return_type.clone(),
                is_recursive,
            },
        );

        // Write function signature
        let params_str: Vec<String> = params
            .iter()
            .map(|(n, t)| format!("{}: {}", n, t.to_string()))
            .collect();
        self.state.writeln(&format!(
            "fn {}({}) -> {} {{",
            name,
            params_str.join(", "),
            return_type.to_string()
        ));

        self.state.indent();
        self.state.push_scope();

        // Add parameters to scope
        for (param_name, param_type) in &params {
            self.state.add_variable(Variable {
                name: param_name.clone(),
                ty: param_type.clone(),
                mutable: false,
            });
        }

        // Generate body
        let num_statements = self.state.rng.gen_range(1..=5);
        for _ in 0..num_statements {
            self.generate_statement();
        }

        // Generate return expression
        let return_expr = self.generate_expression(&return_type);
        self.state.writeln(&return_expr);

        self.state.pop_scope();
        self.state.dedent();
        self.state.writeln("}");
    }

    /// Generate main function
    fn generate_main(&mut self) {
        self.state.writeln("fn main() {");
        self.state.indent();
        self.state.push_scope();

        // Generate statements
        let num_statements = self
            .state
            .rng
            .gen_range(3..=self.state.config.max_statements);
        for _ in 0..num_statements {
            self.generate_statement();
        }

        // Print some results
        let visible = self.state.visible_variables();
        let printable: Vec<&Variable> = visible
            .iter()
            .filter(|v| v.ty.is_comparable())
            .take(5)
            .copied()
            .collect();

        for var in printable {
            self.state
                .writeln(&format!("println(f\"{} = {{{}}}\");", var.name, var.name));
        }

        self.state.pop_scope();
        self.state.dedent();
        self.state.writeln("}");
    }

    /// Generate a random statement
    fn generate_statement(&mut self) {
        let choice: u32 = self.state.rng.gen_range(0..100);

        if choice < 50 {
            // Let binding
            self.generate_let_statement();
        } else if choice < 70 {
            // If statement
            self.generate_if_statement();
        } else if choice < 85 {
            // Loop
            self.generate_loop_statement();
        } else {
            // Expression statement
            self.generate_expression_statement();
        }
    }

    /// Generate a let statement
    fn generate_let_statement(&mut self) {
        let name = self.state.unique_name("x");
        let ty = self.random_simple_type();
        let mutable = self.state.chance(0.3);

        let expr = self.generate_expression(&ty);

        let mut_keyword = if mutable { "mut " } else { "" };
        self.state
            .writeln(&format!("let {}{} = {};", mut_keyword, name, expr));

        self.state.add_variable(Variable {
            name,
            ty,
            mutable,
        });
    }

    /// Generate an if statement
    fn generate_if_statement(&mut self) {
        let cond = self.generate_expression(&VrType::Bool);

        self.state.writeln(&format!("if {} {{", cond));
        self.state.indent();
        self.state.push_scope();

        self.generate_statement();

        self.state.pop_scope();
        self.state.dedent();

        if self.state.chance(0.5) {
            self.state.writeln("} else {");
            self.state.indent();
            self.state.push_scope();

            self.generate_statement();

            self.state.pop_scope();
            self.state.dedent();
        }

        self.state.writeln("}");
    }

    /// Generate a loop statement
    fn generate_loop_statement(&mut self) {
        let loop_type: u32 = self.state.rng.gen_range(0..3);
        let max_iter = self.state.config.max_loop_iterations.min(10);
        let iterations = self.state.rng.gen_range(1..=max_iter);

        match loop_type {
            0 => {
                // For loop
                let var = self.state.unique_name("i");
                self.state
                    .writeln(&format!("for {} in 0..{} {{", var, iterations));
                self.state.indent();
                self.state.push_scope();

                self.state.add_variable(Variable {
                    name: var,
                    ty: VrType::Int,
                    mutable: false,
                });

                self.generate_statement();

                self.state.pop_scope();
                self.state.dedent();
                self.state.writeln("}");
            }
            1 => {
                // While loop
                let counter = self.state.unique_name("counter");
                self.state
                    .writeln(&format!("let mut {} = 0;", counter));

                self.state.add_variable(Variable {
                    name: counter.clone(),
                    ty: VrType::Int,
                    mutable: true,
                });

                self.state
                    .writeln(&format!("while {} < {} {{", counter, iterations));
                self.state.indent();
                self.state.push_scope();

                self.generate_statement();
                self.state
                    .writeln(&format!("{} = {} + 1;", counter, counter));

                self.state.pop_scope();
                self.state.dedent();
                self.state.writeln("}");
            }
            _ => {
                // Simple loop with break
                let counter = self.state.unique_name("n");
                self.state.writeln(&format!("let mut {} = 0;", counter));

                self.state.add_variable(Variable {
                    name: counter.clone(),
                    ty: VrType::Int,
                    mutable: true,
                });

                self.state.writeln("loop {");
                self.state.indent();
                self.state.push_scope();

                self.state
                    .writeln(&format!("if {} >= {} {{ break; }}", counter, iterations));
                self.generate_statement();
                self.state
                    .writeln(&format!("{} = {} + 1;", counter, counter));

                self.state.pop_scope();
                self.state.dedent();
                self.state.writeln("}");
            }
        }
    }

    /// Generate an expression statement
    fn generate_expression_statement(&mut self) {
        // Try to find a mutable variable to assign to
        let mutable_vars: Vec<&Variable> = self
            .state
            .visible_variables()
            .into_iter()
            .filter(|v| v.mutable)
            .collect();

        if !mutable_vars.is_empty() && self.state.chance(0.5) {
            let var = self.state.choose(&mutable_vars).unwrap().clone();
            let expr = self.generate_expression(&var.ty);
            self.state.writeln(&format!("{} = {};", var.name, expr));
        } else {
            // Just evaluate an expression
            let ty = self.random_simple_type();
            let expr = self.generate_expression(&ty);
            self.state.writeln(&format!("let _ = {};", expr));
        }
    }

    /// Generate a random expression of the given type
    fn generate_expression(&mut self, target_type: &VrType) -> String {
        self.state.depth += 1;
        let result = self.generate_expression_inner(target_type);
        self.state.depth -= 1;
        result
    }

    fn generate_expression_inner(&mut self, target_type: &VrType) -> String {
        // If at max depth, generate a leaf
        if self.state.should_stop() {
            return self.generate_leaf(target_type);
        }

        // Try to use an existing variable
        let matching_vars = self.state.variables_of_type(target_type);
        if !matching_vars.is_empty() && self.state.chance(self.state.config.feature_probabilities.variable)
        {
            let var = matching_vars[self.state.rng.gen_range(0..matching_vars.len())].clone();
            return var.name;
        }

        // Generate based on type
        match target_type {
            VrType::Int => self.generate_int_expression(),
            VrType::Float => self.generate_float_expression(),
            VrType::Bool => self.generate_bool_expression(),
            VrType::Text => self.generate_text_expression(),
            VrType::Char => self.generate_char_expression(),
            VrType::Unit => "()".to_string(),
            VrType::List(inner) => self.generate_list_expression(inner),
            VrType::Maybe(inner) => self.generate_maybe_expression(inner),
            VrType::Tuple(types) => self.generate_tuple_expression(types),
            _ => self.generate_leaf(target_type),
        }
    }

    /// Generate an integer expression
    fn generate_int_expression(&mut self) -> String {
        let choice: u32 = self.state.rng.gen_range(0..100);

        if choice < 30 || self.state.should_stop() {
            // Literal
            let val: i64 = self.state.rng.gen_range(-1000..1000);
            val.to_string()
        } else if choice < 60 {
            // Binary operation
            let op = self
                .state
                .choose(&["+", "-", "*", "/", "%"])
                .unwrap()
                .to_string();
            let left = self.generate_expression(&VrType::Int);
            let right = self.generate_expression(&VrType::Int);

            // Avoid division by zero
            if op == "/" || op == "%" {
                format!("({} {} ({} + 1).abs())", left, op, right)
            } else {
                format!("({} {} {})", left, op, right)
            }
        } else if choice < 80 {
            // If expression
            let cond = self.generate_expression(&VrType::Bool);
            let then_expr = self.generate_expression(&VrType::Int);
            let else_expr = self.generate_expression(&VrType::Int);
            format!("(if {} {{ {} }} else {{ {} }})", cond, then_expr, else_expr)
        } else {
            // Function call
            self.generate_function_call(&VrType::Int)
                .unwrap_or_else(|| "0".to_string())
        }
    }

    /// Generate a float expression
    fn generate_float_expression(&mut self) -> String {
        let choice: u32 = self.state.rng.gen_range(0..100);

        if choice < 40 || self.state.should_stop() {
            // Literal
            let val: f64 = self.state.rng.gen_range(-100.0..100.0);
            format!("{:.4}", val)
        } else if choice < 80 {
            // Binary operation
            let op = self
                .state
                .choose(&["+", "-", "*", "/"])
                .unwrap()
                .to_string();
            let left = self.generate_expression(&VrType::Float);
            let right = self.generate_expression(&VrType::Float);

            if op == "/" {
                format!("({} / ({} + 0.001))", left, right)
            } else {
                format!("({} {} {})", left, op, right)
            }
        } else {
            // Int to float conversion
            let int_expr = self.generate_expression(&VrType::Int);
            format!("({} as Float)", int_expr)
        }
    }

    /// Generate a boolean expression
    fn generate_bool_expression(&mut self) -> String {
        let choice: u32 = self.state.rng.gen_range(0..100);

        if choice < 30 || self.state.should_stop() {
            // Literal
            if self.state.chance(0.5) {
                "true"
            } else {
                "false"
            }
            .to_string()
        } else if choice < 50 {
            // Comparison
            let op = self
                .state
                .choose(&["<", ">", "<=", ">=", "==", "!="])
                .unwrap()
                .to_string();
            let left = self.generate_expression(&VrType::Int);
            let right = self.generate_expression(&VrType::Int);
            format!("({} {} {})", left, op, right)
        } else if choice < 70 {
            // Logical operation
            let op = self.state.choose(&["&&", "||"]).unwrap().to_string();
            let left = self.generate_expression(&VrType::Bool);
            let right = self.generate_expression(&VrType::Bool);
            format!("({} {} {})", left, op, right)
        } else if choice < 85 {
            // Negation
            let expr = self.generate_expression(&VrType::Bool);
            format!("(!{})", expr)
        } else {
            // Default
            "true".to_string()
        }
    }

    /// Generate a text expression
    fn generate_text_expression(&mut self) -> String {
        let strings = [
            "\"hello\"",
            "\"world\"",
            "\"foo\"",
            "\"bar\"",
            "\"test\"",
            "\"\"",
            "\"123\"",
        ];
        self.state.choose(&strings).unwrap().to_string()
    }

    /// Generate a char expression
    fn generate_char_expression(&mut self) -> String {
        let chars = ['a', 'b', 'c', 'x', 'y', 'z', '0', '1', ' '];
        let c = self.state.choose(&chars).unwrap();
        format!("'{}'", c)
    }

    /// Generate a list expression
    fn generate_list_expression(&mut self, inner: &VrType) -> String {
        if self.state.should_stop() {
            return format!("[] as List<{}>", inner.to_string());
        }

        let len = self.state.rng.gen_range(0..=5);
        if len == 0 {
            format!("[] as List<{}>", inner.to_string())
        } else {
            let elements: Vec<String> = (0..len)
                .map(|_| self.generate_expression(inner))
                .collect();
            format!("[{}]", elements.join(", "))
        }
    }

    /// Generate a maybe expression
    fn generate_maybe_expression(&mut self, inner: &VrType) -> String {
        if self.state.chance(0.3) {
            "None".to_string()
        } else {
            let val = self.generate_expression(inner);
            format!("Some({})", val)
        }
    }

    /// Generate a tuple expression
    fn generate_tuple_expression(&mut self, types: &[VrType]) -> String {
        let elements: Vec<String> = types.iter().map(|t| self.generate_expression(t)).collect();
        format!("({})", elements.join(", "))
    }

    /// Generate a leaf expression (literal or variable)
    fn generate_leaf(&mut self, target_type: &VrType) -> String {
        // Try to use a variable first
        let matching_vars = self.state.variables_of_type(target_type);
        if !matching_vars.is_empty() && self.state.chance(0.5) {
            let var = matching_vars[self.state.rng.gen_range(0..matching_vars.len())].clone();
            return var.name;
        }

        // Generate literal
        match target_type {
            VrType::Int => {
                let val: i64 = self.state.rng.gen_range(-100..100);
                val.to_string()
            }
            VrType::Float => {
                let val: f64 = self.state.rng.gen_range(-100.0..100.0);
                format!("{:.2}", val)
            }
            VrType::Bool => {
                if self.state.chance(0.5) {
                    "true"
                } else {
                    "false"
                }
                .to_string()
            }
            VrType::Text => "\"\"".to_string(),
            VrType::Char => "'x'".to_string(),
            VrType::Unit => "()".to_string(),
            VrType::List(inner) => format!("[] as List<{}>", inner.to_string()),
            VrType::Maybe(_) => "None".to_string(),
            VrType::Tuple(types) => {
                let defaults: Vec<String> = types.iter().map(|t| self.generate_leaf(t)).collect();
                format!("({})", defaults.join(", "))
            }
            _ => "()".to_string(),
        }
    }

    /// Generate a function call of the given return type
    fn generate_function_call(&mut self, return_type: &VrType) -> Option<String> {
        // Find a function that returns the right type
        let matching_functions: Vec<_> = self
            .state
            .functions
            .values()
            .filter(|f| &f.return_type == return_type)
            .cloned()
            .collect();

        if matching_functions.is_empty() {
            return None;
        }

        let func = &matching_functions[self.state.rng.gen_range(0..matching_functions.len())];
        let args: Vec<String> = func
            .params
            .iter()
            .map(|(_, ty)| self.generate_expression(ty))
            .collect();

        Some(format!("{}({})", func.name, args.join(", ")))
    }

    /// Generate a random simple type
    fn random_simple_type(&mut self) -> VrType {
        let choices = if self.state.config.enable_floats {
            vec![VrType::Int, VrType::Float, VrType::Bool]
        } else {
            vec![VrType::Int, VrType::Bool]
        };

        choices[self.state.rng.gen_range(0..choices.len())].clone()
    }
}

// ============================================================================
// Specialized Generators
// ============================================================================

/// Generator for programs with specific characteristics
pub struct CharacteristicGenerator {
    base_config: GeneratorConfig,
}

impl CharacteristicGenerator {
    pub fn new(seed: u64) -> Self {
        let mut config = GeneratorConfig::default();
        config.seed = seed;
        Self { base_config: config }
    }

    /// Generate a program with deep recursion
    pub fn generate_recursive(&mut self) -> String {
        let mut output = String::new();

        output.push_str("// @test: differential\n");
        output.push_str("// @tier: 0, 3\n");
        output.push_str("// @level: L1\n");
        output.push_str("// @tags: generated, recursion\n\n");

        // Generate recursive Fibonacci
        output.push_str("fn fib(n: Int) -> Int {\n");
        output.push_str("    if n <= 1 { n }\n");
        output.push_str("    else { fib(n - 1) + fib(n - 2) }\n");
        output.push_str("}\n\n");

        // Generate recursive factorial
        output.push_str("fn factorial(n: Int) -> Int {\n");
        output.push_str("    if n <= 1 { 1 }\n");
        output.push_str("    else { n * factorial(n - 1) }\n");
        output.push_str("}\n\n");

        // Generate mutual recursion
        output.push_str("fn is_even(n: Int) -> Bool {\n");
        output.push_str("    if n == 0 { true } else { is_odd(n - 1) }\n");
        output.push_str("}\n\n");

        output.push_str("fn is_odd(n: Int) -> Bool {\n");
        output.push_str("    if n == 0 { false } else { is_even(n - 1) }\n");
        output.push_str("}\n\n");

        output.push_str("fn main() {\n");
        output.push_str("    println(f\"fib(10) = {fib(10)}\");\n");
        output.push_str("    println(f\"factorial(10) = {factorial(10)}\");\n");
        output.push_str("    println(f\"is_even(42) = {is_even(42)}\");\n");
        output.push_str("    println(f\"is_odd(17) = {is_odd(17)}\");\n");
        output.push_str("}\n");

        output
    }

    /// Generate a program with nested loops
    pub fn generate_nested_loops(&mut self) -> String {
        let mut output = String::new();

        output.push_str("// @test: differential\n");
        output.push_str("// @tier: 0, 3\n");
        output.push_str("// @level: L1\n");
        output.push_str("// @tags: generated, loops, nested\n\n");

        output.push_str("fn main() {\n");
        output.push_str("    let mut sum = 0;\n\n");

        output.push_str("    // Triple nested loop\n");
        output.push_str("    for i in 0..5 {\n");
        output.push_str("        for j in 0..5 {\n");
        output.push_str("            for k in 0..5 {\n");
        output.push_str("                sum = sum + i * j + k;\n");
        output.push_str("            }\n");
        output.push_str("        }\n");
        output.push_str("    }\n");
        output.push_str("    println(f\"nested sum = {sum}\");\n\n");

        output.push_str("    // Loop with break and continue\n");
        output.push_str("    let mut count = 0;\n");
        output.push_str("    for i in 0..100 {\n");
        output.push_str("        if i % 2 == 0 { continue; }\n");
        output.push_str("        if i > 20 { break; }\n");
        output.push_str("        count = count + 1;\n");
        output.push_str("    }\n");
        output.push_str("    println(f\"odd count = {count}\");\n\n");

        output.push_str("    // While loop\n");
        output.push_str("    let mut n = 1;\n");
        output.push_str("    while n < 1000 {\n");
        output.push_str("        n = n * 2;\n");
        output.push_str("    }\n");
        output.push_str("    println(f\"power of 2 >= 1000 = {n}\");\n");

        output.push_str("}\n");

        output
    }

    /// Generate a program with complex expressions
    pub fn generate_complex_expressions(&mut self) -> String {
        let mut output = String::new();

        output.push_str("// @test: differential\n");
        output.push_str("// @tier: 0, 3\n");
        output.push_str("// @level: L1\n");
        output.push_str("// @tags: generated, expressions, complex\n\n");

        output.push_str("fn main() {\n");
        output.push_str("    // Deeply nested arithmetic\n");
        output.push_str("    let a = ((1 + 2) * (3 + 4) - (5 * 6)) / 2;\n");
        output.push_str("    println(f\"a = {a}\");\n\n");

        output.push_str("    // Chained comparisons\n");
        output.push_str("    let x = 50;\n");
        output.push_str("    let in_range = x > 0 && x < 100;\n");
        output.push_str("    println(f\"in_range = {in_range}\");\n\n");

        output.push_str("    // Complex boolean\n");
        output.push_str("    let b1 = true;\n");
        output.push_str("    let b2 = false;\n");
        output.push_str("    let b3 = true;\n");
        output.push_str("    let result = (b1 && b2) || (b2 && b3) || (!b2 && b3);\n");
        output.push_str("    println(f\"result = {result}\");\n\n");

        output.push_str("    // Nested if expressions\n");
        output.push_str("    let category = if x < 0 {\n");
        output.push_str("        if x < -100 { \"very negative\" } else { \"negative\" }\n");
        output.push_str("    } else if x == 0 {\n");
        output.push_str("        \"zero\"\n");
        output.push_str("    } else {\n");
        output.push_str("        if x > 100 { \"very positive\" } else { \"positive\" }\n");
        output.push_str("    };\n");
        output.push_str("    println(f\"category = {category}\");\n\n");

        output.push_str("    // Operator precedence test\n");
        output.push_str("    let p1 = 2 + 3 * 4;\n");
        output.push_str("    let p2 = (2 + 3) * 4;\n");
        output.push_str("    let p3 = 2 ** 3 ** 2;\n");
        output.push_str("    println(f\"p1 = {p1}, p2 = {p2}, p3 = {p3}\");\n");

        output.push_str("}\n");

        output
    }

    /// Generate a program with pattern matching
    pub fn generate_pattern_matching(&mut self) -> String {
        let mut output = String::new();

        output.push_str("// @test: differential\n");
        output.push_str("// @tier: 0, 3\n");
        output.push_str("// @level: L1\n");
        output.push_str("// @tags: generated, patterns, match\n\n");

        output.push_str("type Color is Red | Green | Blue;\n\n");

        output.push_str("type Shape is\n");
        output.push_str("    | Circle(Float)\n");
        output.push_str("    | Rectangle { width: Float, height: Float };\n\n");

        output.push_str("fn color_name(c: Color) -> Text {\n");
        output.push_str("    match c {\n");
        output.push_str("        Red => \"red\",\n");
        output.push_str("        Green => \"green\",\n");
        output.push_str("        Blue => \"blue\",\n");
        output.push_str("    }\n");
        output.push_str("}\n\n");

        output.push_str("fn area(s: Shape) -> Float {\n");
        output.push_str("    match s {\n");
        output.push_str("        Circle(r) => 3.14159 * r * r,\n");
        output.push_str("        Rectangle { width, height } => width * height,\n");
        output.push_str("    }\n");
        output.push_str("}\n\n");

        output.push_str("fn classify(n: Int) -> Text {\n");
        output.push_str("    match n {\n");
        output.push_str("        0 => \"zero\",\n");
        output.push_str("        1 | 2 | 3 => \"small\",\n");
        output.push_str("        x if x < 0 => \"negative\",\n");
        output.push_str("        x if x < 100 => \"medium\",\n");
        output.push_str("        _ => \"large\",\n");
        output.push_str("    }\n");
        output.push_str("}\n\n");

        output.push_str("fn main() {\n");
        output.push_str("    println(f\"Red = {color_name(Red)}\");\n");
        output.push_str("    println(f\"Circle area = {area(Circle(5.0))}\");\n");
        output.push_str("    println(f\"Rectangle area = {area(Rectangle { width: 4.0, height: 6.0 })}\");\n");
        output.push_str("    \n");
        output.push_str("    for n in [-10, 0, 2, 50, 200] {\n");
        output.push_str("        println(f\"{n} is {classify(n)}\");\n");
        output.push_str("    }\n");
        output.push_str("}\n");

        output
    }
}

// ============================================================================
// Batch Generation
// ============================================================================

/// Generate multiple random programs
pub fn generate_batch(count: usize, base_seed: u64, output_dir: &Path) -> Result<Vec<PathBuf>> {
    fs::create_dir_all(output_dir)?;

    let mut paths = Vec::new();

    for i in 0..count {
        let seed = base_seed.wrapping_add(i as u64);
        let mut generator = RandomProgramGenerator::with_seed(seed);
        let program = generator.generate();

        let filename = format!("generated_{:04}.vr", i);
        let path = output_dir.join(&filename);
        fs::write(&path, &program)?;

        paths.push(path);
    }

    Ok(paths)
}

/// Generate programs with specific characteristics
pub fn generate_characteristic_batch(
    base_seed: u64,
    output_dir: &Path,
) -> Result<Vec<PathBuf>> {
    fs::create_dir_all(output_dir)?;

    let mut paths = Vec::new();
    let mut gen = CharacteristicGenerator::new(base_seed);

    // Recursive programs
    let path = output_dir.join("recursive.vr");
    fs::write(&path, gen.generate_recursive())?;
    paths.push(path);

    // Nested loops
    let path = output_dir.join("nested_loops.vr");
    fs::write(&path, gen.generate_nested_loops())?;
    paths.push(path);

    // Complex expressions
    let path = output_dir.join("complex_expressions.vr");
    fs::write(&path, gen.generate_complex_expressions())?;
    paths.push(path);

    // Pattern matching
    let path = output_dir.join("pattern_matching.vr");
    fs::write(&path, gen.generate_pattern_matching())?;
    paths.push(path);

    Ok(paths)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_generation() {
        let mut gen = RandomProgramGenerator::with_seed(42);
        let program = gen.generate();

        assert!(program.contains("fn main()"));
        assert!(program.contains("// @test: differential"));
    }

    #[test]
    fn test_deterministic() {
        let mut gen1 = RandomProgramGenerator::with_seed(12345);
        let mut gen2 = RandomProgramGenerator::with_seed(12345);

        let p1 = gen1.generate();
        let p2 = gen2.generate();

        assert_eq!(p1, p2);
    }

    #[test]
    fn test_different_seeds() {
        let mut gen1 = RandomProgramGenerator::with_seed(1);
        let mut gen2 = RandomProgramGenerator::with_seed(2);

        let p1 = gen1.generate();
        let p2 = gen2.generate();

        assert_ne!(p1, p2);
    }

    #[test]
    fn test_characteristic_generators() {
        let mut gen = CharacteristicGenerator::new(42);

        let recursive = gen.generate_recursive();
        assert!(recursive.contains("fn fib"));
        assert!(recursive.contains("fn factorial"));

        let loops = gen.generate_nested_loops();
        assert!(loops.contains("for i in"));
        assert!(loops.contains("for j in"));

        let patterns = gen.generate_pattern_matching();
        assert!(patterns.contains("match"));
        assert!(patterns.contains("type Color"));
    }

    #[test]
    fn test_type_to_string() {
        assert_eq!(VrType::Int.to_string(), "Int");
        assert_eq!(VrType::List(Box::new(VrType::Int)).to_string(), "List<Int>");
        assert_eq!(
            VrType::Tuple(vec![VrType::Int, VrType::Bool]).to_string(),
            "(Int, Bool)"
        );
    }
}
