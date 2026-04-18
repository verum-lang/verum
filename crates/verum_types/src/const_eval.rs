//! Compile-time constant evaluation for meta parameters
//!
//! Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Unified meta-system for compile-time computation
//!
//! This module implements compile-time expression evaluation for meta parameters,
//! enabling features like:
//! - Compile-time arithmetic: N: meta usize = 2 + 3
//! - Compile-time comparisons: N: meta usize{> 0}
//! - Tensor shape computation: Shape: meta [usize] = [2, 3]
//!
//! # Architecture
//!
//! The evaluator operates in two modes:
//! 1. **Value evaluation**: Compute concrete values from expressions
//! 2. **Type evaluation**: Resolve meta types with computed values
//!
//! # Performance
//!
//! All evaluation happens at compile-time with zero runtime overhead.
//! Evaluation is cached to avoid redundant computation.

use crate::ty::Type;
use thiserror::Error;
use verum_ast::{
    decl::{FunctionBody, FunctionDecl, FunctionParam, FunctionParamKind},
    expr::{BinOp, Block, ConditionKind, Expr, ExprKind, UnOp},
    literal::Literal,
    pattern::{Pattern, PatternKind},
    span::Span,
    stmt::{Stmt, StmtKind},
    ty::PathSegment,
};
use verum_common::{ConstValue, List, Map, Maybe, Text};

// ConstValue is imported from verum_common - the unified canonical type
// See: verum_common/src/const_value.rs for the full implementation
//
// Note: This module previously defined its own ConstValue. Now it uses the
// unified type from verum_common which provides:
// - Unit, Bool, Int(i128), UInt(u128), Float(f64), Char, Text, Bytes, Array, Tuple, Maybe
// - Full API: as_i128, as_u128, as_bool_value, as_f64, as_text, as_char_value, etc.
// - Display implementation

/// Errors that can occur during const evaluation
#[derive(Debug, Error)]
pub enum ConstEvalError {
    #[error("type error: expected {expected}, found {actual}")]
    TypeError { expected: Text, actual: Text },

    #[error("overflow in arithmetic operation: {operation}")]
    Overflow { operation: Text },

    #[error("division by zero")]
    DivisionByZero,

    #[error("unbound variable: {name}")]
    UnboundVariable { name: Text },

    #[error("unsupported operation at compile time: {operation}")]
    UnsupportedOperation { operation: Text },

    #[error("array index out of bounds: index {index}, length {length}")]
    IndexOutOfBounds { index: usize, length: usize },

    #[error("expected array, found {actual}")]
    NotAnArray { actual: Text },

    #[error("cannot evaluate non-constant expression")]
    NotConstant,

    /// Undefined function call
    ///
    /// Raised when calling a function that is not registered in the meta interpreter.
    /// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Section 3.1 - Meta function calls
    #[error("undefined function: {name}")]
    UndefinedFunction { name: Text },

    /// Arity mismatch in function call
    ///
    /// Raised when a function is called with wrong number of arguments.
    /// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Section 3.2 - Function call validation
    #[error("function `{name}` expects {expected} arguments, but {actual} were provided")]
    ArityMismatch {
        name: Text,
        expected: usize,
        actual: usize,
    },

    /// Non-meta function call at compile time
    ///
    /// Raised when trying to call a non-meta function during compile-time evaluation.
    /// Only `meta fn` can be called at compile-time.
    /// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Section 2.1 - Meta function restrictions
    #[error("cannot call non-meta function `{name}` at compile time")]
    NonMetaFunction { name: Text },

    /// Recursion depth exceeded
    ///
    /// Raised when meta function evaluation exceeds the recursion limit.
    /// This prevents infinite recursion at compile time.
    /// Quote hygiene: macro-generated code uses hygienic naming to prevent variable capture and scope pollution — .1 - Evaluation limits
    #[error("recursion depth exceeded ({depth}) when evaluating `{name}`")]
    RecursionDepthExceeded { name: Text, depth: usize },

    /// Pattern binding failed
    ///
    /// Raised when a pattern match fails during const evaluation.
    #[error("pattern match failed for value {value}")]
    PatternMatchFailed { value: Text },

    #[error("{0}")]
    Other(Text),
}

pub type Result<T> = std::result::Result<T, ConstEvalError>;

/// Maximum recursion depth for meta function evaluation
///
/// Quote hygiene: macro-generated code uses hygienic naming to prevent variable capture and scope pollution — .1 - Evaluation limits
pub const MAX_RECURSION_DEPTH: usize = 256;

/// A registered meta function for compile-time evaluation
///
/// Stores the function definition needed to interpret function calls at compile time.
/// Only `meta fn` functions can be registered and called during const evaluation.
///
/// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Section 2.1 - Meta function definitions
#[derive(Debug, Clone)]
pub struct MetaFunction {
    /// Function name
    pub name: Text,
    /// Parameter names (in order)
    pub param_names: List<Text>,
    /// Function body expression
    pub body: Block,
    /// Whether the function is actually a meta function
    pub is_meta: bool,
}

impl MetaFunction {
    /// Create a MetaFunction from a FunctionDecl AST node
    ///
    /// Extracts the parameter names and body from the function declaration.
    /// Returns None if the function has no body (extern function).
    pub fn from_decl(decl: &FunctionDecl) -> Option<Self> {
        // Get the function body
        let function_body = decl.body.as_ref()?;

        // Convert FunctionBody to Block
        let body = match function_body {
            FunctionBody::Block(block) => block.clone(),
            FunctionBody::Expr(expr) => {
                // Convert expression body `= expr;` into block `{ expr }`
                Block {
                    stmts: List::new(),
                    expr: Maybe::Some(Box::new(expr.clone())),
                    span: expr.span,
                }
            }
        };

        // Extract parameter names
        let mut param_names = List::with_capacity(decl.params.len());
        for param in &decl.params {
            match &param.kind {
                FunctionParamKind::Regular { pattern, .. } => {
                    if let Some(name) = Self::extract_pattern_name(pattern) {
                        param_names.push(name);
                    } else {
                        // Complex patterns not supported in meta functions yet
                        return None;
                    }
                }
                // Self parameters are not valid in meta functions
                _ => return None,
            }
        }

        Some(Self {
            name: Text::from(decl.name.name.as_str()),
            param_names,
            body,
            is_meta: decl.is_meta,
        })
    }

    /// Extract the name from a simple pattern
    fn extract_pattern_name(pattern: &Pattern) -> Option<Text> {
        match &pattern.kind {
            PatternKind::Ident { name, .. } => Some(Text::from(name.name.as_str())),
            PatternKind::Wildcard => Some(Text::from("_")),
            _ => None,
        }
    }

    /// Get the arity (number of parameters)
    pub fn arity(&self) -> usize {
        self.param_names.len()
    }
}

/// Compile-time constant evaluator
///
/// # Example
///
/// ```ignore
/// use verum_types::const_eval::ConstEvaluator;
/// use verum_common::ConstValue;
/// use verum_ast::{expr::{Expr, ExprKind, BinOp}, literal::Literal, span::Span};
///
/// let mut eval = ConstEvaluator::new();
///
/// // Evaluate: 2 + 3
/// let left = Expr::new(ExprKind::Literal(Literal::int(2, Span::dummy())), Span::dummy());
/// let right = Expr::new(ExprKind::Literal(Literal::int(3, Span::dummy())), Span::dummy());
/// let expr = Expr::new(
///     ExprKind::Binary {
///         op: BinOp::Add,
///         left: Box::new(left),
///         right: Box::new(right),
///     },
///     Span::dummy()
/// );
/// let result = eval.eval(&expr)?;
/// assert_eq!(result, ConstValue::Int(5));
/// ```
pub struct ConstEvaluator {
    /// Environment mapping variable names to values
    env: Map<Text, ConstValue>,
    /// Registry of meta functions available for compile-time calls
    /// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Section 3.1 - Meta function registry
    functions: Map<Text, MetaFunction>,
    /// Current recursion depth for meta function calls
    recursion_depth: usize,
}

impl ConstEvaluator {
    /// Create a new const evaluator
    pub fn new() -> Self {
        Self {
            env: Map::new(),
            functions: Map::new(),
            recursion_depth: 0,
        }
    }

    /// Bind a variable to a value
    pub fn bind(&mut self, name: impl Into<Text>, value: ConstValue) {
        self.env.insert(name.into(), value);
    }

    /// Get the current value of a variable, if bound
    pub fn get(&self, name: &Text) -> Option<ConstValue> {
        self.env.get(name).cloned()
    }

    /// Unbind a variable (remove from environment)
    pub fn unbind(&mut self, name: &Text) {
        self.env.remove(name);
    }

    /// Register a meta function for compile-time evaluation
    ///
    /// Only `meta fn` functions can be registered. Non-meta functions will
    /// be registered but will fail at call time with `NonMetaFunction` error.
    ///
    /// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Section 3.1 - Meta function registry
    ///
    /// # Example
    ///
    /// ```no_run
    /// use verum_types::const_eval::ConstEvaluator;
    /// use verum_ast::decl::FunctionDecl;
    ///
    /// let mut eval = ConstEvaluator::new();
    /// // Register a parsed function declaration
    /// // eval.register_function(&my_meta_fn_decl);
    /// ```
    pub fn register_function(&mut self, decl: &FunctionDecl) {
        if let Some(meta_fn) = MetaFunction::from_decl(decl) {
            self.functions.insert(meta_fn.name.clone(), meta_fn);
        }
    }

    /// Register a pre-built MetaFunction
    pub fn register_meta_function(&mut self, meta_fn: MetaFunction) {
        self.functions.insert(meta_fn.name.clone(), meta_fn);
    }

    /// Check if a function is registered
    pub fn has_function(&self, name: &str) -> bool {
        self.functions.contains_key(&Text::from(name))
    }

    /// Get a registered function by name
    pub fn get_function(&self, name: &str) -> Option<&MetaFunction> {
        self.functions.get(&Text::from(name))
    }

    /// Evaluate an expression to a const value
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The expression is not a compile-time constant
    /// - Type mismatch in operations
    /// - Arithmetic overflow
    /// - Division by zero
    pub fn eval(&mut self, expr: &Expr) -> Result<ConstValue> {
        match &expr.kind {
            // Literals
            ExprKind::Literal(lit) => self.eval_literal(lit),

            // Variable lookup
            ExprKind::Path(path) => {
                // Simple path with single identifier
                if path.segments.len() == 1 {
                    if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                        let name = ident.name.as_str();
                        match self.env.get(&Text::from(name)) {
                            Some(val) => Ok(val.clone()),
                            None => Err(ConstEvalError::UnboundVariable {
                                name: Text::from(name),
                            }),
                        }
                    } else {
                        Err(ConstEvalError::NotConstant)
                    }
                } else {
                    Err(ConstEvalError::NotConstant)
                }
            }

            // Binary operations
            ExprKind::Binary { op, left, right } => {
                let left_val = self.eval(left)?;
                let right_val = self.eval(right)?;
                self.eval_binary(*op, left_val, right_val)
            }

            // Unary operations
            ExprKind::Unary { op, expr } => {
                let val = self.eval(expr)?;
                self.eval_unary(*op, val)
            }

            // Arrays
            ExprKind::Array(arr_expr) => match arr_expr {
                verum_ast::expr::ArrayExpr::List(elements) => {
                    let mut values = List::with_capacity(elements.len());
                    for elem in elements {
                        values.push(self.eval(elem)?);
                    }
                    Ok(ConstValue::Array(values))
                }
                verum_ast::expr::ArrayExpr::Repeat { value, count } => {
                    let val = self.eval(value)?;
                    let count_val = self.eval(count)?;
                    let n = count_val
                        .as_u128()
                        .ok_or_else(|| ConstEvalError::TypeError {
                            expected: Text::from("usize"),
                            actual: format!("{}", count_val).into(),
                        })? as usize;
                    Ok(ConstValue::Array(List::from_iter(vec![val; n])))
                }
            },

            // Array indexing
            ExprKind::Index { expr, index } => {
                let arr = self.eval(expr)?;
                let idx = self.eval(index)?;

                match arr {
                    ConstValue::Array(values) => {
                        let i = idx.as_u128().ok_or_else(|| ConstEvalError::TypeError {
                            expected: Text::from("usize"),
                            actual: format!("{}", idx).into(),
                        })? as usize;

                        values
                            .get(i)
                            .cloned()
                            .ok_or_else(|| ConstEvalError::IndexOutOfBounds {
                                index: i,
                                length: values.len(),
                            })
                    }
                    _ => Err(ConstEvalError::NotAnArray {
                        actual: format!("{}", arr).into(),
                    }),
                }
            }

            // Tuples
            ExprKind::Tuple(elements) => {
                let mut values = List::with_capacity(elements.len());
                for elem in elements {
                    values.push(self.eval(elem)?);
                }
                Ok(ConstValue::Tuple(values))
            }

            // Tuple indexing
            ExprKind::TupleIndex { expr, index } => {
                let tup = self.eval(expr)?;
                match tup {
                    ConstValue::Tuple(values) => {
                        let i = *index as usize;
                        values
                            .get(i)
                            .cloned()
                            .ok_or_else(|| ConstEvalError::IndexOutOfBounds {
                                index: i,
                                length: values.len(),
                            })
                    }
                    _ => Err(ConstEvalError::TypeError {
                        expected: Text::from("tuple"),
                        actual: format!("{}", tup).into(),
                    }),
                }
            }

            // Parenthesized expressions
            ExprKind::Paren(inner) => self.eval(inner),

            // Function calls
            // Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Section 3.1 - Meta function calls at compile time
            ExprKind::Call { func, args, .. } => self.eval_call(func, args),

            // Block expressions
            // Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Section 3.2 - Block evaluation in meta context
            ExprKind::Block(block) => self.eval_block(block),

            // If expressions
            // Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Section 3.3 - Conditional evaluation
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.eval_if(condition, then_branch, else_branch),

            // All other expression kinds are not compile-time constants
            _ => Err(ConstEvalError::NotConstant),
        }
    }

    /// Evaluate a function call at compile time
    ///
    /// Only meta functions can be called at compile time. The function must be
    /// registered in the evaluator's function registry.
    ///
    /// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Section 3.1 - Meta function calls
    fn eval_call(&mut self, func: &Expr, args: &[Expr]) -> Result<ConstValue> {
        // Extract function name from path expression
        let func_name = match &func.kind {
            ExprKind::Path(path) => {
                if path.segments.len() == 1 {
                    if let PathSegment::Name(ident) = &path.segments[0] {
                        Text::from(ident.name.as_str())
                    } else {
                        return Err(ConstEvalError::UnsupportedOperation {
                            operation: Text::from("non-simple path in function call"),
                        });
                    }
                } else {
                    // Multi-segment paths (e.g., module.function) not supported yet
                    return Err(ConstEvalError::UnsupportedOperation {
                        operation: Text::from("qualified function call"),
                    });
                }
            }
            _ => {
                return Err(ConstEvalError::UnsupportedOperation {
                    operation: Text::from("dynamic function call"),
                });
            }
        };

        // Look up the function
        let meta_fn = self
            .functions
            .get(&func_name)
            .cloned()
            .ok_or_else(|| ConstEvalError::UndefinedFunction {
                name: func_name.clone(),
            })?;

        // Check if it's a meta function
        if !meta_fn.is_meta {
            return Err(ConstEvalError::NonMetaFunction { name: func_name });
        }

        // Check arity
        if args.len() != meta_fn.arity() {
            return Err(ConstEvalError::ArityMismatch {
                name: func_name,
                expected: meta_fn.arity(),
                actual: args.len(),
            });
        }

        // Check recursion depth
        if self.recursion_depth >= MAX_RECURSION_DEPTH {
            return Err(ConstEvalError::RecursionDepthExceeded {
                name: func_name,
                depth: self.recursion_depth,
            });
        }

        // Evaluate arguments
        let mut arg_values = List::with_capacity(args.len());
        for arg in args {
            arg_values.push(self.eval(arg)?);
        }

        // Save current environment
        let old_env = self.env.clone();

        // Bind parameters to argument values
        for (param_name, arg_value) in meta_fn.param_names.iter().zip(arg_values.iter()) {
            self.env.insert(param_name.clone(), arg_value.clone());
        }

        // Increment recursion depth
        self.recursion_depth += 1;

        // Evaluate the function body
        let result = self.eval_block(&meta_fn.body);

        // Decrement recursion depth
        self.recursion_depth -= 1;

        // Restore environment
        self.env = old_env;

        result
    }

    /// Evaluate a block expression
    ///
    /// Executes statements in order and returns the value of the final expression.
    /// Let bindings create local variables that shadow outer bindings.
    ///
    /// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Section 3.2 - Block evaluation in meta context
    fn eval_block(&mut self, block: &Block) -> Result<ConstValue> {
        // Save environment for restoration after block
        let old_env = self.env.clone();

        // Evaluate statements
        for stmt in &block.stmts {
            self.eval_stmt(stmt)?;
        }

        // Evaluate final expression or return unit
        let result = if let Maybe::Some(expr) = &block.expr {
            self.eval(expr)
        } else {
            // Block without trailing expression evaluates to unit
            Ok(ConstValue::Unit)
        };

        // Restore environment (let bindings are local to block)
        self.env = old_env;

        result
    }

    /// Evaluate a statement
    ///
    /// Handles let bindings and expression statements.
    fn eval_stmt(&mut self, stmt: &Stmt) -> Result<()> {
        match &stmt.kind {
            StmtKind::Let { pattern, value, .. } => {
                // Evaluate the value if present
                let val = if let Maybe::Some(expr) = value {
                    self.eval(expr)?
                } else {
                    // Uninitialized let - treat as unit
                    ConstValue::Unit
                };

                // Bind the pattern
                self.bind_pattern(pattern, val)?;
                Ok(())
            }

            StmtKind::LetElse {
                pattern,
                value,
                else_block,
                ..
            } => {
                // Evaluate the value
                let val = self.eval(value)?;

                // Try to bind the pattern
                if self.try_bind_pattern(pattern, &val) {
                    Ok(())
                } else {
                    // Pattern didn't match - evaluate else block (should diverge)
                    let _ = self.eval_block(else_block)?;
                    // If else block didn't diverge, it's an error
                    Err(ConstEvalError::PatternMatchFailed {
                        value: format!("{}", val).into(),
                    })
                }
            }

            StmtKind::Expr { expr, .. } => {
                // Evaluate expression for side effects (none at compile time)
                let _ = self.eval(expr)?;
                Ok(())
            }

            StmtKind::Item(_) => {
                // Item declarations in blocks not supported at compile time
                Err(ConstEvalError::UnsupportedOperation {
                    operation: Text::from("item declaration in block"),
                })
            }

            StmtKind::Defer(_) => {
                // Defer statements not supported at compile time
                Err(ConstEvalError::UnsupportedOperation {
                    operation: Text::from("defer in const evaluation"),
                })
            }

            StmtKind::Errdefer(_) => {
                // Errdefer statements not supported at compile time
                Err(ConstEvalError::UnsupportedOperation {
                    operation: Text::from("errdefer in const evaluation"),
                })
            }

            StmtKind::Provide { .. } => {
                // Context provision not supported at compile time
                Err(ConstEvalError::UnsupportedOperation {
                    operation: Text::from("provide in const evaluation"),
                })
            }

            StmtKind::ProvideScope { .. } => {
                // Context provision scope not supported at compile time
                Err(ConstEvalError::UnsupportedOperation {
                    operation: Text::from("provide scope in const evaluation"),
                })
            }

            StmtKind::Empty => {
                // Empty statement is a no-op
                Ok(())
            }
        }
    }

    /// Bind a pattern to a value
    ///
    /// Creates variable bindings for identifiers in the pattern.
    fn bind_pattern(&mut self, pattern: &Pattern, value: ConstValue) -> Result<()> {
        match &pattern.kind {
            PatternKind::Ident { name, .. } => {
                self.env.insert(Text::from(name.name.as_str()), value);
                Ok(())
            }

            PatternKind::Wildcard => {
                // Wildcard doesn't bind anything
                Ok(())
            }

            PatternKind::Tuple(patterns) => {
                if let ConstValue::Tuple(values) = value {
                    if patterns.len() != values.len() {
                        return Err(ConstEvalError::PatternMatchFailed {
                            value: Text::from("tuple length mismatch"),
                        });
                    }
                    for (pat, val) in patterns.iter().zip(values.iter()) {
                        self.bind_pattern(pat, val.clone())?;
                    }
                    Ok(())
                } else {
                    Err(ConstEvalError::PatternMatchFailed {
                        value: format!("{}", value).into(),
                    })
                }
            }

            PatternKind::Array(patterns) => {
                if let ConstValue::Array(values) = value {
                    if patterns.len() != values.len() {
                        return Err(ConstEvalError::PatternMatchFailed {
                            value: Text::from("array length mismatch"),
                        });
                    }
                    for (pat, val) in patterns.iter().zip(values.iter()) {
                        self.bind_pattern(pat, val.clone())?;
                    }
                    Ok(())
                } else {
                    Err(ConstEvalError::PatternMatchFailed {
                        value: format!("{}", value).into(),
                    })
                }
            }

            _ => Err(ConstEvalError::UnsupportedOperation {
                operation: Text::from("complex pattern in let binding"),
            }),
        }
    }

    /// Try to bind a pattern, returning false if it doesn't match
    fn try_bind_pattern(&mut self, pattern: &Pattern, value: &ConstValue) -> bool {
        match &pattern.kind {
            PatternKind::Ident { name, .. } => {
                self.env.insert(Text::from(name.name.as_str()), value.clone());
                true
            }

            PatternKind::Wildcard => true,

            PatternKind::Literal(lit) => {
                // Check if literal matches
                match self.eval_literal(lit) {
                    Ok(lit_val) => lit_val == *value,
                    Err(_) => false,
                }
            }

            PatternKind::Tuple(patterns) => {
                if let ConstValue::Tuple(values) = value {
                    if patterns.len() != values.len() {
                        return false;
                    }
                    for (pat, val) in patterns.iter().zip(values.iter()) {
                        if !self.try_bind_pattern(pat, val) {
                            return false;
                        }
                    }
                    true
                } else {
                    false
                }
            }

            _ => false,
        }
    }

    /// Evaluate an if expression
    ///
    /// Evaluates the condition and then either the then-branch or else-branch.
    ///
    /// Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Section 3.3 - Conditional evaluation
    fn eval_if(
        &mut self,
        condition: &verum_ast::expr::IfCondition,
        then_branch: &Block,
        else_branch: &Maybe<Box<Expr>>,
    ) -> Result<ConstValue> {
        // Evaluate the condition chain
        let condition_result = self.eval_if_condition(condition)?;

        if condition_result {
            self.eval_block(then_branch)
        } else if let Maybe::Some(else_expr) = else_branch {
            self.eval(else_expr)
        } else {
            // No else branch and condition was false - return unit
            Ok(ConstValue::Unit)
        }
    }

    /// Evaluate an if condition (possibly a chain of conditions)
    fn eval_if_condition(&mut self, condition: &verum_ast::expr::IfCondition) -> Result<bool> {
        // All conditions in the chain must be true
        for cond_kind in &condition.conditions {
            let result = match cond_kind {
                ConditionKind::Expr(expr) => {
                    let val = self.eval(expr)?;
                    val.as_bool_value().ok_or_else(|| ConstEvalError::TypeError {
                        expected: Text::from("bool"),
                        actual: format!("{}", val).into(),
                    })?
                }

                ConditionKind::Let { pattern, value } => {
                    let val = self.eval(value)?;
                    self.try_bind_pattern(pattern, &val)
                }
            };

            if !result {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Evaluate a literal
    fn eval_literal(&self, lit: &Literal) -> Result<ConstValue> {
        use verum_ast::literal::LiteralKind;
        match &lit.kind {
            LiteralKind::Int(int_lit) => Ok(ConstValue::Int(int_lit.value)),
            LiteralKind::Float(float_lit) => Ok(ConstValue::Float(float_lit.value)),
            LiteralKind::Bool(b) => Ok(ConstValue::Bool(*b)),
            LiteralKind::Text(string_lit) => Ok(ConstValue::Text(Text::from(string_lit.as_str()))),
            LiteralKind::Char(c) => Ok(ConstValue::Char(*c)),
            _ => Err(ConstEvalError::NotConstant),
        }
    }

    /// Evaluate a binary operation
    fn eval_binary(&self, op: BinOp, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        match op {
            // Arithmetic operations
            BinOp::Add => self.eval_add(left, right),
            BinOp::Sub => self.eval_sub(left, right),
            BinOp::Mul => self.eval_mul(left, right),
            BinOp::Div => self.eval_div(left, right),
            BinOp::Rem => self.eval_rem(left, right),

            // Comparison operations
            BinOp::Eq => self.eval_eq(left, right),
            BinOp::Ne => self.eval_ne(left, right),
            BinOp::Lt => self.eval_lt(left, right),
            BinOp::Le => self.eval_le(left, right),
            BinOp::Gt => self.eval_gt(left, right),
            BinOp::Ge => self.eval_ge(left, right),

            // Logical operations
            BinOp::And => self.eval_and(left, right),
            BinOp::Or => self.eval_or(left, right),

            // Bitwise operations
            BinOp::BitAnd => self.eval_bitand(left, right),
            BinOp::BitOr => self.eval_bitor(left, right),
            BinOp::BitXor => self.eval_bitxor(left, right),
            BinOp::Shl => self.eval_shl(left, right),
            BinOp::Shr => self.eval_shr(left, right),

            _ => Err(ConstEvalError::UnsupportedOperation {
                operation: format!("{}", op).into(),
            }),
        }
    }

    /// Evaluate addition
    fn eval_add(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        match (left, right) {
            (ConstValue::Int(a), ConstValue::Int(b)) => a
                .checked_add(b)
                .map(ConstValue::Int)
                .ok_or_else(|| ConstEvalError::Overflow {
                    operation: format!("{} + {}", a, b).into(),
                }),
            (ConstValue::UInt(a), ConstValue::UInt(b)) => a
                .checked_add(b)
                .map(ConstValue::UInt)
                .ok_or_else(|| ConstEvalError::Overflow {
                    operation: format!("{} + {}", a, b).into(),
                }),
            (ConstValue::Float(a), ConstValue::Float(b)) => Ok(ConstValue::Float(a + b)),
            // Text concatenation
            (ConstValue::Text(a), ConstValue::Text(b)) => {
                let mut result = a.to_string();
                result.push_str(b.as_str());
                Ok(ConstValue::Text(Text::from(result)))
            }
            (a, b) => Err(ConstEvalError::TypeError {
                expected: Text::from("matching numeric types or text"),
                actual: format!("{} and {}", a, b).into(),
            }),
        }
    }

    /// Evaluate subtraction
    fn eval_sub(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        match (left, right) {
            (ConstValue::Int(a), ConstValue::Int(b)) => a
                .checked_sub(b)
                .map(ConstValue::Int)
                .ok_or_else(|| ConstEvalError::Overflow {
                    operation: format!("{} - {}", a, b).into(),
                }),
            (ConstValue::UInt(a), ConstValue::UInt(b)) => a
                .checked_sub(b)
                .map(ConstValue::UInt)
                .ok_or_else(|| ConstEvalError::Overflow {
                    operation: format!("{} - {}", a, b).into(),
                }),
            (ConstValue::Float(a), ConstValue::Float(b)) => Ok(ConstValue::Float(a - b)),
            (a, b) => Err(ConstEvalError::TypeError {
                expected: Text::from("matching numeric types"),
                actual: format!("{} and {}", a, b).into(),
            }),
        }
    }

    /// Evaluate multiplication
    fn eval_mul(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        match (left, right) {
            (ConstValue::Int(a), ConstValue::Int(b)) => a
                .checked_mul(b)
                .map(ConstValue::Int)
                .ok_or_else(|| ConstEvalError::Overflow {
                    operation: format!("{} * {}", a, b).into(),
                }),
            (ConstValue::UInt(a), ConstValue::UInt(b)) => a
                .checked_mul(b)
                .map(ConstValue::UInt)
                .ok_or_else(|| ConstEvalError::Overflow {
                    operation: format!("{} * {}", a, b).into(),
                }),
            (ConstValue::Float(a), ConstValue::Float(b)) => Ok(ConstValue::Float(a * b)),
            (a, b) => Err(ConstEvalError::TypeError {
                expected: Text::from("matching numeric types"),
                actual: format!("{} and {}", a, b).into(),
            }),
        }
    }

    /// Evaluate division
    fn eval_div(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        match (left, right) {
            (ConstValue::Int(a), ConstValue::Int(b)) => {
                if b == 0 {
                    return Err(ConstEvalError::DivisionByZero);
                }
                a.checked_div(b)
                    .map(ConstValue::Int)
                    .ok_or_else(|| ConstEvalError::Overflow {
                        operation: format!("{} / {}", a, b).into(),
                    })
            }
            (ConstValue::UInt(a), ConstValue::UInt(b)) => {
                if b == 0 {
                    return Err(ConstEvalError::DivisionByZero);
                }
                Ok(ConstValue::UInt(a / b))
            }
            (ConstValue::Float(a), ConstValue::Float(b)) => {
                // IEEE 754: float division by zero produces Infinity/NaN, not an error.
                // 1.0/0.0 = +Inf, -1.0/0.0 = -Inf, 0.0/0.0 = NaN
                Ok(ConstValue::Float(a / b))
            }
            (a, b) => Err(ConstEvalError::TypeError {
                expected: Text::from("matching numeric types"),
                actual: format!("{} and {}", a, b).into(),
            }),
        }
    }

    /// Evaluate remainder
    fn eval_rem(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        match (left, right) {
            (ConstValue::Int(a), ConstValue::Int(b)) => {
                if b == 0 {
                    return Err(ConstEvalError::DivisionByZero);
                }
                a.checked_rem(b)
                    .map(ConstValue::Int)
                    .ok_or_else(|| ConstEvalError::Overflow {
                        operation: format!("{} % {}", a, b).into(),
                    })
            }
            (ConstValue::UInt(a), ConstValue::UInt(b)) => {
                if b == 0 {
                    return Err(ConstEvalError::DivisionByZero);
                }
                Ok(ConstValue::UInt(a % b))
            }
            (ConstValue::Float(a), ConstValue::Float(b)) => {
                if b == 0.0 {
                    return Err(ConstEvalError::DivisionByZero);
                }
                Ok(ConstValue::Float(a % b))
            }
            (a, b) => Err(ConstEvalError::TypeError {
                expected: Text::from("matching numeric types"),
                actual: format!("{} and {}", a, b).into(),
            }),
        }
    }

    /// Evaluate equality
    fn eval_eq(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        Ok(ConstValue::Bool(left == right))
    }

    /// Evaluate inequality
    fn eval_ne(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        Ok(ConstValue::Bool(left != right))
    }

    /// Evaluate less than
    fn eval_lt(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        match (left, right) {
            (ConstValue::Int(a), ConstValue::Int(b)) => Ok(ConstValue::Bool(a < b)),
            (ConstValue::UInt(a), ConstValue::UInt(b)) => Ok(ConstValue::Bool(a < b)),
            (ConstValue::Float(a), ConstValue::Float(b)) => Ok(ConstValue::Bool(a < b)),
            (ConstValue::Text(a), ConstValue::Text(b)) => Ok(ConstValue::Bool(a < b)),
            (ConstValue::Char(a), ConstValue::Char(b)) => Ok(ConstValue::Bool(a < b)),
            (a, b) => Err(ConstEvalError::TypeError {
                expected: Text::from("comparable types"),
                actual: format!("{} and {}", a, b).into(),
            }),
        }
    }

    /// Evaluate less than or equal
    fn eval_le(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        match (left, right) {
            (ConstValue::Int(a), ConstValue::Int(b)) => Ok(ConstValue::Bool(a <= b)),
            (ConstValue::UInt(a), ConstValue::UInt(b)) => Ok(ConstValue::Bool(a <= b)),
            (ConstValue::Float(a), ConstValue::Float(b)) => Ok(ConstValue::Bool(a <= b)),
            (ConstValue::Text(a), ConstValue::Text(b)) => Ok(ConstValue::Bool(a <= b)),
            (ConstValue::Char(a), ConstValue::Char(b)) => Ok(ConstValue::Bool(a <= b)),
            (a, b) => Err(ConstEvalError::TypeError {
                expected: Text::from("comparable types"),
                actual: format!("{} and {}", a, b).into(),
            }),
        }
    }

    /// Evaluate greater than
    fn eval_gt(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        match (left, right) {
            (ConstValue::Int(a), ConstValue::Int(b)) => Ok(ConstValue::Bool(a > b)),
            (ConstValue::UInt(a), ConstValue::UInt(b)) => Ok(ConstValue::Bool(a > b)),
            (ConstValue::Float(a), ConstValue::Float(b)) => Ok(ConstValue::Bool(a > b)),
            (ConstValue::Text(a), ConstValue::Text(b)) => Ok(ConstValue::Bool(a > b)),
            (ConstValue::Char(a), ConstValue::Char(b)) => Ok(ConstValue::Bool(a > b)),
            (a, b) => Err(ConstEvalError::TypeError {
                expected: Text::from("comparable types"),
                actual: format!("{} and {}", a, b).into(),
            }),
        }
    }

    /// Evaluate greater than or equal
    fn eval_ge(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        match (left, right) {
            (ConstValue::Int(a), ConstValue::Int(b)) => Ok(ConstValue::Bool(a >= b)),
            (ConstValue::UInt(a), ConstValue::UInt(b)) => Ok(ConstValue::Bool(a >= b)),
            (ConstValue::Float(a), ConstValue::Float(b)) => Ok(ConstValue::Bool(a >= b)),
            (ConstValue::Text(a), ConstValue::Text(b)) => Ok(ConstValue::Bool(a >= b)),
            (ConstValue::Char(a), ConstValue::Char(b)) => Ok(ConstValue::Bool(a >= b)),
            (a, b) => Err(ConstEvalError::TypeError {
                expected: Text::from("comparable types"),
                actual: format!("{} and {}", a, b).into(),
            }),
        }
    }

    /// Evaluate logical AND
    fn eval_and(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        match (left, right) {
            (ConstValue::Bool(a), ConstValue::Bool(b)) => Ok(ConstValue::Bool(a && b)),
            (a, b) => Err(ConstEvalError::TypeError {
                expected: Text::from("bool"),
                actual: format!("{} and {}", a, b).into(),
            }),
        }
    }

    /// Evaluate logical OR
    fn eval_or(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        match (left, right) {
            (ConstValue::Bool(a), ConstValue::Bool(b)) => Ok(ConstValue::Bool(a || b)),
            (a, b) => Err(ConstEvalError::TypeError {
                expected: Text::from("bool"),
                actual: format!("{} and {}", a, b).into(),
            }),
        }
    }

    /// Evaluate bitwise AND
    fn eval_bitand(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        match (left, right) {
            (ConstValue::Int(a), ConstValue::Int(b)) => Ok(ConstValue::Int(a & b)),
            (ConstValue::UInt(a), ConstValue::UInt(b)) => Ok(ConstValue::UInt(a & b)),
            (ConstValue::Bool(a), ConstValue::Bool(b)) => Ok(ConstValue::Bool(a & b)),
            (a, b) => Err(ConstEvalError::TypeError {
                expected: Text::from("matching integer types or bool"),
                actual: format!("{} and {}", a, b).into(),
            }),
        }
    }

    /// Evaluate bitwise OR
    fn eval_bitor(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        match (left, right) {
            (ConstValue::Int(a), ConstValue::Int(b)) => Ok(ConstValue::Int(a | b)),
            (ConstValue::UInt(a), ConstValue::UInt(b)) => Ok(ConstValue::UInt(a | b)),
            (ConstValue::Bool(a), ConstValue::Bool(b)) => Ok(ConstValue::Bool(a | b)),
            (a, b) => Err(ConstEvalError::TypeError {
                expected: Text::from("matching integer types or bool"),
                actual: format!("{} and {}", a, b).into(),
            }),
        }
    }

    /// Evaluate bitwise XOR
    fn eval_bitxor(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        match (left, right) {
            (ConstValue::Int(a), ConstValue::Int(b)) => Ok(ConstValue::Int(a ^ b)),
            (ConstValue::UInt(a), ConstValue::UInt(b)) => Ok(ConstValue::UInt(a ^ b)),
            (ConstValue::Bool(a), ConstValue::Bool(b)) => Ok(ConstValue::Bool(a ^ b)),
            (a, b) => Err(ConstEvalError::TypeError {
                expected: Text::from("matching integer types or bool"),
                actual: format!("{} and {}", a, b).into(),
            }),
        }
    }

    /// Evaluate left shift
    fn eval_shl(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        let shift = match &right {
            ConstValue::Int(n) => {
                if *n < 0 {
                    return Err(ConstEvalError::UnsupportedOperation {
                        operation: Text::from("negative shift amount"),
                    });
                }
                *n as u32
            }
            ConstValue::UInt(n) => *n as u32,
            _ => {
                return Err(ConstEvalError::TypeError {
                    expected: Text::from("integer shift amount"),
                    actual: format!("{}", right).into(),
                })
            }
        };

        match left {
            ConstValue::Int(a) => {
                if shift >= 128 {
                    return Err(ConstEvalError::Overflow {
                        operation: format!("{} << {}", a, shift).into(),
                    });
                }
                a.checked_shl(shift)
                    .map(ConstValue::Int)
                    .ok_or_else(|| ConstEvalError::Overflow {
                        operation: format!("{} << {}", a, shift).into(),
                    })
            }
            ConstValue::UInt(a) => {
                if shift >= 128 {
                    return Err(ConstEvalError::Overflow {
                        operation: format!("{} << {}", a, shift).into(),
                    });
                }
                a.checked_shl(shift)
                    .map(ConstValue::UInt)
                    .ok_or_else(|| ConstEvalError::Overflow {
                        operation: format!("{} << {}", a, shift).into(),
                    })
            }
            _ => Err(ConstEvalError::TypeError {
                expected: Text::from("integer type"),
                actual: format!("{}", left).into(),
            }),
        }
    }

    /// Evaluate right shift (arithmetic for signed, logical for unsigned)
    fn eval_shr(&self, left: ConstValue, right: ConstValue) -> Result<ConstValue> {
        let shift = match &right {
            ConstValue::Int(n) => {
                if *n < 0 {
                    return Err(ConstEvalError::UnsupportedOperation {
                        operation: Text::from("negative shift amount"),
                    });
                }
                *n as u32
            }
            ConstValue::UInt(n) => *n as u32,
            _ => {
                return Err(ConstEvalError::TypeError {
                    expected: Text::from("integer shift amount"),
                    actual: format!("{}", right).into(),
                })
            }
        };

        match left {
            ConstValue::Int(a) => {
                if shift >= 128 {
                    // For arithmetic right shift, result is either 0 or -1 depending on sign
                    return Ok(ConstValue::Int(if a < 0 { -1 } else { 0 }));
                }
                a.checked_shr(shift)
                    .map(ConstValue::Int)
                    .ok_or_else(|| ConstEvalError::Overflow {
                        operation: format!("{} >> {}", a, shift).into(),
                    })
            }
            ConstValue::UInt(a) => {
                if shift >= 128 {
                    return Ok(ConstValue::UInt(0));
                }
                a.checked_shr(shift)
                    .map(ConstValue::UInt)
                    .ok_or_else(|| ConstEvalError::Overflow {
                        operation: format!("{} >> {}", a, shift).into(),
                    })
            }
            _ => Err(ConstEvalError::TypeError {
                expected: Text::from("integer type"),
                actual: format!("{}", left).into(),
            }),
        }
    }

    /// Evaluate a unary operation
    fn eval_unary(&self, op: UnOp, val: ConstValue) -> Result<ConstValue> {
        match op {
            UnOp::Neg => {
                match val {
                    ConstValue::Int(n) => n.checked_neg().map(ConstValue::Int).ok_or_else(|| {
                        ConstEvalError::Overflow {
                            operation: format!("-{}", n).into(),
                        }
                    }),
                    ConstValue::Float(f) => Ok(ConstValue::Float(-f)),
                    _ => Err(ConstEvalError::TypeError {
                        expected: Text::from("numeric type"),
                        actual: format!("{}", val).into(),
                    }),
                }
            }
            UnOp::Not => match val {
                ConstValue::Bool(b) => Ok(ConstValue::Bool(!b)),
                _ => Err(ConstEvalError::TypeError {
                    expected: Text::from("bool"),
                    actual: format!("{}", val).into(),
                }),
            },
            _ => Err(ConstEvalError::UnsupportedOperation {
                operation: format!("{}", op).into(),
            }),
        }
    }

    /// Evaluate a meta type, substituting computed values
    ///
    /// This resolves meta parameters in types by evaluating their expressions
    /// and substituting the results.
    pub fn eval_meta_type(&mut self, ty: &Type) -> Result<Type> {
        match ty {
            Type::Meta {
                name,
                ty,
                refinement,
                value,
            } => {
                // For now, we just return the meta type as-is
                // Full substitution would require expression in Type::Meta
                Ok(Type::Meta {
                    name: name.clone(),
                    ty: Box::new(self.eval_meta_type(ty)?),
                    refinement: refinement.clone(),
                    value: value.clone(),
                })
            }
            Type::Array { element, size } => Ok(Type::Array {
                element: Box::new(self.eval_meta_type(element)?),
                size: *size,
            }),
            Type::Named { path, args } => {
                let mut new_args = List::with_capacity(args.len());
                for arg in args {
                    new_args.push(self.eval_meta_type(arg)?);
                }
                Ok(Type::Named {
                    path: path.clone(),
                    args: new_args,
                })
            }
            // Other type variants pass through unchanged
            _ => Ok(ty.clone()),
        }
    }

    /// Substitute meta parameters in a type with their computed values
    pub fn substitute_meta(&mut self, ty: &Type, env: &Map<Text, ConstValue>) -> Result<Type> {
        // Save current environment
        let old_env = self.env.clone();

        // Extend with new bindings
        for (name, value) in env.iter() {
            self.env.insert(name.clone(), value.clone());
        }

        // Evaluate the type
        let result = self.eval_meta_type(ty);

        // Restore environment
        self.env = old_env;

        result
    }

    /// Compute tensor shape dimensions from a meta array expression
    ///
    /// This evaluates an array expression to extract shape dimensions for tensor types.
    /// For example, `[2, 3, 4]` evaluates to dimensions `[2, 3, 4]` for a 3D tensor.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use verum_types::const_eval::ConstEvaluator;
    /// use verum_ast::{expr::{Expr, ExprKind, ArrayExpr}, span::Span, literal::Literal};
    /// use verum_common::List;
    ///
    /// let mut eval = ConstEvaluator::new();
    ///
    /// // Shape: [2, 3, 4]
    /// let elements: List<_> = vec![
    ///     Expr::new(ExprKind::Literal(Literal::int(2, Span::dummy())), Span::dummy()),
    ///     Expr::new(ExprKind::Literal(Literal::int(3, Span::dummy())), Span::dummy()),
    ///     Expr::new(ExprKind::Literal(Literal::int(4, Span::dummy())), Span::dummy()),
    /// ].into();
    /// let shape_expr = Expr::new(
    ///     ExprKind::Array(ArrayExpr::List(elements)),
    ///     Span::dummy()
    /// );
    /// let dims = eval.compute_tensor_shape(&shape_expr)?;
    /// assert_eq!(dims, List::from(vec![2, 3, 4]));
    /// ```
    pub fn compute_tensor_shape(&mut self, shape_expr: &Expr) -> Result<List<usize>> {
        let shape_value = self.eval(shape_expr)?;

        match shape_value {
            ConstValue::Array(dims) => {
                let mut result = List::with_capacity(dims.len());
                for dim in dims {
                    let size = dim.as_u128().ok_or_else(|| ConstEvalError::TypeError {
                        expected: Text::from("usize"),
                        actual: format!("{}", dim).into(),
                    })? as usize;
                    result.push(size);
                }
                Ok(result)
            }
            _ => Err(ConstEvalError::TypeError {
                expected: Text::from("array"),
                actual: format!("{}", shape_value).into(),
            }),
        }
    }

    /// Compute total number of elements from tensor shape
    ///
    /// Given a shape array like `[2, 3, 4]`, computes the product `2 * 3 * 4 = 24`.
    /// This is useful for validating tensor data sizes.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use verum_types::const_eval::ConstEvaluator;
    /// use verum_ast::{expr::{Expr, ExprKind, ArrayExpr}, span::Span, literal::Literal};
    /// use verum_common::List;
    ///
    /// let mut eval = ConstEvaluator::new();
    ///
    /// // Shape: [2, 3, 4]
    /// let elements: List<_> = vec![
    ///     Expr::new(ExprKind::Literal(Literal::int(2, Span::dummy())), Span::dummy()),
    ///     Expr::new(ExprKind::Literal(Literal::int(3, Span::dummy())), Span::dummy()),
    ///     Expr::new(ExprKind::Literal(Literal::int(4, Span::dummy())), Span::dummy()),
    /// ].into();
    /// let shape_expr = Expr::new(
    ///     ExprKind::Array(ArrayExpr::List(elements)),
    ///     Span::dummy()
    /// );
    /// let total = eval.compute_tensor_elements(&shape_expr)?;
    /// assert_eq!(total, 24);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn compute_tensor_elements(&mut self, shape_expr: &Expr) -> Result<usize> {
        let dims = self.compute_tensor_shape(shape_expr)?;

        if dims.is_empty() {
            return Ok(0);
        }

        let mut total = 1usize;
        for dim in &dims {
            total = total
                .checked_mul(*dim)
                .ok_or_else(|| ConstEvalError::Overflow {
                    operation: format!(
                        "tensor shape product: [{}]",
                        dims.iter()
                            .map(|d| d.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                    .into(),
                })?;
        }

        Ok(total)
    }

    /// Validate that two tensor shapes are compatible for operations
    ///
    /// This checks if two shapes can be used together in tensor operations.
    /// For now, it requires exact shape matches. Future enhancements will support
    /// broadcasting rules.
    pub fn validate_tensor_shapes(&mut self, shape1: &Expr, shape2: &Expr) -> Result<bool> {
        let dims1 = self.compute_tensor_shape(shape1)?;
        let dims2 = self.compute_tensor_shape(shape2)?;

        // Spec: Meta system - Broadcasting rules for tensor operations
        // Broadcasting compatibility check (NumPy-like rules):
        // Two shapes are compatible if:
        // 1. They are exactly equal
        // 2. One is empty (scalar broadcast to any shape)
        // 3. Trailing dimensions match, or one is 1 (can be broadcast)
        Ok(self.check_broadcast_compatible(dims1.as_slice(), dims2.as_slice()))
    }

    /// Check if two tensor shapes are broadcast-compatible
    /// Spec: Tensor broadcasting - dimension compatibility
    fn check_broadcast_compatible(&self, shape1: &[usize], shape2: &[usize]) -> bool {
        // Same shape - trivially compatible
        if shape1 == shape2 {
            return true;
        }

        // One is empty - scalars are broadcast-compatible with anything
        if shape1.is_empty() || shape2.is_empty() {
            return true;
        }

        // Check dimension-by-dimension from the end
        // Shape: [a, b, c] can broadcast with [x, y, z] if:
        // - c == z or c == 1 or z == 1
        // - b == y or b == 1 or y == 1
        // - a == x or a == 1 or x == 1

        let max_rank = shape1.len().max(shape2.len());

        // Pad shorter shape with 1s at the beginning
        let mut s1 = vec![1; max_rank - shape1.len()];
        s1.extend_from_slice(shape1);

        let mut s2 = vec![1; max_rank - shape2.len()];
        s2.extend_from_slice(shape2);

        // Check each dimension
        for i in 0..max_rank {
            let d1 = s1[i];
            let d2 = s2[i];

            // Dimensions must match or one must be 1
            if d1 != d2 && d1 != 1 && d2 != 1 {
                return false;
            }
        }

        true
    }
}

impl Default for ConstEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smallvec::smallvec;
    use verum_ast::{
        expr::{Expr, ExprKind, IfCondition},
        literal::Literal,
        span::Span,
        ty::{Ident, Path},
    };

    /// Helper to create a simple integer literal expression
    fn int_lit(value: i128) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal::int(value, Span::dummy())),
            Span::dummy(),
        )
    }

    /// Helper to create a boolean literal expression
    fn bool_lit(value: bool) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal::bool(value, Span::dummy())),
            Span::dummy(),
        )
    }

    /// Helper to create a path/variable expression
    fn var_expr(name: &str) -> Expr {
        let ident = Ident::new(Text::from(name), Span::dummy());
        Expr::new(
            ExprKind::Path(Path::single(ident)),
            Span::dummy(),
        )
    }

    /// Helper to create a binary expression
    fn binary(op: BinOp, left: Expr, right: Expr) -> Expr {
        Expr::new(
            ExprKind::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
            Span::dummy(),
        )
    }

    /// Helper to create a function call expression
    fn call_expr(func_name: &str, args: Vec<Expr>) -> Expr {
        Expr::new(
            ExprKind::Call {
                func: Box::new(var_expr(func_name)),
                type_args: List::new(),
                args: List::from_iter(args),
            },
            Span::dummy(),
        )
    }

    /// Helper to create an if expression
    fn if_expr(condition: Expr, then_branch: Block, else_branch: Option<Expr>) -> Expr {
        Expr::new(
            ExprKind::If {
                condition: Box::new(IfCondition {
                    conditions: smallvec![ConditionKind::Expr(condition)],
                    span: Span::dummy(),
                }),
                then_branch,
                else_branch: else_branch.map(Box::new),
            },
            Span::dummy(),
        )
    }

    /// Helper to create a block with a final expression
    fn block_expr(expr: Expr) -> Block {
        Block {
            stmts: List::new(),
            expr: Maybe::Some(Box::new(expr)),
            span: Span::dummy(),
        }
    }

    /// Helper to create a MetaFunction manually
    fn meta_fn(name: &str, params: &[&str], body_expr: Expr) -> MetaFunction {
        MetaFunction {
            name: Text::from(name),
            param_names: List::from_iter(params.iter().map(|s| Text::from(*s))),
            body: block_expr(body_expr),
            is_meta: true,
        }
    }

    // =========================================================================
    // Function Registration Tests
    // =========================================================================

    #[test]
    fn test_register_meta_function() {
        let mut eval = ConstEvaluator::new();

        // Create a simple meta function: meta fn double(x) = x + x
        let double_fn = meta_fn(
            "double",
            &["x"],
            binary(BinOp::Add, var_expr("x"), var_expr("x")),
        );

        eval.register_meta_function(double_fn);
        assert!(eval.has_function("double"));
        assert!(!eval.has_function("nonexistent"));
    }

    #[test]
    fn test_get_registered_function() {
        let mut eval = ConstEvaluator::new();

        let add_fn = meta_fn(
            "add",
            &["a", "b"],
            binary(BinOp::Add, var_expr("a"), var_expr("b")),
        );

        eval.register_meta_function(add_fn);

        let func = eval.get_function("add").expect("function should exist");
        assert_eq!(func.name.as_str(), "add");
        assert_eq!(func.arity(), 2);
    }

    // =========================================================================
    // Function Call Tests
    // =========================================================================

    #[test]
    fn test_call_simple_meta_function() {
        let mut eval = ConstEvaluator::new();

        // meta fn double(x) = x + x
        let double_fn = meta_fn(
            "double",
            &["x"],
            binary(BinOp::Add, var_expr("x"), var_expr("x")),
        );
        eval.register_meta_function(double_fn);

        // Call: double(5)
        let call = call_expr("double", vec![int_lit(5)]);
        let result = eval.eval(&call).expect("eval should succeed");

        assert_eq!(result, ConstValue::Int(10));
    }

    #[test]
    fn test_call_two_param_function() {
        let mut eval = ConstEvaluator::new();

        // meta fn add(a, b) = a + b
        let add_fn = meta_fn(
            "add",
            &["a", "b"],
            binary(BinOp::Add, var_expr("a"), var_expr("b")),
        );
        eval.register_meta_function(add_fn);

        // Call: add(3, 7)
        let call = call_expr("add", vec![int_lit(3), int_lit(7)]);
        let result = eval.eval(&call).expect("eval should succeed");

        assert_eq!(result, ConstValue::Int(10));
    }

    #[test]
    fn test_nested_function_calls() {
        let mut eval = ConstEvaluator::new();

        // meta fn double(x) = x + x
        let double_fn = meta_fn(
            "double",
            &["x"],
            binary(BinOp::Add, var_expr("x"), var_expr("x")),
        );
        eval.register_meta_function(double_fn);

        // meta fn quad(x) = double(double(x))
        let quad_fn = meta_fn(
            "quad",
            &["x"],
            call_expr("double", vec![call_expr("double", vec![var_expr("x")])]),
        );
        eval.register_meta_function(quad_fn);

        // Call: quad(3) = double(double(3)) = double(6) = 12
        let call = call_expr("quad", vec![int_lit(3)]);
        let result = eval.eval(&call).expect("eval should succeed");

        assert_eq!(result, ConstValue::Int(12));
    }

    #[test]
    fn test_undefined_function_error() {
        let mut eval = ConstEvaluator::new();

        let call = call_expr("nonexistent", vec![int_lit(1)]);
        let result = eval.eval(&call);

        assert!(matches!(
            result,
            Err(ConstEvalError::UndefinedFunction { .. })
        ));
    }

    #[test]
    fn test_arity_mismatch_error() {
        let mut eval = ConstEvaluator::new();

        // meta fn double(x) = x + x
        let double_fn = meta_fn(
            "double",
            &["x"],
            binary(BinOp::Add, var_expr("x"), var_expr("x")),
        );
        eval.register_meta_function(double_fn);

        // Call with wrong arity: double(1, 2)
        let call = call_expr("double", vec![int_lit(1), int_lit(2)]);
        let result = eval.eval(&call);

        assert!(matches!(result, Err(ConstEvalError::ArityMismatch { .. })));
    }

    #[test]
    fn test_non_meta_function_error() {
        let mut eval = ConstEvaluator::new();

        // Regular function (not meta)
        let regular_fn = MetaFunction {
            name: Text::from("regular"),
            param_names: List::from_iter([Text::from("x")]),
            body: block_expr(var_expr("x")),
            is_meta: false, // Not a meta function
        };
        eval.register_meta_function(regular_fn);

        let call = call_expr("regular", vec![int_lit(1)]);
        let result = eval.eval(&call);

        assert!(matches!(
            result,
            Err(ConstEvalError::NonMetaFunction { .. })
        ));
    }

    // =========================================================================
    // Recursive Function Tests
    // =========================================================================

    #[test]
    fn test_recursive_factorial() {
        let mut eval = ConstEvaluator::new();

        // meta fn factorial(n) = if n <= 1 { 1 } else { n * factorial(n - 1) }
        let factorial_body = if_expr(
            binary(BinOp::Le, var_expr("n"), int_lit(1)),
            block_expr(int_lit(1)),
            Some(binary(
                BinOp::Mul,
                var_expr("n"),
                call_expr(
                    "factorial",
                    vec![binary(BinOp::Sub, var_expr("n"), int_lit(1))],
                ),
            )),
        );

        let factorial_fn = meta_fn("factorial", &["n"], factorial_body);
        eval.register_meta_function(factorial_fn);

        // factorial(5) = 120
        let call = call_expr("factorial", vec![int_lit(5)]);
        let result = eval.eval(&call).expect("eval should succeed");

        assert_eq!(result, ConstValue::Int(120));
    }

    #[test]
    fn test_recursive_fibonacci() {
        let mut eval = ConstEvaluator::new();

        // meta fn fib(n) = if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
        let fib_body = if_expr(
            binary(BinOp::Le, var_expr("n"), int_lit(1)),
            block_expr(var_expr("n")),
            Some(binary(
                BinOp::Add,
                call_expr("fib", vec![binary(BinOp::Sub, var_expr("n"), int_lit(1))]),
                call_expr("fib", vec![binary(BinOp::Sub, var_expr("n"), int_lit(2))]),
            )),
        );

        let fib_fn = meta_fn("fib", &["n"], fib_body);
        eval.register_meta_function(fib_fn);

        // fib(10) = 55
        let call = call_expr("fib", vec![int_lit(10)]);
        let result = eval.eval(&call).expect("eval should succeed");

        assert_eq!(result, ConstValue::Int(55));
    }

    // =========================================================================
    // If Expression Tests
    // =========================================================================

    #[test]
    fn test_if_true_branch() {
        let mut eval = ConstEvaluator::new();

        // if true { 1 } else { 2 }
        let expr = if_expr(bool_lit(true), block_expr(int_lit(1)), Some(int_lit(2)));

        let result = eval.eval(&expr).expect("eval should succeed");
        assert_eq!(result, ConstValue::Int(1));
    }

    #[test]
    fn test_if_false_branch() {
        let mut eval = ConstEvaluator::new();

        // if false { 1 } else { 2 }
        let expr = if_expr(bool_lit(false), block_expr(int_lit(1)), Some(int_lit(2)));

        let result = eval.eval(&expr).expect("eval should succeed");
        assert_eq!(result, ConstValue::Int(2));
    }

    #[test]
    fn test_if_no_else_true() {
        let mut eval = ConstEvaluator::new();

        // if true { 42 }
        let expr = if_expr(bool_lit(true), block_expr(int_lit(42)), None);

        let result = eval.eval(&expr).expect("eval should succeed");
        assert_eq!(result, ConstValue::Int(42));
    }

    #[test]
    fn test_if_no_else_false() {
        let mut eval = ConstEvaluator::new();

        // if false { 42 }
        let expr = if_expr(bool_lit(false), block_expr(int_lit(42)), None);

        let result = eval.eval(&expr).expect("eval should succeed");
        // Returns unit when no else branch and condition is false
        assert_eq!(result, ConstValue::Unit);
    }

    // =========================================================================
    // Block Expression Tests
    // =========================================================================

    #[test]
    fn test_simple_block() {
        let mut eval = ConstEvaluator::new();

        // { 42 }
        let block = Block {
            stmts: List::new(),
            expr: Maybe::Some(Box::new(int_lit(42))),
            span: Span::dummy(),
        };

        let result = eval.eval_block(&block).expect("eval should succeed");
        assert_eq!(result, ConstValue::Int(42));
    }

    #[test]
    fn test_block_with_let() {
        let mut eval = ConstEvaluator::new();

        // { let x = 5; x + 3 }
        let block = Block {
            stmts: List::from_iter([Stmt::new(
                StmtKind::Let {
                    pattern: Pattern::ident(Ident::new(Text::from("x"), Span::dummy()), false, Span::dummy()),
                    ty: Maybe::None,
                    value: Maybe::Some(int_lit(5)),
                },
                Span::dummy(),
            )]),
            expr: Maybe::Some(Box::new(binary(BinOp::Add, var_expr("x"), int_lit(3)))),
            span: Span::dummy(),
        };

        let result = eval.eval_block(&block).expect("eval should succeed");
        assert_eq!(result, ConstValue::Int(8));
    }

    #[test]
    fn test_block_variable_scoping() {
        let mut eval = ConstEvaluator::new();

        // Outer binding
        eval.bind("x", ConstValue::Int(100));

        // { let x = 5; x }
        let block = Block {
            stmts: List::from_iter([Stmt::new(
                StmtKind::Let {
                    pattern: Pattern::ident(Ident::new(Text::from("x"), Span::dummy()), false, Span::dummy()),
                    ty: Maybe::None,
                    value: Maybe::Some(int_lit(5)),
                },
                Span::dummy(),
            )]),
            expr: Maybe::Some(Box::new(var_expr("x"))),
            span: Span::dummy(),
        };

        let result = eval.eval_block(&block).expect("eval should succeed");
        assert_eq!(result, ConstValue::Int(5)); // Inner binding

        // Outer binding should be restored
        let outer = eval.eval(&var_expr("x")).expect("outer x should exist");
        assert_eq!(outer, ConstValue::Int(100));
    }

    // =========================================================================
    // Parameter Isolation Tests
    // =========================================================================

    #[test]
    fn test_function_parameters_isolated() {
        let mut eval = ConstEvaluator::new();

        // Set outer variable
        eval.bind("x", ConstValue::Int(999));

        // meta fn identity(x) = x
        let identity_fn = meta_fn("identity", &["x"], var_expr("x"));
        eval.register_meta_function(identity_fn);

        // Call identity(42)
        let call = call_expr("identity", vec![int_lit(42)]);
        let result = eval.eval(&call).expect("eval should succeed");

        // Should return 42, not 999
        assert_eq!(result, ConstValue::Int(42));

        // Outer x should be unchanged
        let outer = eval.eval(&var_expr("x")).expect("outer x should exist");
        assert_eq!(outer, ConstValue::Int(999));
    }

    // =========================================================================
    // Edge Cases
    // =========================================================================

    #[test]
    fn test_zero_argument_function() {
        let mut eval = ConstEvaluator::new();

        // meta fn answer() = 42
        let answer_fn = meta_fn("answer", &[], int_lit(42));
        eval.register_meta_function(answer_fn);

        let call = call_expr("answer", vec![]);
        let result = eval.eval(&call).expect("eval should succeed");

        assert_eq!(result, ConstValue::Int(42));
    }

    #[test]
    fn test_function_with_complex_expression() {
        let mut eval = ConstEvaluator::new();

        // meta fn compute(a, b, c) = (a + b) * c
        let compute_fn = meta_fn(
            "compute",
            &["a", "b", "c"],
            binary(
                BinOp::Mul,
                binary(BinOp::Add, var_expr("a"), var_expr("b")),
                var_expr("c"),
            ),
        );
        eval.register_meta_function(compute_fn);

        // compute(2, 3, 4) = (2 + 3) * 4 = 20
        let call = call_expr("compute", vec![int_lit(2), int_lit(3), int_lit(4)]);
        let result = eval.eval(&call).expect("eval should succeed");

        assert_eq!(result, ConstValue::Int(20));
    }
}
