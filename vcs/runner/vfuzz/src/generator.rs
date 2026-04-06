//! Random program generation for fuzz testing
//!
//! This module provides grammar-aware program generation that creates
//! syntactically valid (and optionally type-correct) Verum programs.
//!
//! # Generators
//!
//! - `Generator`: Main unified generator with configurable strategies
//! - `GrammarAware`: Generates syntactically valid programs
//! - `TypeAware`: Generates type-correct programs
//! - `EdgeCase`: Generates edge case programs (deep nesting, large literals, etc.)

use rand::prelude::*;
use serde::{Deserialize, Serialize};

/// Configuration for program generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratorConfig {
    /// Maximum AST depth
    pub max_depth: usize,
    /// Maximum statements per function
    pub max_statements: usize,
    /// Maximum number of functions to generate
    pub max_functions: usize,
    /// Maximum number of type definitions
    pub max_types: usize,
    /// Include async constructs
    pub include_async: bool,
    /// Include CBGR references
    pub include_cbgr: bool,
    /// Include refinement types
    pub include_refinements: bool,
    /// Include unsafe blocks
    pub include_unsafe: bool,
    /// Generator kind to use
    pub kind: GeneratorKind,
    /// Probability of generating invalid syntax (for robustness testing)
    pub invalid_syntax_prob: f64,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            max_depth: 10,
            max_statements: 50,
            max_functions: 10,
            max_types: 5,
            include_async: true,
            include_cbgr: true,
            include_refinements: false,
            include_unsafe: false,
            kind: GeneratorKind::Mixed,
            invalid_syntax_prob: 0.0,
        }
    }
}

/// Kind of generator to use
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GeneratorKind {
    /// Grammar-aware generation (syntactically valid)
    Grammar,
    /// Type-aware generation (type-correct)
    TypeAware,
    /// Edge case generation (stress testing)
    EdgeCase,
    /// Mixed generation (randomly selects)
    Mixed,
}

/// Main program generator
pub struct Generator {
    config: GeneratorConfig,
    /// Type context for type-aware generation
    type_context: TypeContext,
}

/// Type context for tracking generated types
#[derive(Debug, Default)]
struct TypeContext {
    /// Available variable names by type
    variables: Vec<(String, VerumType)>,
    /// Available function names
    functions: Vec<String>,
    /// Current depth
    depth: usize,
    /// Current scope level
    scope: usize,
}

/// Verum type representation for generation
#[derive(Debug, Clone, PartialEq, Eq)]
enum VerumType {
    Int,
    Float,
    Bool,
    Text,
    Char,
    Unit,
    List(Box<VerumType>),
    Map(Box<VerumType>, Box<VerumType>),
    Set(Box<VerumType>),
    Maybe(Box<VerumType>),
    Heap(Box<VerumType>),
    Shared(Box<VerumType>),
    Tuple(Vec<VerumType>),
    Ref(Box<VerumType>, RefTier),
    Function(Vec<VerumType>, Box<VerumType>),
    GenRef(Box<VerumType>),
}

/// Reference tier for CBGR
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefTier {
    Tier0,
    Checked,
    Unsafe,
}

impl Generator {
    /// Create a new generator with the given configuration
    pub fn new(config: GeneratorConfig) -> Self {
        Self {
            config,
            type_context: TypeContext::default(),
        }
    }

    /// Generate a random program
    pub fn generate<R: Rng>(&mut self, rng: &mut R) -> String {
        self.type_context = TypeContext::default();

        match self.config.kind {
            GeneratorKind::Grammar => self.generate_grammar_aware(rng),
            GeneratorKind::TypeAware => self.generate_type_aware(rng),
            GeneratorKind::EdgeCase => self.generate_edge_case(rng),
            GeneratorKind::Mixed => match rng.random_range(0..3) {
                0 => self.generate_grammar_aware(rng),
                1 => self.generate_type_aware(rng),
                _ => self.generate_edge_case(rng),
            },
        }
    }

    /// Generate a syntactically valid program using correct Verum syntax
    fn generate_grammar_aware<R: Rng>(&mut self, rng: &mut R) -> String {
        let mut output = String::new();

        // Optional imports using correct Verum import syntax
        if rng.random_bool(0.5) {
            output.push_str("import verum_core.base.{List, Text, Map, Maybe, Set, Heap};\n\n");
        }

        // Optional context definitions
        if rng.random_bool(0.3) {
            output.push_str(&self.generate_context_def(rng, 0));
            output.push_str("\n\n");
        }

        // Generate type definitions
        let num_types = rng.random_range(0..=self.config.max_types);
        for i in 0..num_types {
            output.push_str(&self.generate_type_def(rng, i));
            output.push_str("\n\n");
        }

        // Generate implement blocks for some types
        if num_types > 0 && rng.random_bool(0.4) {
            output.push_str(&self.generate_implement_block(rng, num_types));
            output.push_str("\n\n");
        }

        // Generate functions
        let num_functions = rng.random_range(1..=self.config.max_functions);
        for i in 0..num_functions {
            output.push_str(&self.generate_function(rng, &format!("func_{}", i)));
            output.push_str("\n\n");
        }

        // Generate main function
        output.push_str(&self.generate_main(rng));

        // Optionally introduce syntax errors for robustness testing
        if rng.random::<f64>() < self.config.invalid_syntax_prob {
            self.introduce_syntax_error(&mut output, rng);
        }

        output
    }

    /// Generate a context definition
    /// Syntax: context Name { fn method(&self) -> Type; }
    fn generate_context_def<R: Rng>(&self, rng: &mut R, index: usize) -> String {
        let mut items = String::new();
        let num_items = rng.random_range(1..4);

        for i in 0..num_items {
            let ret_type = self.random_primitive_type(rng);
            items.push_str(&format!("    fn get_{}(&self) -> {};\n", i, ret_type));
        }

        format!("context Context_{} {{\n{}}}", index, items)
    }

    /// Generate a type-correct program using correct Verum syntax
    fn generate_type_aware<R: Rng>(&mut self, rng: &mut R) -> String {
        let mut output = String::new();

        // Correct Verum import syntax: import path.{items};
        output.push_str("import verum_core.base.{List, Text, Map, Maybe, Heap, Set};\n\n");

        // Generate main with type-consistent expressions
        output.push_str("fn main() {\n");

        let num_statements = rng.random_range(1..=self.config.max_statements.min(20));
        for i in 0..num_statements {
            let ty = self.random_type(rng);
            let expr = self.generate_typed_expr(rng, &ty, 0);
            output.push_str(&format!(
                "    let var_{}: {} = {};\n",
                i,
                self.type_to_string(&ty),
                expr
            ));
            self.type_context.variables.push((format!("var_{}", i), ty));
        }

        output.push_str("}\n");

        output
    }

    /// Generate edge case programs for stress testing
    fn generate_edge_case<R: Rng>(&mut self, rng: &mut R) -> String {
        match rng.random_range(0..8) {
            0 => self.generate_deep_nesting(rng),
            1 => self.generate_large_literal(rng),
            2 => self.generate_many_parameters(rng),
            3 => self.generate_unicode_stress(rng),
            4 => self.generate_boundary_numbers(rng),
            5 => self.generate_deep_recursion(rng),
            6 => self.generate_many_locals(rng),
            _ => self.generate_complex_match(rng),
        }
    }

    /// Generate a type definition using correct Verum syntax
    /// Syntax per grammar/verum.ebnf:
    /// - Record: `type Name is { field: Type, ... };`
    /// - Variant: `type Name is Variant1 | Variant2(Type) | ...;`
    /// - Newtype: `type Name is (Type);`
    /// - Protocol: `type Name is protocol { ... };`
    fn generate_type_def<R: Rng>(&self, rng: &mut R, index: usize) -> String {
        match rng.random_range(0..6) {
            0 => {
                // Record type: type Name is { field: Type, ... };
                let fields = (0..rng.random_range(1..5))
                    .map(|i| format!("    field_{}: {}", i, self.random_type_string(rng)))
                    .collect::<Vec<_>>()
                    .join(",\n");
                format!("type Record_{} is {{\n{}\n}};", index, fields)
            }
            1 => {
                // Variant/sum type: type Name is Variant1 | Variant2(Type) | ...;
                let variants = (0..rng.random_range(2..5))
                    .map(|i| {
                        match rng.random_range(0..3) {
                            0 => format!("Variant_{}", i), // Unit variant
                            1 => format!("Variant_{}({})", i, self.random_type_string(rng)), // Tuple variant
                            _ => format!(
                                "Variant_{} {{ value: {} }}",
                                i,
                                self.random_type_string(rng)
                            ), // Record variant
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n    | ");
                format!("type Enum_{} is\n    | {};", index, variants)
            }
            2 => {
                // Newtype: type Name is (Type);
                format!(
                    "type Newtype_{} is ({});",
                    index,
                    self.random_type_string(rng)
                )
            }
            3 => {
                // Unit type: type Name is ();
                format!("type Unit_{} is ();", index)
            }
            4 => {
                // Protocol definition: type Name is protocol { ... };
                self.generate_protocol_def(rng, index)
            }
            _ => {
                // Type alias with refinement: type Name is Type { predicate };
                if rng.random_bool(0.3) {
                    format!("type Positive_{} is Int {{ it > 0 }};", index)
                } else {
                    format!("type Alias_{} is {};", index, self.random_type_string(rng))
                }
            }
        }
    }

    /// Generate a protocol definition
    fn generate_protocol_def<R: Rng>(&self, rng: &mut R, index: usize) -> String {
        let mut items = String::new();
        let num_items = rng.random_range(1..4);

        for i in 0..num_items {
            match rng.random_range(0..3) {
                0 => {
                    // Protocol method
                    let params = if rng.random_bool(0.5) {
                        format!("&self, arg: {}", self.random_primitive_type(rng))
                    } else {
                        "&self".to_string()
                    };
                    let ret = self.random_primitive_type(rng);
                    items.push_str(&format!("    fn method_{}({}) -> {};\n", i, params, ret));
                }
                1 => {
                    // Associated type
                    items.push_str(&format!("    type Item_{};\n", i));
                }
                _ => {
                    // Protocol constant
                    items.push_str(&format!("    const VALUE_{}: Int;\n", i));
                }
            }
        }

        format!("type Protocol_{} is protocol {{\n{}}};", index, items)
    }

    /// Generate an implement block
    fn generate_implement_block<R: Rng>(&self, rng: &mut R, index: usize) -> String {
        let type_name = format!("Record_{}", rng.random_range(0..index.max(1)));
        let mut items = String::new();
        let num_items = rng.random_range(1..3);

        for i in 0..num_items {
            let params = if rng.random_bool(0.5) {
                format!("&self, arg: {}", self.random_primitive_type(rng))
            } else {
                "&self".to_string()
            };
            let ret = self.random_primitive_type(rng);
            items.push_str(&format!(
                "    fn method_{}({}) -> {} {{\n        {}\n    }}\n",
                i,
                params,
                ret,
                self.generate_expr(rng, 0)
            ));
        }

        format!("implement {} {{\n{}}}", type_name, items)
    }

    /// Generate a function with correct Verum syntax
    /// Syntax per grammar/verum.ebnf:
    /// - `fn name(params) -> ReturnType using [Context] { body }`
    /// - `async fn name(params) -> ReturnType { body }`
    fn generate_function<R: Rng>(&mut self, rng: &mut R, name: &str) -> String {
        let num_params = rng.random_range(0..5);
        let params: Vec<_> = (0..num_params)
            .map(|i| format!("p_{}: {}", i, self.random_type_string(rng)))
            .collect();

        let return_type = if rng.random_bool(0.7) {
            format!(" -> {}", self.random_type_string(rng))
        } else {
            String::new()
        };

        let is_async = self.config.include_async && rng.random_bool(0.2);
        let async_kw = if is_async { "async " } else { "" };

        // Optional context clause: using [Context1, Context2]
        let context_clause = if rng.random_bool(0.2) {
            let contexts = self.random_contexts(rng);
            if contexts.len() == 1 {
                format!(" using {}", contexts[0])
            } else {
                format!(" using [{}]", contexts.join(", "))
            }
        } else {
            String::new()
        };

        let mut body = String::new();
        let num_statements = rng.random_range(1..=self.config.max_statements.min(10));

        for i in 0..num_statements {
            body.push_str(&format!(
                "    let local_{} = {};\n",
                i,
                self.generate_expr(rng, 0)
            ));
        }

        if !return_type.is_empty() {
            body.push_str(&format!("    {}\n", self.generate_expr(rng, 0)));
        }

        format!(
            "{}fn {}({}){}{} {{\n{}}}",
            async_kw,
            name,
            params.join(", "),
            return_type,
            context_clause,
            body
        )
    }

    /// Generate random context names
    fn random_contexts<R: Rng>(&self, rng: &mut R) -> Vec<&'static str> {
        let all_contexts = [
            "Database",
            "Logger",
            "FileSystem",
            "Network",
            "Config",
            "Auth",
        ];
        let num = rng.random_range(1..=3);
        (0..num)
            .map(|_| all_contexts[rng.random_range(0..all_contexts.len())])
            .collect()
    }

    /// Generate the main function
    fn generate_main<R: Rng>(&mut self, rng: &mut R) -> String {
        let mut body = String::new();
        let num_statements = rng.random_range(1..=self.config.max_statements);

        for i in 0..num_statements {
            body.push_str(&self.generate_statement(rng, i, 0));
        }

        format!("fn main() {{\n{}}}\n", body)
    }

    /// Generate a statement with correct Verum syntax
    /// Includes CBGR three-tier references and context system
    fn generate_statement<R: Rng>(&mut self, rng: &mut R, index: usize, depth: usize) -> String {
        if depth > self.config.max_depth {
            return format!("    let x_{} = 0;\n", index);
        }

        match rng.random_range(0..14) {
            0..=3 => {
                // Let binding with optional type annotation
                let ty = if rng.random_bool(0.5) {
                    format!(": {}", self.random_type_string(rng))
                } else {
                    String::new()
                };
                format!(
                    "    let var_{}{} = {};\n",
                    index,
                    ty,
                    self.generate_expr(rng, depth)
                )
            }
            4 => {
                // If statement
                format!(
                    "    if {} {{\n        let inner_{} = {};\n    }}\n",
                    self.generate_bool_expr(rng, depth),
                    index,
                    self.generate_expr(rng, depth + 1)
                )
            }
            5 => {
                // For loop
                format!(
                    "    for i in 0..{} {{\n        let item_{} = {};\n    }}\n",
                    rng.random_range(1..10),
                    index,
                    self.generate_expr(rng, depth + 1)
                )
            }
            6 => {
                // While loop
                format!(
                    "    while {} {{\n        break;\n    }}\n",
                    self.generate_bool_expr(rng, depth)
                )
            }
            7 => {
                // Match expression
                self.generate_match_statement(rng, index, depth)
            }
            8 if self.config.include_cbgr => {
                // Three-tier CBGR reference creation
                // Tier 0: &T (managed, default)
                // Tier 1: &checked T (compile-time verified)
                // Tier 2: &unsafe T (no checks)
                let ref_kind = match rng.random_range(0..3) {
                    0 => "&",         // Tier 0: managed reference
                    1 => "&checked ", // Tier 1: checked reference
                    _ => "&",         // Stick with managed for safety
                };
                let mutability = if rng.random_bool(0.3) { "mut " } else { "" };
                format!(
                    "    let ref_{} = {}{}{};\n",
                    index,
                    ref_kind,
                    mutability,
                    self.generate_expr(rng, depth)
                )
            }
            9 if self.config.include_unsafe => {
                // Unsafe reference (Tier 2)
                format!(
                    "    let unsafe_ref_{} = &unsafe {};\n",
                    index,
                    self.generate_expr(rng, depth)
                )
            }
            10 => {
                // Provide statement for context system
                let contexts = ["Database", "Logger", "Config"];
                let ctx = contexts[rng.random_range(0..contexts.len())];
                format!(
                    "    provide {} = {};\n",
                    ctx,
                    self.generate_expr(rng, depth)
                )
            }
            11 => {
                // Defer statement
                format!("    defer {};\n", self.generate_expr(rng, depth))
            }
            12 => {
                // Try-recover expression
                format!(
                    "    let result_{} = try {{\n        {}\n    }} recover {{\n        err => 0\n    }};\n",
                    index,
                    self.generate_expr(rng, depth + 1)
                )
            }
            _ => {
                // Expression statement
                format!("    {};\n", self.generate_expr(rng, depth))
            }
        }
    }

    /// Generate an expression with correct Verum syntax
    /// Includes pipeline (|>), null coalescing (??), optional chaining (?.), await
    fn generate_expr<R: Rng>(&self, rng: &mut R, depth: usize) -> String {
        if depth > self.config.max_depth {
            return self.generate_literal(rng);
        }

        match rng.random_range(0..20) {
            0..=3 => self.generate_literal(rng),
            4..=5 => self.generate_binary_op(rng, depth),
            6 => self.generate_unary_op(rng, depth),
            7 => self.generate_if_expr(rng, depth),
            8 => self.generate_block_expr(rng, depth),
            9 => self.generate_list_expr(rng, depth),
            10 => self.generate_tuple_expr(rng, depth),
            11 => self.generate_call_expr(rng, depth),
            12 => self.generate_lambda_expr(rng, depth),
            13 => self.generate_field_access(rng),
            14 => self.generate_pipeline_expr(rng, depth),
            15 => self.generate_null_coalesce_expr(rng, depth),
            16 => self.generate_optional_chain_expr(rng, depth),
            17 => self.generate_heap_expr(rng, depth),
            18 => self.generate_maybe_expr(rng, depth),
            _ => format!("var_{}", rng.random_range(0..10)),
        }
    }

    /// Generate pipeline expression: expr |> func |> func
    fn generate_pipeline_expr<R: Rng>(&self, rng: &mut R, depth: usize) -> String {
        let num_stages = rng.random_range(2..4);
        let mut expr = self.generate_expr(rng, depth + 1);
        for _ in 0..num_stages {
            let funcs = ["transform", "filter", "map", "process"];
            let func = funcs[rng.random_range(0..funcs.len())];
            expr = format!("{} |> {}", expr, func);
        }
        expr
    }

    /// Generate null coalescing expression: expr ?? default
    fn generate_null_coalesce_expr<R: Rng>(&self, rng: &mut R, depth: usize) -> String {
        format!(
            "{} ?? {}",
            self.generate_expr(rng, depth + 1),
            self.generate_literal(rng)
        )
    }

    /// Generate optional chaining expression: expr?.field
    fn generate_optional_chain_expr<R: Rng>(&self, rng: &mut R, _depth: usize) -> String {
        let fields = ["value", "data", "inner", "result"];
        let field = fields[rng.random_range(0..fields.len())];
        format!("maybe_val?.{}", field)
    }

    /// Generate Heap allocation: Heap(expr)
    fn generate_heap_expr<R: Rng>(&self, rng: &mut R, depth: usize) -> String {
        format!("Heap({})", self.generate_expr(rng, depth + 1))
    }

    /// Generate Maybe expression: Some(expr) or None
    fn generate_maybe_expr<R: Rng>(&self, rng: &mut R, depth: usize) -> String {
        if rng.random_bool(0.7) {
            format!("Some({})", self.generate_expr(rng, depth + 1))
        } else {
            "None".to_string()
        }
    }

    /// Generate a typed expression using correct Verum syntax
    fn generate_typed_expr<R: Rng>(&self, rng: &mut R, ty: &VerumType, depth: usize) -> String {
        if depth > self.config.max_depth {
            return self.generate_typed_literal(rng, ty);
        }

        match ty {
            VerumType::Int => self.generate_int_expr(rng, depth),
            VerumType::Float => self.generate_float_expr(rng, depth),
            VerumType::Bool => self.generate_bool_expr(rng, depth),
            VerumType::Text => self.generate_text_expr(rng, depth),
            VerumType::Char => format!("'{}'", self.random_char(rng)),
            VerumType::Unit => "()".to_string(),
            VerumType::List(inner) => {
                let elements: Vec<_> = (0..rng.random_range(0..5))
                    .map(|_| self.generate_typed_expr(rng, inner, depth + 1))
                    .collect();
                format!("[{}]", elements.join(", "))
            }
            VerumType::Set(inner) => {
                // Set literal syntax: {elem1, elem2, ...}
                let elements: Vec<_> = (0..rng.random_range(0..5))
                    .map(|_| self.generate_typed_expr(rng, inner, depth + 1))
                    .collect();
                format!("{{{}}}", elements.join(", "))
            }
            VerumType::Maybe(inner) => {
                if rng.random_bool(0.7) {
                    format!("Some({})", self.generate_typed_expr(rng, inner, depth + 1))
                } else {
                    "None".to_string()
                }
            }
            VerumType::Heap(inner) => {
                // Heap allocation: Heap(value)
                format!("Heap({})", self.generate_typed_expr(rng, inner, depth + 1))
            }
            VerumType::Shared(inner) => {
                // Shared reference: Shared(value)
                format!(
                    "Shared({})",
                    self.generate_typed_expr(rng, inner, depth + 1)
                )
            }
            VerumType::Tuple(types) => {
                let elements: Vec<_> = types
                    .iter()
                    .map(|t| self.generate_typed_expr(rng, t, depth + 1))
                    .collect();
                format!("({})", elements.join(", "))
            }
            VerumType::Ref(inner, tier) => {
                // Reference expression with appropriate tier
                let prefix = match tier {
                    RefTier::Tier0 => "&",
                    RefTier::Checked => "&checked ",
                    RefTier::Unsafe => "&unsafe ",
                };
                format!(
                    "{}({})",
                    prefix,
                    self.generate_typed_expr(rng, inner, depth + 1)
                )
            }
            VerumType::GenRef(inner) => {
                // GenRef for generation tracking
                format!(
                    "GenRef({})",
                    self.generate_typed_expr(rng, inner, depth + 1)
                )
            }
            _ => self.generate_typed_literal(rng, ty),
        }
    }

    /// Generate a typed literal using correct Verum syntax
    fn generate_typed_literal<R: Rng>(&self, rng: &mut R, ty: &VerumType) -> String {
        match ty {
            VerumType::Int => format!("{}", rng.random_range(-1000..1000)),
            VerumType::Float => format!("{:.2}", rng.random_range(-1000.0..1000.0)),
            VerumType::Bool => if rng.random_bool(0.5) {
                "true"
            } else {
                "false"
            }
            .to_string(),
            VerumType::Text => format!("\"{}\"", self.random_string(rng, 10)),
            VerumType::Char => format!("'{}'", self.random_char(rng)),
            VerumType::Unit => "()".to_string(),
            VerumType::List(_) => "[]".to_string(),
            VerumType::Set(_) => "{}".to_string(),
            VerumType::Map(_, _) => "{}".to_string(),
            VerumType::Maybe(_) => "None".to_string(),
            VerumType::Heap(_) => "Heap(0)".to_string(),
            VerumType::Shared(_) => "Shared(0)".to_string(),
            _ => "()".to_string(),
        }
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
            3 => format!("\"{}\"", self.random_string(rng, 20)),
            4 => "()".to_string(),
            5 => format!("'{}'", self.random_char(rng)),
            6 => format!("0x{:X}", rng.random_range(0..256)),
            _ => format!("0b{:b}", rng.random_range(0..256)),
        }
    }

    /// Generate a binary operation
    fn generate_binary_op<R: Rng>(&self, rng: &mut R, depth: usize) -> String {
        let ops = [
            "+", "-", "*", "/", "%", "&&", "||", "==", "!=", "<", ">", "<=", ">=",
        ];
        let op = ops[rng.random_range(0..ops.len())];
        format!(
            "({} {} {})",
            self.generate_expr(rng, depth + 1),
            op,
            self.generate_expr(rng, depth + 1)
        )
    }

    /// Generate a unary operation
    fn generate_unary_op<R: Rng>(&self, rng: &mut R, depth: usize) -> String {
        let ops = ["-", "!"];
        let op = ops[rng.random_range(0..ops.len())];
        format!("({}{})", op, self.generate_expr(rng, depth + 1))
    }

    /// Generate an if expression
    fn generate_if_expr<R: Rng>(&self, rng: &mut R, depth: usize) -> String {
        format!(
            "if {} {{ {} }} else {{ {} }}",
            self.generate_bool_expr(rng, depth + 1),
            self.generate_expr(rng, depth + 1),
            self.generate_expr(rng, depth + 1)
        )
    }

    /// Generate a block expression
    fn generate_block_expr<R: Rng>(&self, rng: &mut R, depth: usize) -> String {
        let num_stmts = rng.random_range(0..3);
        let mut stmts = String::new();
        for i in 0..num_stmts {
            stmts.push_str(&format!(
                "let tmp_{} = {}; ",
                i,
                self.generate_expr(rng, depth + 1)
            ));
        }
        format!("{{ {}{} }}", stmts, self.generate_expr(rng, depth + 1))
    }

    /// Generate a list expression
    fn generate_list_expr<R: Rng>(&self, rng: &mut R, depth: usize) -> String {
        let num_elements = rng.random_range(0..5);
        let elements: Vec<_> = (0..num_elements)
            .map(|_| self.generate_expr(rng, depth + 1))
            .collect();
        format!("[{}]", elements.join(", "))
    }

    /// Generate a tuple expression
    fn generate_tuple_expr<R: Rng>(&self, rng: &mut R, depth: usize) -> String {
        let num_elements = rng.random_range(2..5);
        let elements: Vec<_> = (0..num_elements)
            .map(|_| self.generate_expr(rng, depth + 1))
            .collect();
        format!("({})", elements.join(", "))
    }

    /// Generate a call expression
    fn generate_call_expr<R: Rng>(&self, rng: &mut R, depth: usize) -> String {
        let funcs = ["len", "abs", "min", "max", "print", "debug"];
        let func = funcs[rng.random_range(0..funcs.len())];
        let num_args = rng.random_range(1..3);
        let args: Vec<_> = (0..num_args)
            .map(|_| self.generate_expr(rng, depth + 1))
            .collect();
        format!("{}({})", func, args.join(", "))
    }

    /// Generate a lambda expression
    fn generate_lambda_expr<R: Rng>(&self, rng: &mut R, depth: usize) -> String {
        let num_params = rng.random_range(1..3);
        let params: Vec<_> = (0..num_params).map(|i| format!("p{}", i)).collect();
        format!(
            "|{}| {}",
            params.join(", "),
            self.generate_expr(rng, depth + 1)
        )
    }

    /// Generate a field access
    fn generate_field_access<R: Rng>(&self, rng: &mut R) -> String {
        let fields = ["field", "value", "data", "inner", "0", "1"];
        format!("x.{}", fields[rng.random_range(0..fields.len())])
    }

    /// Generate a boolean expression
    fn generate_bool_expr<R: Rng>(&self, rng: &mut R, depth: usize) -> String {
        if depth > 3 || rng.random_bool(0.5) {
            return if rng.random_bool(0.5) {
                "true"
            } else {
                "false"
            }
            .to_string();
        }

        match rng.random_range(0..5) {
            0 => format!(
                "({} == {})",
                self.generate_expr(rng, depth + 1),
                self.generate_expr(rng, depth + 1)
            ),
            1 => format!(
                "({} < {})",
                self.generate_expr(rng, depth + 1),
                self.generate_expr(rng, depth + 1)
            ),
            2 => format!(
                "({} > {})",
                self.generate_expr(rng, depth + 1),
                self.generate_expr(rng, depth + 1)
            ),
            3 => format!(
                "({} && {})",
                self.generate_bool_expr(rng, depth + 1),
                self.generate_bool_expr(rng, depth + 1)
            ),
            _ => format!(
                "({} || {})",
                self.generate_bool_expr(rng, depth + 1),
                self.generate_bool_expr(rng, depth + 1)
            ),
        }
    }

    /// Generate an integer expression
    fn generate_int_expr<R: Rng>(&self, rng: &mut R, depth: usize) -> String {
        if depth > 3 || rng.random_bool(0.5) {
            return format!("{}", rng.random_range(-1000..1000));
        }

        match rng.random_range(0..4) {
            0 => format!(
                "({} + {})",
                self.generate_int_expr(rng, depth + 1),
                self.generate_int_expr(rng, depth + 1)
            ),
            1 => format!(
                "({} - {})",
                self.generate_int_expr(rng, depth + 1),
                self.generate_int_expr(rng, depth + 1)
            ),
            2 => format!(
                "({} * {})",
                self.generate_int_expr(rng, depth + 1),
                self.generate_int_expr(rng, depth + 1)
            ),
            _ => format!("abs({})", self.generate_int_expr(rng, depth + 1)),
        }
    }

    /// Generate a float expression
    fn generate_float_expr<R: Rng>(&self, rng: &mut R, depth: usize) -> String {
        if depth > 3 || rng.random_bool(0.5) {
            return format!("{:.2}", rng.random_range(-1000.0..1000.0));
        }

        match rng.random_range(0..3) {
            0 => format!(
                "({} + {})",
                self.generate_float_expr(rng, depth + 1),
                self.generate_float_expr(rng, depth + 1)
            ),
            1 => format!(
                "({} * {})",
                self.generate_float_expr(rng, depth + 1),
                self.generate_float_expr(rng, depth + 1)
            ),
            _ => format!("{:.2}", rng.random_range(-1000.0..1000.0)),
        }
    }

    /// Generate a text expression
    fn generate_text_expr<R: Rng>(&self, rng: &mut R, depth: usize) -> String {
        if depth > 2 || rng.random_bool(0.7) {
            return format!("\"{}\"", self.random_string(rng, 20));
        }

        format!(
            "({} + {})",
            self.generate_text_expr(rng, depth + 1),
            self.generate_text_expr(rng, depth + 1)
        )
    }

    /// Generate a match statement
    fn generate_match_statement<R: Rng>(&self, rng: &mut R, index: usize, depth: usize) -> String {
        let scrutinee = self.generate_expr(rng, depth);
        let num_arms = rng.random_range(2..5);
        let mut arms = String::new();

        for i in 0..num_arms {
            let pattern = match rng.random_range(0..3) {
                0 => format!("{}", rng.random_range(0..10)),
                1 => format!("x_{}", i),
                _ => "_".to_string(),
            };
            arms.push_str(&format!(
                "        {} => {},\n",
                pattern,
                self.generate_expr(rng, depth + 1)
            ));
        }

        format!(
            "    let match_{} = match {} {{\n{}    }};\n",
            index, scrutinee, arms
        )
    }

    /// Generate deeply nested expressions
    /// NOTE: Fixed exponential memory growth - else branch now uses constant instead of duplicating expr
    fn generate_deep_nesting<R: Rng>(&self, rng: &mut R) -> String {
        let depth = rng.random_range(50..100);
        let mut expr = "x".to_string();
        // Limit actual string growth to prevent memory exhaustion
        const MAX_EXPR_SIZE: usize = 10_000;
        for _i in 0..depth {
            if expr.len() > MAX_EXPR_SIZE {
                break; // Prevent unbounded growth
            }
            match rng.random_range(0..3) {
                0 => expr = format!("(({}))", expr),
                1 => expr = format!("{{ {} }}", expr),
                // FIXED: Was using `expr` twice, causing exponential 2^depth growth
                // Now uses constant in else branch to prevent memory explosion
                _ => expr = format!("if true {{ {} }} else {{ 0 }}", expr),
            }
        }
        format!(
            "fn main() {{\n    let x = 0;\n    let result = {};\n}}\n",
            expr
        )
    }

    /// Generate programs with large literals
    fn generate_large_literal<R: Rng>(&self, rng: &mut R) -> String {
        match rng.random_range(0..4) {
            0 => {
                // Large integer
                format!("fn main() {{\n    let x = {};\n}}\n", i64::MAX)
            }
            1 => {
                // Long string
                let len = rng.random_range(1000..10000);
                let s: String = (0..len).map(|_| 'a').collect();
                format!("fn main() {{\n    let x = \"{}\";\n}}\n", s)
            }
            2 => {
                // Large list
                let len = rng.random_range(100..1000);
                let elements: Vec<_> = (0..len).map(|i| format!("{}", i)).collect();
                format!("fn main() {{\n    let x = [{}];\n}}\n", elements.join(", "))
            }
            _ => {
                // Large tuple
                let len = rng.random_range(20..50);
                let elements: Vec<_> = (0..len).map(|i| format!("{}", i)).collect();
                format!("fn main() {{\n    let x = ({});\n}}\n", elements.join(", "))
            }
        }
    }

    /// Generate functions with many parameters
    fn generate_many_parameters<R: Rng>(&self, rng: &mut R) -> String {
        let num_params = rng.random_range(50..100);
        let params: Vec<_> = (0..num_params).map(|i| format!("p{}: Int", i)).collect();
        let sum: Vec<_> = (0..num_params.min(10)).map(|i| format!("p{}", i)).collect();

        format!(
            "fn many_params({}) -> Int {{\n    {}\n}}\n\nfn main() {{\n    let result = many_params({});\n}}\n",
            params.join(", "),
            sum.join(" + "),
            (0..num_params)
                .map(|i| format!("{}", i))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    /// Generate programs with Unicode stress testing
    fn generate_unicode_stress<R: Rng>(&self, rng: &mut R) -> String {
        let unicode_strs = [
            "\u{1F600}\u{1F680}\u{1F4BB}",
            "\u{0000}\u{FFFF}",
            "\u{10FFFF}",
            "\u{200B}\u{200C}\u{200D}",
            "\u{FEFF}",
            "\u{202A}\u{202B}",
            "\u{0301}\u{0302}\u{0303}",
        ];

        let mut program = "fn main() {\n".to_string();
        for (i, s) in unicode_strs.iter().enumerate() {
            if rng.random_bool(0.7) {
                program.push_str(&format!("    let unicode_{} = \"{}\";\n", i, s));
            }
        }
        program.push_str("}\n");
        program
    }

    /// Generate programs with boundary numbers
    fn generate_boundary_numbers<R: Rng>(&self, rng: &mut R) -> String {
        let boundaries = [
            "0",
            "1",
            "-1",
            "127",
            "-128",
            "255",
            "32767",
            "-32768",
            "65535",
            "2147483647",
            "-2147483648",
            "4294967295",
            "9223372036854775807",
            "-9223372036854775808",
            "0.0",
            "-0.0",
            "1.0e308",
            "-1.0e308",
            "1.0e-308",
        ];

        let mut program = "fn main() {\n".to_string();
        for (i, num) in boundaries.iter().enumerate() {
            if rng.random_bool(0.8) {
                program.push_str(&format!("    let boundary_{} = {};\n", i, num));
            }
        }
        program.push_str("}\n");
        program
    }

    /// Generate deeply recursive function
    fn generate_deep_recursion<R: Rng>(&self, rng: &mut R) -> String {
        let depth = rng.random_range(10..50);
        let mut funcs = String::new();

        for i in 0..depth {
            if i == depth - 1 {
                funcs.push_str(&format!("fn rec_{}(n: Int) -> Int {{ n }}\n\n", i));
            } else {
                funcs.push_str(&format!(
                    "fn rec_{}(n: Int) -> Int {{ if n <= 0 {{ 0 }} else {{ rec_{}(n - 1) }} }}\n\n",
                    i,
                    i + 1
                ));
            }
        }

        format!("{}fn main() {{\n    let result = rec_0(10);\n}}\n", funcs)
    }

    /// Generate function with many locals
    fn generate_many_locals<R: Rng>(&self, rng: &mut R) -> String {
        let num_locals = rng.random_range(100..500);
        let mut body = String::new();

        for i in 0..num_locals {
            body.push_str(&format!(
                "    let local_{} = {};\n",
                i,
                rng.random_range(0..1000)
            ));
        }

        let sum_range = num_locals.min(10);
        let sum: Vec<_> = (0..sum_range).map(|i| format!("local_{}", i)).collect();
        body.push_str(&format!("    let result = {};\n", sum.join(" + ")));

        format!("fn main() {{\n{}}}\n", body)
    }

    /// Generate complex match expression using correct Verum syntax
    /// Variant types use: type Name is Variant1 | Variant2(Type) | ...;
    fn generate_complex_match<R: Rng>(&self, rng: &mut R) -> String {
        let num_variants = rng.random_range(5..20);
        let mut variants = Vec::new();

        for i in 0..num_variants {
            if rng.random_bool(0.5) {
                variants.push(format!("V{}(Int)", i));
            } else {
                variants.push(format!("V{}", i));
            }
        }

        let mut arms = String::new();
        for i in 0..num_variants {
            if rng.random_bool(0.5) {
                arms.push_str(&format!("        MyEnum.V{}(x) => x,\n", i));
            } else {
                arms.push_str(&format!("        MyEnum.V{} => {},\n", i, i));
            }
        }

        // Correct Verum syntax: type Name is Variant1 | Variant2 | ...;
        format!(
            "type MyEnum is\n    | {};\n\nfn main() {{\n    let e = MyEnum.V0;\n    let result = match e {{\n{}    }};\n}}\n",
            variants.join("\n    | "),
            arms
        )
    }

    /// Introduce a random syntax error
    fn introduce_syntax_error<R: Rng>(&self, program: &mut String, rng: &mut R) {
        let errors = [
            ("fn ", "fn"),   // Missing space
            ("let ", "le "), // Typo
            ("{", ""),       // Missing brace
            ("}", ""),       // Missing brace
            (";", ""),       // Missing semicolon
            ("(", "(("),     // Extra paren
            (")", "))"),     // Extra paren
            ("->", "->>"),   // Extra char
        ];

        let (from, to) = errors[rng.random_range(0..errors.len())];
        if let Some(pos) = program.find(from) {
            program.replace_range(pos..pos + from.len(), to);
        }
    }

    /// Random primitive type (simple types only)
    fn random_primitive_type<R: Rng>(&self, rng: &mut R) -> &'static str {
        let types = ["Int", "Float", "Bool", "Text", "Char", "()"];
        types[rng.random_range(0..types.len())]
    }

    /// Generate a random type string (includes complex types)
    fn random_type_string<R: Rng>(&self, rng: &mut R) -> String {
        match rng.random_range(0..15) {
            0 => "Int".to_string(),
            1 => "Float".to_string(),
            2 => "Bool".to_string(),
            3 => "Text".to_string(),
            4 => "Char".to_string(),
            5 => "()".to_string(),
            6 => format!("List<{}>", self.random_primitive_type(rng)),
            7 => format!(
                "Map<{}, {}>",
                self.random_primitive_type(rng),
                self.random_primitive_type(rng)
            ),
            8 => format!("Set<{}>", self.random_primitive_type(rng)),
            9 => format!("Maybe<{}>", self.random_primitive_type(rng)),
            10 => format!("Heap<{}>", self.random_primitive_type(rng)),
            11 => format!("Shared<{}>", self.random_primitive_type(rng)),
            12 => {
                // Reference type with three-tier CBGR
                let tier = match rng.random_range(0..3) {
                    0 => "&",         // Tier 0: managed
                    1 => "&checked ", // Tier 1: checked
                    _ => "&",         // Stay with managed for safety
                };
                let mutability = if rng.random_bool(0.3) { "mut " } else { "" };
                format!("{}{}{}", tier, mutability, self.random_primitive_type(rng))
            }
            13 => format!("GenRef<{}>", self.random_primitive_type(rng)),
            _ => format!(
                "({}, {})",
                self.random_primitive_type(rng),
                self.random_primitive_type(rng)
            ),
        }
    }

    /// Random type for generation
    fn random_type<R: Rng>(&self, rng: &mut R) -> VerumType {
        match rng.random_range(0..12) {
            0 => VerumType::Int,
            1 => VerumType::Float,
            2 => VerumType::Bool,
            3 => VerumType::Text,
            4 => VerumType::Char,
            5 => VerumType::Unit,
            6 => VerumType::List(Box::new(VerumType::Int)),
            7 => VerumType::Maybe(Box::new(VerumType::Int)),
            8 => VerumType::Heap(Box::new(VerumType::Int)),
            9 => VerumType::Set(Box::new(VerumType::Text)),
            10 => VerumType::Ref(Box::new(VerumType::Int), RefTier::Tier0),
            _ => VerumType::Tuple(vec![VerumType::Int, VerumType::Bool]),
        }
    }

    /// Convert type to string using correct Verum syntax
    fn type_to_string(&self, ty: &VerumType) -> String {
        match ty {
            VerumType::Int => "Int".to_string(),
            VerumType::Float => "Float".to_string(),
            VerumType::Bool => "Bool".to_string(),
            VerumType::Text => "Text".to_string(),
            VerumType::Char => "Char".to_string(),
            VerumType::Unit => "()".to_string(),
            VerumType::List(inner) => format!("List<{}>", self.type_to_string(inner)),
            VerumType::Map(k, v) => format!(
                "Map<{}, {}>",
                self.type_to_string(k),
                self.type_to_string(v)
            ),
            VerumType::Set(inner) => format!("Set<{}>", self.type_to_string(inner)),
            VerumType::Maybe(inner) => format!("Maybe<{}>", self.type_to_string(inner)),
            VerumType::Heap(inner) => format!("Heap<{}>", self.type_to_string(inner)),
            VerumType::Shared(inner) => format!("Shared<{}>", self.type_to_string(inner)),
            VerumType::Tuple(types) => {
                let inner: Vec<_> = types.iter().map(|t| self.type_to_string(t)).collect();
                format!("({})", inner.join(", "))
            }
            VerumType::Ref(inner, tier) => {
                // Three-tier CBGR references
                let tier_str = match tier {
                    RefTier::Tier0 => "&",           // Managed reference (~15ns overhead)
                    RefTier::Checked => "&checked ", // Compile-time verified (0ns)
                    RefTier::Unsafe => "&unsafe ",   // No checks (0ns, manual proof)
                };
                format!("{}{}", tier_str, self.type_to_string(inner))
            }
            VerumType::Function(params, ret) => {
                let params_str: Vec<_> = params.iter().map(|t| self.type_to_string(t)).collect();
                format!(
                    "fn({}) -> {}",
                    params_str.join(", "),
                    self.type_to_string(ret)
                )
            }
            VerumType::GenRef(inner) => format!("GenRef<{}>", self.type_to_string(inner)),
        }
    }

    /// Generate a random string
    fn random_string<R: Rng>(&self, rng: &mut R, max_len: usize) -> String {
        let len = rng.random_range(0..=max_len);
        let chars: Vec<char> = (0..len)
            .map(|_| {
                let c = rng.random_range(0..62);
                match c {
                    0..=25 => (b'a' + c as u8) as char,
                    26..=51 => (b'A' + (c - 26) as u8) as char,
                    _ => (b'0' + (c - 52) as u8) as char,
                }
            })
            .collect();
        chars.into_iter().collect()
    }

    /// Generate a random character
    fn random_char<R: Rng>(&self, rng: &mut R) -> char {
        let c = rng.random_range(0..62);
        match c {
            0..=25 => (b'a' + c as u8) as char,
            26..=51 => (b'A' + (c - 26) as u8) as char,
            _ => (b'0' + (c - 52) as u8) as char,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_generator_creation() {
        let config = GeneratorConfig::default();
        let generator = Generator::new(config);
        assert_eq!(generator.config.max_depth, 10);
    }

    #[test]
    fn test_grammar_generation() {
        let config = GeneratorConfig {
            kind: GeneratorKind::Grammar,
            ..Default::default()
        };
        let mut generator = Generator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..10 {
            let program = generator.generate(&mut rng);
            assert!(!program.is_empty());
            assert!(program.contains("fn main()") || program.contains("fn "));
        }
    }

    #[test]
    fn test_type_aware_generation() {
        let config = GeneratorConfig {
            kind: GeneratorKind::TypeAware,
            ..Default::default()
        };
        let mut generator = Generator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let program = generator.generate(&mut rng);
        assert!(program.contains("fn main()"));
        assert!(program.contains("let var_"));
    }

    #[test]
    fn test_edge_case_generation() {
        let config = GeneratorConfig {
            kind: GeneratorKind::EdgeCase,
            ..Default::default()
        };
        let mut generator = Generator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..10 {
            let program = generator.generate(&mut rng);
            assert!(!program.is_empty());
        }
    }

    #[test]
    fn test_mixed_generation() {
        let config = GeneratorConfig {
            kind: GeneratorKind::Mixed,
            ..Default::default()
        };
        let mut generator = Generator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..20 {
            let program = generator.generate(&mut rng);
            assert!(!program.is_empty());
        }
    }
}
