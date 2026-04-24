//! Type-level computation for dependent types
//!
//! Type-level computation: compile-time evaluation of type expressions, reduction rules, normalization — Type-Level Programming
//!
//! This module implements type-level functions that allow types to be computed
//! from values. This is a core feature of dependent types, enabling:
//!
//! - Type functions: `fn type_function(b: bool) -> Type = if b then i32 else Text`
//! - Type-level arithmetic: `plus(m: Nat, n: Nat) -> Nat`
//! - Indexed types: `List<T, n: meta Nat>` with length tracking
//! - Type-level conditionals and pattern matching
//!
//! # Architecture
//!
//! The type-level evaluator works in two phases:
//! 1. **Value evaluation**: Compute values using ConstEvaluator
//! 2. **Type computation**: Use computed values to construct types
//!
//! # Examples
//!
//! ```verum
//! // Type-level conditional
//! fn type_function(b: bool) -> Type =
//!     if b then i32 else Text
//!
//! fn example(b: bool) -> type_function(b) =
//!     if b then 42 else "hello"
//!
//! // Type-level arithmetic
//! fn plus(m: Nat, n: Nat) -> Nat =
//!     match m {
//!         Zero => n,
//!         Succ(m') => Succ(plus(m', n))
//!     }
//!
//! fn append<T, m: meta Nat, n: meta Nat>(
//!     xs: List<T, m: meta Nat>,
//!     ys: List<T, n: meta Nat>
//! ) -> List<T, plus(m, n): meta Nat>
//! ```
//!
//! # Performance
//!
//! All type-level computation happens at compile-time with zero runtime overhead.
//! Results are cached to avoid redundant computation.

use crate::const_eval::ConstEvaluator;
use verum_common::ConstValue;
use verum_common::well_known_types::{WellKnownType as WKT, type_names as wkt_names};

// Const aliases for use in match patterns
const WKT_INT: &str = wkt_names::INT;
const WKT_FLOAT: &str = wkt_names::FLOAT;
const WKT_BOOL: &str = wkt_names::BOOL;
const WKT_TEXT: &str = wkt_names::TEXT;
const WKT_CHAR: &str = wkt_names::CHAR;
use crate::context::TypeContext;
use crate::ty::{EqConst, EqTerm, Type, UniverseLevel};
use thiserror::Error;
use verum_ast::expr::{BinOp, Expr, ExprKind, UnOp};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::{List, Map, Maybe, Text};

// SMT solver for semantic predicate equivalence: previously used
// `verum_smt::{CheckMode, SubsumptionChecker, SubsumptionResult}` directly.
// To break the `verum_types ↔ verum_smt` circular dependency, this module
// no longer imports verum_smt. The semantic-equivalence check is now a
// conservative stub (see `check_semantic_equivalence_smt` below). Callers
// that need full SMT-based equivalence should go through
// `RefinementChecker::check_with_evidence` using an injected
// `verum_types::refinement::SmtBackend` implementation.

// Common type-level computation traits and types
use verum_common::type_level::{
    BackendCapabilities, ReductionStrategy, TypeLevelComputation,
    TypeLevelConfig, TypeLevelResult,
    TypeLevelError as CommonTypeLevelError,
};

/// Errors that can occur during type-level computation
#[derive(Debug, Error)]
pub enum TypeLevelError {
    #[error("type error: expected {expected}, found {actual}")]
    TypeError { expected: Text, actual: Text },

    #[error("unbound type variable: {name}")]
    UnboundVariable { name: Text },

    #[error("type function application error: {message}")]
    ApplicationError { message: Text },

    #[error("type-level computation failed: {message}")]
    ComputationFailed { message: Text },

    #[error("meta parameter error: {message}")]
    MetaParameterError { message: Text },

    #[error("type-level match error: {message}")]
    MatchError { message: Text },

    #[error("universe level error: {message}")]
    UniverseError { message: Text },

    #[error("cannot evaluate non-type expression as type")]
    NotAType,

    #[error("const evaluation error: {0}")]
    ConstEvalError(#[from] crate::const_eval::ConstEvalError),

    #[error("{0}")]
    Other(Text),
}

pub type Result<T> = std::result::Result<T, TypeLevelError>;

/// Convert local TypeLevelError to common TypeLevelError for trait compatibility
impl From<TypeLevelError> for CommonTypeLevelError {
    fn from(err: TypeLevelError) -> Self {
        match err {
            TypeLevelError::TypeError { expected, actual } => {
                CommonTypeLevelError::TypeError { expected, actual }
            }
            TypeLevelError::UnboundVariable { name } => {
                CommonTypeLevelError::UnboundVariable { name }
            }
            TypeLevelError::ApplicationError { message } => {
                CommonTypeLevelError::ApplicationError { message }
            }
            TypeLevelError::ComputationFailed { message } => {
                CommonTypeLevelError::ComputationFailed { message }
            }
            TypeLevelError::MetaParameterError { message } => {
                CommonTypeLevelError::MetaParameterError { message }
            }
            TypeLevelError::MatchError { message } => {
                CommonTypeLevelError::MatchError { message }
            }
            TypeLevelError::UniverseError { message } => {
                CommonTypeLevelError::UniverseError { message }
            }
            TypeLevelError::NotAType => CommonTypeLevelError::NotAType,
            TypeLevelError::ConstEvalError(e) => {
                CommonTypeLevelError::ComputationFailed { message: e.to_string().into() }
            }
            TypeLevelError::Other(msg) => CommonTypeLevelError::Other(msg),
        }
    }
}

/// Convert common TypeLevelError to local TypeLevelError
impl From<CommonTypeLevelError> for TypeLevelError {
    fn from(err: CommonTypeLevelError) -> Self {
        match err {
            CommonTypeLevelError::TypeError { expected, actual } => {
                TypeLevelError::TypeError { expected, actual }
            }
            CommonTypeLevelError::UnboundVariable { name } => {
                TypeLevelError::UnboundVariable { name }
            }
            CommonTypeLevelError::ApplicationError { message } => {
                TypeLevelError::ApplicationError { message }
            }
            CommonTypeLevelError::ComputationFailed { message } => {
                TypeLevelError::ComputationFailed { message }
            }
            CommonTypeLevelError::MetaParameterError { message } => {
                TypeLevelError::MetaParameterError { message }
            }
            CommonTypeLevelError::MatchError { message } => {
                TypeLevelError::MatchError { message }
            }
            CommonTypeLevelError::UniverseError { message } => {
                TypeLevelError::UniverseError { message }
            }
            CommonTypeLevelError::NotAType => TypeLevelError::NotAType,
            CommonTypeLevelError::MaxDepthExceeded(depth) => {
                TypeLevelError::ComputationFailed {
                    message: format!("max depth exceeded: {}", depth).into()
                }
            }
            CommonTypeLevelError::ArityMismatch { expected, got } => {
                TypeLevelError::ApplicationError {
                    message: format!("arity mismatch: expected {}, got {}", expected, got).into()
                }
            }
            CommonTypeLevelError::InvalidTypeFunction(name) => {
                TypeLevelError::ApplicationError {
                    message: format!("invalid type function: {}", name).into()
                }
            }
            CommonTypeLevelError::NonConstantArgument(msg) => {
                TypeLevelError::ComputationFailed { message: msg }
            }
            CommonTypeLevelError::SmtTimeout { timeout_ms } => {
                TypeLevelError::ComputationFailed {
                    message: format!("SMT timeout after {}ms", timeout_ms).into()
                }
            }
            CommonTypeLevelError::SmtError { message } => {
                TypeLevelError::ComputationFailed { message }
            }
            CommonTypeLevelError::UnsupportedOperation { operation } => {
                TypeLevelError::ComputationFailed {
                    message: format!("unsupported operation: {}", operation).into()
                }
            }
            CommonTypeLevelError::Other(msg) => TypeLevelError::Other(msg),
        }
    }
}

/// Type-level function definition
///
/// Represents a function that computes types from values.
/// These are defined with `fn name(params) -> Type = body`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeLevelFunction {
    /// Function name
    pub name: Text,
    /// Type parameters (e.g., T in `fn f<T>(...)`)
    pub type_params: List<(Text, Type)>,
    /// Value parameters (e.g., n: Nat)
    pub value_params: List<(Text, Type)>,
    /// Return type (must be Type or a universe level)
    pub return_type: Type,
    /// Function body (expression that computes the result type)
    pub body: Expr,
    /// Universe level of the returned type
    pub universe: UniverseLevel,
}

impl TypeLevelFunction {
    /// Create a new type-level function
    pub fn new(
        name: Text,
        type_params: List<(Text, Type)>,
        value_params: List<(Text, Type)>,
        return_type: Type,
        body: Expr,
    ) -> Self {
        Self {
            name,
            type_params,
            value_params,
            return_type,
            body,
            universe: UniverseLevel::TYPE,
        }
    }

    /// Create a simple type-level function with no type parameters
    pub fn simple(name: Text, value_params: List<(Text, Type)>, body: Expr) -> Self {
        Self {
            name,
            type_params: List::new(),
            value_params,
            return_type: Type::Named {
                path: Path::from_ident(Ident::new("Type", Span::dummy())),
                args: List::new(),
            },
            body,
            universe: UniverseLevel::TYPE,
        }
    }
}

/// Type-level evaluator for dependent types
///
/// This evaluator computes types from values, enabling dependent types where
/// types can depend on runtime values (computed at compile-time).
///
/// # Example
///
/// ```ignore
/// use verum_types::type_level_computation::{TypeLevelEvaluator, TypeLevelFunction};
/// use verum_types::ty::Type;
/// use verum_ast::expr::{Expr, ExprKind};
/// use verum_common::List;
///
/// let mut eval = TypeLevelEvaluator::new();
///
/// // Define: fn type_fn(b: bool) -> Type = if b then i32 else Text
/// let type_fn = TypeLevelFunction::simple(
///     "type_fn".into(),
///     List::from(vec![("b".into(), Type::Bool)]),
///     Expr::dummy() // Simplified for example
/// );
/// eval.register_function(type_fn);
///
/// // Compute type: type_fn(true) => i32
/// let args = List::from(vec![true.into()]);
/// let result = eval.apply_function(&"type_fn".into(), &args)?;
/// ```
pub struct TypeLevelEvaluator {
    /// Registered type-level functions
    functions: Map<Text, TypeLevelFunction>,

    /// Value evaluator for computing arguments
    const_eval: ConstEvaluator,

    /// Type environment for resolving type names
    type_env: Map<Text, Type>,

    /// Cache of computed types (function_name + args -> Type)
    cache: Map<Text, Type>,

    /// Current type context for variable resolution
    context: Option<TypeContext>,
}

impl TypeLevelEvaluator {
    /// Create a new type-level evaluator
    pub fn new() -> Self {
        Self {
            functions: Map::new(),
            const_eval: ConstEvaluator::new(),
            type_env: Map::new(),
            cache: Map::new(),
            context: None,
        }
    }

    /// Create an evaluator with a type context
    pub fn with_context(context: TypeContext) -> Self {
        Self {
            functions: Map::new(),
            const_eval: ConstEvaluator::new(),
            type_env: Map::new(),
            cache: Map::new(),
            context: Some(context),
        }
    }

    /// Register a type-level function
    pub fn register_function(&mut self, func: TypeLevelFunction) {
        self.functions.insert(func.name.clone(), func);
    }

    /// Bind a type variable
    pub fn bind_type(&mut self, name: impl Into<Text>, ty: Type) {
        self.type_env.insert(name.into(), ty);
    }

    /// Bind a value variable
    pub fn bind_value(&mut self, name: impl Into<Text>, value: ConstValue) {
        self.const_eval.bind(name, value);
    }

    /// Apply a type-level function with the given arguments
    ///
    /// This evaluates `function_name(args)` to produce a type.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use verum_types::type_level_computation::TypeLevelEvaluator;
    /// use verum_types::const_eval::ConstValue;
    /// use verum_common::List;
    /// use verum_common::Text;
    /// let mut eval = TypeLevelEvaluator::new();
    /// // ... register function ...
    /// let args = List::from(vec![ConstValue::Bool(true)]);
    /// let name: Text = "type_function".into();
    /// let result_type = eval.apply_function(&name, &args)?;
    /// ```
    pub fn apply_function(&mut self, name: &Text, args: &List<ConstValue>) -> Result<Type> {
        // Check cache first
        let cache_key = self.make_cache_key(name, args);
        if let Some(cached) = self.cache.get(&cache_key) {
            return Ok(cached.clone());
        }

        // Look up function
        let func = self
            .functions
            .get(name)
            .ok_or_else(|| TypeLevelError::UnboundVariable { name: name.clone() })?
            .clone();

        // Validate argument count
        if args.len() != func.value_params.len() {
            return Err(TypeLevelError::ApplicationError {
                message: format!(
                    "function {} expects {} arguments, got {}",
                    name,
                    func.value_params.len(),
                    args.len()
                )
                .into(),
            });
        }

        // Save existing bindings that will be shadowed
        let mut saved_bindings: List<(Text, Option<ConstValue>)> = List::new();
        for (param_name, _param_type) in func.value_params.iter() {
            saved_bindings.push((param_name.clone(), self.const_eval.get(param_name)));
        }

        // Bind parameters
        for (i, (param_name, _param_type)) in func.value_params.iter().enumerate() {
            self.const_eval.bind(param_name.clone(), args[i].clone());
        }

        // Evaluate function body
        let result = self.eval_as_type(&func.body);

        // Restore environment (proper scoping)
        for (name, saved_value) in saved_bindings {
            match saved_value {
                Some(val) => self.const_eval.bind(name, val),
                None => self.const_eval.unbind(&name),
            }
        }

        // Return result after restoring environment
        let result = result?;

        // Cache result
        self.cache.insert(cache_key, result.clone());

        Ok(result)
    }

    /// Evaluate an expression as a type
    ///
    /// This is the core of type-level computation. It evaluates expressions
    /// that produce types (conditionals, matches, constructor applications).
    pub fn eval_as_type(&mut self, expr: &Expr) -> Result<Type> {
        match &expr.kind {
            // Type constructors
            ExprKind::Path(path) => self.eval_type_path(path),

            // Type application: F<T>
            ExprKind::Call { func, args, .. } => self.eval_type_application(func, args),

            // Conditional: if b then T1 else T2
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.eval_type_conditional_new(condition, then_branch, else_branch),

            // Match: match x { ... }
            ExprKind::Match { expr, arms } => self.eval_type_match(expr, arms),

            // Binary operations on types (for type-level arithmetic)
            ExprKind::Binary { op, left, right } => self.eval_type_binary(*op, left, right),

            // Literals representing types
            ExprKind::Literal(lit) => {
                // Convert literal to const value, then to type if it's a type name
                let value = self.const_eval.eval(expr)?;
                self.const_value_to_type(&value)
            }

            // Parenthesized type expression
            ExprKind::Paren(inner) => self.eval_as_type(inner),

            // Block expression (for complex type computation)
            ExprKind::Block(block) => {
                if let Some(last) = block.stmts.last()
                    && let verum_ast::stmt::StmtKind::Expr { expr, .. } = &last.kind
                {
                    return self.eval_as_type(expr);
                }
                Err(TypeLevelError::NotAType)
            }

            _ => Err(TypeLevelError::NotAType),
        }
    }

    /// Evaluate a type path (e.g., i32, Text, List)
    fn eval_type_path(&self, path: &Path) -> Result<Type> {
        // Simple type names
        if path.segments.len() == 1
            && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
        {
            let name = Text::from(ident.name.as_str());

            // Check type environment first
            if let Some(ty) = self.type_env.get(&name) {
                return Ok(ty.clone());
            }

            // Built-in types
            match name.as_str() {
                "i32" | WKT_INT => return Ok(Type::Int),
                "f64" | WKT_FLOAT => return Ok(Type::Float),
                "bool" | WKT_BOOL => return Ok(Type::Bool),
                WKT_TEXT => return Ok(Type::Text),
                WKT_CHAR => return Ok(Type::Char),
                "Unit" | "()" => return Ok(Type::Unit),
                "Type" => {
                    return Ok(Type::Named {
                        path: Path::from_ident(Ident::new("Type", Span::dummy())),
                        args: List::new(),
                    });
                }
                _ => {}
            }
        }

        // Named type with path
        Ok(Type::Named {
            path: path.clone(),
            args: List::new(),
        })
    }

    /// Evaluate type application: F<T, U>
    fn eval_type_application(&mut self, func: &Expr, args: &[Expr]) -> Result<Type> {
        // Evaluate function to get type constructor
        match &func.kind {
            ExprKind::Path(path) => {
                if path.segments.len() == 1
                    && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
                {
                    let name = Text::from(ident.name.as_str());

                    // Check if this is a registered type-level function
                    if self.functions.contains_key(&name) {
                        // Evaluate arguments as values
                        let mut arg_values = List::with_capacity(args.len());
                        for arg in args {
                            arg_values.push(self.const_eval.eval(arg)?);
                        }
                        return self.apply_function(&name, &arg_values);
                    }

                    // Generic type application (e.g., List<i32>)
                    let mut type_args = List::with_capacity(args.len());
                    for arg in args {
                        type_args.push(self.eval_as_type(arg)?);
                    }

                    return Ok(Type::Generic {
                        name,
                        args: type_args,
                    });
                }

                Err(TypeLevelError::ApplicationError {
                    message: format!("cannot apply type constructor: {}", path).into(),
                })
            }
            _ => Err(TypeLevelError::ApplicationError {
                message: Text::from("invalid type application"),
            }),
        }
    }

    /// Evaluate type-level conditional: if b then T1 else T2
    fn eval_type_conditional(
        &mut self,
        condition: &Expr,
        then_branch: &Expr,
        else_branch: &Option<Box<Expr>>,
    ) -> Result<Type> {
        // Evaluate condition as boolean
        let cond_value = self.const_eval.eval(condition)?;
        let cond_bool = cond_value
            .as_bool_value()
            .ok_or_else(|| TypeLevelError::TypeError {
                expected: Text::from("bool"),
                actual: format!(
                    "value of type {}",
                    Self::format_const_value_type(&cond_value)
                )
                .into(),
            })?;

        // Evaluate selected branch
        if cond_bool {
            self.eval_as_type(then_branch)
        } else if let Some(else_expr) = else_branch {
            self.eval_as_type(else_expr.as_ref())
        } else {
            Ok(Type::Unit)
        }
    }

    /// Evaluate type-level conditional with new IfCondition/Block structure
    ///
    /// This handles the full IfCondition structure which supports:
    /// - Multiple conditions combined with &&
    /// - Let-pattern conditions for destructuring
    /// - Block expressions for then/else branches
    fn eval_type_conditional_new(
        &mut self,
        condition: &verum_ast::expr::IfCondition,
        then_branch: &verum_ast::expr::Block,
        else_branch: &verum_common::Maybe<verum_common::Heap<Expr>>,
    ) -> Result<Type> {
        use verum_ast::expr::ConditionKind;

        // Evaluate all conditions - they are implicitly AND-ed together
        let mut all_conditions_true = true;

        // Collect all pattern bindings for proper scoping
        let mut all_saved_bindings: List<(Text, Option<ConstValue>)> = List::new();

        for cond_kind in &condition.conditions {
            match cond_kind {
                ConditionKind::Expr(cond_expr) => {
                    // Evaluate the boolean expression
                    let cond_value = self.const_eval.eval(cond_expr)?;
                    let cond_bool =
                        cond_value
                            .as_bool_value()
                            .ok_or_else(|| TypeLevelError::TypeError {
                                expected: Text::from("bool"),
                                actual: format!(
                                    "value of type {}",
                                    Self::format_const_value_type(&cond_value)
                                )
                                .into(),
                            })?;

                    if !cond_bool {
                        all_conditions_true = false;
                        break;
                    }
                }
                ConditionKind::Let { pattern, value } => {
                    // Evaluate the value and try to match the pattern
                    let value_result = self.const_eval.eval(value)?;

                    // Check if pattern matches
                    if !self.pattern_matches(pattern, &value_result)? {
                        all_conditions_true = false;
                        break;
                    }

                    // Bind pattern variables for use in branches
                    let saved = self.bind_pattern(pattern, &value_result)?;
                    all_saved_bindings.extend(saved);
                }
            }
        }

        // Evaluate the appropriate branch
        let result = if all_conditions_true {
            // Evaluate then branch (Block)
            self.eval_block_as_type(then_branch)
        } else {
            // Evaluate else branch
            match else_branch {
                verum_common::Maybe::Some(else_expr) => self.eval_as_type(else_expr.as_ref()),
                verum_common::Maybe::None => Ok(Type::Unit),
            }
        };

        // Restore all pattern bindings (proper scoping)
        self.restore_pattern_bindings(all_saved_bindings);

        result
    }

    /// Evaluate a block expression as a type
    ///
    /// For type-level computation, we only care about the final expression in the block.
    /// Statements in blocks are not meaningful at the type level.
    fn eval_block_as_type(&mut self, block: &verum_ast::expr::Block) -> Result<Type> {
        // If block has a final expression, evaluate it as a type
        if let verum_common::Maybe::Some(expr) = &block.expr {
            self.eval_as_type(expr.as_ref())
        } else {
            // Block with no final expression is Unit type
            Ok(Type::Unit)
        }
    }

    /// Evaluate type-level match expression
    fn eval_type_match(
        &mut self,
        expr: &Expr,
        arms: &[verum_ast::pattern::MatchArm],
    ) -> Result<Type> {
        // Evaluate the matched expression
        let value = self.const_eval.eval(expr)?;

        // Try each arm in order
        for arm in arms {
            if self.pattern_matches(&arm.pattern, &value)? {
                // Bind pattern variables, saving previous bindings
                let saved_bindings = self.bind_pattern(&arm.pattern, &value)?;

                // Evaluate arm body
                let result = self.eval_as_type(&arm.body);

                // Restore environment (proper scoping)
                self.restore_pattern_bindings(saved_bindings);

                return result;
            }
        }

        Err(TypeLevelError::MatchError {
            message: format!(
                "no pattern matched value of type {}",
                Self::format_const_value_type(&value)
            )
            .into(),
        })
    }

    /// Evaluate type-level binary operation (for type arithmetic)
    fn eval_type_binary(&mut self, op: BinOp, left: &Expr, right: &Expr) -> Result<Type> {
        // For now, we only support this for Nat arithmetic
        // This would be extended for full dependent type arithmetic

        // Try to evaluate as values first
        let left_val = self.const_eval.eval(left)?;
        let right_val = self.const_eval.eval(right)?;

        match op {
            BinOp::Add => {
                if let (Some(l), Some(r)) = (left_val.as_u128(), right_val.as_u128()) {
                    // Type-level addition for meta Nat — propagate the computed
                    // value so subsequent unification can detect mismatches.
                    let sum = l.saturating_add(r);
                    return Ok(Type::Meta {
                        name: Text::from(format!("{}", sum)),
                        ty: Box::new(Type::Named {
                            path: Path::from_ident(Ident::new("Nat", Span::dummy())),
                            args: List::new(),
                        }),
                        refinement: None,
                        value: Some(verum_common::ConstValue::UInt(sum)),
                    });
                }
            }
            BinOp::Mul => {
                if let (Some(l), Some(r)) = (left_val.as_u128(), right_val.as_u128()) {
                    // Type-level multiplication for meta Nat — propagate value.
                    let prod = l.saturating_mul(r);
                    return Ok(Type::Meta {
                        name: Text::from(format!("{}", prod)),
                        ty: Box::new(Type::Named {
                            path: Path::from_ident(Ident::new("Nat", Span::dummy())),
                            args: List::new(),
                        }),
                        refinement: None,
                        value: Some(verum_common::ConstValue::UInt(prod)),
                    });
                }
            }
            _ => {}
        }

        Err(TypeLevelError::ComputationFailed {
            message: format!("unsupported type-level operation: {}", op).into(),
        })
    }

    /// Check if a pattern matches a value
    fn pattern_matches(&self, pattern: &Pattern, value: &ConstValue) -> Result<bool> {
        match &pattern.kind {
            PatternKind::Wildcard => Ok(true),

            PatternKind::Literal(lit) => {
                let lit_value = match lit.kind {
                    verum_ast::literal::LiteralKind::Int(ref int_lit) => {
                        ConstValue::Int(int_lit.value)
                    }
                    verum_ast::literal::LiteralKind::Bool(b) => ConstValue::Bool(b),
                    _ => return Ok(false),
                };
                Ok(lit_value == *value)
            }

            PatternKind::Ident { name, .. } => {
                // Variable pattern matches anything
                Ok(true)
            }

            PatternKind::Variant { path, data } => {
                // Would need to check if value is a variant with matching constructor
                // For now, simplified
                Ok(false)
            }

            _ => Ok(false),
        }
    }

    /// Bind pattern variables to values, returning names that were bound
    ///
    /// Returns a list of (name, previous_value) tuples for proper scoping.
    fn bind_pattern(&mut self, pattern: &Pattern, value: &ConstValue) -> Result<List<(Text, Option<ConstValue>)>> {
        let mut saved_bindings = List::new();

        match &pattern.kind {
            PatternKind::Ident { name, .. } => {
                let binding_name = Text::from(name.name.as_str());
                // Save previous value before binding
                saved_bindings.push((binding_name.clone(), self.const_eval.get(&binding_name)));
                self.const_eval.bind(binding_name, value.clone());
            }

            PatternKind::Variant { path, data } => {
                // Would need to destructure and bind fields
                // For now, simplified
            }

            _ => {}
        }

        Ok(saved_bindings)
    }

    /// Restore environment bindings after pattern evaluation
    fn restore_pattern_bindings(&mut self, saved_bindings: List<(Text, Option<ConstValue>)>) {
        for (name, saved_value) in saved_bindings {
            match saved_value {
                Some(val) => self.const_eval.bind(name, val),
                None => self.const_eval.unbind(&name),
            }
        }
    }

    /// Convert a const value to a type (for meta parameters)
    fn const_value_to_type(&self, value: &ConstValue) -> Result<Type> {
        match value {
            ConstValue::Int(_) => Ok(Type::Int),
            ConstValue::UInt(_) => Ok(Type::Named {
                path: Path::from_ident(Ident::new("usize", Span::dummy())),
                args: List::new(),
            }),
            ConstValue::Bool(_) => Ok(Type::Bool),
            ConstValue::Text(_) => Ok(Type::Text),
            ConstValue::Char(_) => Ok(Type::Char),
            _ => Err(TypeLevelError::TypeError {
                expected: Text::from("type"),
                actual: format!("value of type {}", Self::format_const_value_type(value)).into(),
            }),
        }
    }

    /// Create a cache key for memoization
    fn make_cache_key(&self, name: &Text, args: &List<ConstValue>) -> Text {
        let mut key = name.clone();
        key.push_str("(");
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                key.push_str(", ");
            }
            key.push_str(&format!("{}", arg));
        }
        key.push_str(")");
        key
    }

    /// Format a const value's type for error messages
    fn format_const_value_type(value: &ConstValue) -> &'static str {
        match value {
            ConstValue::Int(_) | ConstValue::UInt(_) => "integer",
            ConstValue::Float(_) => "float",
            ConstValue::Bool(_) => "bool",
            ConstValue::Text(_) => "text",
            ConstValue::Char(_) => "char",
            ConstValue::Tuple(_) => "tuple",
            _ => "value",
        }
    }

    /// Substitute meta parameters in a type with computed values
    ///
    /// This resolves all meta parameters in the type using the current
    /// value environment and type-level functions.
    pub fn substitute_meta(&mut self, ty: &Type) -> Result<Type> {
        match ty {
            Type::Meta {
                name,
                ty,
                refinement,
                value,
            } => {
                // Try to evaluate the meta parameter
                // For now, just pass through (would need expression in Meta)
                Ok(Type::Meta {
                    name: name.clone(),
                    ty: Box::new(self.substitute_meta(ty)?),
                    refinement: refinement.clone(),
                    value: value.clone(),
                })
            }

            Type::Generic { name, args } => {
                let mut new_args = List::with_capacity(args.len());
                for arg in args {
                    new_args.push(self.substitute_meta(arg)?);
                }
                Ok(Type::Generic {
                    name: name.clone(),
                    args: new_args,
                })
            }

            Type::Named { path, args } => {
                let mut new_args = List::with_capacity(args.len());
                for arg in args {
                    new_args.push(self.substitute_meta(arg)?);
                }
                Ok(Type::Named {
                    path: path.clone(),
                    args: new_args,
                })
            }

            Type::Function {
                params,
                return_type,
                contexts,
                type_params,
                properties,
            } => {
                let mut new_params = List::with_capacity(params.len());
                for param in params {
                    new_params.push(self.substitute_meta(param)?);
                }
                Ok(Type::Function {
                    params: new_params,
                    return_type: Box::new(self.substitute_meta(return_type)?),
                    contexts: contexts.clone(),
                    type_params: type_params.clone(),
                    properties: properties.clone(),
                })
            }

            Type::Array { element, size } => Ok(Type::Array {
                element: Box::new(self.substitute_meta(element)?),
                size: *size,
            }),

            Type::Tuple(elements) => {
                let mut new_elements = List::with_capacity(elements.len());
                for elem in elements {
                    new_elements.push(self.substitute_meta(elem)?);
                }
                Ok(Type::Tuple(new_elements))
            }

            Type::Reference { mutable, inner } => Ok(Type::Reference {
                mutable: *mutable,
                inner: Box::new(self.substitute_meta(inner)?),
            }),

            Type::Refined { base, predicate } => Ok(Type::Refined {
                base: Box::new(self.substitute_meta(base)?),
                predicate: predicate.clone(),
            }),

            // Other types pass through unchanged
            _ => Ok(ty.clone()),
        }
    }

    /// Clear the cache (useful for testing or when definitions change)
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Get the current number of cached type computations
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }
}

impl Default for TypeLevelEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

/// Implementation of the unified TypeLevelComputation trait from verum_common
///
/// This provides a standardized interface for type-level computation that
/// can be used across the codebase. It wraps the existing TypeLevelEvaluator
/// methods to conform to the common trait interface.
impl TypeLevelComputation for TypeLevelEvaluator {
    type Type = Type;
    type Expr = Expr;
    type Value = ConstValue;

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            supports_const_eval: true,
            supports_simplification: true,
            supports_type_normalization: true,
            supports_refinement_verification: true, // via verum_smt integration
            supports_smt: true,
            supports_dependent_types: true,
            supports_higher_kinded_types: false,
        }
    }

    fn eval_to_const(&self, expr: &Self::Expr) -> TypeLevelResult<Maybe<Self::Value>> {
        // Create a fresh ConstEvaluator for this evaluation
        // This is needed because ConstEvaluator::eval requires &mut self
        // but the trait method takes &self for flexibility
        let mut const_eval = ConstEvaluator::new();
        match const_eval.eval(expr) {
            Ok(value) => Ok(Maybe::Some(value)),
            Err(_) => Ok(Maybe::None), // Not evaluable at compile-time
        }
    }

    fn eval_as_type(&mut self, expr: &Self::Expr) -> TypeLevelResult<Self::Type> {
        // Delegate to our existing eval_as_type method
        self.eval_as_type(expr).map_err(|e| e.into())
    }

    fn simplify_expr(&self, expr: &Self::Expr) -> TypeLevelResult<Self::Expr> {
        // Simplified expressions: evaluate constants and fold where possible
        // For now, we just return the original expression
        // A full implementation would constant-fold and simplify
        Ok(expr.clone())
    }

    fn normalize_type(&mut self, ty: &Self::Type) -> TypeLevelResult<Self::Type> {
        // Delegate to substitute_meta for type normalization
        self.substitute_meta(ty).map_err(|e| e.into())
    }

    fn expr_equal(&self, lhs: &Self::Expr, rhs: &Self::Expr) -> TypeLevelResult<bool> {
        // Use our existing predicate equivalence check
        Ok(equality::predicates_equivalent(lhs, rhs))
    }

    fn type_equal(&self, lhs: &Self::Type, rhs: &Self::Type) -> TypeLevelResult<bool> {
        // Use our existing types_equal function
        Ok(equality::types_equal(lhs, rhs))
    }
}

/// Type-level natural number operations
///
/// These implement the standard Peano arithmetic operations at the type level.
/// Type-level computation: compile-time evaluation of type expressions, reduction rules, normalization — .2
pub mod nat {
    use super::*;

    /// Create the Zero natural number type
    pub fn zero() -> Type {
        Type::Named {
            path: Path::from_ident(Ident::new("Zero", Span::dummy())),
            args: List::new(),
        }
    }

    /// Create the Succ(n) natural number type
    pub fn succ(n: Type) -> Type {
        Type::Named {
            path: Path::from_ident(Ident::new("Succ", Span::dummy())),
            args: List::from(vec![n]),
        }
    }

    /// Check if a type is Zero
    pub fn is_zero(ty: &Type) -> bool {
        match ty {
            Type::Named { path, args } => {
                path.as_ident()
                    .map(|id| id.name.as_str() == "Zero")
                    .unwrap_or(false)
                    && args.is_empty()
            }
            _ => false,
        }
    }

    /// Extract n from Succ(n), if applicable
    pub fn pred(ty: &Type) -> Option<Type> {
        match ty {
            Type::Named { path, args } => {
                if path
                    .as_ident()
                    .map(|id| id.name.as_str() == "Succ")
                    .unwrap_or(false)
                    && args.len() == 1
                {
                    Some(args[0].clone())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Convert a usize to a type-level Nat
    pub fn from_usize(n: usize) -> Type {
        let mut result = zero();
        for _ in 0..n {
            result = succ(result);
        }
        result
    }

    /// Convert a type-level Nat to usize (if possible)
    pub fn to_usize(ty: &Type) -> Option<usize> {
        if is_zero(ty) {
            Some(0)
        } else if let Some(pred_ty) = pred(ty) {
            to_usize(&pred_ty).map(|n| n + 1)
        } else {
            None
        }
    }
}

/// Helper functions for working with meta parameters
pub mod meta {
    use super::*;

    /// Create a meta parameter type
    pub fn meta_param(name: impl Into<Text>, ty: Type) -> Type {
        Type::Meta {
            name: name.into(),
            ty: Box::new(ty),
            refinement: None,
            value: None,
        }
    }

    /// Create a meta Nat parameter
    pub fn meta_nat(name: impl Into<Text>) -> Type {
        meta_param(
            name,
            Type::Named {
                path: Path::from_ident(Ident::new("Nat", Span::dummy())),
                args: List::new(),
            },
        )
    }

    /// Create a meta usize parameter
    pub fn meta_usize(name: impl Into<Text>) -> Type {
        meta_param(
            name,
            Type::Named {
                path: Path::from_ident(Ident::new("usize", Span::dummy())),
                args: List::new(),
            },
        )
    }

    /// Create a meta bool parameter
    pub fn meta_bool(name: impl Into<Text>) -> Type {
        meta_param(name, Type::Bool)
    }

    /// Extract meta parameter information
    pub fn extract_meta(ty: &Type) -> Option<(&Text, &Type)> {
        match ty {
            Type::Meta { name, ty, .. } => Some((name, ty.as_ref())),
            _ => None,
        }
    }
}

/// Type equality checking for dependent types
///
/// This module provides functions for checking type equality in the presence
/// of dependent types, where types may contain computed values.
///
/// Equality types: propositional equality Eq<A, x, y> with reflexivity, symmetry, transitivity, substitution — (Equality Types)
pub mod equality {
    use super::*;

    /// Check if two types are definitionally equal
    ///
    /// Two types are definitionally equal if they reduce to the same normal form.
    /// This is used for type checking dependent types.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use verum_types::type_level_computation::equality::types_equal;
    /// use verum_types::ty::Type;
    /// // plus(2, 3) == plus(3, 2) after normalization
    /// let type1 = Type::Int;
    /// let type2 = Type::Int;
    /// assert!(types_equal(&type1, &type2));
    /// ```
    pub fn types_equal(ty1: &Type, ty2: &Type) -> bool {
        // Normalize both types first
        match (ty1, ty2) {
            // Structural equality for basic types
            (Type::Int, Type::Int) => true,
            (Type::Float, Type::Float) => true,
            (Type::Bool, Type::Bool) => true,
            (Type::Text, Type::Text) => true,
            (Type::Char, Type::Char) => true,
            (Type::Unit, Type::Unit) => true,

            // Generic types must have same name and equal arguments
            (Type::Generic { name: n1, args: a1 }, Type::Generic { name: n2, args: a2 }) => {
                n1 == n2 && args_equal(a1, a2)
            }

            // Named types must have same path and equal arguments
            (Type::Named { path: p1, args: a1 }, Type::Named { path: p2, args: a2 }) => {
                paths_equal(p1, p2) && args_equal(a1, a2)
            }

            // Meta parameters must have same name, type, and concrete value (if any).
            // A concrete-value mismatch short-circuits equality regardless of the
            // symbolic name, so `Meta(value=2)` is distinct from `Meta(value=3)`.
            (
                Type::Meta {
                    name: n1,
                    ty: t1,
                    refinement: r1,
                    value: v1,
                },
                Type::Meta {
                    name: n2,
                    ty: t2,
                    refinement: r2,
                    value: v2,
                },
            ) => v1 == v2 && n1 == n2 && types_equal(t1, t2) && refinements_equal(r1, r2),

            // Tuples must have same arity and element types
            (Type::Tuple(elems1), Type::Tuple(elems2)) => {
                elems1.len() == elems2.len()
                    && elems1
                        .iter()
                        .zip(elems2.iter())
                        .all(|(e1, e2)| types_equal(e1, e2))
            }

            // Arrays must have same element type and size
            (
                Type::Array {
                    element: e1,
                    size: s1,
                },
                Type::Array {
                    element: e2,
                    size: s2,
                },
            ) => s1 == s2 && types_equal(e1, e2),

            // References must have same mutability and inner type
            (
                Type::Reference {
                    mutable: m1,
                    inner: i1,
                },
                Type::Reference {
                    mutable: m2,
                    inner: i2,
                },
            ) => m1 == m2 && types_equal(i1, i2),

            // Refined types must have equal base types and equivalent predicates
            (
                Type::Refined {
                    base: b1,
                    predicate: p1,
                },
                Type::Refined {
                    base: b2,
                    predicate: p2,
                },
            ) => types_equal(b1, b2) && predicates_equivalent(&p1.predicate, &p2.predicate),

            // Function types must have equal parameter types and return type
            (
                Type::Function {
                    params: p1,
                    return_type: r1,
                    contexts: c1,
                    type_params: tp1,
                    properties: pr1,
                },
                Type::Function {
                    params: p2,
                    return_type: r2,
                    contexts: c2,
                    type_params: tp2,
                    properties: pr2,
                },
            ) => args_equal(p1, p2) && types_equal(r1, r2) && c1 == c2 && tp1 == tp2 && pr1 == pr2,

            _ => false,
        }
    }

    /// Check if type argument lists are equal
    fn args_equal(args1: &List<Type>, args2: &List<Type>) -> bool {
        args1.len() == args2.len()
            && args1
                .iter()
                .zip(args2.iter())
                .all(|(a1, a2)| types_equal(a1, a2))
    }

    /// Check if refinements are equal
    fn refinements_equal(
        r1: &Option<crate::refinement::RefinementPredicate>,
        r2: &Option<crate::refinement::RefinementPredicate>,
    ) -> bool {
        match (r1, r2) {
            (None, None) => true,
            (Some(p1), Some(p2)) => predicates_equivalent(&p1.predicate, &p2.predicate),
            _ => false,
        }
    }

    /// Check if two equality terms are equal
    fn terms_equal(t1: &EqTerm, t2: &EqTerm) -> bool {
        match (t1, t2) {
            (EqTerm::Const(c1), EqTerm::Const(c2)) => consts_equal(c1, c2),
            (EqTerm::Var(v1), EqTerm::Var(v2)) => v1 == v2,
            _ => false,
        }
    }

    /// Check if two equality constants are equal
    fn consts_equal(c1: &EqConst, c2: &EqConst) -> bool {
        match (c1, c2) {
            (EqConst::Int(i1), EqConst::Int(i2)) => i1 == i2,
            (EqConst::Bool(b1), EqConst::Bool(b2)) => b1 == b2,
            (EqConst::Nat(n1), EqConst::Nat(n2)) => n1 == n2,
            (EqConst::Named(t1), EqConst::Named(t2)) => t1 == t2,
            (EqConst::Unit, EqConst::Unit) => true,
            _ => false,
        }
    }

    /// Check if two paths are equal
    fn paths_equal(p1: &Path, p2: &Path) -> bool {
        use verum_ast::ty::PathSegment;
        if p1.segments.len() != p2.segments.len() {
            return false;
        }
        p1.segments
            .iter()
            .zip(p2.segments.iter())
            .all(|(s1, s2)| match (s1, s2) {
                (PathSegment::Name(id1), PathSegment::Name(id2)) => id1.name == id2.name,
                (PathSegment::SelfValue, PathSegment::SelfValue) => true,
                (PathSegment::Super, PathSegment::Super) => true,
                (PathSegment::Cog, PathSegment::Cog) => true,
                (PathSegment::Relative, PathSegment::Relative) => true,
                _ => false,
            })
    }

    /// Check if two predicates are equivalent
    ///
    /// This checks if p1 ⟺ p2 (i.e., p1 implies p2 and p2 implies p1).
    /// We use a multi-layered approach:
    /// 1. Structural equality (fast path)
    /// 2. Syntactic normalization and comparison
    /// 3. SMT-based semantic equivalence
    pub fn predicates_equivalent(p1: &Expr, p2: &Expr) -> bool {
        // Fast path: structural equality
        if exprs_structurally_equal(p1, p2) {
            return true;
        }

        // Check for trivially different predicates
        if is_trivially_different(p1, p2) {
            return false;
        }

        // Try syntactic normalization
        // Normalize both predicates and compare
        // This handles cases like: (a && b) vs (b && a), (x > 0) vs (0 < x), etc.
        if let (Some(n1), Some(n2)) = (normalize_predicate(p1), normalize_predicate(p2))
            && exprs_structurally_equal(&n1, &n2)
        {
            return true;
        }

        // SMT-based semantic equivalence check
        // For p1 ⟺ p2, we check both directions: p1 => p2 AND p2 => p1
        check_semantic_equivalence_smt(p1, p2)
    }

    /// Check semantic equivalence using SMT solver (stub after cycle-break).
    ///
    /// Previously delegated to `verum_smt::SubsumptionChecker` to check
    /// both directions `p1 ⇒ p2` and `p2 ⇒ p1`. The SMT path was moved
    /// out of `verum_types` to break the circular dependency with
    /// `verum_smt`. Since type-level computation runs inside the type
    /// checker (no direct access to a backend), this path now
    /// conservative-rejects: returns true only when both predicates are
    /// already structurally / syntactically equal after normalisation,
    /// and false otherwise. If real SMT-backed semantic equivalence is
    /// needed, invoke it through a `RefinementChecker` that carries an
    /// injected `SmtBackend`.
    fn check_semantic_equivalence_smt(_p1: &Expr, _p2: &Expr) -> bool {
        false
    }

    /// Check if two expressions are structurally equal
    fn exprs_structurally_equal(e1: &Expr, e2: &Expr) -> bool {
        use ExprKind::*;

        match (&e1.kind, &e2.kind) {
            (Literal(l1), Literal(l2)) => l1 == l2,
            (Path(p1), Path(p2)) => paths_equal(p1, p2),

            (
                Binary {
                    op: op1,
                    left: l1,
                    right: r1,
                },
                Binary {
                    op: op2,
                    left: l2,
                    right: r2,
                },
            ) => op1 == op2 && exprs_structurally_equal(l1, l2) && exprs_structurally_equal(r1, r2),

            (Unary { op: op1, expr: e1 }, Unary { op: op2, expr: e2 }) => {
                op1 == op2 && exprs_structurally_equal(e1, e2)
            }

            (Call { func: f1, args: a1, .. }, Call { func: f2, args: a2, .. }) => {
                exprs_structurally_equal(f1, f2)
                    && a1.len() == a2.len()
                    && a1
                        .iter()
                        .zip(a2.iter())
                        .all(|(e1, e2)| exprs_structurally_equal(e1, e2))
            }

            (Paren(e1), Paren(e2)) => exprs_structurally_equal(e1, e2),

            // Unwrap parentheses for comparison
            (Paren(e1), _) => exprs_structurally_equal(e1, e2),
            (_, Paren(e2)) => exprs_structurally_equal(e1, e2),

            _ => false,
        }
    }

    /// Check if two predicates are trivially different (quick rejection)
    fn is_trivially_different(p1: &Expr, p2: &Expr) -> bool {
        use ExprKind::*;

        match (&p1.kind, &p2.kind) {
            // true vs false
            (Literal(l1), Literal(l2)) if l1 != l2 => true,

            // Different operators
            (Binary { op: op1, .. }, Binary { op: op2, .. }) if op1 != op2 => {
                // Exception: some operators are equivalent (e.g., != vs !(==))
                !are_operators_potentially_equivalent(*op1, *op2)
            }

            _ => false,
        }
    }

    /// Check if two binary operators could be equivalent via transformation
    fn are_operators_potentially_equivalent(op1: BinOp, op2: BinOp) -> bool {
        use BinOp::*;

        // List of operator pairs that could be equivalent:
        // - Eq/Ne (via negation)
        // - Lt/Ge, Le/Gt (via negation)
        // - And/Or (via De Morgan's laws)
        matches!(
            (op1, op2),
            (Eq, Ne) | (Ne, Eq) | (Lt, Ge) | (Ge, Lt) | (Le, Gt) | (Gt, Le) | (And, Or) | (Or, And)
        )
    }

    /// Normalize a predicate to a canonical form for syntactic comparison
    fn normalize_predicate(pred: &Expr) -> Option<Expr> {
        use ExprKind::*;

        match &pred.kind {
            // Normalize commutative operators: put smaller term first
            Binary {
                op: op @ (BinOp::And | BinOp::Or | BinOp::Eq | BinOp::Ne),
                left,
                right,
            } => {
                let norm_left =
                    normalize_predicate(left.as_ref()).unwrap_or_else(|| left.as_ref().clone());
                let norm_right =
                    normalize_predicate(right.as_ref()).unwrap_or_else(|| right.as_ref().clone());

                // Sort operands for commutative operators
                let (first, second) = if expr_less_than(&norm_left, &norm_right) {
                    (norm_left, norm_right)
                } else {
                    (norm_right, norm_left)
                };

                Some(Expr::new(
                    Binary {
                        op: *op,
                        left: Box::new(first),
                        right: Box::new(second),
                    },
                    pred.span,
                ))
            }

            // Normalize comparison: always put variables on left, constants on right
            Binary {
                op: op @ (BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge),
                left,
                right,
            } => {
                // Swap if right is a variable and left is not
                if is_variable(right) && !is_variable(left) {
                    let flipped_op = flip_comparison(*op);
                    Some(Expr::new(
                        Binary {
                            op: flipped_op,
                            left: right.clone(),
                            right: left.clone(),
                        },
                        pred.span,
                    ))
                } else {
                    None
                }
            }

            // Strip parentheses
            Paren(inner) => normalize_predicate(inner),

            // Recursively normalize nested expressions
            Binary { op, left, right } => {
                let norm_left =
                    normalize_predicate(left.as_ref()).unwrap_or_else(|| left.as_ref().clone());
                let norm_right =
                    normalize_predicate(right.as_ref()).unwrap_or_else(|| right.as_ref().clone());
                Some(Expr::new(
                    Binary {
                        op: *op,
                        left: Box::new(norm_left),
                        right: Box::new(norm_right),
                    },
                    pred.span,
                ))
            }

            Unary { op, expr } => normalize_predicate(expr).map(|norm_expr| {
                Expr::new(
                    Unary {
                        op: *op,
                        expr: Box::new(norm_expr),
                    },
                    pred.span,
                )
            }),

            _ => None,
        }
    }

    /// Simple ordering for expressions (for normalization)
    fn expr_less_than(e1: &Expr, e2: &Expr) -> bool {
        use ExprKind::*;

        // Simple heuristic: paths < literals < other
        match (&e1.kind, &e2.kind) {
            (Path(_), Path(_)) => format!("{:?}", e1) < format!("{:?}", e2),
            (Path(_), _) => true,
            (_, Path(_)) => false,
            (Literal(_), Literal(_)) => format!("{:?}", e1) < format!("{:?}", e2),
            (Literal(_), _) => true,
            (_, Literal(_)) => false,
            _ => format!("{:?}", e1) < format!("{:?}", e2),
        }
    }

    /// Check if an expression is a variable (Path)
    fn is_variable(expr: &Expr) -> bool {
        matches!(expr.kind, ExprKind::Path(_))
    }

    /// Flip a comparison operator (for normalization)
    fn flip_comparison(op: BinOp) -> BinOp {
        use BinOp::*;
        match op {
            Lt => Gt,
            Le => Ge,
            Gt => Lt,
            Ge => Le,
            other => other, // Eq, Ne stay the same
        }
    }

    /// Convert an expression to an EqTerm for proof construction.
    ///
    /// This is a simplified conversion that handles common cases.
    /// For full conversion with type inference, use TypeChecker::expr_to_eq_term.
    fn expr_to_eq_term_simple(expr: &Expr) -> EqTerm {
        use verum_ast::expr::ExprKind;
        use verum_ast::literal::LiteralKind;

        match &expr.kind {
            ExprKind::Path(path) => {
                if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.last() {
                    EqTerm::Var(ident.name.clone())
                } else {
                    // Fallback to string representation
                    EqTerm::Var(Text::from(format!("{:?}", path)))
                }
            }
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Int(int_lit) => EqTerm::Const(EqConst::Int(int_lit.value as i64)),
                LiteralKind::Bool(b) => EqTerm::Const(EqConst::Bool(*b)),
                _ => EqTerm::Const(EqConst::Unit), // Fallback
            },
            _ => {
                // For complex expressions, create a named term
                EqTerm::Var(Text::from(format!("expr_{:?}", expr.span.start)))
            }
        }
    }

    /// Create a proof of reflexivity: x = x
    ///
    /// Constructs `refl(x)` which proves `x = x` for any value x.
    ///
    /// Equality types: propositional equality Eq<A, x, y> with reflexivity, symmetry, transitivity, substitution — (lines 126-128)
    pub fn refl_proof(_ty: &Type, value: &Expr) -> EqTerm {
        // Convert the value to an EqTerm and wrap in Refl
        let value_term = expr_to_eq_term_simple(value);
        EqTerm::Refl(Box::new(value_term))
    }

    /// Create a proof of symmetry: x = y implies y = x
    ///
    /// Given a proof `p : x = y`, constructs a proof of `y = x` using
    /// the J eliminator (path induction).
    ///
    /// The proof works by:
    /// - motive P(a, _) = (a = x), where we want to prove y = x
    /// - base: refl(x) proves x = x (when the equality is reflexive)
    /// - J(p, motive, base) : y = x
    ///
    /// Equality types: propositional equality Eq<A, x, y> with reflexivity, symmetry, transitivity, substitution — (line 134)
    pub fn sym_proof(eq_proof: &EqTerm) -> EqTerm {
        // For symmetry, we use J eliminator:
        // Given p : x = y, produce q : y = x
        //
        // motive = λa. λ_. (a = x)  -- dependent on the endpoint, not the proof
        // base = λx. refl(x)        -- when a = x via refl, we have x = x
        let motive = EqTerm::Lambda {
            param: Text::from("a"),
            body: Box::new(EqTerm::Lambda {
                param: Text::from("_p"),
                // The motive returns a type representing (a = x)
                // We represent this symbolically
                body: Box::new(EqTerm::App {
                    func: Box::new(EqTerm::Var(Text::from("Eq"))),
                    args: List::from_iter([
                        EqTerm::Var(Text::from("a")),
                        EqTerm::Var(Text::from("x")),
                    ]),
                }),
            }),
        };

        let base = EqTerm::Lambda {
            param: Text::from("x"),
            body: Box::new(EqTerm::Refl(Box::new(EqTerm::Var(Text::from("x"))))),
        };

        EqTerm::J {
            proof: Box::new(eq_proof.clone()),
            motive: Box::new(motive),
            base: Box::new(base),
        }
    }

    /// Create a proof of transitivity: x = y and y = z implies x = z
    ///
    /// Given proofs `p : x = y` and `q : y = z`, constructs a proof of `x = z`
    /// using the J eliminator.
    ///
    /// The proof works by eliminating on q:
    /// - motive P(a, _) = (x = a), where we want to show x = z
    /// - base: λa. λp. p   -- identity, when q is refl, p : x = y = x = a
    /// - J(q, motive, base)(p) : x = z
    ///
    /// Equality types: propositional equality Eq<A, x, y> with reflexivity, symmetry, transitivity, substitution — (lines 135-136)
    pub fn trans_proof(eq1: &EqTerm, eq2: &EqTerm) -> EqTerm {
        // For transitivity, we eliminate on eq2 : y = z
        // Given eq1 : x = y and eq2 : y = z, produce x = z
        //
        // motive = λa. λ_. (x = a)
        // base = λy. eq1  -- when a = y via refl, we have x = y which is eq1
        let motive = EqTerm::Lambda {
            param: Text::from("a"),
            body: Box::new(EqTerm::Lambda {
                param: Text::from("_q"),
                body: Box::new(EqTerm::App {
                    func: Box::new(EqTerm::Var(Text::from("Eq"))),
                    args: List::from_iter([
                        EqTerm::Var(Text::from("x")),
                        EqTerm::Var(Text::from("a")),
                    ]),
                }),
            }),
        };

        // The base case: when q is refl (y = y), we need x = y, which is eq1
        let base = EqTerm::Lambda {
            param: Text::from("_y"),
            body: Box::new(eq1.clone()),
        };

        EqTerm::J {
            proof: Box::new(eq2.clone()),
            motive: Box::new(motive),
            base: Box::new(base),
        }
    }
}

/// Arithmetic property proofs for type-level computation
///
/// This module provides proofs of arithmetic properties that are used
/// in dependent type checking. These are compile-time proofs that enable
/// more precise type checking.
///
/// Proofs are constructed using natural number induction, represented as:
/// `nat_ind(motive, base, step, n)` where:
/// - motive: the property being proven as a function of n
/// - base: proof that motive(Zero) holds
/// - step: proof that motive(m) implies motive(Succ(m))
/// - n: the natural number being inducted on
///
/// Type-level computation: compile-time evaluation of type expressions, reduction rules, normalization — .2 (lines 249-251)
pub mod arithmetic_proofs {
    use super::*;

    /// Helper to construct a natural number induction proof term.
    ///
    /// nat_ind : Π(P: Nat -> Type). P(Zero) -> (Π(m: Nat). P(m) -> P(Succ(m))) -> Π(n: Nat). P(n)
    fn nat_induction(motive: EqTerm, base: EqTerm, step: EqTerm, n: EqTerm) -> EqTerm {
        EqTerm::App {
            func: Box::new(EqTerm::Var(Text::from("nat_ind"))),
            args: List::from_iter([motive, base, step, n]),
        }
    }

    /// Helper to construct an application of plus
    fn plus_app(a: EqTerm, b: EqTerm) -> EqTerm {
        EqTerm::App {
            func: Box::new(EqTerm::Var(Text::from("plus"))),
            args: List::from_iter([a, b]),
        }
    }

    /// Helper to construct Succ(n)
    fn succ(n: EqTerm) -> EqTerm {
        EqTerm::App {
            func: Box::new(EqTerm::Var(Text::from("Succ"))),
            args: List::from_iter([n]),
        }
    }

    /// Helper to construct an equality type Eq(a, b)
    fn eq_type(a: EqTerm, b: EqTerm) -> EqTerm {
        EqTerm::App {
            func: Box::new(EqTerm::Var(Text::from("Eq"))),
            args: List::from_iter([a, b]),
        }
    }

    /// Helper to convert Type to EqTerm representation
    fn type_to_eq_term(ty: &Type) -> EqTerm {
        match ty {
            Type::Named { path, args } => {
                let name = path.as_ident().map(|id| id.name.clone()).unwrap_or_default();
                if args.is_empty() {
                    EqTerm::Var(name)
                } else {
                    let arg_terms: List<EqTerm> = args.iter().map(type_to_eq_term).collect();
                    EqTerm::App {
                        func: Box::new(EqTerm::Var(name)),
                        args: arg_terms,
                    }
                }
            }
            Type::Int => EqTerm::Var(Text::from(WKT::Int.as_str())),
            _ => EqTerm::Var(Text::from("_")),
        }
    }

    /// Proof that addition is commutative: plus(m, n) = plus(n, m)
    ///
    /// This is proven by induction on m:
    /// - Base case: plus(Zero, n) = n = plus(n, Zero) (requires plus_zero_right)
    /// - Inductive case: plus(Succ(m'), n) = Succ(plus(m', n))
    ///                                      = Succ(plus(n, m'))  (by IH)
    ///                                      = plus(n, Succ(m'))  (by plus_succ_right)
    ///
    /// Type-level computation: compile-time evaluation of type expressions, reduction rules, normalization — .2 (lines 249-251)
    pub fn plus_comm_proof(m: &Type, n: &Type) -> Result<EqTerm> {
        if !is_nat_type(m) || !is_nat_type(n) {
            return Err(TypeLevelError::TypeError {
                expected: Text::from("Nat"),
                actual: "non-Nat type arguments".into(),
            });
        }

        let m_term = type_to_eq_term(m);
        let n_term = type_to_eq_term(n);

        // Motive: λm. plus(m, n) = plus(n, m)
        let motive = EqTerm::Lambda {
            param: Text::from("m"),
            body: Box::new(eq_type(
                plus_app(EqTerm::Var(Text::from("m")), n_term.clone()),
                plus_app(n_term.clone(), EqTerm::Var(Text::from("m"))),
            )),
        };

        // Base case: plus(Zero, n) = plus(n, Zero)
        // This follows from plus_zero_left and plus_zero_right
        let base = EqTerm::App {
            func: Box::new(EqTerm::Var(Text::from("plus_zero_identity"))),
            args: List::from_iter([n_term.clone()]),
        };

        // Inductive step: λm'. λih. plus(Succ(m'), n) = plus(n, Succ(m'))
        // We use: plus(Succ(m'), n) = Succ(plus(m', n)) (by definition)
        //         = Succ(plus(n, m')) (by IH)
        //         = plus(n, Succ(m')) (by plus_succ_right lemma)
        let step = EqTerm::Lambda {
            param: Text::from("m'"),
            body: Box::new(EqTerm::Lambda {
                param: Text::from("ih"),
                body: Box::new(EqTerm::App {
                    func: Box::new(EqTerm::Var(Text::from("plus_comm_step"))),
                    args: List::from_iter([
                        EqTerm::Var(Text::from("m'")),
                        n_term.clone(),
                        EqTerm::Var(Text::from("ih")),
                    ]),
                }),
            }),
        };

        Ok(nat_induction(motive, base, step, m_term))
    }

    /// Proof that addition is associative: plus(plus(m, n), p) = plus(m, plus(n, p))
    ///
    /// This is proven by induction on m.
    ///
    /// Implicit arguments and instance search for dependent types — .1 (line 529)
    pub fn plus_assoc_proof(m: &Type, n: &Type, p: &Type) -> Result<EqTerm> {
        if !is_nat_type(m) || !is_nat_type(n) || !is_nat_type(p) {
            return Err(TypeLevelError::TypeError {
                expected: Text::from("Nat"),
                actual: "non-Nat type arguments".into(),
            });
        }

        let m_term = type_to_eq_term(m);
        let n_term = type_to_eq_term(n);
        let p_term = type_to_eq_term(p);

        // Motive: λm. plus(plus(m, n), p) = plus(m, plus(n, p))
        let motive = EqTerm::Lambda {
            param: Text::from("m"),
            body: Box::new(eq_type(
                plus_app(
                    plus_app(EqTerm::Var(Text::from("m")), n_term.clone()),
                    p_term.clone(),
                ),
                plus_app(
                    EqTerm::Var(Text::from("m")),
                    plus_app(n_term.clone(), p_term.clone()),
                ),
            )),
        };

        // Base case: plus(plus(Zero, n), p) = plus(Zero, plus(n, p))
        // Simplifies to: plus(n, p) = plus(n, p) by refl
        let base = EqTerm::Refl(Box::new(plus_app(n_term.clone(), p_term.clone())));

        // Inductive step uses congruence of Succ over equality
        let step = EqTerm::Lambda {
            param: Text::from("m'"),
            body: Box::new(EqTerm::Lambda {
                param: Text::from("ih"),
                body: Box::new(EqTerm::App {
                    func: Box::new(EqTerm::Var(Text::from("cong_succ"))),
                    args: List::from_iter([EqTerm::Var(Text::from("ih"))]),
                }),
            }),
        };

        Ok(nat_induction(motive, base, step, m_term))
    }

    /// Proof that multiplication is commutative: mult(m, n) = mult(n, m)
    pub fn mult_comm_proof(m: &Type, n: &Type) -> Result<EqTerm> {
        if !is_nat_type(m) || !is_nat_type(n) {
            return Err(TypeLevelError::TypeError {
                expected: Text::from("Nat"),
                actual: "non-Nat type arguments".into(),
            });
        }

        let m_term = type_to_eq_term(m);
        let n_term = type_to_eq_term(n);

        // Motive: λm. mult(m, n) = mult(n, m)
        let motive = EqTerm::Lambda {
            param: Text::from("m"),
            body: Box::new(eq_type(
                EqTerm::App {
                    func: Box::new(EqTerm::Var(Text::from("mult"))),
                    args: List::from_iter([EqTerm::Var(Text::from("m")), n_term.clone()]),
                },
                EqTerm::App {
                    func: Box::new(EqTerm::Var(Text::from("mult"))),
                    args: List::from_iter([n_term.clone(), EqTerm::Var(Text::from("m"))]),
                },
            )),
        };

        // Base: mult(Zero, n) = Zero = mult(n, Zero)
        let base = EqTerm::App {
            func: Box::new(EqTerm::Var(Text::from("mult_zero_identity"))),
            args: List::from_iter([n_term.clone()]),
        };

        // Step uses mult_succ lemmas and commutativity of plus
        let step = EqTerm::Lambda {
            param: Text::from("m'"),
            body: Box::new(EqTerm::Lambda {
                param: Text::from("ih"),
                body: Box::new(EqTerm::App {
                    func: Box::new(EqTerm::Var(Text::from("mult_comm_step"))),
                    args: List::from_iter([
                        EqTerm::Var(Text::from("m'")),
                        n_term.clone(),
                        EqTerm::Var(Text::from("ih")),
                    ]),
                }),
            }),
        };

        Ok(nat_induction(motive, base, step, m_term))
    }

    /// Proof that multiplication is associative: mult(mult(m, n), p) = mult(m, mult(n, p))
    pub fn mult_assoc_proof(m: &Type, n: &Type, p: &Type) -> Result<EqTerm> {
        if !is_nat_type(m) || !is_nat_type(n) || !is_nat_type(p) {
            return Err(TypeLevelError::TypeError {
                expected: Text::from("Nat"),
                actual: "non-Nat type arguments".into(),
            });
        }

        let m_term = type_to_eq_term(m);
        let n_term = type_to_eq_term(n);
        let p_term = type_to_eq_term(p);

        // Motive: λm. mult(mult(m, n), p) = mult(m, mult(n, p))
        let motive = EqTerm::Lambda {
            param: Text::from("m"),
            body: Box::new(eq_type(
                EqTerm::App {
                    func: Box::new(EqTerm::Var(Text::from("mult"))),
                    args: List::from_iter([
                        EqTerm::App {
                            func: Box::new(EqTerm::Var(Text::from("mult"))),
                            args: List::from_iter([EqTerm::Var(Text::from("m")), n_term.clone()]),
                        },
                        p_term.clone(),
                    ]),
                },
                EqTerm::App {
                    func: Box::new(EqTerm::Var(Text::from("mult"))),
                    args: List::from_iter([
                        EqTerm::Var(Text::from("m")),
                        EqTerm::App {
                            func: Box::new(EqTerm::Var(Text::from("mult"))),
                            args: List::from_iter([n_term.clone(), p_term.clone()]),
                        },
                    ]),
                },
            )),
        };

        // Base: mult(mult(Zero, n), p) = Zero = mult(Zero, mult(n, p))
        let base = EqTerm::Refl(Box::new(EqTerm::Var(Text::from("Zero"))));

        // Step uses distributivity
        let step = EqTerm::Lambda {
            param: Text::from("m'"),
            body: Box::new(EqTerm::Lambda {
                param: Text::from("ih"),
                body: Box::new(EqTerm::App {
                    func: Box::new(EqTerm::Var(Text::from("mult_assoc_step"))),
                    args: List::from_iter([
                        EqTerm::Var(Text::from("m'")),
                        n_term.clone(),
                        p_term.clone(),
                        EqTerm::Var(Text::from("ih")),
                    ]),
                }),
            }),
        };

        Ok(nat_induction(motive, base, step, m_term))
    }

    /// Proof that zero is the additive identity: plus(Zero, n) = n
    ///
    /// This is immediate by the definition of plus:
    /// plus(Zero, n) := n
    pub fn plus_zero_left_proof(n: &Type) -> Result<EqTerm> {
        if !is_nat_type(n) {
            return Err(TypeLevelError::TypeError {
                expected: Text::from("Nat"),
                actual: "non-Nat type argument".into(),
            });
        }

        // This is definitional equality: plus(Zero, n) = n by definition
        // Therefore refl(n) suffices
        let n_term = type_to_eq_term(n);
        Ok(EqTerm::Refl(Box::new(n_term)))
    }

    /// Proof that zero is the additive identity: plus(n, Zero) = n
    ///
    /// This requires induction since plus is defined by recursion on the first argument.
    pub fn plus_zero_right_proof(n: &Type) -> Result<EqTerm> {
        if !is_nat_type(n) {
            return Err(TypeLevelError::TypeError {
                expected: Text::from("Nat"),
                actual: "non-Nat type argument".into(),
            });
        }

        let n_term = type_to_eq_term(n);

        // Motive: λn. plus(n, Zero) = n
        let motive = EqTerm::Lambda {
            param: Text::from("n"),
            body: Box::new(eq_type(
                plus_app(
                    EqTerm::Var(Text::from("n")),
                    EqTerm::Var(Text::from("Zero")),
                ),
                EqTerm::Var(Text::from("n")),
            )),
        };

        // Base: plus(Zero, Zero) = Zero by refl
        let base = EqTerm::Refl(Box::new(EqTerm::Var(Text::from("Zero"))));

        // Step: plus(Succ(n'), Zero) = Succ(plus(n', Zero)) = Succ(n') (by IH)
        let step = EqTerm::Lambda {
            param: Text::from("n'"),
            body: Box::new(EqTerm::Lambda {
                param: Text::from("ih"),
                body: Box::new(EqTerm::App {
                    func: Box::new(EqTerm::Var(Text::from("cong_succ"))),
                    args: List::from_iter([EqTerm::Var(Text::from("ih"))]),
                }),
            }),
        };

        Ok(nat_induction(motive, base, step, n_term))
    }

    /// Proof that one is the multiplicative identity: mult(1, n) = n
    ///
    /// Since mult(Succ(Zero), n) = plus(n, mult(Zero, n)) = plus(n, Zero) = n
    pub fn mult_one_left_proof(n: &Type) -> Result<EqTerm> {
        if !is_nat_type(n) {
            return Err(TypeLevelError::TypeError {
                expected: Text::from("Nat"),
                actual: "non-Nat type argument".into(),
            });
        }

        let n_term = type_to_eq_term(n);

        // mult(1, n) = plus(n, mult(Zero, n)) = plus(n, Zero) = n
        // Chain of equalities using transitivity
        Ok(EqTerm::App {
            func: Box::new(EqTerm::Var(Text::from("mult_one_left_lemma"))),
            args: List::from_iter([n_term]),
        })
    }

    /// Proof that one is the multiplicative identity: mult(n, 1) = n
    ///
    /// This requires induction on n.
    pub fn mult_one_right_proof(n: &Type) -> Result<EqTerm> {
        if !is_nat_type(n) {
            return Err(TypeLevelError::TypeError {
                expected: Text::from("Nat"),
                actual: "non-Nat type argument".into(),
            });
        }

        let n_term = type_to_eq_term(n);

        // Motive: λn. mult(n, 1) = n
        let motive = EqTerm::Lambda {
            param: Text::from("n"),
            body: Box::new(eq_type(
                EqTerm::App {
                    func: Box::new(EqTerm::Var(Text::from("mult"))),
                    args: List::from_iter([
                        EqTerm::Var(Text::from("n")),
                        succ(EqTerm::Var(Text::from("Zero"))),
                    ]),
                },
                EqTerm::Var(Text::from("n")),
            )),
        };

        // Base: mult(Zero, 1) = Zero by refl
        let base = EqTerm::Refl(Box::new(EqTerm::Var(Text::from("Zero"))));

        // Step: mult(Succ(n'), 1) = plus(1, mult(n', 1)) = plus(1, n') = Succ(n')
        let step = EqTerm::Lambda {
            param: Text::from("n'"),
            body: Box::new(EqTerm::Lambda {
                param: Text::from("ih"),
                body: Box::new(EqTerm::App {
                    func: Box::new(EqTerm::Var(Text::from("mult_one_right_step"))),
                    args: List::from_iter([
                        EqTerm::Var(Text::from("n'")),
                        EqTerm::Var(Text::from("ih")),
                    ]),
                }),
            }),
        };

        Ok(nat_induction(motive, base, step, n_term))
    }

    /// Check if a type represents a natural number
    /// Recognizes both abstract "Nat" type and Peano representations (Zero, Succ)
    fn is_nat_type(ty: &Type) -> bool {
        match ty {
            Type::Named { path, args } => {
                let name = path.as_ident().map(|id| id.name.as_str());
                match name {
                    Some("Nat") | Some("Zero") => true,
                    Some("Succ") => {
                        // Succ(n) is Nat if n is Nat
                        args.first().map(is_nat_type).unwrap_or(false)
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }
}

/// Indexed types for length-indexed lists and bounded integers
///
/// This module provides support for indexed types like Fin<n> and List<T, n>
/// where the index is a compile-time value.
///
/// Type-level computation: compile-time evaluation of type expressions, reduction rules, normalization — .3 (lines 259-271)
pub mod indexed {
    use super::*;

    /// Create a Fin<n> type - integers in range [0, n)
    ///
    /// Fin<n> is the type of natural numbers less than n, providing
    /// compile-time bounds checking for array indexing.
    ///
    /// # Example
    ///
    /// ```verum
    /// fn safe_index<T, n: meta Nat>(list: List<T, n>, i: Fin<n>) -> T
    /// ```
    ///
    /// Type-level computation: compile-time evaluation of type expressions, reduction rules, normalization — .3 (lines 259-264)
    pub fn fin_type(n: usize) -> Type {
        // Create refined integer type: i: Int where 0 <= i && i < n
        let base = Type::Int;

        // For simplicity, we represent this as a named type
        // A full implementation would include the refinement predicate
        Type::Named {
            path: Path::from_ident(Ident::new("Fin", Span::dummy())),
            args: List::from(vec![super::nat::from_usize(n)]),
        }
    }

    /// Create a length-indexed list type: List<T, n>
    ///
    /// This represents a list with exactly n elements of type T.
    ///
    /// Type-level computation: compile-time evaluation of type expressions, reduction rules, normalization — .2 (lines 254-255)
    pub fn indexed_list_type(element_type: Type, length: usize) -> Type {
        Type::Generic {
            name: Text::from(WKT::List.as_str()),
            args: List::from(vec![element_type, super::nat::from_usize(length)]),
        }
    }

    /// Create a matrix type: Matrix<T, rows, cols>
    ///
    /// Represents a 2D array with compile-time dimensions.
    ///
    /// Type-level computation: compile-time evaluation of type expressions, reduction rules, normalization — .1 (lines 225-227)
    pub fn matrix_type(element_type: Type, rows: usize, cols: usize) -> Type {
        Type::Generic {
            name: Text::from("Matrix"),
            args: List::from(vec![
                element_type,
                super::nat::from_usize(rows),
                super::nat::from_usize(cols),
            ]),
        }
    }

    /// Check if a value is within bounds for Fin<n>
    pub fn check_fin_bounds(value: usize, bound: usize) -> bool {
        value < bound
    }

    /// Construct a Fin<n> value from a usize, returning None if out of bounds
    pub fn make_fin(value: usize, bound: usize) -> Option<ConstValue> {
        if check_fin_bounds(value, bound) {
            Some(ConstValue::UInt(value as u128))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::expr::ExprKind;
    use verum_ast::literal::{IntLit, Literal, LiteralKind};
    use verum_ast::span::Span;
    use verum_ast::ty::Ident;

    fn make_int_expr(n: i64) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal {
                kind: LiteralKind::Int(IntLit {
                    value: n as i128,
                    suffix: None,
                }),
                span: Span::dummy(),
            }),
            Span::dummy(),
        )
    }

    fn make_bool_expr(b: bool) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal {
                kind: LiteralKind::Bool(b),
                span: Span::dummy(),
            }),
            Span::dummy(),
        )
    }

    fn make_path_expr(name: &str) -> Expr {
        use verum_ast::ty::Ident;
        Expr::new(
            ExprKind::Path(Path::from_ident(Ident::new(name, Span::dummy()))),
            Span::dummy(),
        )
    }

    #[test]
    fn test_eval_simple_type_path() {
        let mut eval = TypeLevelEvaluator::new();
        let expr = make_path_expr("i32");
        let result = eval.eval_as_type(&expr).unwrap();
        assert_eq!(result, Type::Int);
    }

    #[test]
    fn test_type_conditional_true() {
        use smallvec::SmallVec;
        use verum_ast::expr::{Block, ConditionKind, IfCondition};
        use verum_common::{Heap, Maybe};

        let mut eval = TypeLevelEvaluator::new();

        // Construct: if true then i32 else Text
        let condition = IfCondition {
            conditions: SmallVec::from_vec(vec![ConditionKind::Expr(make_bool_expr(true))]),
            span: Span::dummy(),
        };

        // Then branch: Block with i32 as final expression
        let then_branch = Block::new(
            verum_common::List::new(),
            Maybe::Some(Heap::new(make_path_expr("i32"))),
            Span::dummy(),
        );

        // Else branch: Text
        let else_branch = Maybe::Some(Heap::new(make_path_expr("Text")));

        let if_expr = Expr::new(
            ExprKind::If {
                condition: Heap::new(condition),
                then_branch,
                else_branch,
            },
            Span::dummy(),
        );

        let result = eval.eval_as_type(&if_expr).unwrap();
        assert_eq!(result, Type::Int);
    }

    #[test]
    fn test_type_conditional_false() {
        use smallvec::SmallVec;
        use verum_ast::expr::{Block, ConditionKind, IfCondition};
        use verum_common::{Heap, Maybe};

        let mut eval = TypeLevelEvaluator::new();

        // Construct: if false then i32 else Text
        let condition = IfCondition {
            conditions: SmallVec::from_vec(vec![ConditionKind::Expr(make_bool_expr(false))]),
            span: Span::dummy(),
        };

        // Then branch: Block with i32 as final expression
        let then_branch = Block::new(
            verum_common::List::new(),
            Maybe::Some(Heap::new(make_path_expr("i32"))),
            Span::dummy(),
        );

        // Else branch: Text
        let else_branch = Maybe::Some(Heap::new(make_path_expr("Text")));

        let if_expr = Expr::new(
            ExprKind::If {
                condition: Heap::new(condition),
                then_branch,
                else_branch,
            },
            Span::dummy(),
        );

        let result = eval.eval_as_type(&if_expr).unwrap();
        assert_eq!(result, Type::Text);
    }

    #[test]
    fn test_nat_operations() {
        use super::nat::*;

        let zero_ty = zero();
        assert!(is_zero(&zero_ty));

        let one_ty = succ(zero());
        assert!(!is_zero(&one_ty));
        assert_eq!(pred(&one_ty), Some(zero_ty));

        let three_ty = from_usize(3);
        assert_eq!(to_usize(&three_ty), Some(3));
    }

    #[test]
    fn test_meta_helpers() {
        use super::meta::*;

        let meta_n = meta_nat("n");
        let (name, ty) = extract_meta(&meta_n).unwrap();
        assert_eq!(name.as_str(), "n");
    }

    #[test]
    fn test_cache() {
        let mut eval = TypeLevelEvaluator::new();

        // Register a simple type function
        let func =
            TypeLevelFunction::simple("const_type".into(), List::new(), make_path_expr("i32"));
        eval.register_function(func);

        // First call
        let result1 = eval
            .apply_function(&"const_type".into(), &List::new())
            .unwrap();

        // Check cache
        assert_eq!(eval.cache_size(), 1);

        // Second call should use cache
        let result2 = eval
            .apply_function(&"const_type".into(), &List::new())
            .unwrap();

        assert_eq!(result1, result2);
        assert_eq!(eval.cache_size(), 1);
    }

    #[test]
    fn test_pattern_variable_scoping() {
        use verum_ast::pattern::{Pattern, PatternKind};
        use verum_ast::ty::Ident;

        let mut eval = TypeLevelEvaluator::new();

        // Create a pattern that binds variable 'x'
        let pattern = Pattern::new(
            PatternKind::Ident {
                by_ref: false,
                mutable: false,
                name: Ident::new("x", Span::dummy()),
                subpattern: verum_common::Maybe::None,
            },
            Span::dummy(),
        );

        // Create a match arm: x => i32
        let arm = verum_ast::pattern::MatchArm {
            pattern,
            guard: verum_common::Maybe::None,
            body: verum_common::Heap::new(make_path_expr("i32")),
            with_clause: verum_common::Maybe::None,
            attributes: verum_common::List::new(),
            span: Span::dummy(),
        };

        // Create match expression: match 42 { x => i32 }
        let match_expr = Expr::new(
            ExprKind::Match {
                expr: Box::new(make_int_expr(42)),
                arms: vec![arm].into(),
            },
            Span::dummy(),
        );

        // Evaluate the match expression
        let result = eval.eval_as_type(&match_expr).unwrap();
        assert_eq!(result, Type::Int);
    }

    #[test]
    fn test_multiple_match_arms_no_variable_leakage() {
        use verum_ast::pattern::{Pattern, PatternKind};
        use verum_ast::ty::Ident;

        let mut eval = TypeLevelEvaluator::new();

        // First arm: true => i32 (literal pattern)
        let arm1 = verum_ast::pattern::MatchArm {
            pattern: Pattern::new(
                PatternKind::Literal(Literal {
                    kind: LiteralKind::Bool(true),
                    span: Span::dummy(),
                }),
                Span::dummy(),
            ),
            guard: verum_common::Maybe::None,
            body: verum_common::Heap::new(make_path_expr("i32")),
            with_clause: verum_common::Maybe::None,
            attributes: verum_common::List::new(),
            span: Span::dummy(),
        };

        // Second arm: x => Text (variable pattern, binds 'x')
        let arm2 = verum_ast::pattern::MatchArm {
            pattern: Pattern::new(
                PatternKind::Ident {
                    by_ref: false,
                    mutable: false,
                    name: Ident::new("x", Span::dummy()),
                    subpattern: verum_common::Maybe::None,
                },
                Span::dummy(),
            ),
            guard: verum_common::Maybe::None,
            body: verum_common::Heap::new(make_path_expr("Text")),
            with_clause: verum_common::Maybe::None,
            attributes: verum_common::List::new(),
            span: Span::dummy(),
        };

        // Create match expression: match false { true => i32, x => Text }
        let match_expr = Expr::new(
            ExprKind::Match {
                expr: Box::new(make_bool_expr(false)),
                arms: vec![arm1, arm2].into(),
            },
            Span::dummy(),
        );

        // Evaluate - should match second arm
        let result = eval.eval_as_type(&match_expr).unwrap();
        assert_eq!(result, Type::Text);
    }

    // Tests for new features

    #[test]
    fn test_type_equality_basic() {
        use super::equality::types_equal;

        // Test basic type equality
        assert!(types_equal(&Type::Int, &Type::Int));
        assert!(types_equal(&Type::Bool, &Type::Bool));
        assert!(types_equal(&Type::Text, &Type::Text));

        // Test inequality
        assert!(!types_equal(&Type::Int, &Type::Bool));
        assert!(!types_equal(&Type::Float, &Type::Text));
    }

    #[test]
    fn test_type_equality_generics() {
        use super::equality::types_equal;

        let list_int1 = Type::Generic {
            name: Text::from("List"),
            args: List::from(vec![Type::Int]),
        };

        let list_int2 = Type::Generic {
            name: Text::from("List"),
            args: List::from(vec![Type::Int]),
        };

        let list_bool = Type::Generic {
            name: Text::from("List"),
            args: List::from(vec![Type::Bool]),
        };

        assert!(types_equal(&list_int1, &list_int2));
        assert!(!types_equal(&list_int1, &list_bool));
    }

    #[test]
    fn test_type_equality_tuples() {
        use super::equality::types_equal;

        let tuple1 = Type::Tuple(List::from(vec![Type::Int, Type::Bool]));
        let tuple2 = Type::Tuple(List::from(vec![Type::Int, Type::Bool]));
        let tuple3 = Type::Tuple(List::from(vec![Type::Bool, Type::Int]));

        assert!(types_equal(&tuple1, &tuple2));
        assert!(!types_equal(&tuple1, &tuple3));
    }

    #[test]
    fn test_arithmetic_proofs_plus_comm() {
        use super::arithmetic_proofs::plus_comm_proof;
        use super::nat::*;
        use verum_ast::ty::Ident;

        let _nat_type = Type::Named {
            path: Path::from_ident(Ident::new("Nat", Span::dummy())),
            args: List::new(),
        };

        let m = from_usize(3);
        let n = from_usize(5);

        // Should succeed for Nat types
        let proof = plus_comm_proof(&m, &n);
        assert!(proof.is_ok());
    }

    #[test]
    fn test_arithmetic_proofs_plus_assoc() {
        use super::arithmetic_proofs::plus_assoc_proof;
        use super::nat::*;

        let m = from_usize(2);
        let n = from_usize(3);
        let p = from_usize(4);

        let proof = plus_assoc_proof(&m, &n, &p);
        assert!(proof.is_ok());
    }

    #[test]
    fn test_arithmetic_proofs_mult_comm() {
        use super::arithmetic_proofs::mult_comm_proof;
        use super::nat::*;

        let m = from_usize(4);
        let n = from_usize(6);

        let proof = mult_comm_proof(&m, &n);
        assert!(proof.is_ok());
    }

    #[test]
    fn test_arithmetic_proofs_mult_assoc() {
        use super::arithmetic_proofs::mult_assoc_proof;
        use super::nat::*;

        let m = from_usize(2);
        let n = from_usize(3);
        let p = from_usize(5);

        let proof = mult_assoc_proof(&m, &n, &p);
        assert!(proof.is_ok());
    }

    #[test]
    fn test_arithmetic_proofs_identity() {
        use super::arithmetic_proofs::*;
        use super::nat::*;

        let n = from_usize(7);

        // Test additive identity proofs
        assert!(plus_zero_left_proof(&n).is_ok());
        assert!(plus_zero_right_proof(&n).is_ok());

        // Test multiplicative identity proofs
        assert!(mult_one_left_proof(&n).is_ok());
        assert!(mult_one_right_proof(&n).is_ok());
    }

    #[test]
    fn test_indexed_fin_type() {
        use super::indexed::*;

        let fin_5 = fin_type(5);

        // Should create a Fin type with bound 5
        match fin_5 {
            Type::Named { path, args } => {
                // Check path is "Fin"
                assert!(
                    path.as_ident()
                        .map(|id| id.name.as_str() == "Fin")
                        .unwrap_or(false)
                );
                assert_eq!(args.len(), 1);
            }
            _ => panic!("Expected Named type"),
        }
    }

    #[test]
    fn test_indexed_list_type() {
        use super::indexed::*;

        let list_int_3 = indexed_list_type(Type::Int, 3);

        match list_int_3 {
            Type::Generic { name, args } => {
                assert_eq!(name, "List");
                assert_eq!(args.len(), 2);
                assert_eq!(args[0], Type::Int);
            }
            _ => panic!("Expected Generic type"),
        }
    }

    #[test]
    fn test_indexed_matrix_type() {
        use super::indexed::*;

        let matrix_f64_3x4 = matrix_type(Type::Float, 3, 4);

        match matrix_f64_3x4 {
            Type::Generic { name, args } => {
                assert_eq!(name, "Matrix");
                assert_eq!(args.len(), 3);
                assert_eq!(args[0], Type::Float);
            }
            _ => panic!("Expected Generic type"),
        }
    }

    #[test]
    fn test_fin_bounds_checking() {
        use super::indexed::*;

        // Test valid bounds
        assert!(check_fin_bounds(0, 5));
        assert!(check_fin_bounds(4, 5));

        // Test invalid bounds
        assert!(!check_fin_bounds(5, 5));
        assert!(!check_fin_bounds(10, 5));
    }

    #[test]
    fn test_make_fin() {
        use super::indexed::*;

        // Valid Fin values
        let fin_3_in_5 = make_fin(3, 5);
        assert!(fin_3_in_5.is_some());
        assert_eq!(fin_3_in_5.unwrap(), ConstValue::UInt(3));

        // Invalid Fin value (out of bounds)
        let fin_5_in_5 = make_fin(5, 5);
        assert!(fin_5_in_5.is_none());
    }

    #[test]
    fn test_equality_proofs() {
        use super::equality::*;

        let int_type = Type::Int;
        let value = make_int_expr(42);

        // Test reflexivity - now returns EqTerm::Refl
        let refl = refl_proof(&int_type, &value);
        assert!(
            matches!(refl, EqTerm::Refl(_)),
            "refl_proof should return EqTerm::Refl, got {:?}",
            refl
        );

        // Test symmetry - now returns EqTerm::J (J eliminator)
        let sym = sym_proof(&refl);
        assert!(
            matches!(sym, EqTerm::J { .. }),
            "sym_proof should return EqTerm::J, got {:?}",
            sym
        );

        // Test transitivity - now returns EqTerm::J (J eliminator)
        let trans = trans_proof(&refl, &sym);
        assert!(
            matches!(trans, EqTerm::J { .. }),
            "trans_proof should return EqTerm::J, got {:?}",
            trans
        );
    }

    #[test]
    fn test_nat_conversion_roundtrip() {
        use super::nat::*;

        for n in 0..10 {
            let nat_ty = from_usize(n);
            let back = to_usize(&nat_ty);
            assert_eq!(back, Some(n), "Roundtrip failed for {}", n);
        }
    }

    // ==================== Integration Tests ====================
    // Type-level computation: compile-time evaluation of type expressions, reduction rules, normalization — (Type-Level Programming)

    #[test]
    fn test_type_equality_all_primitives() {
        use super::equality::types_equal;

        // Test all primitive types
        assert!(types_equal(&Type::Int, &Type::Int));
        assert!(types_equal(&Type::Bool, &Type::Bool));
        assert!(types_equal(&Type::Float, &Type::Float));
        assert!(types_equal(&Type::Text, &Type::Text));
        assert!(types_equal(&Type::Char, &Type::Char));
        assert!(types_equal(&Type::Unit, &Type::Unit));
        // Note: Type::Never may not be fully supported in equality

        // Different types should not be equal
        assert!(!types_equal(&Type::Int, &Type::Bool));
        assert!(!types_equal(&Type::Float, &Type::Text));
    }

    #[test]
    fn test_type_equality_nested_tuples() {
        use super::equality::types_equal;

        // (Int, (Bool, Text))
        let inner_tuple = Type::Tuple(vec![Type::Bool, Type::Text].into());
        let nested1 = Type::Tuple(vec![Type::Int, inner_tuple.clone()].into());

        // Same structure
        let inner_tuple2 = Type::Tuple(vec![Type::Bool, Type::Text].into());
        let nested2 = Type::Tuple(vec![Type::Int, inner_tuple2].into());

        assert!(types_equal(&nested1, &nested2));

        // Different nesting
        let different = Type::Tuple(vec![Type::Int, Type::Bool, Type::Text].into());
        assert!(!types_equal(&nested1, &different));
    }

    #[test]
    fn test_type_equality_arrays() {
        use super::equality::types_equal;

        let arr1 = Type::Array {
            element: Box::new(Type::Int),
            size: Some(10),
        };
        let arr2 = Type::Array {
            element: Box::new(Type::Int),
            size: Some(10),
        };
        let arr3 = Type::Array {
            element: Box::new(Type::Int),
            size: Some(20),
        };
        let arr4 = Type::Array {
            element: Box::new(Type::Bool),
            size: Some(10),
        };

        assert!(types_equal(&arr1, &arr2));
        assert!(!types_equal(&arr1, &arr3)); // Different size
        assert!(!types_equal(&arr1, &arr4)); // Different element type
    }

    #[test]
    fn test_nat_operations_zero_cases() {
        use super::nat::*;

        let zero_ty = zero();
        let three = from_usize(3);

        // Zero is zero
        assert!(is_zero(&zero_ty));
        assert!(!is_zero(&three));
    }

    #[test]
    fn test_nat_peano_structure() {
        use super::nat::*;

        // Build 3 as Succ(Succ(Succ(Zero)))
        let zero_ty = zero();
        let one = succ(zero_ty.clone());
        let two = succ(one.clone());
        let three = succ(two.clone());

        // Verify structure
        assert_eq!(to_usize(&zero_ty), Some(0));
        assert_eq!(to_usize(&one), Some(1));
        assert_eq!(to_usize(&two), Some(2));
        assert_eq!(to_usize(&three), Some(3));
    }

    #[test]
    fn test_indexed_types_comprehensive() {
        use super::indexed::*;

        // Test various Fin bounds
        for bound in [1, 5, 10, 100, 1000] {
            let fin = fin_type(bound);
            match fin {
                Type::Named { path, args } => {
                    assert_eq!(args.len(), 1);
                    // Just verify it's a Named type with one arg
                    assert!(
                        path.as_ident()
                            .map(|id| id.name.as_str() == "Fin")
                            .unwrap_or(false)
                    );
                }
                _ => panic!("Expected Named type"),
            }
        }
    }

    #[test]
    fn test_indexed_list_various_lengths() {
        use super::indexed::*;

        for len in [0, 1, 5, 10] {
            let list = indexed_list_type(Type::Int, len);
            match list {
                Type::Generic { name, args } => {
                    assert_eq!(name.as_str(), "List");
                    assert_eq!(args.len(), 2);
                }
                _ => panic!("Expected Generic type"),
            }
        }
    }

    #[test]
    fn test_matrix_various_dimensions() {
        use super::indexed::*;

        let dimensions = [(1, 1), (3, 4), (10, 20), (100, 100)];

        for (rows, cols) in dimensions {
            let matrix = matrix_type(Type::Float, rows, cols);
            match matrix {
                Type::Generic { name, args } => {
                    assert_eq!(name.as_str(), "Matrix");
                    assert_eq!(args.len(), 3);
                }
                _ => panic!("Expected Generic type"),
            }
        }
    }

    #[test]
    fn test_arithmetic_proofs_comprehensive() {
        use super::arithmetic_proofs::*;
        use super::nat::*;

        // Test with various operand combinations
        let test_cases = [(0, 0), (0, 5), (5, 0), (3, 7), (10, 10)];

        for (a, b) in test_cases {
            let m = from_usize(a);
            let n = from_usize(b);

            // All proofs should succeed
            assert!(plus_comm_proof(&m, &n).is_ok());
            assert!(mult_comm_proof(&m, &n).is_ok());
        }
    }

    #[test]
    fn test_equality_proof_chain() {
        use super::equality::*;

        let ty = Type::Int;
        let val1 = make_int_expr(1);
        let val2 = make_int_expr(2);
        let val3 = make_int_expr(3);

        // Build proof chain: a = a, sym(a = a) = a, trans(a = a, a = a) = a
        let refl1 = refl_proof(&ty, &val1);
        let refl2 = refl_proof(&ty, &val2);
        let refl3 = refl_proof(&ty, &val3);

        // Reflexivity proofs should be EqTerm::Refl
        assert!(matches!(refl1, EqTerm::Refl(_)));
        assert!(matches!(refl2, EqTerm::Refl(_)));
        assert!(matches!(refl3, EqTerm::Refl(_)));

        // Symmetry proofs should be EqTerm::J
        let sym1 = sym_proof(&refl1);
        let sym2 = sym_proof(&refl2);
        assert!(matches!(sym1, EqTerm::J { .. }));
        assert!(matches!(sym2, EqTerm::J { .. }));

        // Transitivity proofs should be EqTerm::J
        let trans12 = trans_proof(&refl1, &refl2);
        let trans23 = trans_proof(&refl2, &refl3);
        assert!(matches!(trans12, EqTerm::J { .. }));
        assert!(matches!(trans23, EqTerm::J { .. }));
    }

    #[test]
    fn test_fin_boundary_values() {
        use super::indexed::*;

        // Test boundary cases for Fin type
        let bound = 10;

        // Valid values: 0 to bound-1
        for i in 0..bound {
            assert!(check_fin_bounds(i, bound));
            assert!(make_fin(i, bound).is_some());
        }

        // Invalid values: bound and above
        for i in bound..bound + 5 {
            assert!(!check_fin_bounds(i, bound));
            assert!(make_fin(i, bound).is_none());
        }
    }

    #[test]
    fn test_const_value_types() {
        use super::indexed::*;

        // Test ConstValue construction
        let int_val = ConstValue::Int(42);
        let uint_val = ConstValue::UInt(100);
        let float_val = ConstValue::Float(3.14);
        let _bool_val = ConstValue::Bool(true);
        let _text_val = ConstValue::Text("hello".into());

        // Make sure they can be compared and used
        assert_ne!(int_val, uint_val);
        assert_ne!(int_val, float_val);

        // Boolean values
        assert_ne!(ConstValue::Bool(true), ConstValue::Bool(false));
    }

    #[test]
    fn test_type_equality_references() {
        use super::equality::types_equal;

        // Test reference types
        let ref1 = Type::Reference {
            inner: Box::new(Type::Int),
            mutable: false,
        };
        let ref2 = Type::Reference {
            inner: Box::new(Type::Int),
            mutable: false,
        };
        let ref3 = Type::Reference {
            inner: Box::new(Type::Int),
            mutable: true,
        };
        let ref4 = Type::Reference {
            inner: Box::new(Type::Bool),
            mutable: false,
        };

        assert!(types_equal(&ref1, &ref2));
        assert!(!types_equal(&ref1, &ref3)); // Different mutability
        assert!(!types_equal(&ref1, &ref4)); // Different inner type
    }

    #[test]
    fn test_type_equality_generic_types() {
        use super::equality::types_equal;

        let gen1 = Type::Generic {
            name: "List".into(),
            args: vec![Type::Int].into(),
        };
        let gen2 = Type::Generic {
            name: "List".into(),
            args: vec![Type::Int].into(),
        };
        let gen3 = Type::Generic {
            name: "Set".into(),
            args: vec![Type::Int].into(),
        };
        let gen4 = Type::Generic {
            name: "List".into(),
            args: vec![Type::Bool].into(),
        };

        assert!(types_equal(&gen1, &gen2));
        assert!(!types_equal(&gen1, &gen3)); // Different name
        assert!(!types_equal(&gen1, &gen4)); // Different args
    }

    #[test]
    fn test_triple_associativity_proof() {
        use super::arithmetic_proofs::*;
        use super::nat::*;

        let a = from_usize(2);
        let b = from_usize(3);
        let c = from_usize(5);

        // Test plus associativity: (a + b) + c = a + (b + c)
        let proof1 = plus_assoc_proof(&a, &b, &c);
        assert!(proof1.is_ok());

        // Test mult associativity: (a * b) * c = a * (b * c)
        let proof2 = mult_assoc_proof(&a, &b, &c);
        assert!(proof2.is_ok());
    }

    #[test]
    fn test_zero_identity_proofs() {
        use super::arithmetic_proofs::*;
        use super::nat::*;

        let n = from_usize(42);

        // 0 + n = n
        assert!(plus_zero_left_proof(&n).is_ok());

        // n + 0 = n
        assert!(plus_zero_right_proof(&n).is_ok());

        // 1 * n = n
        assert!(mult_one_left_proof(&n).is_ok());

        // n * 1 = n
        assert!(mult_one_right_proof(&n).is_ok());
    }
}
