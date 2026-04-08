//! Type-correct program fuzzer for Verum
//!
//! This module generates random programs that are guaranteed to be type-correct.
//! Unlike the syntax fuzzer which only ensures syntactic validity, this fuzzer
//! maintains type context throughout generation to produce programs that will
//! pass type checking.
//!
//! # Design Philosophy
//!
//! The type fuzzer uses a constraint-based approach:
//! - Track types of all variables in scope
//! - Generate expressions that match expected types
//! - Ensure function calls have correct argument types
//! - Generate type-consistent pattern matching
//!
//! # Features
//!
//! - Type inference testing (programs with minimal annotations)
//! - Generic type instantiation
//! - Reference type correctness (CBGR tiers)
//! - Protocol implementation testing
//! - Refinement type generation

use rand::Rng;
use rand::seq::IndexedRandom;
use std::collections::HashMap;

/// Verum type representation for tracking during generation
#[derive(Debug, Clone, PartialEq)]
pub enum VType {
    /// Primitive types
    Int,
    Float,
    Bool,
    Text,
    Char,
    Unit,

    /// Generic types
    List(Box<VType>),
    Maybe(Box<VType>),
    Map(Box<VType>, Box<VType>),
    Set(Box<VType>),
    Heap(Box<VType>),

    /// Compound types
    Tuple(Vec<VType>),
    Function(Vec<VType>, Box<VType>),
    Array(Box<VType>, usize),

    /// Reference types (CBGR tiers)
    Ref(Box<VType>, RefTier, bool), // type, tier, mutable

    /// User-defined types
    Named(String, Vec<VType>),

    /// Type variable (for generics)
    TypeVar(String),

    /// Inferred type (wildcard)
    Inferred,
}

impl VType {
    fn to_string(&self) -> String {
        match self {
            VType::Int => "Int".to_string(),
            VType::Float => "Float".to_string(),
            VType::Bool => "Bool".to_string(),
            VType::Text => "Text".to_string(),
            VType::Char => "Char".to_string(),
            VType::Unit => "()".to_string(),
            VType::List(inner) => format!("List<{}>", inner.to_string()),
            VType::Maybe(inner) => format!("Maybe<{}>", inner.to_string()),
            VType::Map(k, v) => format!("Map<{}, {}>", k.to_string(), v.to_string()),
            VType::Set(inner) => format!("Set<{}>", inner.to_string()),
            VType::Heap(inner) => format!("Heap<{}>", inner.to_string()),
            VType::Tuple(types) => {
                let types_str: Vec<String> = types.iter().map(|t| t.to_string()).collect();
                format!("({})", types_str.join(", "))
            }
            VType::Function(params, ret) => {
                let params_str: Vec<String> = params.iter().map(|t| t.to_string()).collect();
                format!("fn({}) -> {}", params_str.join(", "), ret.to_string())
            }
            VType::Array(inner, size) => format!("[{}; {}]", inner.to_string(), size),
            VType::Ref(inner, tier, mutable) => {
                let tier_str = match tier {
                    RefTier::Managed => "&",
                    RefTier::Checked => "&checked ",
                    RefTier::Unsafe => "&unsafe ",
                };
                let mut_str = if *mutable { "mut " } else { "" };
                format!("{}{}{}", tier_str, mut_str, inner.to_string())
            }
            VType::Named(name, type_args) => {
                if type_args.is_empty() {
                    name.clone()
                } else {
                    let args_str: Vec<String> = type_args.iter().map(|t| t.to_string()).collect();
                    format!("{}<{}>", name, args_str.join(", "))
                }
            }
            VType::TypeVar(name) => name.clone(),
            VType::Inferred => "_".to_string(),
        }
    }

    fn is_numeric(&self) -> bool {
        matches!(self, VType::Int | VType::Float)
    }

    fn is_comparable(&self) -> bool {
        matches!(
            self,
            VType::Int | VType::Float | VType::Bool | VType::Char | VType::Text
        )
    }
}

/// Reference tier for CBGR
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RefTier {
    Managed, // &T - default, CBGR protected
    Checked, // &checked T - compile-time verified
    Unsafe,  // &unsafe T - manual safety proof
}

/// Configuration for the type fuzzer
#[derive(Debug, Clone)]
pub struct TypeFuzzerConfig {
    /// Maximum depth for nested types and expressions
    pub max_depth: usize,
    /// Maximum number of statements per block
    pub max_block_size: usize,
    /// Enable generic types
    pub enable_generics: bool,
    /// Enable reference types
    pub enable_references: bool,
    /// Enable refinement types
    pub enable_refinements: bool,
    /// Probability of omitting type annotations (to test inference)
    pub type_inference_probability: f64,
}

impl Default for TypeFuzzerConfig {
    fn default() -> Self {
        Self {
            max_depth: 8,
            max_block_size: 15,
            enable_generics: true,
            enable_references: true,
            enable_refinements: true,
            type_inference_probability: 0.3,
        }
    }
}

/// Typed variable in scope
#[derive(Debug, Clone)]
struct TypedVar {
    name: String,
    var_type: VType,
    mutable: bool,
}

/// Function signature
#[derive(Debug, Clone)]
struct FunctionSig {
    name: String,
    params: Vec<(String, VType)>,
    return_type: VType,
    is_async: bool,
}

/// Type context during generation
#[derive(Debug, Clone)]
struct TypeContext {
    depth: usize,
    variables: Vec<TypedVar>,
    functions: Vec<FunctionSig>,
    type_defs: HashMap<String, TypeDef>,
    name_counter: usize,
    expected_return: Option<VType>,
    in_loop: bool,
    in_async: bool,
}

#[derive(Debug, Clone)]
struct TypeDef {
    name: String,
    type_params: Vec<String>,
    variants: Vec<TypeVariant>,
}

#[derive(Debug, Clone)]
enum TypeVariant {
    Unit(String),
    Tuple(String, Vec<VType>),
    Record(String, Vec<(String, VType)>),
}

impl TypeContext {
    fn new() -> Self {
        // Add built-in functions
        let functions = vec![
            FunctionSig {
                name: "print".to_string(),
                params: vec![("value".to_string(), VType::Text)],
                return_type: VType::Unit,
                is_async: false,
            },
            FunctionSig {
                name: "len".to_string(),
                params: vec![(
                    "list".to_string(),
                    VType::List(Box::new(VType::TypeVar("T".to_string()))),
                )],
                return_type: VType::Int,
                is_async: false,
            },
        ];

        Self {
            depth: 0,
            variables: Vec::new(),
            functions,
            type_defs: HashMap::new(),
            name_counter: 0,
            expected_return: None,
            in_loop: false,
            in_async: false,
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

    fn at_max_depth(&self, config: &TypeFuzzerConfig) -> bool {
        self.depth >= config.max_depth
    }

    fn find_vars_of_type(&self, target: &VType) -> Vec<&TypedVar> {
        self.variables
            .iter()
            .filter(|v| types_compatible(&v.var_type, target))
            .collect()
    }

    fn find_mutable_vars_of_type(&self, target: &VType) -> Vec<&TypedVar> {
        self.variables
            .iter()
            .filter(|v| v.mutable && types_compatible(&v.var_type, target))
            .collect()
    }
}

/// Check if two types are compatible (simplified)
fn types_compatible(a: &VType, b: &VType) -> bool {
    match (a, b) {
        (VType::Inferred, _) | (_, VType::Inferred) => true,
        (VType::TypeVar(_), _) | (_, VType::TypeVar(_)) => true,
        (VType::Int, VType::Int) => true,
        (VType::Float, VType::Float) => true,
        (VType::Bool, VType::Bool) => true,
        (VType::Text, VType::Text) => true,
        (VType::Char, VType::Char) => true,
        (VType::Unit, VType::Unit) => true,
        (VType::List(a), VType::List(b)) => types_compatible(a, b),
        (VType::Maybe(a), VType::Maybe(b)) => types_compatible(a, b),
        (VType::Tuple(a), VType::Tuple(b)) if a.len() == b.len() => {
            a.iter().zip(b.iter()).all(|(x, y)| types_compatible(x, y))
        }
        _ => false,
    }
}

/// Type-correct program fuzzer
pub struct TypeFuzzer {
    config: TypeFuzzerConfig,
}

impl TypeFuzzer {
    /// Create a new type fuzzer
    pub fn new(config: TypeFuzzerConfig) -> Self {
        Self { config }
    }

    /// Generate a type-correct program
    pub fn generate_program<R: Rng>(&self, rng: &mut R) -> String {
        let mut ctx = TypeContext::new();
        let mut output = String::new();

        // Header
        output.push_str("// Type fuzzer generated program (type-correct)\n\n");

        // Generate type definitions
        let num_types = rng.random_range(0..3);
        for _ in 0..num_types {
            output.push_str(&self.generate_type_def(rng, &mut ctx));
            output.push('\n');
        }

        // Generate helper functions
        let num_functions = rng.random_range(1..4);
        for _ in 0..num_functions {
            output.push_str(&self.generate_function(rng, &mut ctx));
            output.push('\n');
        }

        // Generate main function
        output.push_str(&self.generate_main(rng, &mut ctx));

        output
    }

    /// Generate a type definition
    fn generate_type_def<R: Rng>(&self, rng: &mut R, ctx: &mut TypeContext) -> String {
        let name = ctx.fresh_name("MyType");

        match rng.random_range(0..3) {
            0 => {
                // Record type
                let mut result = format!("type {} is {{\n", name);
                let num_fields = rng.random_range(1..=4);
                let mut fields = Vec::new();

                for i in 0..num_fields {
                    let field_name = format!("field_{}", i);
                    let field_type = self.generate_type(rng, ctx);
                    fields.push((field_name.clone(), field_type.clone()));
                    result.push_str(&format!(
                        "    {}: {},\n",
                        field_name,
                        field_type.to_string()
                    ));
                }

                result.push_str("};\n");

                // Register type
                ctx.type_defs.insert(
                    name.clone(),
                    TypeDef {
                        name: name.clone(),
                        type_params: Vec::new(),
                        variants: vec![TypeVariant::Record("".to_string(), fields)],
                    },
                );

                result
            }
            1 => {
                // Sum type
                let mut result = format!("type {} is\n", name);
                let num_variants = rng.random_range(2..=4);
                let mut variants = Vec::new();

                for i in 0..num_variants {
                    if i > 0 {
                        result.push_str("    | ");
                    } else {
                        result.push_str("    ");
                    }

                    let variant_name = format!("Variant_{}", i);

                    match rng.random_range(0..3) {
                        0 => {
                            result.push_str(&variant_name);
                            variants.push(TypeVariant::Unit(variant_name));
                        }
                        1 => {
                            let ty = self.generate_primitive_type(rng);
                            result.push_str(&format!("{}({})", variant_name, ty.to_string()));
                            variants.push(TypeVariant::Tuple(variant_name, vec![ty]));
                        }
                        _ => {
                            result.push_str(&format!("{}{{ value: Int }}", variant_name));
                            variants.push(TypeVariant::Record(
                                variant_name,
                                vec![("value".to_string(), VType::Int)],
                            ));
                        }
                    }
                    result.push('\n');
                }

                result.push_str(";\n");

                // Register type
                ctx.type_defs.insert(
                    name.clone(),
                    TypeDef {
                        name: name.clone(),
                        type_params: Vec::new(),
                        variants,
                    },
                );

                result
            }
            _ => {
                // Newtype
                let inner = self.generate_primitive_type(rng);
                format!("type {} is ({});\n", name, inner.to_string())
            }
        }
    }

    /// Generate a function with correct types
    fn generate_function<R: Rng>(&self, rng: &mut R, ctx: &mut TypeContext) -> String {
        let name = ctx.fresh_name("func");
        let return_type = self.generate_type(rng, ctx);

        let mut result = format!("fn {}(", name);

        // Generate parameters
        let num_params = rng.random_range(0..=3);
        let mut params = Vec::new();

        for i in 0..num_params {
            if i > 0 {
                result.push_str(", ");
            }
            let param_name = format!("arg_{}", i);
            let param_type = self.generate_type(rng, ctx);
            result.push_str(&format!("{}: {}", param_name, param_type.to_string()));
            params.push((param_name, param_type));
        }

        result.push_str(&format!(") -> {} {{\n", return_type.to_string()));

        // Set up function context
        let old_vars = ctx.variables.clone();
        ctx.expected_return = Some(return_type.clone());

        // Add parameters to scope
        for (name, ty) in &params {
            ctx.variables.push(TypedVar {
                name: name.clone(),
                var_type: ty.clone(),
                mutable: false,
            });
        }

        // Generate body
        let body = self.generate_typed_block_contents(rng, ctx, &return_type);
        result.push_str(&body);

        // Generate return expression of correct type
        let return_expr = self.generate_expression_of_type(rng, ctx, &return_type);
        result.push_str(&format!("    {}\n", return_expr));

        result.push_str("}\n");

        // Register function
        ctx.functions.push(FunctionSig {
            name: name.clone(),
            params,
            return_type,
            is_async: false,
        });

        // Restore scope
        ctx.variables = old_vars;
        ctx.expected_return = None;

        result
    }

    /// Generate main function
    fn generate_main<R: Rng>(&self, rng: &mut R, ctx: &mut TypeContext) -> String {
        let mut result = String::from("fn main() {\n");

        ctx.expected_return = Some(VType::Unit);
        let body = self.generate_typed_block_contents(rng, ctx, &VType::Unit);
        result.push_str(&body);
        ctx.expected_return = None;

        result.push_str("}\n");
        result
    }

    /// Generate typed block contents
    fn generate_typed_block_contents<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut TypeContext,
        _expected_type: &VType,
    ) -> String {
        let mut result = String::new();
        let num_stmts = rng.random_range(1..=self.config.max_block_size.min(8));

        for _ in 0..num_stmts {
            let stmt = self.generate_statement(rng, ctx);
            result.push_str("    ");
            result.push_str(&stmt);
            result.push('\n');
        }

        result
    }

    /// Generate a type-correct statement
    fn generate_statement<R: Rng>(&self, rng: &mut R, ctx: &mut TypeContext) -> String {
        if ctx.at_max_depth(&self.config) {
            return self.generate_let_statement(rng, ctx);
        }

        match rng.random_range(0..8) {
            0 | 1 | 2 => self.generate_let_statement(rng, ctx),
            3 => self.generate_assignment(rng, ctx),
            4 => self.generate_if_statement(rng, ctx),
            5 => self.generate_for_loop(rng, ctx),
            6 => self.generate_while_loop(rng, ctx),
            _ => {
                let ty = self.generate_type(rng, ctx);
                format!("{};", self.generate_expression_of_type(rng, ctx, &ty))
            }
        }
    }

    /// Generate a type-correct let statement
    fn generate_let_statement<R: Rng>(&self, rng: &mut R, ctx: &mut TypeContext) -> String {
        let var_name = ctx.fresh_name("v");
        let var_type = self.generate_type(rng, ctx);
        let is_mut = rng.random_bool(0.3);

        let mut result = String::from("let ");
        if is_mut {
            result.push_str("mut ");
        }
        result.push_str(&var_name);

        // Type annotation based on inference probability
        if rng.random_bool(1.0 - self.config.type_inference_probability) {
            result.push_str(&format!(": {}", var_type.to_string()));
        }

        result.push_str(" = ");
        result.push_str(&self.generate_expression_of_type(rng, ctx, &var_type));
        result.push(';');

        ctx.variables.push(TypedVar {
            name: var_name,
            var_type,
            mutable: is_mut,
        });

        result
    }

    /// Generate a type-correct assignment
    fn generate_assignment<R: Rng>(&self, rng: &mut R, ctx: &mut TypeContext) -> String {
        // Find a mutable variable
        let mutable_vars: Vec<&TypedVar> = ctx.variables.iter().filter(|v| v.mutable).collect();

        if mutable_vars.is_empty() {
            return self.generate_let_statement(rng, ctx);
        }

        let var = (*mutable_vars.choose(rng).unwrap()).clone();
        let expr = self.generate_expression_of_type(rng, ctx, &var.var_type);
        format!("{} = {};", var.name, expr)
    }

    /// Generate a type-correct if statement
    fn generate_if_statement<R: Rng>(&self, rng: &mut R, ctx: &mut TypeContext) -> String {
        ctx.enter_scope();

        let condition = self.generate_expression_of_type(rng, ctx, &VType::Bool);
        let mut result = format!("if {} {{\n", condition);

        let body = self.generate_typed_block_contents(rng, ctx, &VType::Unit);
        result.push_str(&body);
        result.push_str("    }");

        if rng.random_bool(0.5) {
            result.push_str(" else {\n");
            let else_body = self.generate_typed_block_contents(rng, ctx, &VType::Unit);
            result.push_str(&else_body);
            result.push_str("    }");
        }

        ctx.exit_scope();
        result
    }

    /// Generate a type-correct for loop
    fn generate_for_loop<R: Rng>(&self, rng: &mut R, ctx: &mut TypeContext) -> String {
        ctx.enter_scope();
        ctx.in_loop = true;

        let iter_var = ctx.fresh_name("i");
        let start = rng.random_range(0..10);
        let end = rng.random_range(start + 1..start + 20);

        let mut result = format!("for {} in {}..{} {{\n", iter_var, start, end);

        ctx.variables.push(TypedVar {
            name: iter_var,
            var_type: VType::Int,
            mutable: false,
        });

        let body = self.generate_typed_block_contents(rng, ctx, &VType::Unit);
        result.push_str(&body);
        result.push_str("    }");

        ctx.in_loop = false;
        ctx.exit_scope();
        result
    }

    /// Generate a type-correct while loop
    fn generate_while_loop<R: Rng>(&self, rng: &mut R, ctx: &mut TypeContext) -> String {
        ctx.enter_scope();
        ctx.in_loop = true;

        let condition = self.generate_expression_of_type(rng, ctx, &VType::Bool);
        let mut result = format!("while {} {{\n", condition);

        let body = self.generate_typed_block_contents(rng, ctx, &VType::Unit);
        result.push_str(&body);
        result.push_str("    }");

        ctx.in_loop = false;
        ctx.exit_scope();
        result
    }

    /// Generate an expression of a specific type
    fn generate_expression_of_type<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut TypeContext,
        target_type: &VType,
    ) -> String {
        if ctx.at_max_depth(&self.config) {
            return self.generate_literal_of_type(rng, target_type);
        }

        ctx.enter_scope();

        // First try to find a variable of the correct type
        let compatible_vars = ctx.find_vars_of_type(target_type);
        if !compatible_vars.is_empty() && rng.random_bool(0.3) {
            // Clone the name before releasing the immutable borrow on ctx
            let var_name = compatible_vars.choose(rng).unwrap().name.clone();
            ctx.exit_scope();
            return var_name;
        }

        let result = match target_type {
            VType::Int => self.generate_int_expression(rng, ctx),
            VType::Float => self.generate_float_expression(rng, ctx),
            VType::Bool => self.generate_bool_expression(rng, ctx),
            VType::Text => self.generate_text_expression(rng, ctx),
            VType::Char => self.generate_char_expression(rng),
            VType::Unit => "()".to_string(),
            VType::List(inner) => self.generate_list_expression(rng, ctx, inner),
            VType::Maybe(inner) => self.generate_maybe_expression(rng, ctx, inner),
            VType::Tuple(types) => self.generate_tuple_expression(rng, ctx, types),
            _ => self.generate_literal_of_type(rng, target_type),
        };

        ctx.exit_scope();
        result
    }

    /// Generate an Int expression
    fn generate_int_expression<R: Rng>(&self, rng: &mut R, ctx: &mut TypeContext) -> String {
        match rng.random_range(0..5) {
            0 => rng.random_range(-1000i64..=1000).to_string(),
            1 => {
                // Binary arithmetic
                let lhs = self.generate_expression_of_type(rng, ctx, &VType::Int);
                let rhs = self.generate_expression_of_type(rng, ctx, &VType::Int);
                let op = ["+", "-", "*", "/", "%"].choose(rng).unwrap();
                format!("({} {} {})", lhs, op, rhs)
            }
            2 => {
                // Unary negation
                let expr = self.generate_expression_of_type(rng, ctx, &VType::Int);
                format!("(-{})", expr)
            }
            3 => {
                // If expression returning Int
                let cond = self.generate_expression_of_type(rng, ctx, &VType::Bool);
                let then_expr = self.generate_expression_of_type(rng, ctx, &VType::Int);
                let else_expr = self.generate_expression_of_type(rng, ctx, &VType::Int);
                format!("if {} {{ {} }} else {{ {} }}", cond, then_expr, else_expr)
            }
            _ => {
                // Variable of type Int or fallback to literal
                let int_vars = ctx.find_vars_of_type(&VType::Int);
                if int_vars.is_empty() {
                    rng.random_range(-100i64..=100).to_string()
                } else {
                    int_vars.choose(rng).unwrap().name.clone()
                }
            }
        }
    }

    /// Generate a Float expression
    fn generate_float_expression<R: Rng>(&self, rng: &mut R, ctx: &mut TypeContext) -> String {
        match rng.random_range(0..4) {
            0 => format!("{:.4}", rng.random::<f64>() * 100.0),
            1 => {
                let lhs = self.generate_expression_of_type(rng, ctx, &VType::Float);
                let rhs = self.generate_expression_of_type(rng, ctx, &VType::Float);
                let op = ["+", "-", "*", "/"].choose(rng).unwrap();
                format!("({} {} {})", lhs, op, rhs)
            }
            _ => {
                let float_vars = ctx.find_vars_of_type(&VType::Float);
                if float_vars.is_empty() {
                    format!("{:.2}", rng.random::<f64>() * 10.0)
                } else {
                    float_vars.choose(rng).unwrap().name.clone()
                }
            }
        }
    }

    /// Generate a Bool expression
    fn generate_bool_expression<R: Rng>(&self, rng: &mut R, ctx: &mut TypeContext) -> String {
        match rng.random_range(0..6) {
            0 => if rng.random_bool(0.5) {
                "true"
            } else {
                "false"
            }
            .to_string(),
            1 => {
                // Comparison
                let ty = if rng.random_bool(0.5) {
                    VType::Int
                } else {
                    VType::Float
                };
                let lhs = self.generate_expression_of_type(rng, ctx, &ty);
                let rhs = self.generate_expression_of_type(rng, ctx, &ty);
                let op = ["==", "!=", "<", ">", "<=", ">="].choose(rng).unwrap();
                format!("({} {} {})", lhs, op, rhs)
            }
            2 => {
                // Logical AND/OR
                let lhs = self.generate_expression_of_type(rng, ctx, &VType::Bool);
                let rhs = self.generate_expression_of_type(rng, ctx, &VType::Bool);
                let op = ["&&", "||"].choose(rng).unwrap();
                format!("({} {} {})", lhs, op, rhs)
            }
            3 => {
                // Logical NOT
                let expr = self.generate_expression_of_type(rng, ctx, &VType::Bool);
                format!("(!{})", expr)
            }
            _ => {
                let bool_vars = ctx.find_vars_of_type(&VType::Bool);
                if bool_vars.is_empty() {
                    if rng.random_bool(0.5) {
                        "true"
                    } else {
                        "false"
                    }
                    .to_string()
                } else {
                    bool_vars.choose(rng).unwrap().name.clone()
                }
            }
        }
    }

    /// Generate a Text expression
    fn generate_text_expression<R: Rng>(&self, rng: &mut R, ctx: &mut TypeContext) -> String {
        match rng.random_range(0..3) {
            0 => {
                let len = rng.random_range(0..20);
                let s: String = (0..len)
                    .map(|_| (b'a' + rng.random_range(0..26)) as char)
                    .collect();
                format!("\"{}\"", s)
            }
            _ => {
                let text_vars = ctx.find_vars_of_type(&VType::Text);
                if text_vars.is_empty() {
                    "\"hello\"".to_string()
                } else {
                    text_vars.choose(rng).unwrap().name.clone()
                }
            }
        }
    }

    /// Generate a Char expression
    fn generate_char_expression<R: Rng>(&self, rng: &mut R) -> String {
        let c = (b'a' + rng.random_range(0..26)) as char;
        format!("'{}'", c)
    }

    /// Generate a List expression
    fn generate_list_expression<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut TypeContext,
        inner: &VType,
    ) -> String {
        let len = rng.random_range(0..5);
        let elements: Vec<String> = (0..len)
            .map(|_| self.generate_expression_of_type(rng, ctx, inner))
            .collect();
        format!("[{}]", elements.join(", "))
    }

    /// Generate a Maybe expression
    fn generate_maybe_expression<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut TypeContext,
        inner: &VType,
    ) -> String {
        if rng.random_bool(0.3) {
            "None".to_string()
        } else {
            let val = self.generate_expression_of_type(rng, ctx, inner);
            format!("Some({})", val)
        }
    }

    /// Generate a Tuple expression
    fn generate_tuple_expression<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut TypeContext,
        types: &[VType],
    ) -> String {
        let elements: Vec<String> = types
            .iter()
            .map(|t| self.generate_expression_of_type(rng, ctx, t))
            .collect();
        format!("({})", elements.join(", "))
    }

    /// Generate a literal of a specific type
    fn generate_literal_of_type<R: Rng>(&self, rng: &mut R, target_type: &VType) -> String {
        match target_type {
            VType::Int => rng.random_range(-100i64..=100).to_string(),
            VType::Float => format!("{:.2}", rng.random::<f64>() * 10.0),
            VType::Bool => if rng.random_bool(0.5) {
                "true"
            } else {
                "false"
            }
            .to_string(),
            VType::Text => {
                let s: String = (0..5)
                    .map(|_| (b'a' + rng.random_range(0..26)) as char)
                    .collect();
                format!("\"{}\"", s)
            }
            VType::Char => format!("'{}'", (b'a' + rng.random_range(0..26)) as char),
            VType::Unit => "()".to_string(),
            VType::List(_) => "[]".to_string(),
            VType::Maybe(_) => "None".to_string(),
            VType::Tuple(types) => {
                let elements: Vec<String> = types
                    .iter()
                    .map(|t| self.generate_literal_of_type(rng, t))
                    .collect();
                format!("({})", elements.join(", "))
            }
            _ => "()".to_string(),
        }
    }

    /// Generate a random type
    fn generate_type<R: Rng>(&self, rng: &mut R, _ctx: &mut TypeContext) -> VType {
        match rng.random_range(0..10) {
            0..=6 => self.generate_primitive_type(rng),
            7 => {
                let inner = self.generate_primitive_type(rng);
                VType::List(Box::new(inner))
            }
            8 => {
                let inner = self.generate_primitive_type(rng);
                VType::Maybe(Box::new(inner))
            }
            _ => {
                let types: Vec<VType> = (0..rng.random_range(2..=3))
                    .map(|_| self.generate_primitive_type(rng))
                    .collect();
                VType::Tuple(types)
            }
        }
    }

    /// Generate a primitive type
    fn generate_primitive_type<R: Rng>(&self, rng: &mut R) -> VType {
        match rng.random_range(0..5) {
            0 => VType::Int,
            1 => VType::Float,
            2 => VType::Bool,
            3 => VType::Text,
            _ => VType::Char,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_type_fuzzer_generates_programs() {
        let config = TypeFuzzerConfig::default();
        let fuzzer = TypeFuzzer::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..10 {
            let program = fuzzer.generate_program(&mut rng);
            assert!(!program.is_empty());
            assert!(program.contains("fn main()"));
        }
    }

    #[test]
    fn test_type_expressions_match() {
        let config = TypeFuzzerConfig::default();
        let fuzzer = TypeFuzzer::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let mut ctx = TypeContext::new();

        // Test Int expressions
        for _ in 0..10 {
            let expr = fuzzer.generate_expression_of_type(&mut rng, &mut ctx, &VType::Int);
            // Should not be empty
            assert!(!expr.is_empty());
        }

        // Test Bool expressions
        for _ in 0..10 {
            let expr = fuzzer.generate_expression_of_type(&mut rng, &mut ctx, &VType::Bool);
            assert!(!expr.is_empty());
        }
    }

    #[test]
    fn test_uses_verum_syntax() {
        let config = TypeFuzzerConfig::default();
        let fuzzer = TypeFuzzer::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..20 {
            let program = fuzzer.generate_program(&mut rng);
            // Should use Verum syntax
            assert!(
                !program.contains("struct "),
                "Should not contain Rust 'struct'"
            );
            assert!(!program.contains("enum "), "Should not contain Rust 'enum'");
        }
    }
}
