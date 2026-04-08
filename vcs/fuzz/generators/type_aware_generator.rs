//! Type-aware program generator for Verum
//!
//! This module generates programs that are guaranteed to be type-correct.
//! Unlike the grammar generator which only ensures syntactic validity,
//! this generator maintains full type information during generation and
//! ensures all expressions have consistent types.
//!
//! # Type Tracking
//!
//! The generator maintains:
//! - A type environment mapping variables to their types
//! - A function signature registry
//! - Constraints for generic type parameters
//!
//! # Generated Programs
//!
//! All generated programs should:
//! - Pass type checking without errors
//! - Have correct type annotations
//! - Use variables according to their declared types

use rand::Rng;
use rand::distr::Distribution;
use rand::distr::weighted::WeightedIndex;
use rand::seq::IndexedRandom;
use std::collections::HashMap;

/// Represents Verum types for generation
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VerumType {
    /// Primitive types
    Int,
    Float,
    Bool,
    Text,
    Unit,
    /// List type with element type
    List(Box<VerumType>),
    /// Map type with key and value types
    Map(Box<VerumType>, Box<VerumType>),
    /// Maybe (optional) type
    Maybe(Box<VerumType>),
    /// Set type
    Set(Box<VerumType>),
    /// Tuple type
    Tuple(Vec<VerumType>),
    /// Function type (params -> return)
    Function(Vec<VerumType>, Box<VerumType>),
    /// Reference types (CBGR tiers)
    Ref(Box<VerumType>, RefTier),
    /// Named type (struct/enum)
    Named(String),
    /// Generic type parameter
    TypeVar(String),
}

/// CBGR reference tiers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefTier {
    /// Tier 0: Full CBGR protection (~15ns overhead)
    Managed,
    /// Tier 1: Compiler-proven safe (0ns overhead)
    Checked,
    /// Tier 2: Manual safety proof (0ns overhead)
    Unsafe,
}

impl VerumType {
    /// Convert type to Verum syntax string
    pub fn to_syntax(&self) -> String {
        match self {
            VerumType::Int => "Int".to_string(),
            VerumType::Float => "Float".to_string(),
            VerumType::Bool => "Bool".to_string(),
            VerumType::Text => "Text".to_string(),
            VerumType::Unit => "Unit".to_string(),
            VerumType::List(inner) => format!("List<{}>", inner.to_syntax()),
            VerumType::Map(k, v) => format!("Map<{}, {}>", k.to_syntax(), v.to_syntax()),
            VerumType::Maybe(inner) => format!("Maybe<{}>", inner.to_syntax()),
            VerumType::Set(inner) => format!("Set<{}>", inner.to_syntax()),
            VerumType::Tuple(types) => {
                let inner: Vec<_> = types.iter().map(|t| t.to_syntax()).collect();
                format!("({})", inner.join(", "))
            }
            VerumType::Function(params, ret) => {
                let params_str: Vec<_> = params.iter().map(|t| t.to_syntax()).collect();
                format!("fn({}) -> {}", params_str.join(", "), ret.to_syntax())
            }
            VerumType::Ref(inner, tier) => {
                let prefix = match tier {
                    RefTier::Managed => "&",
                    RefTier::Checked => "&checked ",
                    RefTier::Unsafe => "&unsafe ",
                };
                format!("{}{}", prefix, inner.to_syntax())
            }
            VerumType::Named(name) => name.clone(),
            VerumType::TypeVar(name) => name.clone(),
        }
    }

    /// Check if this type can be compared with ==
    pub fn is_equatable(&self) -> bool {
        match self {
            VerumType::Int
            | VerumType::Float
            | VerumType::Bool
            | VerumType::Text
            | VerumType::Unit => true,
            VerumType::List(inner) | VerumType::Set(inner) | VerumType::Maybe(inner) => {
                inner.is_equatable()
            }
            VerumType::Tuple(types) => types.iter().all(|t| t.is_equatable()),
            _ => false,
        }
    }

    /// Check if this type supports arithmetic operations
    pub fn is_numeric(&self) -> bool {
        matches!(self, VerumType::Int | VerumType::Float)
    }

    /// Check if this type is orderable
    pub fn is_orderable(&self) -> bool {
        matches!(self, VerumType::Int | VerumType::Float | VerumType::Text)
    }
}

/// Type environment for tracking variable types
#[derive(Debug, Clone)]
struct TypeEnv {
    /// Variables in scope with their types
    variables: HashMap<String, VerumType>,
    /// Functions with their signatures
    functions: HashMap<String, FunctionSig>,
    /// Type definitions
    type_defs: HashMap<String, TypeDef>,
    /// Scope stack for nested scopes
    scope_stack: Vec<HashMap<String, VerumType>>,
}

/// Function signature
#[derive(Debug, Clone)]
struct FunctionSig {
    params: Vec<(String, VerumType)>,
    return_type: VerumType,
    is_async: bool,
}

/// Type definition (struct or enum)
#[derive(Debug, Clone)]
enum TypeDef {
    Struct {
        fields: Vec<(String, VerumType)>,
    },
    Enum {
        variants: Vec<(String, Option<VerumType>)>,
    },
}

impl TypeEnv {
    fn new() -> Self {
        let mut env = Self {
            variables: HashMap::new(),
            functions: HashMap::new(),
            type_defs: HashMap::new(),
            scope_stack: Vec::new(),
        };

        // Add built-in functions
        env.functions.insert(
            "print".to_string(),
            FunctionSig {
                params: vec![("msg".to_string(), VerumType::Text)],
                return_type: VerumType::Unit,
                is_async: false,
            },
        );

        env.functions.insert(
            "len".to_string(),
            FunctionSig {
                params: vec![(
                    "list".to_string(),
                    VerumType::List(Box::new(VerumType::TypeVar("T".to_string()))),
                )],
                return_type: VerumType::Int,
                is_async: false,
            },
        );

        env
    }

    fn push_scope(&mut self) {
        self.scope_stack.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        if let Some(scope) = self.scope_stack.pop() {
            for key in scope.keys() {
                self.variables.remove(key);
            }
        }
    }

    fn add_variable(&mut self, name: String, ty: VerumType) {
        self.variables.insert(name.clone(), ty);
        if let Some(scope) = self.scope_stack.last_mut() {
            scope.insert(name, VerumType::Unit);
        }
    }

    fn get_variable(&self, name: &str) -> Option<&VerumType> {
        self.variables.get(name)
    }

    fn get_variables_of_type(&self, ty: &VerumType) -> Vec<String> {
        self.variables
            .iter()
            .filter(|(_, t)| *t == ty)
            .map(|(name, _)| name.clone())
            .collect()
    }

    fn get_numeric_variables(&self) -> Vec<(String, VerumType)> {
        self.variables
            .iter()
            .filter(|(_, t)| t.is_numeric())
            .map(|(name, ty)| (name.clone(), ty.clone()))
            .collect()
    }
}

/// Configuration for type-aware generator
#[derive(Debug, Clone)]
pub struct TypeAwareConfig {
    pub max_depth: usize,
    pub max_statements: usize,
    pub max_list_size: usize,
    pub enable_generics: bool,
    pub enable_refinements: bool,
    pub enable_cbgr: bool,
    pub enable_async: bool,
}

impl Default for TypeAwareConfig {
    fn default() -> Self {
        Self {
            max_depth: 4,
            max_statements: 10,
            max_list_size: 5,
            enable_generics: true,
            enable_refinements: false,
            enable_cbgr: true,
            enable_async: true,
        }
    }
}

/// Type-aware program generator
pub struct TypeAwareGenerator {
    config: TypeAwareConfig,
    type_weights: WeightedIndex<u32>,
}

impl TypeAwareGenerator {
    /// Create a new type-aware generator
    pub fn new(config: TypeAwareConfig) -> Self {
        let type_weights = WeightedIndex::new(&[
            30, // Int
            20, // Float
            15, // Bool
            20, // Text
            5,  // Unit
            5,  // List
            3,  // Maybe
            2,  // Tuple
        ])
        .unwrap();

        Self {
            config,
            type_weights,
        }
    }

    /// Generate a complete type-correct program
    pub fn generate_program<R: Rng>(&self, rng: &mut R) -> String {
        let mut env = TypeEnv::new();
        let mut program = String::new();

        program.push_str("// Type-correct generated program\n\n");
        program.push_str("use verum_std::core::{List, Text, Map, Maybe, Set}\n\n");

        // Generate struct definitions
        let num_structs = rng.random_range(0..3);
        for i in 0..num_structs {
            let (struct_def, name) = self.generate_struct_def(rng, &env, i);
            program.push_str(&struct_def);
            program.push('\n');

            // Register the type
            let fields = self.extract_struct_fields(&struct_def);
            env.type_defs.insert(name, TypeDef::Struct { fields });
        }

        // Generate helper functions
        let num_funcs = rng.random_range(1..4);
        for i in 0..num_funcs {
            let func = self.generate_function(rng, &mut env, i);
            program.push_str(&func);
            program.push('\n');
        }

        // Generate main function
        program.push_str(&self.generate_main(rng, &mut env));

        program
    }

    fn generate_struct_def<R: Rng>(
        &self,
        rng: &mut R,
        _env: &TypeEnv,
        idx: usize,
    ) -> (String, String) {
        let name = format!("Data_{}", idx);
        let mut result = format!("struct {} {{\n", name);

        let num_fields = rng.random_range(2..=4);
        for i in 0..num_fields {
            let field_type = self.generate_simple_type(rng);
            result.push_str(&format!("    field_{}: {},\n", i, field_type.to_syntax()));
        }

        result.push_str("}\n");
        (result, name)
    }

    fn extract_struct_fields(&self, _struct_def: &str) -> Vec<(String, VerumType)> {
        // Simplified extraction - in production would parse properly
        Vec::new()
    }

    fn generate_function<R: Rng>(&self, rng: &mut R, env: &mut TypeEnv, idx: usize) -> String {
        let name = format!("helper_{}", idx);
        let is_async = self.config.enable_async && rng.random_bool(0.2);

        // Generate parameter types
        let num_params = rng.random_range(0..=3);
        let mut params: Vec<(String, VerumType)> = Vec::new();
        for i in 0..num_params {
            let param_name = format!("p_{}", i);
            let param_type = self.generate_simple_type(rng);
            params.push((param_name, param_type));
        }

        // Generate return type
        let return_type = self.generate_simple_type(rng);

        // Build signature
        let mut result = String::new();
        if is_async {
            result.push_str("async ");
        }
        result.push_str(&format!("fn {}(", name));

        let params_str: Vec<String> = params
            .iter()
            .map(|(n, t)| format!("{}: {}", n, t.to_syntax()))
            .collect();
        result.push_str(&params_str.join(", "));
        result.push_str(&format!(") -> {} {{\n", return_type.to_syntax()));

        // Register function
        env.functions.insert(
            name.clone(),
            FunctionSig {
                params: params.clone(),
                return_type: return_type.clone(),
                is_async,
            },
        );

        // Generate body
        env.push_scope();
        for (param_name, param_type) in &params {
            env.add_variable(param_name.clone(), param_type.clone());
        }

        let body = self.generate_typed_block(rng, env, &return_type, 0);
        result.push_str(&body);

        env.pop_scope();
        result.push_str("}\n");
        result
    }

    fn generate_main<R: Rng>(&self, rng: &mut R, env: &mut TypeEnv) -> String {
        let mut result = String::from("fn main() {\n");

        env.push_scope();
        let body = self.generate_typed_block(rng, env, &VerumType::Unit, 0);
        result.push_str(&body);
        env.pop_scope();

        result.push_str("}\n");
        result
    }

    fn generate_typed_block<R: Rng>(
        &self,
        rng: &mut R,
        env: &mut TypeEnv,
        expected_type: &VerumType,
        depth: usize,
    ) -> String {
        let mut result = String::new();

        // Generate statements
        let num_stmts = rng.random_range(1..=self.config.max_statements);
        for _ in 0..num_stmts - 1 {
            let stmt = self.generate_statement(rng, env, depth);
            result.push_str("    ");
            result.push_str(&stmt);
            result.push('\n');
        }

        // Final expression should match expected type
        result.push_str("    ");
        if *expected_type == VerumType::Unit {
            let stmt = self.generate_statement(rng, env, depth);
            result.push_str(&stmt);
        } else {
            let expr = self.generate_typed_expression(rng, env, expected_type, depth);
            result.push_str(&expr);
        }
        result.push('\n');

        result
    }

    fn generate_statement<R: Rng>(&self, rng: &mut R, env: &mut TypeEnv, depth: usize) -> String {
        match rng.random_range(0..5) {
            0..=2 => self.generate_let_statement(rng, env, depth),
            3 => self.generate_if_statement(rng, env, depth),
            _ => {
                // Expression statement
                let ty = self.generate_simple_type(rng);
                format!("{};", self.generate_typed_expression(rng, env, &ty, depth))
            }
        }
    }

    fn generate_let_statement<R: Rng>(
        &self,
        rng: &mut R,
        env: &mut TypeEnv,
        depth: usize,
    ) -> String {
        let var_name = format!("v_{}", rng.random_range(0..1000));
        let var_type = self.generate_simple_type(rng);
        let expr = self.generate_typed_expression(rng, env, &var_type, depth);

        env.add_variable(var_name.clone(), var_type.clone());

        format!("let {}: {} = {};", var_name, var_type.to_syntax(), expr)
    }

    fn generate_if_statement<R: Rng>(
        &self,
        rng: &mut R,
        env: &mut TypeEnv,
        depth: usize,
    ) -> String {
        let cond = self.generate_typed_expression(rng, env, &VerumType::Bool, depth);
        let mut result = format!("if {} {{\n", cond);

        env.push_scope();
        let then_stmt = self.generate_statement(rng, env, depth + 1);
        result.push_str(&format!("        {}\n", then_stmt));
        env.pop_scope();

        result.push_str("    }");

        if rng.random_bool(0.5) {
            result.push_str(" else {\n");
            env.push_scope();
            let else_stmt = self.generate_statement(rng, env, depth + 1);
            result.push_str(&format!("        {}\n", else_stmt));
            env.pop_scope();
            result.push_str("    }");
        }

        result
    }

    /// Generate an expression of the specified type
    pub fn generate_typed_expression<R: Rng>(
        &self,
        rng: &mut R,
        env: &mut TypeEnv,
        expected_type: &VerumType,
        depth: usize,
    ) -> String {
        if depth >= self.config.max_depth {
            return self.generate_literal_of_type(rng, expected_type);
        }

        // Try to use an existing variable of the correct type
        if rng.random_bool(0.3) {
            let vars = env.get_variables_of_type(expected_type);
            if !vars.is_empty() {
                return vars.choose(rng).unwrap().clone();
            }
        }

        match expected_type {
            VerumType::Int => self.generate_int_expression(rng, env, depth),
            VerumType::Float => self.generate_float_expression(rng, env, depth),
            VerumType::Bool => self.generate_bool_expression(rng, env, depth),
            VerumType::Text => self.generate_text_expression(rng, env, depth),
            VerumType::Unit => "()".to_string(),
            VerumType::List(inner) => self.generate_list_expression(rng, env, inner, depth),
            VerumType::Maybe(inner) => self.generate_maybe_expression(rng, env, inner, depth),
            VerumType::Tuple(types) => self.generate_tuple_expression(rng, env, types, depth),
            _ => self.generate_literal_of_type(rng, expected_type),
        }
    }

    fn generate_int_expression<R: Rng>(
        &self,
        rng: &mut R,
        env: &mut TypeEnv,
        depth: usize,
    ) -> String {
        match rng.random_range(0..6) {
            0 => rng.random_range(-1000..1000).to_string(),
            1 => {
                // Binary arithmetic
                let lhs = self.generate_typed_expression(rng, env, &VerumType::Int, depth + 1);
                let rhs = self.generate_typed_expression(rng, env, &VerumType::Int, depth + 1);
                let op = ["+", "-", "*", "/", "%"].choose(rng).unwrap();
                format!("({} {} {})", lhs, op, rhs)
            }
            2 => {
                // Unary negation
                let expr = self.generate_typed_expression(rng, env, &VerumType::Int, depth + 1);
                format!("(-{})", expr)
            }
            3 => {
                // If expression
                let cond = self.generate_typed_expression(rng, env, &VerumType::Bool, depth + 1);
                let then_expr =
                    self.generate_typed_expression(rng, env, &VerumType::Int, depth + 1);
                let else_expr =
                    self.generate_typed_expression(rng, env, &VerumType::Int, depth + 1);
                format!("if {} {{ {} }} else {{ {} }}", cond, then_expr, else_expr)
            }
            4 => {
                // Use existing Int variable
                let int_vars = env.get_variables_of_type(&VerumType::Int);
                if !int_vars.is_empty() {
                    int_vars.choose(rng).unwrap().clone()
                } else {
                    rng.random_range(0..100).to_string()
                }
            }
            _ => {
                // List length
                let list_expr = self.generate_typed_expression(
                    rng,
                    env,
                    &VerumType::List(Box::new(VerumType::Int)),
                    depth + 1,
                );
                format!("len({})", list_expr)
            }
        }
    }

    fn generate_float_expression<R: Rng>(
        &self,
        rng: &mut R,
        env: &mut TypeEnv,
        depth: usize,
    ) -> String {
        match rng.random_range(0..4) {
            0 => format!("{:.2}", rng.random::<f64>() * 100.0),
            1 => {
                let lhs = self.generate_typed_expression(rng, env, &VerumType::Float, depth + 1);
                let rhs = self.generate_typed_expression(rng, env, &VerumType::Float, depth + 1);
                let op = ["+", "-", "*", "/"].choose(rng).unwrap();
                format!("({} {} {})", lhs, op, rhs)
            }
            2 => {
                // Int to Float conversion
                let int_expr = self.generate_typed_expression(rng, env, &VerumType::Int, depth + 1);
                format!("({} as Float)", int_expr)
            }
            _ => {
                let float_vars = env.get_variables_of_type(&VerumType::Float);
                if !float_vars.is_empty() {
                    float_vars.choose(rng).unwrap().clone()
                } else {
                    format!("{:.2}", rng.random::<f64>() * 100.0)
                }
            }
        }
    }

    fn generate_bool_expression<R: Rng>(
        &self,
        rng: &mut R,
        env: &mut TypeEnv,
        depth: usize,
    ) -> String {
        match rng.random_range(0..6) {
            0 => if rng.random_bool(0.5) {
                "true"
            } else {
                "false"
            }
            .to_string(),
            1 => {
                // Comparison
                let lhs = self.generate_typed_expression(rng, env, &VerumType::Int, depth + 1);
                let rhs = self.generate_typed_expression(rng, env, &VerumType::Int, depth + 1);
                let op = ["==", "!=", "<", ">", "<=", ">="].choose(rng).unwrap();
                format!("({} {} {})", lhs, op, rhs)
            }
            2 => {
                // Logical and/or
                let lhs = self.generate_typed_expression(rng, env, &VerumType::Bool, depth + 1);
                let rhs = self.generate_typed_expression(rng, env, &VerumType::Bool, depth + 1);
                let op = ["&&", "||"].choose(rng).unwrap();
                format!("({} {} {})", lhs, op, rhs)
            }
            3 => {
                // Logical not
                let expr = self.generate_typed_expression(rng, env, &VerumType::Bool, depth + 1);
                format!("(!{})", expr)
            }
            4 => {
                // Text comparison
                let lhs = self.generate_typed_expression(rng, env, &VerumType::Text, depth + 1);
                let rhs = self.generate_typed_expression(rng, env, &VerumType::Text, depth + 1);
                let op = ["==", "!="].choose(rng).unwrap();
                format!("({} {} {})", lhs, op, rhs)
            }
            _ => {
                let bool_vars = env.get_variables_of_type(&VerumType::Bool);
                if !bool_vars.is_empty() {
                    bool_vars.choose(rng).unwrap().clone()
                } else {
                    "true".to_string()
                }
            }
        }
    }

    fn generate_text_expression<R: Rng>(
        &self,
        rng: &mut R,
        env: &mut TypeEnv,
        depth: usize,
    ) -> String {
        match rng.random_range(0..4) {
            0 => {
                // String literal
                let len = rng.random_range(0..20);
                let s: String = (0..len)
                    .map(|_| (b'a' + rng.random_range(0..26)) as char)
                    .collect();
                format!("\"{}\"", s)
            }
            1 => {
                // String concatenation
                let lhs = self.generate_typed_expression(rng, env, &VerumType::Text, depth + 1);
                let rhs = self.generate_typed_expression(rng, env, &VerumType::Text, depth + 1);
                format!("({} + {})", lhs, rhs)
            }
            2 => {
                // If expression
                let cond = self.generate_typed_expression(rng, env, &VerumType::Bool, depth + 1);
                let then_expr =
                    self.generate_typed_expression(rng, env, &VerumType::Text, depth + 1);
                let else_expr =
                    self.generate_typed_expression(rng, env, &VerumType::Text, depth + 1);
                format!("if {} {{ {} }} else {{ {} }}", cond, then_expr, else_expr)
            }
            _ => {
                let text_vars = env.get_variables_of_type(&VerumType::Text);
                if !text_vars.is_empty() {
                    text_vars.choose(rng).unwrap().clone()
                } else {
                    "\"\"".to_string()
                }
            }
        }
    }

    fn generate_list_expression<R: Rng>(
        &self,
        rng: &mut R,
        env: &mut TypeEnv,
        inner_type: &VerumType,
        depth: usize,
    ) -> String {
        match rng.random_range(0..3) {
            0 => {
                // List literal
                let len = rng.random_range(0..self.config.max_list_size);
                let elements: Vec<String> = (0..len)
                    .map(|_| self.generate_typed_expression(rng, env, inner_type, depth + 1))
                    .collect();
                format!("[{}]", elements.join(", "))
            }
            1 => {
                // List with single element repeated
                let elem = self.generate_typed_expression(rng, env, inner_type, depth + 1);
                format!("[{}]", elem)
            }
            _ => {
                // Empty list
                "[]".to_string()
            }
        }
    }

    fn generate_maybe_expression<R: Rng>(
        &self,
        rng: &mut R,
        env: &mut TypeEnv,
        inner_type: &VerumType,
        depth: usize,
    ) -> String {
        if rng.random_bool(0.3) {
            "None".to_string()
        } else {
            let inner = self.generate_typed_expression(rng, env, inner_type, depth + 1);
            format!("Some({})", inner)
        }
    }

    fn generate_tuple_expression<R: Rng>(
        &self,
        rng: &mut R,
        env: &mut TypeEnv,
        types: &[VerumType],
        depth: usize,
    ) -> String {
        let elements: Vec<String> = types
            .iter()
            .map(|t| self.generate_typed_expression(rng, env, t, depth + 1))
            .collect();
        format!("({})", elements.join(", "))
    }

    fn generate_literal_of_type<R: Rng>(&self, rng: &mut R, ty: &VerumType) -> String {
        match ty {
            VerumType::Int => rng.random_range(-100..100).to_string(),
            VerumType::Float => format!("{:.2}", rng.random::<f64>() * 100.0),
            VerumType::Bool => if rng.random_bool(0.5) {
                "true"
            } else {
                "false"
            }
            .to_string(),
            VerumType::Text => {
                let len = rng.random_range(0..10);
                let s: String = (0..len)
                    .map(|_| (b'a' + rng.random_range(0..26)) as char)
                    .collect();
                format!("\"{}\"", s)
            }
            VerumType::Unit => "()".to_string(),
            VerumType::List(_) => "[]".to_string(),
            VerumType::Maybe(_) => "None".to_string(),
            VerumType::Tuple(types) => {
                let elements: Vec<String> = types
                    .iter()
                    .map(|t| self.generate_literal_of_type(rng, t))
                    .collect();
                format!("({})", elements.join(", "))
            }
            _ => "()".to_string(),
        }
    }

    fn generate_simple_type<R: Rng>(&self, rng: &mut R) -> VerumType {
        match self.type_weights.sample(rng) {
            0 => VerumType::Int,
            1 => VerumType::Float,
            2 => VerumType::Bool,
            3 => VerumType::Text,
            4 => VerumType::Unit,
            5 => VerumType::List(Box::new(self.generate_primitive_type(rng))),
            6 => VerumType::Maybe(Box::new(self.generate_primitive_type(rng))),
            _ => {
                let types = vec![
                    self.generate_primitive_type(rng),
                    self.generate_primitive_type(rng),
                ];
                VerumType::Tuple(types)
            }
        }
    }

    fn generate_primitive_type<R: Rng>(&self, rng: &mut R) -> VerumType {
        match rng.random_range(0..5) {
            0 => VerumType::Int,
            1 => VerumType::Float,
            2 => VerumType::Bool,
            3 => VerumType::Text,
            _ => VerumType::Unit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_type_aware_generator() {
        let config = TypeAwareConfig::default();
        let generator = TypeAwareGenerator::new(config);

        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..5 {
            let program = generator.generate_program(&mut rng);
            assert!(!program.is_empty());
            assert!(program.contains("fn main()"));
            // All let bindings should have type annotations
            for line in program.lines() {
                if line.contains("let ") && line.contains(" = ") {
                    assert!(
                        line.contains(":"),
                        "Let binding missing type annotation: {}",
                        line
                    );
                }
            }
        }
    }

    #[test]
    fn test_type_to_syntax() {
        assert_eq!(VerumType::Int.to_syntax(), "Int");
        assert_eq!(VerumType::Float.to_syntax(), "Float");
        assert_eq!(
            VerumType::List(Box::new(VerumType::Int)).to_syntax(),
            "List<Int>"
        );
        assert_eq!(
            VerumType::Map(Box::new(VerumType::Text), Box::new(VerumType::Int)).to_syntax(),
            "Map<Text, Int>"
        );
    }

    #[test]
    fn test_type_properties() {
        assert!(VerumType::Int.is_numeric());
        assert!(VerumType::Float.is_numeric());
        assert!(!VerumType::Text.is_numeric());

        assert!(VerumType::Int.is_equatable());
        assert!(VerumType::Text.is_equatable());
        assert!(VerumType::List(Box::new(VerumType::Int)).is_equatable());
    }
}
