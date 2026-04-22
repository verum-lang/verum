//! Translation from Verum AST to Z3 expressions.
//!
//! This module handles the conversion of Verum expressions, types, and
//! refinement predicates into Z3's internal representation for solving.
//!
//! ## Features
//!
//! - **Full type mapping**: All Verum primitive and compound types
//! - **Expression translation**: Arithmetic, logical, comparison operations
//! - **Refinement predicates**: Support for complex refinement constraints
//! - **Quantifiers**: forall and exists (partial support)
//! - **Function calls**: Built-in functions (abs, min, max, pow)
//! - **Field access**: Array length, vector size
//! - **Tensor support**: Full Z3 Array theory implementation
//!
//! ## Translation Strategy
//!
//! The translator maintains a binding environment mapping variable names to Z3 AST nodes.
//! Verum refinement predicates use the special variable `it` to refer to the value being constrained.
//!
//! ### Type Mapping
//!
//! | Verum Type | Z3 Sort | Notes |
//! |------------|---------|-------|
//! | `Int` | `Int` | Unbounded integers |
//! | `Float` | `Real` | Real numbers (approximation) |
//! | `Bool` | `Bool` | Boolean values |
//! | `Text` | Uninterpreted | Limited support |
//! | Refinement `T{p}` | Base type + constraint | Predicate becomes assertion |
//! | `Tensor<T, [N, M]>` | `Array[Int -> Array[Int -> T]]` | Nested array theory |
//!
//! ### Tensor Support (Z3 Array Theory)
//!
//! Tensors are translated to nested Z3 Arrays:
//!
//! - **1D tensor**: `Tensor<i32, [N]>` → `Array[Int -> Int]`
//! - **2D tensor**: `Tensor<f32, [N, M]>` → `Array[Int -> Array[Int -> Real]]`
//! - **3D tensor**: `Tensor<bool, [N, M, K]>` → `Array[Int -> Array[Int -> Array[Int -> Bool]]]`
//!
//! #### Element Access
//!
//! - `tensor[i]` → `Array::select(tensor, i)`
//! - `tensor[i][j]` → `Array::select(Array::select(tensor, i), j)`
//! - `tensor[i][j][k]` → Nested select operations
//!
//! #### Dimension Constraints
//!
//! For each dimension, symbolic or concrete size constraints are generated:
//! - Concrete: `let tensor: Tensor<i32, [10, 20]>` → sizes are `10` and `20`
//! - Symbolic: `let tensor: Tensor<i32, [N, M]>` → `N` and `M` are Int constants
//!
//! #### Bounds Checking
//!
//! Index bounds are verified: `0 <= i && i < dimension_size`
//!
//! ### Supported Operations
//!
//! - **Arithmetic**: +, -, *, /, % (modulo)
//! - **Comparison**: ==, !=, <, <=, >, >=
//! - **Logical**: &&, ||, !
//! - **Functions**: abs, min, max, pow
//! - **If-then-else**: `cond ? then_val : else_val` → `(ite cond then_val else_val)`
//! - **Array/Tensor access**: `arr[i]` → `Array::select(arr, i)`

use crate::context::Context;
use verum_ast::{BinOp, Expr, ExprKind, Literal, LiteralKind, Pattern, PatternKind, Type, TypeKind};
use verum_common::{List, Map, Maybe, Text};
use verum_common::ToText;
use z3::ast::{
    Array, Ast, BV, Bool, Dynamic, Float, Int, Real, RoundingMode, String as Z3String,
    exists_const, forall_const,
};
use z3::{FuncDecl, Pattern as Z3Pattern, Sort, SortKind, Symbol};

// ============================================================================
// IEEE 754 Floating-Point Theory Configuration
// ============================================================================

/// Configuration options for SMT translation.
///
/// Controls how various Verum types and expressions are translated to Z3,
/// particularly around floating-point precision and special value handling.
#[derive(Debug, Clone)]
pub struct TranslationConfig {
    /// Use IEEE 754 FPA (Floating-Point Arithmetic) theory for float types.
    ///
    /// When `true`:
    /// - Float types are translated to Z3's IEEE 754 double-precision sort (11 exponent, 53 significand bits)
    /// - Arithmetic operations use proper FPA semantics with rounding modes
    /// - NaN, infinity, and subnormal values are handled precisely
    /// - Verification is more precise but potentially slower
    ///
    /// When `false` (default):
    /// - Float types are approximated as Real numbers
    /// - Faster solving but may miss floating-point edge cases
    /// - Suitable for programs that don't rely on precise FP semantics
    pub precise_floats: bool,

    /// Default rounding mode for FPA operations when `precise_floats` is enabled.
    ///
    /// Defaults to `RoundNearestTiesToEven` (IEEE 754 default).
    pub default_rounding_mode: FloatRoundingMode,

    /// Float precision to use: Float32 (single) or Float64 (double).
    ///
    /// Defaults to Float64 for maximum precision.
    pub float_precision: FloatPrecision,
}

impl Default for TranslationConfig {
    fn default() -> Self {
        Self {
            precise_floats: false,
            default_rounding_mode: FloatRoundingMode::NearestTiesToEven,
            float_precision: FloatPrecision::Float64,
        }
    }
}

impl TranslationConfig {
    /// Create a configuration with precise IEEE 754 float support enabled.
    pub fn with_precise_floats() -> Self {
        Self {
            precise_floats: true,
            ..Default::default()
        }
    }

    /// Create a configuration for fast approximate float handling.
    pub fn approximate() -> Self {
        Self {
            precise_floats: false,
            ..Default::default()
        }
    }

    /// Set the default rounding mode.
    pub fn with_rounding_mode(mut self, mode: FloatRoundingMode) -> Self {
        self.default_rounding_mode = mode;
        self
    }

    /// Set the float precision.
    pub fn with_precision(mut self, precision: FloatPrecision) -> Self {
        self.float_precision = precision;
        self
    }
}

/// IEEE 754 rounding modes for floating-point operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatRoundingMode {
    /// Round to nearest, ties to even (IEEE 754 default)
    NearestTiesToEven,
    /// Round to nearest, ties away from zero
    NearestTiesToAway,
    /// Round toward positive infinity
    TowardPositive,
    /// Round toward negative infinity
    TowardNegative,
    /// Round toward zero (truncation)
    TowardZero,
}

impl FloatRoundingMode {
    /// Convert to Z3 RoundingMode AST node.
    pub fn to_z3_rounding_mode(&self) -> RoundingMode {
        match self {
            FloatRoundingMode::NearestTiesToEven => RoundingMode::round_nearest_ties_to_even(),
            FloatRoundingMode::NearestTiesToAway => RoundingMode::round_nearest_ties_to_away(),
            FloatRoundingMode::TowardPositive => RoundingMode::round_towards_positive(),
            FloatRoundingMode::TowardNegative => RoundingMode::round_towards_negative(),
            FloatRoundingMode::TowardZero => RoundingMode::round_towards_zero(),
        }
    }
}

/// Floating-point precision options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatPrecision {
    /// IEEE 754 single precision (32-bit): 8 exponent bits, 24 significand bits
    Float32,
    /// IEEE 754 double precision (64-bit): 11 exponent bits, 53 significand bits
    Float64,
}

impl FloatPrecision {
    /// Get the Z3 Sort for this precision.
    pub fn to_z3_sort(&self) -> Sort {
        match self {
            FloatPrecision::Float32 => Sort::float32(),
            FloatPrecision::Float64 => Sort::double(),
        }
    }

    /// Get exponent and significand bit widths.
    pub fn bit_widths(&self) -> (u32, u32) {
        match self {
            FloatPrecision::Float32 => (8, 24),
            FloatPrecision::Float64 => (11, 53),
        }
    }
}

/// Types of floating-point special value checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatCheck {
    /// Check if value is NaN (Not a Number)
    IsNaN,
    /// Check if value is infinite (+/- infinity)
    IsInfinite,
    /// Check if value is a normal number (not zero, subnormal, infinite, or NaN)
    IsNormal,
    /// Check if value is subnormal (denormalized)
    IsSubnormal,
    /// Check if value is zero (+0 or -0)
    IsZero,
    /// Check if value is negative
    IsNegative,
    /// Check if value is positive
    IsPositive,
}

/// Translator for converting Verum AST to Z3.
///
/// The translator maintains a mapping from variable names to Z3 AST nodes,
/// allowing refinement predicates like `Int{> 0}` to reference variables.
///
/// ## Floating-Point Support
///
/// The translator supports two modes for floating-point values:
///
/// 1. **Approximate mode** (default): Floats are translated to Z3 Real numbers.
///    Faster solving but may miss IEEE 754 edge cases.
///
/// 2. **Precise mode**: Floats use Z3's FPA (Floating-Point Arithmetic) theory.
///    Handles NaN, infinity, subnormal values, and rounding modes accurately.
///
/// Configure via `TranslationConfig::with_precise_floats()`.
///
/// ## Bounds Checking
///
/// Tensor dimension constraints are NOT automatically enforced during translation.
/// Instead, bounds checking constraints should be generated separately using
/// `create_dimension_constraints` and `create_bounds_constraint`, then asserted
/// at the verification level (e.g., in `verum_verification` or `verum_smt::refinement`).
///
/// This separation allows:
/// - Fine-grained control over when bounds are checked
/// - Different verification strategies (eager vs lazy checking)
/// - Conditional bounds checking based on optimization level
#[derive(Debug)]
pub struct Translator<'ctx> {
    context: &'ctx Context,
    /// Variable bindings: name -> Z3 expression
    bindings: Map<Text, Dynamic>,
    /// Translation configuration
    config: TranslationConfig,
}

impl<'ctx> Translator<'ctx> {
    /// Create a new translator with default configuration.
    pub fn new(context: &'ctx Context) -> Self {
        Self {
            context,
            bindings: Map::new(),
            config: TranslationConfig::default(),
        }
    }

    /// Create a new translator with the specified configuration.
    pub fn with_config(context: &'ctx Context, config: TranslationConfig) -> Self {
        Self {
            context,
            bindings: Map::new(),
            config,
        }
    }

    /// Get the current configuration.
    pub fn config(&self) -> &TranslationConfig {
        &self.config
    }

    /// Check if precise floats mode is enabled.
    pub fn uses_precise_floats(&self) -> bool {
        self.config.precise_floats
    }

    /// Get the default Z3 rounding mode based on configuration.
    fn get_rounding_mode(&self) -> RoundingMode {
        self.config.default_rounding_mode.to_z3_rounding_mode()
    }

    /// Get the Z3 sort for float types based on configuration.
    fn get_float_sort(&self) -> Sort {
        if self.config.precise_floats {
            self.config.float_precision.to_z3_sort()
        } else {
            Sort::real()
        }
    }

    /// Bind a variable to a Z3 expression.
    ///
    /// This allows refinement predicates to reference variables by name.
    /// The special variable `it` is commonly used in refinement types.
    pub fn bind(&mut self, name: Text, value: Dynamic) {
        self.bindings.insert(name, value);
    }

    /// Get a bound variable.
    pub fn get(&self, name: &str) -> Maybe<&Dynamic> {
        self.bindings.get(&name.to_text())
    }

    /// Unbind a variable.
    pub fn unbind(&mut self, name: &str) -> Maybe<Dynamic> {
        self.bindings.remove(&name.to_text())
    }

    /// Check if a variable is bound.
    pub fn contains(&self, name: &str) -> bool {
        self.bindings.contains_key(&name.to_text())
    }

    /// Get the context reference.
    pub fn context(&self) -> &'ctx Context {
        self.context
    }

    /// Get all binding names.
    ///
    /// Returns an iterator over all currently bound variable names.
    /// This is useful for cloning translator state in dependent type checking.
    pub fn binding_names(&self) -> impl Iterator<Item = Text> + '_ {
        self.bindings.keys().cloned()
    }

    /// Get all bindings as an iterator.
    ///
    /// Returns an iterator over (name, value) pairs for all bindings.
    pub fn bindings_iter(&self) -> impl Iterator<Item = (&Text, &Dynamic)> {
        self.bindings.iter()
    }

    /// Get the number of bound variables.
    pub fn binding_count(&self) -> usize {
        self.bindings.len()
    }

    /// Clear all bindings.
    ///
    /// Removes all variable bindings, returning the translator to an empty state.
    pub fn clear_bindings(&mut self) {
        self.bindings.clear();
    }

    /// Translate an Verum expression to Z3.
    ///
    /// This is the main entry point for expression translation.
    /// It recursively translates the expression tree into Z3 AST nodes.
    /// Translate an expression but force identifier-heads to the Bool
    /// sort. Used by propositional operator translation so that a bare
    /// `P && Q` with no inferred types encodes as two fresh Bool
    /// constants rather than running aground on the Int-default.
    fn translate_expr_as_bool(&self, expr: &Expr) -> Result<Bool, TranslationError> {
        match &expr.kind {
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    let name = ident.as_str();
                    if let Maybe::Some(var) = self.bindings.get(&name.to_text()) {
                        if let Some(b) = var.as_bool() {
                            return Ok(b);
                        }
                    }
                    match name {
                        "true" | "True" => return Ok(Bool::from_bool(true)),
                        "false" | "False" => return Ok(Bool::from_bool(false)),
                        _ => {}
                    }
                    return Ok(Bool::new_const(name));
                }
                // Fall through for non-ident paths — let the regular
                // translator try (e.g. qualified paths), then cast.
                let d = self.translate_expr(expr)?;
                d.as_bool().ok_or_else(|| {
                    TranslationError::TypeMismatch(Text::from(format!(
                        "expected Bool but got non-boolean path: {:?}",
                        path
                    )))
                })
            }
            ExprKind::Paren(inner) => self.translate_expr_as_bool(inner),
            _ => {
                // For everything else, go through the normal translator
                // and demand the result be Bool.
                let d = self.translate_expr(expr)?;
                d.as_bool().ok_or_else(|| {
                    TranslationError::TypeMismatch(Text::from(
                        "expected boolean expression",
                    ))
                })
            }
        }
    }

    pub fn translate_expr(&self, expr: &Expr) -> Result<Dynamic, TranslationError> {
        match &expr.kind {
            ExprKind::Literal(lit) => self.translate_literal(lit),

            ExprKind::Path(path) => {
                // Handle simple identifiers and 'it' for refinement predicates
                if let Some(ident) = path.as_ident() {
                    let name = ident.as_str();

                    // Check if it's a bound variable
                    if let Maybe::Some(var) = self.bindings.get(&name.to_text()) {
                        return Ok(var.clone());
                    }

                    // Check for boolean constants. Both casings are
                    // accepted: `true`/`false` is the Verum keyword form,
                    // `True`/`False` appears in prop-logic contexts mirrored
                    // from mainstream proof assistants and in imported
                    // stdlib names; the translator treats them as
                    // interchangeable Z3 Bool literals rather than fresh
                    // uninterpreted Int constants.
                    match name {
                        "true" | "True" => {
                            let bool_val = Bool::from_bool(true);
                            return Ok(Dynamic::from_ast(&bool_val));
                        }
                        "false" | "False" => {
                            let bool_val = Bool::from_bool(false);
                            return Ok(Dynamic::from_ast(&bool_val));
                        }
                        _ => {}
                    }

                    // Otherwise create a fresh variable based on inferred type
                    // For now, default to integer
                    let int_var = Int::new_const(name);
                    Ok(Dynamic::from_ast(&int_var))
                } else {
                    Err(TranslationError::UnsupportedPath(Text::from(format!(
                        "{:?}",
                        path
                    ))))
                }
            }

            ExprKind::Binary { op, left, right } => self.translate_binary_op(*op, left, right),

            ExprKind::Unary { op, expr } => self.translate_unary_op(*op, expr),

            ExprKind::Call { func, args, .. } => self.translate_call(func, args),

            ExprKind::Paren(inner) => self.translate_expr(inner),

            ExprKind::Field { expr, field } => {
                // Handle field access like vec.length
                let field_name = field.as_str();

                match field_name {
                    "length" | "len" | "size" => {
                        // For now, treat array length as an uninterpreted function
                        // In the future, we can use Z3's array theory properly
                        // Return a symbolic length constant based on the expression
                        let base_name = format!("length_{:?}", expr);
                        let int_var = Int::new_const(base_name.as_str());
                        Ok(Dynamic::from_ast(&int_var))
                    }
                    _ => Err(TranslationError::UnsupportedExpr(Text::from(format!(
                        "field access: {}",
                        field_name
                    )))),
                }
            }

            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                // Handle method calls like vec.len()
                let method_name = method.as_str();

                match method_name {
                    "len" | "length" | "size" if args.is_empty() => {
                        // Similar to field access
                        let base_name = format!("length_{:?}", receiver);
                        let int_var = Int::new_const(base_name.as_str());
                        Ok(Dynamic::from_ast(&int_var))
                    }
                    _ => Err(TranslationError::UnsupportedFunction(Text::from(format!(
                        "method: {}",
                        method_name
                    )))),
                }
            }

            // If-then-else expression: translated to Z3 ite
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.translate_if(condition, then_branch, else_branch),

            // Index expressions for array access
            ExprKind::Index { expr, index } => self.translate_index(expr, index),

            // Cast expressions - extract the inner expression
            ExprKind::Cast { expr, .. } => self.translate_expr(expr),

            // Tuple expressions - create Z3 datatype tuples
            ExprKind::Tuple(exprs) => self.translate_tuple(exprs),

            // Interpolated string expressions - concatenate parts and expressions
            ExprKind::InterpolatedString {
                handler,
                parts,
                exprs: interp_exprs,
            } => {
                let parts_vec: Vec<String> = parts.iter().map(|p| p.to_string()).collect();
                self.translate_interpolated_string(handler, &parts_vec[..], interp_exprs)
            }

            // Block expressions - evaluate to their final expression
            ExprKind::Block(block) => {
                match &block.expr {
                    Maybe::Some(expr) => self.translate_expr(expr),
                    Maybe::None => {
                        // Empty block - treat as unit/true
                        let bool_val = Bool::from_bool(true);
                        Ok(Dynamic::from_ast(&bool_val))
                    }
                }
            }

            // Attenuate expressions - pass through to context (capability attenuation is compile-time)
            ExprKind::Attenuate { context, .. } => {
                // Attenuate expressions restrict capability, recursively process the context
                // For SMT translation, capability attenuation is a compile-time concept, so we just translate the context
                self.translate_expr(context)
            }

            // Universal quantifier: forall x: T. predicate(x)
            // Pi type `(x: A) -> B(x)`: return type depends on input value.
            // Translated to Z3 `forall_const()` with Pattern::new() for MBQI instantiation.
            ExprKind::Forall { bindings, body } => self.translate_forall(bindings, body),

            // Existential quantifier: exists x: T. predicate(x)
            // Sigma type `(x: A, B(x))`: second component depends on first value.
            // Translated to Z3 `exists_const()` for existential quantification.
            // Quantifier expressions: forall/exists translated to Z3 quantified formulas
            ExprKind::Exists { bindings, body } => self.translate_exists(bindings, body),

            // Unsupported expression kinds
            _ => Err(TranslationError::UnsupportedExpr(Text::from(format!(
                "{:?}",
                expr.kind
            )))),
        }
    }

    /// Translate an if-then-else expression.
    fn translate_if(
        &self,
        condition: &verum_ast::expr::IfCondition,
        then_branch: &verum_ast::Block,
        else_branch: &Option<Box<Expr>>,
    ) -> Result<Dynamic, TranslationError> {
        use verum_ast::expr::ConditionKind;

        // Extract the first condition expression (SMT doesn't support let-patterns or multiple conditions)
        if condition.conditions.len() != 1 {
            return Err(TranslationError::UnsupportedExpr(
                "SMT only supports single-condition if expressions".to_text(),
            ));
        }

        let cond_expr = match &condition.conditions[0] {
            ConditionKind::Expr(expr) => expr,
            ConditionKind::Let { .. } => {
                return Err(TranslationError::UnsupportedExpr(
                    "if-let patterns not supported in SMT".to_text(),
                ));
            }
        };

        let cond_z3 = self.translate_expr(cond_expr)?;
        let cond_bool = cond_z3
            .as_bool()
            .ok_or_else(|| TranslationError::TypeMismatch("condition must be boolean".to_text()))?;

        // Translate then branch
        let then_value = match &then_branch.expr {
            Some(expr) => self.translate_expr(expr)?,
            None => {
                // No expression - use true
                let bool_val = Bool::from_bool(true);
                Dynamic::from_ast(&bool_val)
            }
        };

        // Translate else branch
        let else_value = match else_branch {
            Some(else_expr) => self.translate_expr(else_expr)?,
            None => {
                // No else branch - use false
                let bool_val = Bool::from_bool(false);
                Dynamic::from_ast(&bool_val)
            }
        };

        // Create if-then-else based on types
        if let (Some(then_int), Some(else_int)) = (then_value.as_int(), else_value.as_int()) {
            let result = cond_bool.ite(&then_int, &else_int);
            Ok(Dynamic::from_ast(&result))
        } else if let (Some(then_bool), Some(else_bool)) =
            (then_value.as_bool(), else_value.as_bool())
        {
            let result = cond_bool.ite(&then_bool, &else_bool);
            Ok(Dynamic::from_ast(&result))
        } else if let (Some(then_real), Some(else_real)) =
            (then_value.as_real(), else_value.as_real())
        {
            let result = cond_bool.ite(&then_real, &else_real);
            Ok(Dynamic::from_ast(&result))
        } else {
            Err(TranslationError::TypeMismatch(
                "if branches must have same type".to_text(),
            ))
        }
    }

    /// Translate a tuple expression to Z3 datatype.
    ///
    /// For tuples, we create a Z3 datatype constructor:
    /// - Unit tuple `()` → true (boolean)
    /// - Tuple `(a, b, c)` → Tuple_3(field_0: T0, field_1: T1, field_2: T2)
    ///
    /// Z3 datatypes provide a structured way to represent product types.
    fn translate_tuple(&self, exprs: &[Expr]) -> Result<Dynamic, TranslationError> {
        if exprs.is_empty() {
            // Unit type - treat as true
            let bool_val = Bool::from_bool(true);
            return Ok(Dynamic::from_ast(&bool_val));
        }

        // Translate each tuple element
        let mut translated_exprs = List::new();
        for expr in exprs {
            translated_exprs.push(self.translate_expr(expr)?);
        }

        // For simplicity in SMT translation, encode tuples as nested pairs
        // or as uninterpreted functions
        // For now, create a symbolic tuple constant
        // A full datatype implementation would require more complex Z3 API usage

        // Use the first element's sort as a representative
        // In a full implementation, we'd create a proper datatype
        // For now, return the first element if singleton, or a symbolic value
        if translated_exprs.len() == 1 {
            Ok(translated_exprs[0].clone())
        } else {
            // Create a symbolic Int constant for multi-element tuples
            // This is a simplification for SMT verification purposes
            let const_name = format!("tuple_{}", exprs.len());
            let tuple_const = Int::new_const(const_name.as_str());
            Ok(Dynamic::from_ast(&tuple_const))
        }
    }

    /// Translate an interpolated string expression to Z3.
    ///
    /// Interpolated strings like `f"Hello {name}"` are translated to string concatenation:
    /// - Parts: ["Hello ", ""]
    /// - Exprs: [name]
    /// Result: concat("Hello ", to_str(name))
    ///
    /// For safe interpolations (sql, html, etc.), we model them as uninterpreted functions
    /// that take the interpolated parts and return a safe string.
    fn translate_interpolated_string(
        &self,
        handler: &str,
        parts: &[String],
        exprs: &[Expr],
    ) -> Result<Dynamic, TranslationError> {
        // For SMT translation, we model interpolated strings as string concatenation
        // using Z3's String theory

        if exprs.is_empty() {
            // No interpolations - just concatenate parts
            let result = parts.join("");
            let str_val = Z3String::from(result.as_str());
            return Ok(Dynamic::from_ast(&str_val));
        }

        // Build concatenation of parts and expressions
        // Pattern: part[0] + expr[0] + part[1] + expr[1] + ... + part[n]
        let mut string_parts: Vec<Z3String> = Vec::new();

        for (i, part) in parts.iter().enumerate() {
            // Add the string part if non-empty
            if !part.is_empty() {
                string_parts.push(Z3String::from(part.as_str()));
            }

            // Add the expression (if exists)
            if i < exprs.len() {
                let expr_z3 = self.translate_expr(&exprs[i])?;

                // Convert expression to string
                // For integers, use int-to-str function
                // For strings, use directly
                // For others, create symbolic string representation
                let expr_str = if let Some(str_val) = expr_z3.as_string() {
                    str_val
                } else if let Some(int_val) = expr_z3.as_int() {
                    // Use Z3 C API for int-to-string conversion
                    let ctx = int_val.get_ctx();
                    unsafe {
                        let str_ast =
                            z3_sys::Z3_mk_int_to_str(ctx.get_z3_context(), int_val.get_z3_ast());
                        if let Some(ast) = str_ast {
                            Z3String::wrap(ctx, ast)
                        } else {
                            // Fallback: create symbolic string
                            let symbolic_name = format!("str_of_int_{}", i);
                            Z3String::new_const(symbolic_name.as_str())
                        }
                    }
                } else {
                    // For other types, create a symbolic string representation
                    let symbolic_name = format!("str_of_expr_{}", i);
                    Z3String::new_const(symbolic_name.as_str())
                };

                string_parts.push(expr_str);
            }
        }

        // Concatenate all parts
        let result_str = if string_parts.is_empty() {
            Z3String::from("")
        } else if string_parts.len() == 1 {
            string_parts.into_iter().next().unwrap()
        } else {
            // Convert to references for concat
            let str_refs: Vec<&Z3String> = string_parts.iter().collect();
            Z3String::concat(&str_refs)
        };

        // For safe interpolation handlers (sql, html, etc.), wrap in an uninterpreted function
        if matches!(handler, "sql" | "html" | "uri" | "json" | "xml" | "gql") {
            // Create an uninterpreted function for safe handling
            let func_name = format!("{}_safe", handler);
            let string_sort = Sort::string();
            let func_decl = FuncDecl::new(Symbol::String(func_name), &[&string_sort], &string_sort);

            let safe_result = func_decl.apply(&[&result_str]);
            Ok(safe_result)
        } else {
            // Regular format string - return concatenated result
            Ok(Dynamic::from_ast(&result_str))
        }
    }

    /// Translate an index expression for array/tensor access.
    ///
    /// This implements Z3 Array select operations:
    /// - tensor[i] -> Array::select(tensor, i)
    /// - tensor[i][j] -> Array::select(Array::select(tensor, i), j)
    fn translate_index(&self, expr: &Expr, index: &Expr) -> Result<Dynamic, TranslationError> {
        // Translate the base expression (array/tensor)
        let base_z3 = self.translate_expr(expr)?;

        // Translate the index expression
        let index_z3 = self.translate_expr(index)?;

        // Get the index as an Int (Z3 arrays use Int indices)
        let index_int = index_z3.as_int().ok_or_else(|| {
            TranslationError::TypeMismatch("array index must be integer".to_text())
        })?;

        // Try to interpret the base as an array
        if let Some(array) = base_z3.as_array() {
            // Direct array select
            let result = array.select(&index_int);
            Ok(result)
        } else {
            // The base might be a nested select result (Dynamic)
            // We need to construct a select operation manually
            // This handles nested indexing like tensor[i][j]
            use z3::ast::Ast;

            // Get the context from the base AST node
            let ctx = base_z3.get_ctx();

            unsafe {
                // Use Z3 C API directly for select operation on Dynamic
                let select_ast = z3_sys::Z3_mk_select(
                    ctx.get_z3_context(),
                    base_z3.get_z3_ast(),
                    index_int.get_z3_ast(),
                );

                if let Some(ast) = select_ast {
                    Ok(Dynamic::wrap(ctx, ast))
                } else {
                    Err(TranslationError::TypeMismatch(
                        "failed to create array select operation".to_text(),
                    ))
                }
            }
        }
    }

    /// Translate a literal value to Z3.
    fn translate_literal(&self, lit: &Literal) -> Result<Dynamic, TranslationError> {
        match &lit.kind {
            LiteralKind::Bool(b) => {
                let bool_val = Bool::from_bool(*b);
                Ok(Dynamic::from_ast(&bool_val))
            }

            LiteralKind::Int(i) => {
                let int_val = Int::from_i64(i.value as i64);
                Ok(Dynamic::from_ast(&int_val))
            }

            LiteralKind::Float(f) => {
                if self.config.precise_floats {
                    // Use IEEE 754 FPA theory for precise floating-point semantics
                    let float_val = match self.config.float_precision {
                        FloatPrecision::Float32 => Float::from_f32(f.value as f32),
                        FloatPrecision::Float64 => Float::from_f64(f.value),
                    };
                    Ok(Dynamic::from_ast(&float_val))
                } else {
                    // Approximate mode: Convert float to Z3 real
                    // Use rational representation for better precision
                    let scaled = (f.value * 1000000.0).round() as i64;
                    let real_val = Real::from_rational(scaled, 1000000);
                    Ok(Dynamic::from_ast(&real_val))
                }
            }

            LiteralKind::Text(s) => {
                // Translate string literals using Z3 String theory
                // Z3 supports string operations: length, concatenation, substring, contains, etc.
                let str_val = Z3String::from(s.as_str());
                Ok(Dynamic::from_ast(&str_val))
            }

            LiteralKind::Char(c) => {
                // Translate character literals as bounded integers (Unicode code points)
                // Unicode range: U+0000 to U+10FFFF (0 to 1,114,111)
                // This allows comparison, ordering, and arithmetic on characters
                let code_point = *c as u32 as i64;
                let char_val = Int::from_i64(code_point);
                Ok(Dynamic::from_ast(&char_val))
            }

            LiteralKind::ByteChar(b) => {
                // Translate byte character literals as bounded integers (0-255)
                let byte_val = Int::from_i64(*b as i64);
                Ok(Dynamic::from_ast(&byte_val))
            }

            LiteralKind::ByteString(bytes) => {
                // Translate byte string as an array of bounded integers (0-255)
                // For SMT verification, represent as a sequence/array
                let first_byte = Int::from_i64(bytes.first().copied().unwrap_or(0) as i64);
                Ok(Dynamic::from_ast(&first_byte))
            }

            LiteralKind::Tagged { tag, content } => {
                // Translate tagged literals as uninterpreted functions
                // e.g., d#"2025-11-05" → date("2025-11-05")
                // This allows verification of properties without full parsing

                // Create an uninterpreted function for the tag
                let func_name = format!("tagged_{}", tag);
                let string_sort = Sort::string();
                let func_decl =
                    FuncDecl::new(Symbol::String(func_name), &[&string_sort], &string_sort);

                // Apply the function to the content
                let content_str = Z3String::from(content.as_str());
                let result = func_decl.apply(&[&content_str]);
                Ok(result)
            }

            LiteralKind::Contract(content) => {
                // Contract literals represent logical predicates/constraints
                // e.g., contract#"it > 0" or contract#"requires x > 0"
                // These should ideally be parsed and translated as boolean expressions
                // For now, create a symbolic boolean constant to represent the contract
                // In a full implementation, we'd parse the contract content and translate it

                let contract_name = format!(
                    "contract_{}",
                    content
                        .chars()
                        .filter(|c| c.is_alphanumeric() || *c == '_')
                        .take(32)
                        .collect::<std::string::String>()
                );
                let contract_bool = Bool::new_const(contract_name.as_str());
                Ok(Dynamic::from_ast(&contract_bool))
            }

            LiteralKind::ContextAdaptive(ca_lit) => {
                // Context-adaptive literals change interpretation based on expected type
                // e.g., #FF5733 can be Color, u32, ByteArray, etc.
                // Translate based on the kind of adaptive literal
                use verum_ast::literal::ContextAdaptiveKind;

                match &ca_lit.kind {
                    ContextAdaptiveKind::Hex(value) => {
                        // Hex literals: treat as unsigned integers
                        // For values that fit in i64, use Int sort
                        if *value <= i64::MAX as u64 {
                            let int_val = Int::from_i64(*value as i64);
                            Ok(Dynamic::from_ast(&int_val))
                        } else {
                            // For larger values, create a symbolic constant
                            let const_name = format!("hex_{:x}", value);
                            let int_val = Int::new_const(const_name.as_str());
                            Ok(Dynamic::from_ast(&int_val))
                        }
                    }

                    ContextAdaptiveKind::Numeric(num_str) => {
                        // Numeric literals that adapt to context
                        // Try to parse as integer first, then float
                        if let Ok(i) = num_str.parse::<i64>() {
                            let int_val = Int::from_i64(i);
                            Ok(Dynamic::from_ast(&int_val))
                        } else if let Ok(f) = num_str.parse::<f64>() {
                            let scaled = (f * 1000000.0).round() as i64;
                            let real_val = Real::from_rational(scaled, 1000000);
                            Ok(Dynamic::from_ast(&real_val))
                        } else {
                            // Fall back to symbolic constant
                            let const_name = format!(
                                "numeric_{}",
                                num_str
                                    .chars()
                                    .filter(|c| c.is_alphanumeric() || *c == '_')
                                    .collect::<std::string::String>()
                            );
                            let int_val = Int::new_const(const_name.as_str());
                            Ok(Dynamic::from_ast(&int_val))
                        }
                    }

                    ContextAdaptiveKind::Identifier(ident) => {
                        // Identifier-style adaptive literals: @username, $variable
                        // Treat as string constants
                        let str_val = Z3String::from(ident.as_str());
                        Ok(Dynamic::from_ast(&str_val))
                    }
                }
            }

            LiteralKind::InterpolatedString(interp_str) => {
                // Interpolated strings are handled at the expression level (ExprKind::InterpolatedString)
                // When they appear as literals, they're not yet expanded
                // Create a symbolic string constant representing the interpolated result
                let const_name = format!(
                    "interp_{}_{}",
                    interp_str.prefix,
                    interp_str
                        .content
                        .chars()
                        .filter(|c| c.is_alphanumeric() || *c == '_')
                        .take(32)
                        .collect::<std::string::String>()
                );
                let str_val = Z3String::new_const(const_name.as_str());
                Ok(Dynamic::from_ast(&str_val))
            }

            LiteralKind::Composite(comp_lit) => {
                // Composite literals: domain-specific structured data
                // e.g., mat#"[[1, 2], [3, 4]]", vec#"<1, 2, 3>", chem#"H2O"
                // Translate as uninterpreted functions that construct values
                use verum_ast::literal::CompositeType;

                let func_name = format!("composite_{}", comp_lit.tag);

                match comp_lit.composite_type() {
                    Some(CompositeType::Matrix) | Some(CompositeType::Vector) => {
                        // For matrices and vectors, create an array-like representation
                        // Create a symbolic array constant
                        let const_name = format!(
                            "{}_{}",
                            comp_lit.tag,
                            comp_lit
                                .content
                                .chars()
                                .filter(|c| c.is_alphanumeric() || *c == '_')
                                .take(32)
                                .collect::<std::string::String>()
                        );
                        // Use Array sort for structured data
                        let array_const =
                            Array::new_const(const_name.as_str(), &Sort::int(), &Sort::int());
                        Ok(Dynamic::from_ast(&array_const))
                    }

                    Some(CompositeType::Chemistry)
                    | Some(CompositeType::Music)
                    | Some(CompositeType::Interval) => {
                        // For other composite types, use uninterpreted functions
                        let string_sort = Sort::string();
                        let func_decl =
                            FuncDecl::new(Symbol::String(func_name), &[&string_sort], &string_sort);

                        let content_str = Z3String::from(comp_lit.content.as_str());
                        let result = func_decl.apply(&[&content_str]);
                        Ok(result)
                    }

                    None => {
                        // Unknown composite type - create generic uninterpreted function
                        let string_sort = Sort::string();
                        let func_decl =
                            FuncDecl::new(Symbol::String(func_name), &[&string_sort], &string_sort);

                        let content_str = Z3String::from(comp_lit.content.as_str());
                        let result = func_decl.apply(&[&content_str]);
                        Ok(result)
                    }
                }
            }
        }
    }

    /// Translate a binary operation to Z3.
    fn translate_binary_op(
        &self,
        op: BinOp,
        left: &Expr,
        right: &Expr,
    ) -> Result<Dynamic, TranslationError> {
        // Propositional operators ({&&, ||, ->, <->, !}) require boolean
        // operands. If either sub-expression is a bare identifier with no
        // inferred type — the translator's default is `Int::new_const` —
        // we'd get a spurious TypeMismatch on `P && Q` even though P/Q
        // are clearly propositions. Resolve the impedance mismatch by
        // re-translating both sides as fresh Bool constants for those
        // four operators, mirroring the convention in any predicative
        // type theory.
        if matches!(op, BinOp::And | BinOp::Or | BinOp::Imply | BinOp::Iff) {
            let left_bool = self.translate_expr_as_bool(left)?;
            let right_bool = self.translate_expr_as_bool(right)?;
            return self.translate_bool_binop(op, &left_bool, &right_bool);
        }

        let left_z3 = self.translate_expr(left)?;
        let right_z3 = self.translate_expr(right)?;

        // Equality / disequality between Bool-ish terms. The default
        // translate-then-unify flow sends bare identifiers through the
        // Int-default path, so `p == p` where `p : Bool` and `p &&
        // true == p` get caught in a Bool-vs-Int mismatch. Short-circuit
        // those cases: if either side already translated as Bool, force
        // both sides to Bool and dispatch through `translate_bool_binop`
        // so propositional identity-laws (`p && true == p`,
        // `p || false == p`) close cleanly.
        if matches!(op, BinOp::Eq | BinOp::Ne)
            && (left_z3.as_bool().is_some() || right_z3.as_bool().is_some())
        {
            let left_bool = self.translate_expr_as_bool(left)?;
            let right_bool = self.translate_expr_as_bool(right)?;
            return self.translate_bool_binop(op, &left_bool, &right_bool);
        }

        // Try to cast to integers first
        if let (Some(left_int), Some(right_int)) = (left_z3.as_int(), right_z3.as_int()) {
            match op {
                BinOp::Add => {
                    let result = left_int + right_int;
                    Ok(Dynamic::from_ast(&result))
                }
                BinOp::Sub => {
                    let result = left_int - right_int;
                    Ok(Dynamic::from_ast(&result))
                }
                BinOp::Mul => {
                    let result = left_int * right_int;
                    Ok(Dynamic::from_ast(&result))
                }
                BinOp::Div => {
                    let result = left_int / right_int;
                    Ok(Dynamic::from_ast(&result))
                }
                BinOp::Rem => {
                    let result = left_int.modulo(&right_int);
                    Ok(Dynamic::from_ast(&result))
                }
                BinOp::Eq => {
                    let result = left_int.eq(&right_int);
                    Ok(Dynamic::from_ast(&result))
                }
                BinOp::Ne => {
                    let result = left_int.eq(&right_int).not();
                    Ok(Dynamic::from_ast(&result))
                }
                BinOp::Lt => {
                    let result = left_int.lt(&right_int);
                    Ok(Dynamic::from_ast(&result))
                }
                BinOp::Le => {
                    let result = left_int.le(&right_int);
                    Ok(Dynamic::from_ast(&result))
                }
                BinOp::Gt => {
                    let result = left_int.gt(&right_int);
                    Ok(Dynamic::from_ast(&result))
                }
                BinOp::Ge => {
                    let result = left_int.ge(&right_int);
                    Ok(Dynamic::from_ast(&result))
                }
                BinOp::And | BinOp::Or | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                    Err(TranslationError::TypeMismatch(Text::from(format!(
                        "operator {} requires boolean operands, got integers",
                        op.as_str()
                    ))))
                }
                _ => Err(TranslationError::UnsupportedOp(op.as_str().to_text())),
            }
        } else if self.config.precise_floats {
            // Try IEEE 754 FPA operations when precise_floats is enabled
            if let (Some(left_float), Some(right_float)) =
                (self.try_as_float(&left_z3), self.try_as_float(&right_z3))
            {
                return self.translate_float_binop(op, &left_float, &right_float);
            }

            // Fall through to Real or error
            if let (Some(left_real), Some(right_real)) =
                (left_z3.as_real(), right_z3.as_real())
            {
                self.translate_real_binop(op, &left_real, &right_real)
            } else if let (Some(left_bool), Some(right_bool)) =
                (left_z3.as_bool(), right_z3.as_bool())
            {
                self.translate_bool_binop(op, &left_bool, &right_bool)
            } else {
                Err(TranslationError::TypeMismatch(Text::from(format!(
                    "incompatible types for binary operation {}",
                    op.as_str()
                ))))
            }
        } else if let (Some(left_real), Some(right_real)) = (left_z3.as_real(), right_z3.as_real())
        {
            // Real number operations (approximate float mode)
            self.translate_real_binop(op, &left_real, &right_real)
        } else if let (Some(left_bool), Some(right_bool)) = (left_z3.as_bool(), right_z3.as_bool())
        {
            // Boolean operations
            self.translate_bool_binop(op, &left_bool, &right_bool)
        } else {
            Err(TranslationError::TypeMismatch(Text::from(format!(
                "incompatible types for binary operation {}",
                op.as_str()
            ))))
        }
    }

    /// Translate a binary operation on Real values.
    fn translate_real_binop(
        &self,
        op: BinOp,
        left: &Real,
        right: &Real,
    ) -> Result<Dynamic, TranslationError> {
        match op {
            BinOp::Add => {
                let result = left.clone() + right.clone();
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Sub => {
                let result = left.clone() - right.clone();
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Mul => {
                let result = left.clone() * right.clone();
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Div => {
                let result = left.clone() / right.clone();
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Eq => {
                let result = left.eq(right);
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Ne => {
                let result = left.eq(right).not();
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Lt => {
                let result = left.lt(right);
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Le => {
                let result = left.le(right);
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Gt => {
                let result = left.gt(right);
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Ge => {
                let result = left.ge(right);
                Ok(Dynamic::from_ast(&result))
            }
            _ => Err(TranslationError::UnsupportedOp(op.as_str().to_text())),
        }
    }

    /// Translate a binary operation on Boolean values.
    fn translate_bool_binop(
        &self,
        op: BinOp,
        left: &Bool,
        right: &Bool,
    ) -> Result<Dynamic, TranslationError> {
        match op {
            BinOp::And => {
                let result = Bool::and(&[left, right]);
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Or => {
                let result = Bool::or(&[left, right]);
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Eq => {
                let result = left.eq(right);
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Ne => {
                let result = left.eq(right).not();
                Ok(Dynamic::from_ast(&result))
            }
            // Logical implication and bi-implication. Essential for
            // VC-building: proof-search wraps `hypothesis ⇒ goal` into a
            // `Binary { op: Imply, .. }` tree before handing the whole
            // formula to the translator, so without these arms the
            // solver never sees a translated validity obligation.
            BinOp::Imply => Ok(Dynamic::from_ast(&left.implies(right))),
            BinOp::Iff => Ok(Dynamic::from_ast(&left.iff(right))),
            _ => Err(TranslationError::UnsupportedOp(op.as_str().to_text())),
        }
    }

    /// Try to extract a Float from a Dynamic AST node.
    ///
    /// Returns `Some(Float)` if the Dynamic represents an IEEE 754 floating-point value,
    /// `None` otherwise.
    fn try_as_float(&self, dyn_ast: &Dynamic) -> Option<Float> {
        // Check if the sort is a floating-point sort
        let sort = dyn_ast.get_sort();
        if sort.kind() == SortKind::FloatingPoint {
            // The Dynamic represents a float - wrap it
            let ctx = dyn_ast.get_ctx();
            let z3_ast = dyn_ast.get_z3_ast();
            Some(unsafe { Float::wrap(ctx, z3_ast) })
        } else {
            None
        }
    }

    /// Translate a binary operation on IEEE 754 Float values (FPA theory).
    ///
    /// Uses the configured rounding mode for arithmetic operations.
    /// Comparison operations are exact per IEEE 754 semantics.
    fn translate_float_binop(
        &self,
        op: BinOp,
        left: &Float,
        right: &Float,
    ) -> Result<Dynamic, TranslationError> {
        let rm = self.get_rounding_mode();

        match op {
            // Arithmetic operations with rounding
            BinOp::Add => {
                let result = left.add_with_rounding_mode(right.clone(), &rm);
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Sub => {
                let result = left.sub_with_rounding_mode(right.clone(), &rm);
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Mul => {
                let result = left.mul_with_rounding_mode(right.clone(), &rm);
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Div => {
                let result = left.div_with_rounding_mode(right.clone(), &rm);
                Ok(Dynamic::from_ast(&result))
            }

            // Comparison operations (exact, no rounding)
            BinOp::Eq => {
                let result = left.eq(right);
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Ne => {
                let result = left.eq(right).not();
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Lt => {
                let result = left.lt(right);
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Le => {
                let result = left.le(right);
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Gt => {
                let result = left.gt(right);
                Ok(Dynamic::from_ast(&result))
            }
            BinOp::Ge => {
                let result = left.ge(right);
                Ok(Dynamic::from_ast(&result))
            }

            _ => Err(TranslationError::UnsupportedOp(Text::from(format!(
                "unsupported float operation: {}",
                op.as_str()
            )))),
        }
    }

    /// Translate a unary operation to Z3.
    fn translate_unary_op(
        &self,
        op: verum_ast::UnOp,
        expr: &Expr,
    ) -> Result<Dynamic, TranslationError> {
        let expr_z3 = self.translate_expr(expr)?;

        match op {
            verum_ast::UnOp::Neg => {
                if let Some(int_val) = expr_z3.as_int() {
                    let result = -int_val;
                    Ok(Dynamic::from_ast(&result))
                } else if self.config.precise_floats {
                    // Try FPA negation first when precise floats enabled
                    if let Some(float_val) = self.try_as_float(&expr_z3) {
                        let result = float_val.unary_neg();
                        return Ok(Dynamic::from_ast(&result));
                    }
                    if let Some(real_val) = expr_z3.as_real() {
                        let result = -real_val;
                        Ok(Dynamic::from_ast(&result))
                    } else {
                        Err(TranslationError::TypeMismatch(
                            "negation requires numeric type".to_text(),
                        ))
                    }
                } else if let Some(real_val) = expr_z3.as_real() {
                    let result = -real_val;
                    Ok(Dynamic::from_ast(&result))
                } else {
                    Err(TranslationError::TypeMismatch(
                        "negation requires numeric type".to_text(),
                    ))
                }
            }

            verum_ast::UnOp::Not => {
                if let Some(bool_val) = expr_z3.as_bool() {
                    let result = bool_val.not();
                    Ok(Dynamic::from_ast(&result))
                } else {
                    // Same reasoning as the propositional-operator arm in
                    // `translate_binary_op`: if the operand came in as a
                    // bare identifier the Int-default kicked in, but `!`
                    // clearly wants a Bool. Re-translate as Bool and
                    // retry before giving up.
                    let bool_operand = self.translate_expr_as_bool(expr)?;
                    Ok(Dynamic::from_ast(&bool_operand.not()))
                }
            }

            _ => Err(TranslationError::UnsupportedOp(Text::from(format!(
                "{:?}",
                op
            )))),
        }
    }

    // ========================================================================
    // IEEE 754 Floating-Point Special Value Handling
    // ========================================================================

    /// Check if a floating-point expression has a special property (NaN, infinite, etc.).
    ///
    /// This method translates an expression and checks it against the specified
    /// floating-point property using Z3's FPA theory predicates.
    ///
    /// # Arguments
    ///
    /// * `expr` - The expression to check (must be a Float in precise mode)
    /// * `check` - The type of check to perform
    ///
    /// # Returns
    ///
    /// A Z3 Bool expression that is true iff the expression has the specified property.
    ///
    /// # Errors
    ///
    /// Returns an error if precise floats are disabled or the expression is not a float.
    pub fn translate_float_special_check(
        &self,
        expr: &Expr,
        check: FloatCheck,
    ) -> Result<Bool, TranslationError> {
        if !self.config.precise_floats {
            return Err(TranslationError::UnsupportedExpr(
                "float special value checks require precise_floats mode".to_text(),
            ));
        }

        let expr_z3 = self.translate_expr(expr)?;
        let float_val = self.try_as_float(&expr_z3).ok_or_else(|| {
            TranslationError::TypeMismatch(
                "float special check requires floating-point expression".to_text(),
            )
        })?;

        Ok(self.check_float_property(&float_val, check))
    }

    /// Check a floating-point property on a Float AST node.
    fn check_float_property(&self, float: &Float, check: FloatCheck) -> Bool {
        match check {
            FloatCheck::IsNaN => float.is_nan(),
            FloatCheck::IsInfinite => float.is_infinite(),
            FloatCheck::IsNormal => float.is_normal(),
            FloatCheck::IsSubnormal => float.is_subnormal(),
            FloatCheck::IsZero => float.is_zero(),
            FloatCheck::IsNegative => {
                // Negative: value < 0 (excluding NaN)
                let zero = self.float_zero();
                let lt_zero = float.lt(&zero);
                let not_nan = float.is_nan().not();
                Bool::and(&[&lt_zero, &not_nan])
            }
            FloatCheck::IsPositive => {
                // Positive: value > 0 (excluding NaN)
                let zero = self.float_zero();
                let gt_zero = float.gt(&zero);
                let not_nan = float.is_nan().not();
                Bool::and(&[&gt_zero, &not_nan])
            }
        }
    }

    /// Create a floating-point zero value based on configuration.
    fn float_zero(&self) -> Float {
        match self.config.float_precision {
            FloatPrecision::Float32 => Float::from_f32(0.0),
            FloatPrecision::Float64 => Float::from_f64(0.0),
        }
    }

    /// Create a floating-point NaN value based on configuration.
    pub fn float_nan(&self) -> Result<Float, TranslationError> {
        if !self.config.precise_floats {
            return Err(TranslationError::UnsupportedExpr(
                "float NaN requires precise_floats mode".to_text(),
            ));
        }
        Ok(match self.config.float_precision {
            FloatPrecision::Float32 => Float::nan32(),
            FloatPrecision::Float64 => Float::nan64(),
        })
    }

    /// Create a floating-point positive infinity value.
    pub fn float_positive_infinity(&self) -> Result<Float, TranslationError> {
        if !self.config.precise_floats {
            return Err(TranslationError::UnsupportedExpr(
                "float infinity requires precise_floats mode".to_text(),
            ));
        }
        Ok(match self.config.float_precision {
            FloatPrecision::Float32 => Float::from_f32(f32::INFINITY),
            FloatPrecision::Float64 => Float::from_f64(f64::INFINITY),
        })
    }

    /// Create a floating-point negative infinity value.
    pub fn float_negative_infinity(&self) -> Result<Float, TranslationError> {
        if !self.config.precise_floats {
            return Err(TranslationError::UnsupportedExpr(
                "float infinity requires precise_floats mode".to_text(),
            ));
        }
        Ok(match self.config.float_precision {
            FloatPrecision::Float32 => Float::from_f32(f32::NEG_INFINITY),
            FloatPrecision::Float64 => Float::from_f64(f64::NEG_INFINITY),
        })
    }

    /// Create a symbolic floating-point variable.
    ///
    /// Returns a fresh FPA constant with the configured precision.
    pub fn new_float_const(&self, name: &str) -> Result<Float, TranslationError> {
        if !self.config.precise_floats {
            return Err(TranslationError::UnsupportedExpr(
                "float constants require precise_floats mode".to_text(),
            ));
        }
        Ok(match self.config.float_precision {
            FloatPrecision::Float32 => Float::new_const_float32(name),
            FloatPrecision::Float64 => Float::new_const_double(name),
        })
    }

    /// Get the absolute value of a floating-point expression.
    pub fn float_abs(&self, expr: &Expr) -> Result<Dynamic, TranslationError> {
        if !self.config.precise_floats {
            return Err(TranslationError::UnsupportedExpr(
                "float abs requires precise_floats mode".to_text(),
            ));
        }

        let expr_z3 = self.translate_expr(expr)?;
        let float_val = self.try_as_float(&expr_z3).ok_or_else(|| {
            TranslationError::TypeMismatch("float abs requires floating-point expression".to_text())
        })?;

        let result = float_val.unary_abs();
        Ok(Dynamic::from_ast(&result))
    }

    /// Create a constraint that a float is finite (not NaN and not infinite).
    pub fn float_is_finite(&self, expr: &Expr) -> Result<Bool, TranslationError> {
        if !self.config.precise_floats {
            return Err(TranslationError::UnsupportedExpr(
                "float finite check requires precise_floats mode".to_text(),
            ));
        }

        let expr_z3 = self.translate_expr(expr)?;
        let float_val = self.try_as_float(&expr_z3).ok_or_else(|| {
            TranslationError::TypeMismatch(
                "float finite check requires floating-point expression".to_text(),
            )
        })?;

        // Finite = not NaN and not infinite
        let not_nan = float_val.is_nan().not();
        let not_inf = float_val.is_infinite().not();
        Ok(Bool::and(&[&not_nan, &not_inf]))
    }

    /// Translate a function call to Z3.
    fn translate_call(&self, func: &Expr, args: &[Expr]) -> Result<Dynamic, TranslationError> {
        // Handle known functions
        if let ExprKind::Path(path) = &func.kind {
            if let Some(ident) = path.as_ident() {
                let func_name = ident.as_str();

                match func_name {
                    // Mathematical functions
                    "abs" if args.len() == 1 => {
                        let arg = self.translate_expr(&args[0])?;
                        if let Some(int_val) = arg.as_int() {
                            let zero = Int::from_i64(0);
                            let neg_val = -&int_val;
                            let result = int_val.ge(&zero).ite(&int_val, &neg_val);
                            Ok(Dynamic::from_ast(&result))
                        } else if let Some(real_val) = arg.as_real() {
                            let zero = Real::from_rational(0, 1);
                            let neg_val = -&real_val;
                            let result = real_val.ge(&zero).ite(&real_val, &neg_val);
                            Ok(Dynamic::from_ast(&result))
                        } else {
                            Err(TranslationError::TypeMismatch(
                                "abs requires numeric type".to_text(),
                            ))
                        }
                    }

                    "min" if args.len() == 2 => {
                        let left = self.translate_expr(&args[0])?;
                        let right = self.translate_expr(&args[1])?;
                        if let (Some(left_int), Some(right_int)) = (left.as_int(), right.as_int()) {
                            let result = left_int.lt(&right_int).ite(&left_int, &right_int);
                            Ok(Dynamic::from_ast(&result))
                        } else if let (Some(left_real), Some(right_real)) =
                            (left.as_real(), right.as_real())
                        {
                            let result = left_real.lt(&right_real).ite(&left_real, &right_real);
                            Ok(Dynamic::from_ast(&result))
                        } else {
                            Err(TranslationError::TypeMismatch(
                                "min requires numeric types".to_text(),
                            ))
                        }
                    }

                    "max" if args.len() == 2 => {
                        let left = self.translate_expr(&args[0])?;
                        let right = self.translate_expr(&args[1])?;
                        if let (Some(left_int), Some(right_int)) = (left.as_int(), right.as_int()) {
                            let result = left_int.gt(&right_int).ite(&left_int, &right_int);
                            Ok(Dynamic::from_ast(&result))
                        } else if let (Some(left_real), Some(right_real)) =
                            (left.as_real(), right.as_real())
                        {
                            let result = left_real.gt(&right_real).ite(&left_real, &right_real);
                            Ok(Dynamic::from_ast(&result))
                        } else {
                            Err(TranslationError::TypeMismatch(
                                "max requires numeric types".to_text(),
                            ))
                        }
                    }

                    // Length function for arrays/vectors
                    "len" | "length" if args.len() == 1 => {
                        // For now, treat length as an uninterpreted function
                        // In a more complete implementation, we'd track array lengths
                        let base_name = format!("length_{:?}", args[0]);
                        let int_var = Int::new_const(base_name.as_str());
                        Ok(Dynamic::from_ast(&int_var))
                    }

                    // Power function
                    "pow" if args.len() == 2 => {
                        let base = self.translate_expr(&args[0])?;
                        let exp = self.translate_expr(&args[1])?;
                        if let (Some(base_int), Some(exp_int)) = (base.as_int(), exp.as_int()) {
                            let result = base_int.power(&exp_int);
                            Ok(Dynamic::from_ast(&result))
                        } else {
                            Err(TranslationError::TypeMismatch(
                                "pow requires integer types".to_text(),
                            ))
                        }
                    }

                    // CBGR Generation Predicates
                    // CBGR uses epoch-based generation tracking with dual counters for
                    // memory safety. `generation(ref)` returns the generation counter (u32)
                    // stored in a ThinRef (16 bytes: ptr + generation + epoch_caps).
                    "generation" if args.len() == 1 => {
                        // Treat generation as an uninterpreted function that returns u32
                        // In full implementation, we'd extract from reference bitvector
                        let ref_name = format!("generation_{:?}", args[0]);
                        let gen_var = BV::new_const(ref_name.as_str(), 32);
                        // Generation is always >= 0 (it's unsigned)
                        Ok(Dynamic::from_ast(&gen_var))
                    }

                    // epoch(ref) -> u16: Get epoch counter
                    "epoch" if args.len() == 1 => {
                        // Treat epoch as an uninterpreted function that returns u16
                        let ref_name = format!("epoch_{:?}", args[0]);
                        let epoch_var = BV::new_const(ref_name.as_str(), 16);
                        Ok(Dynamic::from_ast(&epoch_var))
                    }

                    // valid(ref) -> Bool: Check if reference is still valid
                    "valid" if args.len() == 1 => {
                        // Treat validity as an uninterpreted boolean function
                        // In full implementation, check generation <= current_generation
                        let ref_name = format!("valid_{:?}", args[0]);
                        let valid_var = Bool::new_const(ref_name.as_str());
                        Ok(Dynamic::from_ast(&valid_var))
                    }

                    // same_allocation(a, b) -> Bool: Check if refs point to same allocation
                    "same_allocation" if args.len() == 2 => {
                        // Treat same_allocation as comparing generations
                        let gen_a = format!("generation_{:?}", args[0]);
                        let gen_b = format!("generation_{:?}", args[1]);
                        let epoch_a = format!("epoch_{:?}", args[0]);
                        let epoch_b = format!("epoch_{:?}", args[1]);

                        // Same allocation if both generation AND epoch match
                        let gen_a_var = BV::new_const(gen_a.as_str(), 32);
                        let gen_b_var = BV::new_const(gen_b.as_str(), 32);
                        let epoch_a_var = BV::new_const(epoch_a.as_str(), 16);
                        let epoch_b_var = BV::new_const(epoch_b.as_str(), 16);

                        let gen_eq = gen_a_var.eq(&gen_b_var);
                        let epoch_eq = epoch_a_var.eq(&epoch_b_var);
                        let result = Bool::and(&[&gen_eq, &epoch_eq]);
                        Ok(Dynamic::from_ast(&result))
                    }

                    // Quantifiers - now supported via translate_forall/translate_exists
                    "forall" | "exists" => {
                        Err(TranslationError::QuantifierError(Text::from(format!(
                            "{} function calls are not supported; use expression syntax: forall (x: T) => body",
                            func_name
                        ))))
                    }

                    _ => Err(TranslationError::UnsupportedFunction(func_name.to_text())),
                }
            } else {
                Err(TranslationError::UnsupportedFunction(Text::from(format!(
                    "{:?}",
                    path
                ))))
            }
        } else {
            Err(TranslationError::UnsupportedExpr(
                "function call with non-path".to_text(),
            ))
        }
    }

    // ==================== Quantifier Translation ====================
    //
    // Dependent Types — Pi and Sigma Type Quantifier Translation
    // Pi types `(x: A) -> B(x)` become universal quantifiers;
    // Sigma types `(x: A, B(x))` become existential quantifiers.
    // In proof terms, quantifiers appear as `forall x. P(x)` and `exists x. P(x)`.
    //
    // Quantifiers are translated to Z3 using:
    // - forall_const() for universal quantification
    // - exists_const() for existential quantification
    // - Pattern::new() for guided instantiation (MBQI)
    //
    // Example translations:
    //   forall (x: Int) => x + 0 == x
    //   -> Z3: forall x: Int. x + 0 = x
    //
    //   exists (x: Int) => x * x == 4
    //   -> Z3: exists x: Int. x * x = 4
    //
    //   fn sum(list: List<Int{x > 0}>) -> Int{result >= 0}
    //   -> Z3: forall i. 0 <= i < len(list) => list[i] > 0 => result >= 0

    /// Translate a universal quantifier (forall) to Z3.
    ///
    /// Universal quantifiers express that a predicate holds for ALL values of a type.
    ///
    /// ## Supported Forms
    ///
    /// - Type-based: `forall x: T. P(x)` → `∀x:T. P(x)`
    /// - Domain-based: `forall x in S. P(x)` → `∀x. x ∈ S → P(x)`
    /// - Guarded: `forall x in S where Q(x). P(x)` → `∀x. x ∈ S → Q(x) → P(x)`
    /// - Multiple bindings: `forall x: Int, y: Int. P(x, y)` → `∀x,y:Int. P(x,y)`
    ///
    /// ## Translation Strategy
    ///
    /// 1. Extract bound variable names from each binding's pattern
    /// 2. Create fresh Z3 constants for each bound variable
    /// 3. Bind variables in the translator context
    /// 4. Translate domain constraints and guards (if any)
    /// 5. Translate the body expression
    /// 6. Construct implications for domain membership and guards
    /// 7. Create the Z3 forall quantifier with pattern hints
    ///
    /// ## Example
    ///
    /// ```verum
    /// forall x: Int. x + 0 == x
    /// forall x in items. x > 0
    /// forall x in items where x != 0. 1 / x > 0
    /// ```
    ///
    /// Quantifier expressions: translated to Z3 forall_const/exists_const with domain guards
    fn translate_forall(
        &self,
        bindings: &[verum_ast::expr::QuantifierBinding],
        body: &Expr,
    ) -> Result<Dynamic, TranslationError> {
        if bindings.is_empty() {
            return Err(TranslationError::QuantifierError(
                "forall requires at least one binding".to_text(),
            ));
        }

        // For single binding with type annotation, use optimized path
        if bindings.len() == 1 {
            let binding = &bindings[0];
            return self.translate_single_forall(binding, body);
        }

        // Multiple bindings: create nested quantifiers
        // forall x: A, y: B. P(x, y) → forall x: A. forall y: B. P(x, y)
        let mut result_body = self.translate_single_forall(&bindings[bindings.len() - 1], body)?;

        for i in (0..bindings.len() - 1).rev() {
            // Create a synthetic body expression that represents the inner quantifier
            // This is a limitation - we return the innermost result for now
            // Full nested quantifier support requires more complex AST manipulation
            result_body = self.translate_single_forall_with_body(&bindings[i], result_body)?;
        }

        Ok(result_body)
    }

    /// Translate a single forall binding to Z3.
    fn translate_single_forall(
        &self,
        binding: &verum_ast::expr::QuantifierBinding,
        body: &Expr,
    ) -> Result<Dynamic, TranslationError> {
        // Extract variable name from pattern
        let var_name = self.extract_pattern_name(&binding.pattern)?;

        // Get the type - either explicit or inferred from domain
        let ty = match (&binding.ty, &binding.domain) {
            (verum_common::Maybe::Some(t), _) => t.clone(),
            (verum_common::Maybe::None, verum_common::Maybe::Some(_domain)) => {
                // Domain-based quantification - infer Int for now
                // Full implementation would infer element type from domain
                verum_ast::ty::Type::int(binding.span)
            }
            _ => {
                return Err(TranslationError::QuantifierError(
                    "quantifier binding must have type annotation or domain".to_text(),
                ))
            }
        };

        // Create Z3 constant for the bound variable based on type
        let bound_var = self.create_typed_const(var_name.as_str(), &ty)?;

        // Create a new translator with the bound variable in scope
        let mut inner_translator = Translator::new(self.context);

        // Copy existing bindings
        for (name, value) in self.bindings_iter() {
            inner_translator.bind(name.clone(), value.clone());
        }

        // Bind the quantified variable
        inner_translator.bind(var_name.clone(), bound_var.clone());

        // Translate the body with the bound variable in scope
        let body_z3 = inner_translator.translate_expr(body)?;

        // Body must be boolean
        let mut body_bool = body_z3.as_bool().ok_or_else(|| {
            TranslationError::QuantifierError("forall body must be a boolean expression".to_text())
        })?;

        // Handle domain constraint: forall x in S. P(x) → forall x. x ∈ S → P(x)
        if let verum_common::Maybe::Some(domain) = &binding.domain {
            let domain_z3 = inner_translator.translate_expr(domain)?;
            // Create membership constraint (simplified: domain_z3 is assumed boolean)
            if let Some(domain_bool) = domain_z3.as_bool() {
                body_bool = domain_bool.implies(&body_bool);
            }
        }

        // Handle guard: forall x where Q(x). P(x) → forall x. Q(x) → P(x)
        if let verum_common::Maybe::Some(guard) = &binding.guard {
            let guard_z3 = inner_translator.translate_expr(guard)?;
            if let Some(guard_bool) = guard_z3.as_bool() {
                body_bool = guard_bool.implies(&body_bool);
            }
        }

        // Generate instantiation patterns from the body
        let patterns = self.generate_quantifier_patterns(&bound_var, body)?;

        // Create the Z3 forall quantifier based on variable type
        match (bound_var.as_int(), bound_var.as_bool(), bound_var.as_real()) {
            (Some(_), _, _) => {
                let int_const = Int::new_const(var_name.as_str());
                let pattern_refs: Vec<&Z3Pattern> = patterns.iter().collect();
                let forall = forall_const(&[&int_const as &dyn Ast], &pattern_refs, &body_bool);
                Ok(Dynamic::from_ast(&forall))
            }
            (_, Some(_), _) => {
                let bool_const = Bool::new_const(var_name.as_str());
                let pattern_refs: Vec<&Z3Pattern> = patterns.iter().collect();
                let forall = forall_const(&[&bool_const as &dyn Ast], &pattern_refs, &body_bool);
                Ok(Dynamic::from_ast(&forall))
            }
            (_, _, Some(_)) => {
                let real_const = Real::new_const(var_name.as_str());
                let pattern_refs: Vec<&Z3Pattern> = patterns.iter().collect();
                let forall = forall_const(&[&real_const as &dyn Ast], &pattern_refs, &body_bool);
                Ok(Dynamic::from_ast(&forall))
            }
            _ => {
                // Default to Int for unknown types
                let int_const = Int::new_const(var_name.as_str());
                let pattern_refs: Vec<&Z3Pattern> = patterns.iter().collect();
                let forall = forall_const(&[&int_const as &dyn Ast], &pattern_refs, &body_bool);
                Ok(Dynamic::from_ast(&forall))
            }
        }
    }

    /// Translate a single forall binding with an already-translated body.
    fn translate_single_forall_with_body(
        &self,
        binding: &verum_ast::expr::QuantifierBinding,
        inner_body: Dynamic,
    ) -> Result<Dynamic, TranslationError> {
        // Extract variable name from pattern
        let var_name = self.extract_pattern_name(&binding.pattern)?;

        // Get the type
        let ty = match &binding.ty {
            verum_common::Maybe::Some(t) => t.clone(),
            verum_common::Maybe::None => verum_ast::ty::Type::int(binding.span),
        };

        // Create Z3 constant for the bound variable based on type
        let bound_var = self.create_typed_const(var_name.as_str(), &ty)?;

        let mut body_bool = inner_body.as_bool().ok_or_else(|| {
            TranslationError::QuantifierError("forall body must be a boolean expression".to_text())
        })?;

        // Handle domain and guard constraints
        let mut inner_translator = Translator::new(self.context);
        for (name, value) in self.bindings_iter() {
            inner_translator.bind(name.clone(), value.clone());
        }
        inner_translator.bind(var_name.clone(), bound_var.clone());

        if let verum_common::Maybe::Some(domain) = &binding.domain {
            let domain_z3 = inner_translator.translate_expr(domain)?;
            if let Some(domain_bool) = domain_z3.as_bool() {
                body_bool = domain_bool.implies(&body_bool);
            }
        }

        if let verum_common::Maybe::Some(guard) = &binding.guard {
            let guard_z3 = inner_translator.translate_expr(guard)?;
            if let Some(guard_bool) = guard_z3.as_bool() {
                body_bool = guard_bool.implies(&body_bool);
            }
        }

        // Create the Z3 forall quantifier
        match (bound_var.as_int(), bound_var.as_bool(), bound_var.as_real()) {
            (Some(_), _, _) => {
                let int_const = Int::new_const(var_name.as_str());
                let forall = forall_const(&[&int_const as &dyn Ast], &[], &body_bool);
                Ok(Dynamic::from_ast(&forall))
            }
            (_, Some(_), _) => {
                let bool_const = Bool::new_const(var_name.as_str());
                let forall = forall_const(&[&bool_const as &dyn Ast], &[], &body_bool);
                Ok(Dynamic::from_ast(&forall))
            }
            (_, _, Some(_)) => {
                let real_const = Real::new_const(var_name.as_str());
                let forall = forall_const(&[&real_const as &dyn Ast], &[], &body_bool);
                Ok(Dynamic::from_ast(&forall))
            }
            _ => {
                let int_const = Int::new_const(var_name.as_str());
                let forall = forall_const(&[&int_const as &dyn Ast], &[], &body_bool);
                Ok(Dynamic::from_ast(&forall))
            }
        }
    }

    /// Translate an existential quantifier (exists) to Z3.
    ///
    /// Existential quantifiers express that there EXISTS at least one value
    /// satisfying a predicate.
    ///
    /// ## Supported Forms
    ///
    /// - Type-based: `exists x: T. P(x)` → `∃x:T. P(x)`
    /// - Domain-based: `exists x in S. P(x)` → `∃x. x ∈ S ∧ P(x)`
    /// - Guarded: `exists x in S where Q(x). P(x)` → `∃x. x ∈ S ∧ Q(x) ∧ P(x)`
    ///
    /// Note: For exists, domain and guard are conjoined (∧) not implication (→)
    ///
    /// Quantifier expressions: translated to Z3 forall_const/exists_const with domain guards
    fn translate_exists(
        &self,
        bindings: &[verum_ast::expr::QuantifierBinding],
        body: &Expr,
    ) -> Result<Dynamic, TranslationError> {
        if bindings.is_empty() {
            return Err(TranslationError::QuantifierError(
                "exists requires at least one binding".to_text(),
            ));
        }

        // For single binding, use optimized path
        if bindings.len() == 1 {
            return self.translate_single_exists(&bindings[0], body);
        }

        // Multiple bindings: create nested quantifiers
        let mut result_body = self.translate_single_exists(&bindings[bindings.len() - 1], body)?;

        for i in (0..bindings.len() - 1).rev() {
            result_body = self.translate_single_exists_with_body(&bindings[i], result_body)?;
        }

        Ok(result_body)
    }

    /// Translate a single exists binding to Z3.
    fn translate_single_exists(
        &self,
        binding: &verum_ast::expr::QuantifierBinding,
        body: &Expr,
    ) -> Result<Dynamic, TranslationError> {
        // Extract variable name from pattern
        let var_name = self.extract_pattern_name(&binding.pattern)?;

        // Get the type - either explicit or inferred from domain
        let ty = match (&binding.ty, &binding.domain) {
            (verum_common::Maybe::Some(t), _) => t.clone(),
            (verum_common::Maybe::None, verum_common::Maybe::Some(_)) => {
                verum_ast::ty::Type::int(binding.span)
            }
            _ => {
                return Err(TranslationError::QuantifierError(
                    "quantifier binding must have type annotation or domain".to_text(),
                ))
            }
        };

        // Create Z3 constant for the bound variable based on type
        let bound_var = self.create_typed_const(var_name.as_str(), &ty)?;

        // Create a new translator with the bound variable in scope
        let mut inner_translator = Translator::new(self.context);

        // Copy existing bindings
        for (name, value) in self.bindings_iter() {
            inner_translator.bind(name.clone(), value.clone());
        }

        // Bind the quantified variable
        inner_translator.bind(var_name.clone(), bound_var.clone());

        // Translate the body with the bound variable in scope
        let body_z3 = inner_translator.translate_expr(body)?;

        // Body must be boolean
        let mut body_bool = body_z3.as_bool().ok_or_else(|| {
            TranslationError::QuantifierError("exists body must be a boolean expression".to_text())
        })?;

        // Handle domain constraint: exists x in S. P(x) → exists x. x ∈ S ∧ P(x)
        if let verum_common::Maybe::Some(domain) = &binding.domain {
            let domain_z3 = inner_translator.translate_expr(domain)?;
            if let Some(domain_bool) = domain_z3.as_bool() {
                body_bool = Bool::and(&[&domain_bool, &body_bool]);
            }
        }

        // Handle guard: exists x where Q(x). P(x) → exists x. Q(x) ∧ P(x)
        if let verum_common::Maybe::Some(guard) = &binding.guard {
            let guard_z3 = inner_translator.translate_expr(guard)?;
            if let Some(guard_bool) = guard_z3.as_bool() {
                body_bool = Bool::and(&[&guard_bool, &body_bool]);
            }
        }

        // Generate instantiation patterns from the body
        let patterns = self.generate_quantifier_patterns(&bound_var, body)?;

        // Create the Z3 exists quantifier based on variable type
        match (bound_var.as_int(), bound_var.as_bool(), bound_var.as_real()) {
            (Some(_), _, _) => {
                let int_const = Int::new_const(var_name.as_str());
                let pattern_refs: Vec<&Z3Pattern> = patterns.iter().collect();
                let exists = exists_const(&[&int_const as &dyn Ast], &pattern_refs, &body_bool);
                Ok(Dynamic::from_ast(&exists))
            }
            (_, Some(_), _) => {
                let bool_const = Bool::new_const(var_name.as_str());
                let pattern_refs: Vec<&Z3Pattern> = patterns.iter().collect();
                let exists = exists_const(&[&bool_const as &dyn Ast], &pattern_refs, &body_bool);
                Ok(Dynamic::from_ast(&exists))
            }
            (_, _, Some(_)) => {
                let real_const = Real::new_const(var_name.as_str());
                let pattern_refs: Vec<&Z3Pattern> = patterns.iter().collect();
                let exists = exists_const(&[&real_const as &dyn Ast], &pattern_refs, &body_bool);
                Ok(Dynamic::from_ast(&exists))
            }
            _ => {
                // Default to Int for unknown types
                let int_const = Int::new_const(var_name.as_str());
                let pattern_refs: Vec<&Z3Pattern> = patterns.iter().collect();
                let exists = exists_const(&[&int_const as &dyn Ast], &pattern_refs, &body_bool);
                Ok(Dynamic::from_ast(&exists))
            }
        }
    }

    /// Translate a single exists binding with an already-translated body.
    fn translate_single_exists_with_body(
        &self,
        binding: &verum_ast::expr::QuantifierBinding,
        inner_body: Dynamic,
    ) -> Result<Dynamic, TranslationError> {
        let var_name = self.extract_pattern_name(&binding.pattern)?;

        let ty = match &binding.ty {
            verum_common::Maybe::Some(t) => t.clone(),
            verum_common::Maybe::None => verum_ast::ty::Type::int(binding.span),
        };

        let bound_var = self.create_typed_const(var_name.as_str(), &ty)?;

        let mut body_bool = inner_body.as_bool().ok_or_else(|| {
            TranslationError::QuantifierError("exists body must be a boolean expression".to_text())
        })?;

        let mut inner_translator = Translator::new(self.context);
        for (name, value) in self.bindings_iter() {
            inner_translator.bind(name.clone(), value.clone());
        }
        inner_translator.bind(var_name.clone(), bound_var.clone());

        if let verum_common::Maybe::Some(domain) = &binding.domain {
            let domain_z3 = inner_translator.translate_expr(domain)?;
            if let Some(domain_bool) = domain_z3.as_bool() {
                body_bool = Bool::and(&[&domain_bool, &body_bool]);
            }
        }

        if let verum_common::Maybe::Some(guard) = &binding.guard {
            let guard_z3 = inner_translator.translate_expr(guard)?;
            if let Some(guard_bool) = guard_z3.as_bool() {
                body_bool = Bool::and(&[&guard_bool, &body_bool]);
            }
        }

        match (bound_var.as_int(), bound_var.as_bool(), bound_var.as_real()) {
            (Some(_), _, _) => {
                let int_const = Int::new_const(var_name.as_str());
                let exists = exists_const(&[&int_const as &dyn Ast], &[], &body_bool);
                Ok(Dynamic::from_ast(&exists))
            }
            (_, Some(_), _) => {
                let bool_const = Bool::new_const(var_name.as_str());
                let exists = exists_const(&[&bool_const as &dyn Ast], &[], &body_bool);
                Ok(Dynamic::from_ast(&exists))
            }
            (_, _, Some(_)) => {
                let real_const = Real::new_const(var_name.as_str());
                let exists = exists_const(&[&real_const as &dyn Ast], &[], &body_bool);
                Ok(Dynamic::from_ast(&exists))
            }
            _ => {
                let int_const = Int::new_const(var_name.as_str());
                let exists = exists_const(&[&int_const as &dyn Ast], &[], &body_bool);
                Ok(Dynamic::from_ast(&exists))
            }
        }
    }

    /// Extract the variable name from a quantifier pattern.
    ///
    /// Quantifier patterns in Verum are restricted to simple identifier patterns
    /// for the bound variable.
    ///
    /// ## Supported Patterns
    ///
    /// - `x` - Simple identifier
    /// - `(x)` - Parenthesized identifier
    ///
    /// ## Unsupported Patterns
    ///
    /// - `_` - Wildcard (variable must be named for SMT translation)
    /// - `(x, y)` - Tuple patterns (multi-variable quantifiers not yet supported)
    /// - Pattern matching on constructors
    fn extract_pattern_name(&self, pattern: &Pattern) -> Result<Text, TranslationError> {
        match &pattern.kind {
            PatternKind::Ident { name, .. } => Ok(name.as_str().to_text()),
            PatternKind::Wildcard => Err(TranslationError::QuantifierError(
                "wildcard pattern not allowed in quantifiers; use a named variable".to_text(),
            )),
            PatternKind::Paren(inner) => {
                // Unwrap parenthesized pattern
                self.extract_pattern_name(inner)
            }
            _ => Err(TranslationError::QuantifierError(Text::from(format!(
                "unsupported quantifier pattern: {:?}",
                pattern.kind
            )))),
        }
    }

    /// Create a Z3 constant with the appropriate type.
    ///
    /// This is used to create bound variables for quantifiers with the
    /// correct Z3 sort based on the Verum type annotation.
    fn create_typed_const(&self, name: &str, ty: &Type) -> Result<Dynamic, TranslationError> {
        match &ty.kind {
            TypeKind::Int => {
                let const_var = Int::new_const(name);
                Ok(Dynamic::from_ast(&const_var))
            }
            TypeKind::Bool => {
                let const_var = Bool::new_const(name);
                Ok(Dynamic::from_ast(&const_var))
            }
            TypeKind::Float => {
                if self.config.precise_floats {
                    let const_var = match self.config.float_precision {
                        FloatPrecision::Float32 => Float::new_const_float32(name),
                        FloatPrecision::Float64 => Float::new_const_double(name),
                    };
                    Ok(Dynamic::from_ast(&const_var))
                } else {
                    let const_var = Real::new_const(name);
                    Ok(Dynamic::from_ast(&const_var))
                }
            }
            TypeKind::Refined { base, .. } => {
                // For refined types, use the base type for the Z3 constant
                self.create_typed_const(name, base)
            }
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    match ident.as_str() {
                        "Int" | "i32" | "i64" | "isize" | "u32" | "u64" | "usize" => {
                            let const_var = Int::new_const(name);
                            Ok(Dynamic::from_ast(&const_var))
                        }
                        "Bool" | "bool" => {
                            let const_var = Bool::new_const(name);
                            Ok(Dynamic::from_ast(&const_var))
                        }
                        "Float" | "f32" | "f64" => {
                            if self.config.precise_floats {
                                let const_var = match self.config.float_precision {
                                    FloatPrecision::Float32 => Float::new_const_float32(name),
                                    FloatPrecision::Float64 => Float::new_const_double(name),
                                };
                                Ok(Dynamic::from_ast(&const_var))
                            } else {
                                let const_var = Real::new_const(name);
                                Ok(Dynamic::from_ast(&const_var))
                            }
                        }
                        "Text" | "String" => {
                            let const_var = Z3String::new_const(name);
                            Ok(Dynamic::from_ast(&const_var))
                        }
                        type_name => {
                            // For unknown types, default to Int (can be extended)
                            // In a full implementation, we'd look up the type in a type environment
                            let const_var = Int::new_const(name);
                            Ok(Dynamic::from_ast(&const_var))
                        }
                    }
                } else {
                    // Complex path - default to Int
                    let const_var = Int::new_const(name);
                    Ok(Dynamic::from_ast(&const_var))
                }
            }
            _ => {
                // For unsupported types, create an uninterpreted constant
                // This allows verification to proceed with symbolic reasoning
                let const_var = Int::new_const(name);
                Ok(Dynamic::from_ast(&const_var))
            }
        }
    }

    /// Generate instantiation patterns for quantifier MBQI.
    ///
    /// Z3's Model-Based Quantifier Instantiation (MBQI) uses patterns to guide
    /// when quantifiers should be instantiated. Good patterns improve both
    /// performance and completeness of verification.
    ///
    /// ## Pattern Generation Strategy
    ///
    /// 1. **Function applications**: If the body contains f(x), use f(x) as a pattern
    /// 2. **Array accesses**: If the body contains arr[x], use (select arr x) as a pattern
    /// 3. **Method calls**: If the body contains obj.method(x), use method(obj, x) as a pattern
    /// 4. **Field accesses**: If the body contains x.field, use field(x) as a pattern
    ///
    /// ## Example
    ///
    /// For `forall (x: Int) => f(x) > 0`, the pattern would be `f(x)`.
    /// This tells Z3: "instantiate this forall whenever you see f(something)".
    ///
    /// ## Notes
    ///
    /// - Empty patterns list lets Z3 auto-generate patterns (may be less efficient)
    /// - Multiple patterns create multi-patterns (all must match for instantiation)
    /// - Patterns are sorted by priority: function apps > method calls > index > field > arithmetic
    fn generate_quantifier_patterns(
        &self,
        bound_var: &Dynamic,
        body: &Expr,
    ) -> Result<List<Z3Pattern>, TranslationError> {
        // Extract the variable name from the bound_var
        // For now we use a heuristic based on the AST string representation
        let var_name_str = format!("{:?}", bound_var);

        // Try to extract the actual variable name from the Z3 representation
        // The format is typically something like "Int(name)" or similar
        let var_name = if let Some(start) = var_name_str.find("name: ") {
            let rest = &var_name_str[start + 6..];
            if let Some(end) = rest.find([',', ')']) {
                rest[..end].trim_matches('"').to_string()
            } else {
                // Fallback: use a generic name
                "x".to_string()
            }
        } else if let Some(start) = var_name_str.find('(') {
            if let Some(end) = var_name_str[start + 1..].find(')') {
                var_name_str[start + 1..start + 1 + end].to_string()
            } else {
                "x".to_string()
            }
        } else {
            "x".to_string()
        };

        // Create bound vars list with the extracted name
        let bound_vars = vec![var_name.to_text()];

        // Extract pattern triggers from the body
        let triggers = self.extract_pattern_triggers(body, &bound_vars);

        // If no triggers found, return empty list (let Z3 use MBQI auto-generation)
        if triggers.is_empty() {
            return Ok(List::new());
        }

        // Create Z3 variable mapping
        let mut z3_vars = Map::new();
        z3_vars.insert(var_name.to_text(), bound_var.clone());

        // Use default configuration
        let config = PatternGenConfig::default();

        // Convert triggers to Z3 patterns
        self.triggers_to_z3_patterns(&triggers, &z3_vars, &config)
    }

    /// Generate quantifier patterns with a custom configuration.
    ///
    /// This is a more flexible version of `generate_quantifier_patterns` that
    /// allows specifying the variable name and configuration explicitly.
    ///
    /// # Arguments
    ///
    /// * `var_name` - Name of the bound variable
    /// * `bound_var` - Z3 AST for the bound variable
    /// * `body` - Quantifier body expression
    /// * `config` - Pattern generation configuration
    ///
    /// # Returns
    ///
    /// List of Z3 patterns for the quantifier
    pub fn generate_quantifier_patterns_with_config(
        &self,
        var_name: &str,
        bound_var: &Dynamic,
        body: &Expr,
        config: &PatternGenConfig,
    ) -> Result<List<Z3Pattern>, TranslationError> {
        let bound_vars = vec![var_name.to_text()];
        let triggers = self.extract_pattern_triggers(body, &bound_vars);

        if triggers.is_empty() {
            return Ok(List::new());
        }

        let mut z3_vars = Map::new();
        z3_vars.insert(var_name.to_text(), bound_var.clone());

        if config.enable_multi_patterns {
            // Group related triggers and create multi-patterns
            let groups = self.group_triggers(&triggers);
            self.groups_to_z3_multi_patterns(&groups, &z3_vars)
        } else {
            // Create single-term patterns
            self.triggers_to_z3_patterns(&triggers, &z3_vars, config)
        }
    }

    /// Generate patterns for multiple bound variables.
    ///
    /// Used for quantifiers with multiple bound variables like:
    /// `forall (x: Int, y: Int) => f(x, y) > 0`
    ///
    /// # Arguments
    ///
    /// * `var_names` - Names of the bound variables
    /// * `bound_vars` - Z3 ASTs for the bound variables (parallel with var_names)
    /// * `body` - Quantifier body expression
    ///
    /// # Returns
    ///
    /// List of Z3 patterns for the quantifier
    pub fn generate_multi_var_patterns(
        &self,
        var_names: &[Text],
        bound_vars: &[Dynamic],
        body: &Expr,
    ) -> Result<List<Z3Pattern>, TranslationError> {
        if var_names.len() != bound_vars.len() {
            return Err(TranslationError::PatternError(
                "var_names and bound_vars must have same length".to_text(),
            ));
        }

        // Extract triggers using all bound variable names
        let triggers = self.extract_pattern_triggers(body, var_names);

        if triggers.is_empty() {
            return Ok(List::new());
        }

        // Create Z3 variable mapping
        let mut z3_vars = Map::new();
        for (name, var) in var_names.iter().zip(bound_vars.iter()) {
            z3_vars.insert(name.clone(), var.clone());
        }

        let config = PatternGenConfig::default();
        self.triggers_to_z3_patterns(&triggers, &z3_vars, &config)
    }

    /// Create a Z3 variable for a given type.
    ///
    /// This is used to create fresh variables when verifying refinement constraints.
    pub fn create_var(&self, name: &str, ty: &Type) -> Result<Dynamic, TranslationError> {
        match &ty.kind {
            TypeKind::Int => {
                let var = Int::new_const(name);
                Ok(Dynamic::from_ast(&var))
            }

            TypeKind::Bool => {
                let var = Bool::new_const(name);
                Ok(Dynamic::from_ast(&var))
            }

            TypeKind::Float => {
                let var = Real::new_const(name);
                Ok(Dynamic::from_ast(&var))
            }

            TypeKind::Refined { base, .. } => {
                // Create variable for the base type
                self.create_var(name, base)
            }

            TypeKind::Path(path) => {
                // Try to resolve named types
                if let Some(ident) = path.as_ident() {
                    match ident.as_str() {
                        "Int" => {
                            let var = Int::new_const(name);
                            Ok(Dynamic::from_ast(&var))
                        }
                        "Bool" => {
                            let var = Bool::new_const(name);
                            Ok(Dynamic::from_ast(&var))
                        }
                        "Float" => {
                            let var = Real::new_const(name);
                            Ok(Dynamic::from_ast(&var))
                        }
                        _ => Err(TranslationError::UnsupportedType(Text::from(format!(
                            "named type: {}",
                            ident.as_str()
                        )))),
                    }
                } else {
                    Err(TranslationError::UnsupportedType(Text::from(format!(
                        "path: {:?}",
                        path
                    ))))
                }
            }

            TypeKind::Tensor { element, shape, .. } => {
                // Create a symbolic tensor variable using Z3 Array theory
                // For multi-dimensional tensors: Array[Int -> Array[Int -> ... -> T]]
                // For now, create a base array and store shape info separately
                self.create_tensor_var(name, element, shape)
            }

            _ => Err(TranslationError::UnsupportedType(Text::from(format!(
                "{:?}",
                ty.kind
            )))),
        }
    }

    /// Create a Z3 variable for a tensor type using Array theory.
    ///
    /// For a tensor with shape [N, M, K], we create nested arrays:
    /// Array[Int -> Array[Int -> Array[Int -> T]]]
    ///
    /// The shape constraints are tracked separately and can be verified.
    fn create_tensor_var(
        &self,
        name: &str,
        element: &Type,
        shape: &[Expr],
    ) -> Result<Dynamic, TranslationError> {
        if shape.is_empty() {
            return Err(TranslationError::TensorShapeError(
                "tensor must have at least one dimension".to_text(),
            ));
        }

        // Get the element sort for the innermost array
        let element_sort = self.element_type_to_sort(element)?;

        // Build nested array sorts from innermost to outermost
        // For a 3D tensor [N, M, K] with element type T:
        // - Inner: Array[Int -> T]
        // - Middle: Array[Int -> Array[Int -> T]]
        // - Outer: Array[Int -> Array[Int -> Array[Int -> T]]]
        let mut current_sort = element_sort;
        for _ in 0..shape.len() {
            current_sort = Sort::array(&Sort::int(), &current_sort);
        }

        // Create the tensor variable with the nested array sort
        let tensor_array = Array::new_const(name, &Sort::int(), &current_sort);

        // Note: Dimension size constraints are NOT automatically enforced here.
        // Bounds checking should be performed at the verification level by:
        //
        // 1. Calling `create_dimension_constraints(tensor_name, shape)` to get dimension sizes
        // 2. For each index operation on the tensor, calling `create_bounds_constraint(index, dim_size)`
        // 3. Asserting the resulting constraint in the SMT solver
        //
        // This separation of concerns allows the verification layer to control when and how
        // bounds checking is performed (e.g., selective checking, optimization-dependent).
        //
        // Example usage in verification code:
        // ```
        // let dim_constraints = translator.create_dimension_constraints("tensor", &shape)?;
        // for (dim_idx, dim_size) in dim_constraints {
        //     let bounds_check = translator.create_bounds_constraint(&index, &dim_size);
        //     solver.assert(&bounds_check);
        // }
        // ```

        Ok(Dynamic::from_ast(&tensor_array))
    }

    /// Translate a Verum element type to a Z3 Sort.
    ///
    /// Maps:
    /// - Int -> Sort::int()
    /// - Float -> Sort::real() or Sort::double() (if precise_floats enabled)
    /// - Bool -> Sort::bool()
    fn element_type_to_sort(&self, ty: &Type) -> Result<Sort, TranslationError> {
        match &ty.kind {
            TypeKind::Int => Ok(Sort::int()),
            TypeKind::Bool => Ok(Sort::bool()),
            TypeKind::Float => Ok(self.get_float_sort()),

            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    let tn = ident.as_str();
                    match tn {
                        _ if verum_common::well_known_types::type_names::is_integer_type(tn) => Ok(Sort::int()),
                        "Bool" => Ok(Sort::bool()),
                        _ if verum_common::well_known_types::type_names::is_float_type(tn) => Ok(self.get_float_sort()),
                        name => Err(TranslationError::UnsupportedType(Text::from(format!(
                            "unsupported element type: {}",
                            name
                        )))),
                    }
                } else {
                    Err(TranslationError::UnsupportedType(Text::from(format!(
                        "complex path element type: {:?}",
                        path
                    ))))
                }
            }

            TypeKind::Refined { base, .. } => {
                // Use the base type for the element sort
                self.element_type_to_sort(base)
            }

            _ => Err(TranslationError::UnsupportedType(Text::from(format!(
                "unsupported tensor element type: {:?}",
                ty.kind
            )))),
        }
    }

    /// Generate bounds checking constraints for tensor indexing.
    ///
    /// For an index `i` accessing dimension with size `N`, generates:
    /// `0 <= i && i < N`
    ///
    /// This method should be called at the verification level (not during translation)
    /// to generate bounds checking assertions for tensor operations.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // After translating tensor and index expressions:
    /// let index_z3 = translator.translate_expr(&index_expr)?.as_int().unwrap();
    /// let dim_constraints = translator.create_dimension_constraints("my_tensor", &shape)?;
    /// let (_, dim_size) = &dim_constraints[0]; // First dimension
    /// let bounds_check = translator.create_bounds_constraint(&index_z3, dim_size);
    /// solver.assert(&bounds_check); // Ensure index is within bounds
    /// ```
    pub fn create_bounds_constraint(
        &self,
        index: &z3::ast::Int,
        dimension_size: &z3::ast::Int,
    ) -> z3::ast::Bool {
        let zero = Int::from_i64(0);
        let lower_bound = index.ge(&zero);
        let upper_bound = index.lt(dimension_size);
        Bool::and(&[&lower_bound, &upper_bound])
    }

    /// Generate all dimension size constraints for a tensor.
    ///
    /// For a tensor variable with dimensions [N, M, K], this generates
    /// symbolic constants for each dimension and returns them for verification.
    ///
    /// This method extracts dimension information that can be used with
    /// `create_bounds_constraint` to verify safe tensor indexing operations.
    ///
    /// # Arguments
    ///
    /// * `tensor_name` - Name of the tensor variable (used to generate unique dimension constant names)
    /// * `shape` - Shape expressions (can be concrete integers or symbolic meta parameters)
    ///
    /// # Returns
    ///
    /// List of (dimension_index, dimension_size_constant) pairs
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // For tensor: Tensor<f32, [10, N, 20]>
    /// let dims = translator.create_dimension_constraints("my_tensor", &shape)?;
    /// // dims[0] = (0, Int::from_i64(10))    // Concrete dimension 0
    /// // dims[1] = (1, Int::new_const("N"))  // Symbolic dimension 1
    /// // dims[2] = (2, Int::from_i64(20))    // Concrete dimension 2
    /// ```
    pub fn create_dimension_constraints(
        &self,
        tensor_name: &str,
        shape: &[Expr],
    ) -> Result<List<(usize, Int)>, TranslationError> {
        let mut constraints = List::new();

        for (dim_idx, dim_expr) in shape.iter().enumerate() {
            let dim_name = format!("{}_dim{}", tensor_name, dim_idx);
            let dim_size = match &dim_expr.kind {
                ExprKind::Literal(lit) => {
                    if let LiteralKind::Int(i) = &lit.kind {
                        // Concrete dimension size
                        Int::from_i64(i.value as i64)
                    } else {
                        return Err(TranslationError::TensorShapeError(
                            "tensor shape must be integer literal or meta parameter".to_text(),
                        ));
                    }
                }
                ExprKind::Path(path) => {
                    // Meta parameter - symbolic dimension
                    let meta_name = if let Some(ident) = path.as_ident() {
                        ident.as_str()
                    } else {
                        &dim_name
                    };
                    Int::new_const(meta_name)
                }
                _ => {
                    return Err(TranslationError::TensorShapeError(
                        "complex tensor shape expressions not yet supported".to_text(),
                    ));
                }
            };

            constraints.push((dim_idx, dim_size));
        }

        Ok(constraints)
    }

    /// Translate a tensor type to Z3 Array sort.
    ///
    /// This method creates the appropriate Z3 sort for tensor types:
    /// - 1D tensor: Array[Int -> Element]
    /// - 2D tensor: Array[Int -> Array[Int -> Element]]
    /// - ND tensor: nested arrays
    pub fn translate_tensor_type(
        &self,
        element: &Type,
        shape: &[Expr],
    ) -> Result<TensorSort, TranslationError> {
        // Validate shape expressions are constants or meta parameters
        let mut shape_sizes = List::new();
        for dim_expr in shape {
            match &dim_expr.kind {
                ExprKind::Literal(lit) => {
                    if let LiteralKind::Int(i) = &lit.kind {
                        shape_sizes.push(i.value as usize);
                    } else {
                        return Err(TranslationError::UnsupportedExpr(
                            "tensor shape must be integer literal".to_text(),
                        ));
                    }
                }
                ExprKind::Path(path) => {
                    // Meta parameter - symbolic size
                    // Symbolic dimensions are represented as 0 in the dimension array.
                    // The actual symbolic constraint is handled by creating a Z3 integer
                    // constant with the meta parameter name. This enables shape-dependent
                    // verification where tensor dimensions are constrained by SMT formulas.
                    //
                    // For example, Matrix<T, N, M> creates symbolic constants N and M
                    // that can appear in verification conditions like N > 0 && M > 0.
                    //
                    // The symbolic constant is created lazily when the tensor is used
                    // in verification conditions, not at type translation time.
                    shape_sizes.push(0); // Marker for symbolic dimension (resolved during SMT encoding)
                }
                _ => {
                    return Err(TranslationError::UnsupportedExpr(
                        "complex tensor shape expressions not yet supported".to_text(),
                    ));
                }
            }
        }

        // Get element sort name
        let element_sort = self.translate_type(element)?;

        Ok(TensorSort {
            element_type: element_sort,
            dimensions: shape_sizes,
            ndim: shape.len(),
        })
    }

    /// Translate a type to a Z3 sort.
    pub fn translate_type(&self, ty: &Type) -> Result<Text, TranslationError> {
        match &ty.kind {
            TypeKind::Int => Ok("Int".to_text()),
            TypeKind::Bool => Ok("Bool".to_text()),
            TypeKind::Float => Ok("Real".to_text()),
            TypeKind::Refined { base, .. } => self.translate_type(base),
            TypeKind::Tensor { element, shape, .. } => {
                // For tensor types, return element type (array theory handled separately)
                self.translate_type(element)
            }
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    match ident.as_str() {
                        "Int" => Ok("Int".to_text()),
                        "Bool" => Ok("Bool".to_text()),
                        "Float" => Ok("Real".to_text()),
                        name => Ok(name.to_text()),
                    }
                } else {
                    Err(TranslationError::UnsupportedType(Text::from(format!(
                        "path: {:?}",
                        path
                    ))))
                }
            }
            _ => Err(TranslationError::UnsupportedType(Text::from(format!(
                "{:?}",
                ty.kind
            )))),
        }
    }
}

/// Z3 sort information for tensor types
#[derive(Debug, Clone)]
pub struct TensorSort {
    /// Element type sort name (Int, Real, Bool)
    pub element_type: Text,
    /// Dimension sizes (0 means symbolic/meta parameter)
    pub dimensions: List<usize>,
    /// Number of dimensions
    pub ndim: usize,
}

impl TensorSort {
    /// Check if all dimensions are concrete (no meta parameters)
    pub fn is_concrete(&self) -> bool {
        self.dimensions.iter().all(|&d| d > 0)
    }

    /// Get total number of elements (product of dimensions)
    /// Returns None if any dimension is symbolic
    pub fn total_elements(&self) -> Maybe<usize> {
        if !self.is_concrete() {
            return Maybe::None;
        }

        let product = self.dimensions.iter().product();
        Maybe::Some(product)
    }

    /// Get dimension at index
    pub fn dim(&self, index: usize) -> Maybe<usize> {
        if index >= self.ndim {
            Maybe::None
        } else {
            Maybe::Some(self.dimensions[index])
        }
    }
}

/// Errors that can occur during translation.
#[derive(Debug, Clone, thiserror::Error)]
pub enum TranslationError {
    /// Unsupported expression kind
    #[error("unsupported expression: {0}")]
    UnsupportedExpr(Text),

    /// Unsupported literal kind
    #[error("unsupported literal: {0}")]
    UnsupportedLiteral(Text),

    /// Unsupported operation
    #[error("unsupported operation: {0}")]
    UnsupportedOp(Text),

    /// Unsupported function
    #[error("unsupported function: {0}")]
    UnsupportedFunction(Text),

    /// Unsupported type
    #[error("unsupported type: {0}")]
    UnsupportedType(Text),

    /// Unsupported path
    #[error("unsupported path: {0}")]
    UnsupportedPath(Text),

    /// Type mismatch
    #[error("type mismatch: {0}")]
    TypeMismatch(Text),

    /// Unbound variable
    #[error("unbound variable: {0}")]
    UnboundVariable(Text),

    /// Tensor shape error
    #[error("tensor shape error: {0}")]
    TensorShapeError(Text),

    /// Quantifier error
    #[error("quantifier error: {0}")]
    QuantifierError(Text),

    /// Pattern generation error
    #[error("pattern generation error: {0}")]
    PatternError(Text),
}

// ============================================================================
// Quantifier Pattern Extraction and Generation
// ============================================================================
//
// This section implements pattern-based quantifier instantiation for Z3.
// Patterns guide Z3's MBQI (Model-Based Quantifier Instantiation) to find
// relevant ground instances of quantified formulas.
//
// Pattern Selection Strategy:
// 1. Function applications containing bound variables
// 2. Method calls on receivers that reference bound variables
// 3. Array/map index operations with bound variable indices
// 4. Field accesses on bound variables
//
// Multi-patterns: When multiple triggers share the same bound variable,
// they can be grouped into a multi-pattern requiring all to match.

/// Represents a pattern trigger extracted from a quantifier body.
///
/// Triggers are terms that guide Z3's quantifier instantiation.
/// When Z3 encounters a ground term matching a trigger, it instantiates
/// the quantifier with the corresponding substitution.
#[derive(Debug, Clone)]
pub enum PatternTrigger {
    /// Function application: f(args) where args reference bound vars
    FunctionApp {
        /// Function name (path to function)
        func_name: Text,
        /// Arguments that may include bound variables
        args: List<Expr>,
        /// Which bound variables are referenced
        bound_var_refs: List<Text>,
    },

    /// Method call: receiver.method(args)
    MethodCall {
        /// The receiver expression
        receiver: Box<Expr>,
        /// Method name
        method: Text,
        /// Method arguments
        args: List<Expr>,
        /// Which bound variables are referenced
        bound_var_refs: List<Text>,
    },

    /// Index access: expr[index] where index references a bound var
    IndexAccess {
        /// Base expression (array, list, map, etc.)
        base: Box<Expr>,
        /// Index expression containing bound variable
        index: Box<Expr>,
        /// Which bound variables are referenced
        bound_var_refs: List<Text>,
    },

    /// Field access: expr.field where expr is a bound variable
    FieldAccess {
        /// Base expression (should be or contain bound variable)
        base: Box<Expr>,
        /// Field name
        field: Text,
        /// Which bound variables are referenced
        bound_var_refs: List<Text>,
    },

    /// Binary operation involving bound variable (useful for arithmetic)
    BinaryOp {
        /// The operation
        op: BinOp,
        /// Left operand
        left: Box<Expr>,
        /// Right operand
        right: Box<Expr>,
        /// Which bound variables are referenced
        bound_var_refs: List<Text>,
    },
}

impl PatternTrigger {
    /// Get the bound variable references for this trigger
    pub fn bound_var_refs(&self) -> &List<Text> {
        match self {
            PatternTrigger::FunctionApp { bound_var_refs, .. } => bound_var_refs,
            PatternTrigger::MethodCall { bound_var_refs, .. } => bound_var_refs,
            PatternTrigger::IndexAccess { bound_var_refs, .. } => bound_var_refs,
            PatternTrigger::FieldAccess { bound_var_refs, .. } => bound_var_refs,
            PatternTrigger::BinaryOp { bound_var_refs, .. } => bound_var_refs,
        }
    }

    /// Check if this trigger references a specific bound variable
    pub fn references_var(&self, var_name: &str) -> bool {
        self.bound_var_refs()
            .iter()
            .any(|v| v.as_str() == var_name)
    }

    /// Get the priority score for this trigger (higher = better pattern)
    ///
    /// Function applications and method calls are preferred over arithmetic.
    pub fn priority(&self) -> u32 {
        match self {
            PatternTrigger::FunctionApp { .. } => 100,
            PatternTrigger::MethodCall { .. } => 90,
            PatternTrigger::IndexAccess { .. } => 80,
            PatternTrigger::FieldAccess { .. } => 70,
            PatternTrigger::BinaryOp { .. } => 50,
        }
    }
}

/// Pattern extraction context for traversing quantifier bodies
struct PatternExtractor {
    /// Bound variable names to look for
    bound_vars: List<Text>,
    /// Collected triggers
    triggers: List<PatternTrigger>,
    /// Seen expressions (to avoid duplicates)
    seen_exprs: std::collections::HashSet<String>,
}

impl PatternExtractor {
    fn new(bound_vars: &[Text]) -> Self {
        Self {
            bound_vars: bound_vars.iter().cloned().collect(),
            triggers: List::new(),
            seen_exprs: std::collections::HashSet::new(),
        }
    }

    /// Extract pattern triggers from a quantifier body expression
    fn extract(&mut self, body: &Expr) {
        self.visit_expr(body);
    }

    /// Check if an expression references any bound variable
    #[allow(dead_code)] // Part of pattern extraction API - used in quantifier trigger generation
    fn references_bound_vars(&self, expr: &Expr) -> bool {
        self.collect_bound_var_refs(expr)
            .iter()
            .next()
            .is_some()
    }

    /// Collect all bound variable references in an expression
    fn collect_bound_var_refs(&self, expr: &Expr) -> List<Text> {
        let mut refs = List::new();
        self.collect_refs_recursive(expr, &mut refs);
        refs
    }

    fn collect_refs_recursive(&self, expr: &Expr, refs: &mut List<Text>) {
        match &expr.kind {
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    let name = ident.as_str();
                    if self.bound_vars.iter().any(|v| v.as_str() == name) {
                        if !refs.iter().any(|r| r.as_str() == name) {
                            refs.push(name.to_text());
                        }
                    }
                }
            }
            ExprKind::Binary { left, right, .. } => {
                self.collect_refs_recursive(left, refs);
                self.collect_refs_recursive(right, refs);
            }
            ExprKind::Unary { expr, .. } => {
                self.collect_refs_recursive(expr, refs);
            }
            ExprKind::Call { func, args, .. } => {
                self.collect_refs_recursive(func, refs);
                for arg in args.iter() {
                    self.collect_refs_recursive(arg, refs);
                }
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                self.collect_refs_recursive(receiver, refs);
                for arg in args.iter() {
                    self.collect_refs_recursive(arg, refs);
                }
            }
            ExprKind::Index { expr, index } => {
                self.collect_refs_recursive(expr, refs);
                self.collect_refs_recursive(index, refs);
            }
            ExprKind::Field { expr, .. } => {
                self.collect_refs_recursive(expr, refs);
            }
            ExprKind::Paren(inner) => {
                self.collect_refs_recursive(inner, refs);
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                for cond in condition.conditions.iter() {
                    match cond {
                        verum_ast::expr::ConditionKind::Expr(e) => {
                            self.collect_refs_recursive(e, refs);
                        }
                        verum_ast::expr::ConditionKind::Let { value, .. } => {
                            self.collect_refs_recursive(value, refs);
                        }
                    }
                }
                if let Some(expr) = &then_branch.expr {
                    self.collect_refs_recursive(expr, refs);
                }
                if let Some(else_expr) = else_branch {
                    self.collect_refs_recursive(else_expr, refs);
                }
            }
            ExprKind::Tuple(exprs) => {
                for e in exprs.iter() {
                    self.collect_refs_recursive(e, refs);
                }
            }
            ExprKind::Cast { expr, .. } => {
                self.collect_refs_recursive(expr, refs);
            }
            _ => {}
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        // Create a key for deduplication
        let expr_key = format!("{:?}", expr.kind);

        match &expr.kind {
            ExprKind::Call { func, args, .. } => {
                // Check if any args reference bound variables
                let bound_var_refs = self.collect_bound_var_refs_from_slice(args);
                if !bound_var_refs.is_empty() && !self.seen_exprs.contains(&expr_key) {
                    self.seen_exprs.insert(expr_key);

                    // Extract function name
                    if let Some(func_name) = self.extract_func_name(func) {
                        self.triggers.push(PatternTrigger::FunctionApp {
                            func_name,
                            args: args.iter().cloned().collect(),
                            bound_var_refs,
                        });
                    }
                }
                // Recurse into args
                for arg in args.iter() {
                    self.visit_expr(arg);
                }
            }

            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                // Check if receiver or args reference bound variables
                let mut bound_var_refs = self.collect_bound_var_refs(receiver);
                for arg in args.iter() {
                    for r in self.collect_bound_var_refs(arg).iter() {
                        if !bound_var_refs.iter().any(|v| v.as_str() == r.as_str()) {
                            bound_var_refs.push(r.clone());
                        }
                    }
                }

                if !bound_var_refs.is_empty() && !self.seen_exprs.contains(&expr_key) {
                    self.seen_exprs.insert(expr_key);
                    self.triggers.push(PatternTrigger::MethodCall {
                        receiver: Box::new(receiver.as_ref().clone()),
                        method: method.as_str().to_text(),
                        args: args.iter().cloned().collect(),
                        bound_var_refs,
                    });
                }

                // Recurse
                self.visit_expr(receiver);
                for arg in args.iter() {
                    self.visit_expr(arg);
                }
            }

            ExprKind::Index { expr, index } => {
                // Check if index references bound variables
                let index_refs = self.collect_bound_var_refs(index);
                let base_refs = self.collect_bound_var_refs(expr);

                if (!index_refs.is_empty() || !base_refs.is_empty())
                    && !self.seen_exprs.contains(&expr_key)
                {
                    self.seen_exprs.insert(expr_key);

                    let mut bound_var_refs = index_refs;
                    for r in base_refs.iter() {
                        if !bound_var_refs.iter().any(|v| v.as_str() == r.as_str()) {
                            bound_var_refs.push(r.clone());
                        }
                    }

                    self.triggers.push(PatternTrigger::IndexAccess {
                        base: Box::new(expr.as_ref().clone()),
                        index: Box::new(index.as_ref().clone()),
                        bound_var_refs,
                    });
                }

                // Recurse
                self.visit_expr(expr);
                self.visit_expr(index);
            }

            ExprKind::Field { expr, field } => {
                // Check if expr references bound variables
                let bound_var_refs = self.collect_bound_var_refs(expr);
                if !bound_var_refs.is_empty() && !self.seen_exprs.contains(&expr_key) {
                    self.seen_exprs.insert(expr_key);
                    self.triggers.push(PatternTrigger::FieldAccess {
                        base: Box::new(expr.as_ref().clone()),
                        field: field.as_str().to_text(),
                        bound_var_refs,
                    });
                }

                self.visit_expr(expr);
            }

            ExprKind::Binary { op, left, right } => {
                // Binary ops can be useful patterns for arithmetic
                let left_refs = self.collect_bound_var_refs(left);
                let right_refs = self.collect_bound_var_refs(right);

                // Only create binary op patterns for certain operations
                let is_pattern_worthy = matches!(
                    op,
                    BinOp::Add
                        | BinOp::Sub
                        | BinOp::Mul
                        | BinOp::Div
                        | BinOp::Rem
                        | BinOp::Eq
                        | BinOp::Ne
                        | BinOp::Lt
                        | BinOp::Le
                        | BinOp::Gt
                        | BinOp::Ge
                );

                if is_pattern_worthy
                    && (!left_refs.is_empty() || !right_refs.is_empty())
                    && !self.seen_exprs.contains(&expr_key)
                {
                    self.seen_exprs.insert(expr_key);

                    let mut bound_var_refs = left_refs;
                    for r in right_refs.iter() {
                        if !bound_var_refs.iter().any(|v| v.as_str() == r.as_str()) {
                            bound_var_refs.push(r.clone());
                        }
                    }

                    self.triggers.push(PatternTrigger::BinaryOp {
                        op: *op,
                        left: Box::new(left.as_ref().clone()),
                        right: Box::new(right.as_ref().clone()),
                        bound_var_refs,
                    });
                }

                // Recurse
                self.visit_expr(left);
                self.visit_expr(right);
            }

            ExprKind::Unary { expr, .. } => {
                self.visit_expr(expr);
            }

            ExprKind::Paren(inner) => {
                self.visit_expr(inner);
            }

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                for cond in condition.conditions.iter() {
                    match cond {
                        verum_ast::expr::ConditionKind::Expr(e) => self.visit_expr(e),
                        verum_ast::expr::ConditionKind::Let { value, .. } => {
                            self.visit_expr(value)
                        }
                    }
                }
                if let Some(expr) = &then_branch.expr {
                    self.visit_expr(expr);
                }
                if let Some(else_expr) = else_branch {
                    self.visit_expr(else_expr);
                }
            }

            ExprKind::Block(block) => {
                if let Some(expr) = &block.expr {
                    self.visit_expr(expr);
                }
            }

            ExprKind::Tuple(exprs) => {
                for e in exprs.iter() {
                    self.visit_expr(e);
                }
            }

            ExprKind::Cast { expr, .. } => {
                self.visit_expr(expr);
            }

            // Leaf nodes - no recursion needed
            ExprKind::Literal(_) | ExprKind::Path(_) => {}

            // Other expression types - recurse as needed
            _ => {}
        }
    }

    fn collect_bound_var_refs_from_slice(&self, exprs: &[Expr]) -> List<Text> {
        let mut refs = List::new();
        for expr in exprs.iter() {
            for r in self.collect_bound_var_refs(expr).iter() {
                if !refs.iter().any(|v: &Text| v.as_str() == r.as_str()) {
                    refs.push(r.clone());
                }
            }
        }
        refs
    }

    fn extract_func_name(&self, func: &Expr) -> Option<Text> {
        match &func.kind {
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    Some(ident.as_str().to_text())
                } else if !path.segments.is_empty() {
                    // Multi-segment path - use full path as name
                    let name = path
                        .segments
                        .iter()
                        .filter_map(|seg| match seg {
                            verum_ast::PathSegment::Name(ident) => Some(ident.name.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("::");
                    Some(name.to_text())
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// Configuration for pattern generation
#[derive(Debug, Clone)]
pub struct PatternGenConfig {
    /// Maximum number of patterns to generate per quantifier
    pub max_patterns: usize,
    /// Minimum priority score for patterns to include
    pub min_priority: u32,
    /// Whether to generate multi-patterns (multiple terms per pattern)
    pub enable_multi_patterns: bool,
    /// Whether to include arithmetic patterns
    pub include_arithmetic: bool,
}

impl Default for PatternGenConfig {
    fn default() -> Self {
        Self {
            max_patterns: 5,
            min_priority: 50,
            enable_multi_patterns: true,
            include_arithmetic: false, // Arithmetic patterns can cause matching loops
        }
    }
}

impl<'ctx> Translator<'ctx> {
    /// Extract pattern triggers from a quantifier body.
    ///
    /// Analyzes the body expression to find function applications, method calls,
    /// and other operations that involve the bound variables. These are used
    /// to guide Z3's quantifier instantiation.
    ///
    /// # Arguments
    ///
    /// * `body` - The quantifier body expression
    /// * `bound_vars` - Names of the quantified variables
    ///
    /// # Returns
    ///
    /// List of pattern triggers ordered by priority
    pub fn extract_pattern_triggers(
        &self,
        body: &Expr,
        bound_vars: &[Text],
    ) -> List<PatternTrigger> {
        let mut extractor = PatternExtractor::new(bound_vars);
        extractor.extract(body);

        // Sort triggers by priority (highest first)
        let mut triggers = extractor.triggers;
        triggers.sort_by(|a, b| b.priority().cmp(&a.priority()));

        triggers
    }

    /// Convert pattern triggers to Z3 patterns.
    ///
    /// Takes extracted triggers and creates Z3 Pattern objects that can be
    /// passed to forall_const or exists_const.
    ///
    /// # Arguments
    ///
    /// * `triggers` - List of pattern triggers to convert
    /// * `z3_vars` - Mapping from variable names to Z3 AST nodes
    /// * `config` - Pattern generation configuration
    ///
    /// # Returns
    ///
    /// List of Z3 patterns ready for quantifier construction
    pub fn triggers_to_z3_patterns(
        &self,
        triggers: &[PatternTrigger],
        z3_vars: &Map<Text, Dynamic>,
        config: &PatternGenConfig,
    ) -> Result<List<Z3Pattern>, TranslationError> {
        let mut patterns = List::new();

        for trigger in triggers.iter() {
            // Skip low-priority triggers
            if trigger.priority() < config.min_priority {
                continue;
            }

            // Skip arithmetic if not enabled
            if !config.include_arithmetic && matches!(trigger, PatternTrigger::BinaryOp { .. }) {
                continue;
            }

            // Try to translate this trigger to a Z3 pattern
            if let Some(pattern) = self.translate_trigger_to_z3(trigger, z3_vars)? {
                patterns.push(pattern);

                // Stop if we've reached the maximum
                if patterns.len() >= config.max_patterns {
                    break;
                }
            }
        }

        Ok(patterns)
    }

    /// Translate a single trigger to a Z3 pattern.
    ///
    /// Creates a Z3 Pattern from a PatternTrigger by building the corresponding
    /// Z3 AST term and wrapping it in a Pattern.
    fn translate_trigger_to_z3(
        &self,
        trigger: &PatternTrigger,
        z3_vars: &Map<Text, Dynamic>,
    ) -> Result<Option<Z3Pattern>, TranslationError> {
        match trigger {
            PatternTrigger::FunctionApp {
                func_name, args, ..
            } => {
                // Create uninterpreted function with Int args and Int return
                // This is a simplification - ideally we'd track types
                let arg_sorts: List<Sort> = args.iter().map(|_| Sort::int()).collect();
                let arg_sort_refs: List<&Sort> = arg_sorts.iter().collect();
                let return_sort = Sort::int();

                let func_decl = FuncDecl::new(
                    Symbol::String(func_name.to_string()),
                    &arg_sort_refs,
                    &return_sort,
                );

                // Translate arguments, substituting bound variables
                let mut z3_args: List<Dynamic> = List::new();
                for arg in args.iter() {
                    let z3_arg = self.translate_pattern_arg(arg, z3_vars)?;
                    z3_args.push(z3_arg);
                }

                // Apply function
                let z3_arg_refs: List<&dyn Ast> =
                    z3_args.iter().map(|a| a as &dyn Ast).collect();
                let app = func_decl.apply(&z3_arg_refs);

                Ok(Some(Z3Pattern::new(&[&app])))
            }

            PatternTrigger::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                // Encode method call as function: method(receiver, args...)
                let total_args = 1 + args.len();
                let arg_sorts: List<Sort> = (0..total_args).map(|_| Sort::int()).collect();
                let arg_sort_refs: List<&Sort> = arg_sorts.iter().collect();
                let return_sort = Sort::int();

                let func_decl = FuncDecl::new(
                    Symbol::String(method.to_string()),
                    &arg_sort_refs,
                    &return_sort,
                );

                // Translate receiver and args
                let mut z3_args: List<Dynamic> = List::new();
                z3_args.push(self.translate_pattern_arg(receiver, z3_vars)?);
                for arg in args.iter() {
                    z3_args.push(self.translate_pattern_arg(arg, z3_vars)?);
                }

                let z3_arg_refs: List<&dyn Ast> =
                    z3_args.iter().map(|a| a as &dyn Ast).collect();
                let app = func_decl.apply(&z3_arg_refs);

                Ok(Some(Z3Pattern::new(&[&app])))
            }

            PatternTrigger::IndexAccess { base, index, .. } => {
                // Encode as array select: select(base, index)
                let base_z3 = self.translate_pattern_arg(base, z3_vars)?;
                let index_z3 = self.translate_pattern_arg(index, z3_vars)?;

                // Try to get the index as Int
                if let Some(index_int) = index_z3.as_int() {
                    // Create select function
                    let select_decl = FuncDecl::new(
                        Symbol::String("select".to_string()),
                        &[&Sort::array(&Sort::int(), &Sort::int()), &Sort::int()],
                        &Sort::int(),
                    );

                    let app = select_decl.apply(&[&base_z3 as &dyn Ast, &index_int as &dyn Ast]);
                    Ok(Some(Z3Pattern::new(&[&app])))
                } else {
                    // Fall back to uninterpreted function
                    let select_decl = FuncDecl::new(
                        Symbol::String("index".to_string()),
                        &[&Sort::int(), &Sort::int()],
                        &Sort::int(),
                    );

                    let app =
                        select_decl.apply(&[&base_z3 as &dyn Ast, &index_z3 as &dyn Ast]);
                    Ok(Some(Z3Pattern::new(&[&app])))
                }
            }

            PatternTrigger::FieldAccess { base, field, .. } => {
                // Encode field access as function: field(base)
                let func_decl = FuncDecl::new(
                    Symbol::String(format!("field_{}", field)),
                    &[&Sort::int()],
                    &Sort::int(),
                );

                let base_z3 = self.translate_pattern_arg(base, z3_vars)?;
                let app = func_decl.apply(&[&base_z3 as &dyn Ast]);

                Ok(Some(Z3Pattern::new(&[&app])))
            }

            PatternTrigger::BinaryOp {
                op, left, right, ..
            } => {
                // For binary ops, we need to be careful - only create patterns
                // for operations that won't cause matching loops
                let left_z3 = self.translate_pattern_arg(left, z3_vars)?;
                let right_z3 = self.translate_pattern_arg(right, z3_vars)?;

                // Only handle if we can get both as Int
                if let (Some(left_int), Some(right_int)) = (left_z3.as_int(), right_z3.as_int()) {
                    let result: Dynamic = match op {
                        BinOp::Add => Dynamic::from_ast(&(left_int.clone() + right_int.clone())),
                        BinOp::Sub => Dynamic::from_ast(&(left_int.clone() - right_int.clone())),
                        BinOp::Mul => Dynamic::from_ast(&(left_int.clone() * right_int.clone())),
                        _ => return Ok(None), // Skip other ops for patterns
                    };

                    // Note: Arithmetic patterns can be problematic for MBQI
                    // Only return if explicitly enabled in config
                    Ok(Some(Z3Pattern::new(&[&result])))
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// Translate a pattern argument to Z3, using bound variable mappings.
    fn translate_pattern_arg(
        &self,
        expr: &Expr,
        z3_vars: &Map<Text, Dynamic>,
    ) -> Result<Dynamic, TranslationError> {
        match &expr.kind {
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    let name = ident.as_str();

                    // Check if it's a bound variable
                    if let Some(z3_var) = z3_vars.get(&name.to_text()) {
                        return Ok(z3_var.clone());
                    }

                    // Check if it's already bound in the translator
                    if let Maybe::Some(var) = self.bindings.get(&name.to_text()) {
                        return Ok(var.clone());
                    }

                    // Create a fresh constant
                    let int_var = Int::new_const(name);
                    Ok(Dynamic::from_ast(&int_var))
                } else {
                    // Complex path - create symbolic constant
                    let path_str = format!("{:?}", path);
                    let int_var = Int::new_const(path_str.as_str());
                    Ok(Dynamic::from_ast(&int_var))
                }
            }

            ExprKind::Literal(lit) => self.translate_literal(lit),

            ExprKind::Binary { op, left, right } => {
                self.translate_binary_op(*op, left, right)
            }

            ExprKind::Unary { op, expr } => self.translate_unary_op(*op, expr),

            ExprKind::Paren(inner) => self.translate_pattern_arg(inner, z3_vars),

            _ => {
                // For complex expressions, create a symbolic constant
                let expr_str = format!("expr_{:?}", expr.kind);
                let int_var = Int::new_const(expr_str.as_str());
                Ok(Dynamic::from_ast(&int_var))
            }
        }
    }

    /// Group triggers into multi-patterns.
    ///
    /// Multi-patterns require multiple terms to match before instantiation.
    /// This can help avoid unnecessary instantiations.
    ///
    /// Triggers that share the same bound variables are grouped together.
    pub fn group_triggers(&self, triggers: &[PatternTrigger]) -> List<List<PatternTrigger>> {
        let mut groups: List<List<PatternTrigger>> = List::new();
        let mut used: std::collections::HashSet<usize> = std::collections::HashSet::new();

        for (i, trigger) in triggers.iter().enumerate() {
            if used.contains(&i) {
                continue;
            }

            let mut group = List::new();
            group.push(trigger.clone());
            used.insert(i);

            // Find other triggers that share bound variables
            let refs = trigger.bound_var_refs();
            for (j, other) in triggers.iter().enumerate().skip(i + 1) {
                if used.contains(&j) {
                    continue;
                }

                let other_refs = other.bound_var_refs();

                // Check for overlap
                let has_overlap = refs.iter().any(|r| {
                    other_refs.iter().any(|o| r.as_str() == o.as_str())
                });

                if has_overlap {
                    group.push(other.clone());
                    used.insert(j);
                }
            }

            groups.push(group);
        }

        groups
    }

    /// Create Z3 multi-patterns from trigger groups.
    ///
    /// Each group becomes a single Z3 Pattern containing multiple terms.
    pub fn groups_to_z3_multi_patterns(
        &self,
        groups: &[List<PatternTrigger>],
        z3_vars: &Map<Text, Dynamic>,
    ) -> Result<List<Z3Pattern>, TranslationError> {
        let mut patterns = List::new();

        for group in groups.iter() {
            let mut terms: List<Dynamic> = List::new();

            for trigger in group.iter() {
                if let Some(pattern) = self.translate_trigger_to_z3(trigger, z3_vars)? {
                    // Extract the terms from the pattern
                    // Note: Z3 Pattern API doesn't expose terms, so we translate directly
                    if let Some(term) = self.trigger_to_z3_term(trigger, z3_vars)? {
                        terms.push(term);
                    }
                }
            }

            if !terms.is_empty() {
                // Create multi-pattern with all terms
                let term_refs: List<&dyn Ast> = terms.iter().map(|t| t as &dyn Ast).collect();
                let pattern = Z3Pattern::new(&term_refs);
                patterns.push(pattern);
            }
        }

        Ok(patterns)
    }

    /// Convert a trigger to a Z3 term (without wrapping in Pattern).
    fn trigger_to_z3_term(
        &self,
        trigger: &PatternTrigger,
        z3_vars: &Map<Text, Dynamic>,
    ) -> Result<Option<Dynamic>, TranslationError> {
        match trigger {
            PatternTrigger::FunctionApp {
                func_name, args, ..
            } => {
                let arg_sorts: List<Sort> = args.iter().map(|_| Sort::int()).collect();
                let arg_sort_refs: List<&Sort> = arg_sorts.iter().collect();
                let return_sort = Sort::int();

                let func_decl = FuncDecl::new(
                    Symbol::String(func_name.to_string()),
                    &arg_sort_refs,
                    &return_sort,
                );

                let mut z3_args: List<Dynamic> = List::new();
                for arg in args.iter() {
                    z3_args.push(self.translate_pattern_arg(arg, z3_vars)?);
                }

                let z3_arg_refs: List<&dyn Ast> =
                    z3_args.iter().map(|a| a as &dyn Ast).collect();
                let app = func_decl.apply(&z3_arg_refs);

                Ok(Some(app))
            }

            PatternTrigger::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                let total_args = 1 + args.len();
                let arg_sorts: List<Sort> = (0..total_args).map(|_| Sort::int()).collect();
                let arg_sort_refs: List<&Sort> = arg_sorts.iter().collect();
                let return_sort = Sort::int();

                let func_decl = FuncDecl::new(
                    Symbol::String(method.to_string()),
                    &arg_sort_refs,
                    &return_sort,
                );

                let mut z3_args: List<Dynamic> = List::new();
                z3_args.push(self.translate_pattern_arg(receiver, z3_vars)?);
                for arg in args.iter() {
                    z3_args.push(self.translate_pattern_arg(arg, z3_vars)?);
                }

                let z3_arg_refs: List<&dyn Ast> =
                    z3_args.iter().map(|a| a as &dyn Ast).collect();
                let app = func_decl.apply(&z3_arg_refs);

                Ok(Some(app))
            }

            PatternTrigger::IndexAccess { base, index, .. } => {
                let base_z3 = self.translate_pattern_arg(base, z3_vars)?;
                let index_z3 = self.translate_pattern_arg(index, z3_vars)?;

                let select_decl = FuncDecl::new(
                    Symbol::String("index".to_string()),
                    &[&Sort::int(), &Sort::int()],
                    &Sort::int(),
                );

                let app = select_decl.apply(&[&base_z3 as &dyn Ast, &index_z3 as &dyn Ast]);
                Ok(Some(app))
            }

            PatternTrigger::FieldAccess { base, field, .. } => {
                let func_decl = FuncDecl::new(
                    Symbol::String(format!("field_{}", field)),
                    &[&Sort::int()],
                    &Sort::int(),
                );

                let base_z3 = self.translate_pattern_arg(base, z3_vars)?;
                let app = func_decl.apply(&[&base_z3 as &dyn Ast]);

                Ok(Some(app))
            }

            PatternTrigger::BinaryOp {
                op, left, right, ..
            } => {
                let left_z3 = self.translate_pattern_arg(left, z3_vars)?;
                let right_z3 = self.translate_pattern_arg(right, z3_vars)?;

                if let (Some(left_int), Some(right_int)) = (left_z3.as_int(), right_z3.as_int()) {
                    let result: Dynamic = match op {
                        BinOp::Add => Dynamic::from_ast(&(left_int.clone() + right_int.clone())),
                        BinOp::Sub => Dynamic::from_ast(&(left_int.clone() - right_int.clone())),
                        BinOp::Mul => Dynamic::from_ast(&(left_int.clone() * right_int.clone())),
                        _ => return Ok(None),
                    };
                    Ok(Some(result))
                } else {
                    Ok(None)
                }
            }
        }
    }
}
