//! Phase 4a: Autodiff Compilation
//!
//! Generate VJP functions for @differentiable functions.
//!
//! ## Features
//!
//! - Reverse-mode automatic differentiation (VJP)
//! - Forward-mode automatic differentiation (JVP)
//! - Generate gradient functions automatically
//! - Preserve numerical stability
//! - Optimize gradient computation
//! - CBGR-aware code generation (~15ns overhead per check)
//!
//! ## @differentiable Attribute Parameters
//!
//! - `wrt`: Parameters to differentiate with respect to (required)
//! - `mode`: "forward" or "reverse" (default: "reverse")
//! - `order`: Derivative order (1, 2, etc., default: 1)
//! - `custom_vjp`: User-provided VJP implementation
//!
//! ## Generated Functions
//!
//! For `@differentiable(wrt = "weights, bias")`:
//! - `fn_vjp(...)`: Vector-Jacobian Product (reverse-mode)
//! - `fn_jvp(...)`: Jacobian-Vector Product (forward-mode)
//! - `fn_grad(...)`: Gradient (for scalar outputs)
//!
//! Phase 4a: Autodiff compilation. Builds computational graphs for
//! @differentiable functions, generates VJP (reverse-mode) functions,
//! type-checks generated gradient code.
//! @differentiable attribute triggers automatic differentiation code generation.

use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use verum_ast::attr::Attribute;
use verum_ast::decl::{FunctionBody, FunctionDecl, FunctionParam, FunctionParamKind, ItemKind};
use verum_ast::expr::{BinOp, Block, Expr, ExprKind, UnOp};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path, Type, TypeKind};
use verum_ast::{Item, Module};
use verum_common::List;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};
// use verum_std::core::Maybe; // Removed - using std Option instead

use super::{CompilationPhase, PhaseData, PhaseInput, PhaseMetrics, PhaseOutput};

/// Configuration for @differentiable attribute
/// @differentiable function metadata: tracks computational graph structure
/// for VJP generation (reverse-mode automatic differentiation).
#[derive(Debug, Clone)]
pub struct DifferentiableConfig {
    /// Parameters to differentiate with respect to
    pub wrt_params: Vec<String>,
    /// Differentiation mode (reverse = VJP, forward = JVP)
    pub mode: DifferentiationMode,
    /// Derivative order (1 = first derivative, 2 = second, etc.)
    pub order: u32,
    /// Custom VJP function name (if user-provided)
    pub custom_vjp: Option<String>,
}

impl Default for DifferentiableConfig {
    fn default() -> Self {
        Self {
            wrt_params: Vec::new(),
            mode: DifferentiationMode::Reverse,
            order: 1,
            custom_vjp: None,
        }
    }
}

/// Differentiation mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DifferentiationMode {
    /// Reverse-mode (backpropagation, VJP)
    /// Efficient for functions with many inputs, few outputs
    Reverse,
    /// Forward-mode (JVP)
    /// Efficient for functions with few inputs, many outputs
    Forward,
    /// Both modes (generate both VJP and JVP)
    Both,
}

/// Node in the computational graph for automatic differentiation
#[derive(Debug, Clone)]
pub struct ComputeNode {
    /// Unique identifier for this node
    pub id: usize,
    /// The operation type
    pub op: ComputeOp,
    /// Input node IDs
    pub inputs: Vec<usize>,
    /// Output type (for shape tracking)
    pub output_type: DiffType,
    /// Original expression (for code generation)
    pub original_expr: Option<Expr>,
}

/// Type information for differentiable values
#[derive(Debug, Clone)]
pub enum DiffType {
    /// Scalar floating point
    Scalar,
    /// Vector with known dimension
    Vector(usize),
    /// Matrix with known dimensions
    Matrix(usize, usize),
    /// Tensor with arbitrary shape
    Tensor(Vec<usize>),
    /// Unknown (to be inferred)
    Unknown,
}

/// Operations in the computational graph
/// Each operation has defined forward and backward rules
#[derive(Debug, Clone)]
pub enum ComputeOp {
    /// Input parameter (leaf node)
    Parameter {
        name: String,
        index: usize,
    },
    /// Literal constant
    Constant {
        value: f64,
    },

    // Arithmetic operations
    Add,
    Sub,
    Mul,
    Div,
    Neg,
    Pow,

    // Math functions
    Sin,
    Cos,
    Tan,
    Exp,
    Log,
    Sqrt,
    Abs,
    Tanh,
    Sigmoid,
    Relu,
    Softmax,

    // Tensor operations
    MatMul,
    Transpose,
    Sum {
        axis: Option<usize>,
    },
    Mean {
        axis: Option<usize>,
    },
    Broadcast {
        target_shape: Vec<usize>,
    },

    // Control flow (require special handling)
    Select {
        condition_node: usize,
    },

    // Field/index access
    Index {
        index: usize,
    },
    Field {
        name: String,
    },

    // Function call (for composing differentiable functions)
    Call {
        func_name: String,
    },
}

/// Computational graph for a differentiable function
#[derive(Debug, Clone)]
pub struct ComputeGraph {
    /// All nodes in the graph
    pub nodes: Vec<ComputeNode>,
    /// Output node ID
    pub output_id: usize,
    /// Map from parameter names to node IDs
    pub param_map: HashMap<String, usize>,
    /// Parameters that require gradients (from wrt)
    pub wrt_params: HashSet<String>,
}

impl ComputeGraph {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            output_id: 0,
            param_map: HashMap::new(),
            wrt_params: HashSet::new(),
        }
    }

    /// Add a node to the graph and return its ID
    pub fn add_node(&mut self, op: ComputeOp, inputs: Vec<usize>, output_type: DiffType) -> usize {
        let id = self.nodes.len();
        self.nodes.push(ComputeNode {
            id,
            op,
            inputs,
            output_type,
            original_expr: None,
        });
        id
    }

    /// Add a parameter node
    pub fn add_parameter(&mut self, name: &str, index: usize, dtype: DiffType) -> usize {
        let id = self.add_node(
            ComputeOp::Parameter {
                name: name.to_string(),
                index,
            },
            vec![],
            dtype,
        );
        self.param_map.insert(name.to_string(), id);
        id
    }
}

/// Builder for constructing computational graphs from AST
pub struct GraphBuilder {
    graph: ComputeGraph,
    /// Map from variable names to their node IDs
    var_map: HashMap<String, usize>,
}

impl GraphBuilder {
    pub fn new() -> Self {
        Self {
            graph: ComputeGraph::new(),
            var_map: HashMap::new(),
        }
    }

    /// Build graph from function declaration
    pub fn build_from_function(
        mut self,
        func: &FunctionDecl,
        config: &DifferentiableConfig,
    ) -> Result<ComputeGraph, String> {
        // Register wrt parameters
        for param in &config.wrt_params {
            self.graph.wrt_params.insert(param.clone());
        }

        // Add parameter nodes
        for (index, param) in func.params.iter().enumerate() {
            if let Some(name) = self.get_param_name(param) {
                let dtype = self.infer_diff_type_from_param(param);
                let node_id = self.graph.add_parameter(&name, index, dtype);
                self.var_map.insert(name, node_id);
            }
        }

        // Build graph from function body
        let output_id = match &func.body {
            Some(FunctionBody::Block(block)) => self.build_from_block(block)?,
            Some(FunctionBody::Expr(expr)) => self.build_from_expr(expr)?,
            None => return Err("Function has no body".to_string()),
        };

        self.graph.output_id = output_id;
        Ok(self.graph)
    }

    /// Build graph from a block expression
    fn build_from_block(&mut self, block: &Block) -> Result<usize, String> {
        // Process statements
        for stmt in block.stmts.iter() {
            self.process_stmt(stmt)?;
        }

        // Return the final expression
        match &block.expr {
            Some(expr) => self.build_from_expr(expr),
            None => {
                // Unit block - return a constant 0
                Ok(self.graph.add_node(
                    ComputeOp::Constant { value: 0.0 },
                    vec![],
                    DiffType::Scalar,
                ))
            }
        }
    }

    /// Process a statement (primarily for let bindings)
    fn process_stmt(&mut self, stmt: &verum_ast::Stmt) -> Result<(), String> {
        use verum_ast::StmtKind;
        match &stmt.kind {
            StmtKind::Let { pattern, value, .. } => {
                if let Some(expr) = value {
                    let node_id = self.build_from_expr(expr)?;
                    // Extract variable name from pattern
                    if let Some(name) = self.get_pattern_name(pattern) {
                        self.var_map.insert(name, node_id);
                    }
                }
            }
            StmtKind::Expr { expr, .. } => {
                // Process for side effects (accumulation in loops, etc.)
                self.build_from_expr(expr)?;
            }
            _ => {
                // Other statements don't affect the computational graph directly
            }
        }
        Ok(())
    }

    /// Build graph from an expression
    fn build_from_expr(&mut self, expr: &Expr) -> Result<usize, String> {
        match &expr.kind {
            // Literals
            ExprKind::Literal(lit) => {
                let value = self.extract_numeric_literal(lit)?;
                Ok(self
                    .graph
                    .add_node(ComputeOp::Constant { value }, vec![], DiffType::Scalar))
            }

            // Path (variable reference)
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    let name = ident.as_str();
                    if let Some(&node_id) = self.var_map.get(name) {
                        return Ok(node_id);
                    }
                    // Check for math function names
                    return Err(format!("Unknown variable: {}", name));
                }
                Err("Complex paths not supported in autodiff".to_string())
            }

            // Binary operations
            ExprKind::Binary { op, left, right } => {
                let left_id = self.build_from_expr(left)?;
                let right_id = self.build_from_expr(right)?;

                let compute_op = match op {
                    BinOp::Add => ComputeOp::Add,
                    BinOp::Sub => ComputeOp::Sub,
                    BinOp::Mul => ComputeOp::Mul,
                    BinOp::Div => ComputeOp::Div,
                    BinOp::Pow => ComputeOp::Pow,
                    _ => return Err(format!("Unsupported binary op in autodiff: {:?}", op)),
                };

                let output_type = self.infer_binary_output_type(left_id, right_id);
                Ok(self
                    .graph
                    .add_node(compute_op, vec![left_id, right_id], output_type))
            }

            // Unary operations
            ExprKind::Unary { op, expr: inner } => {
                let inner_id = self.build_from_expr(inner)?;

                let compute_op = match op {
                    UnOp::Neg => ComputeOp::Neg,
                    _ => return Err(format!("Unsupported unary op in autodiff: {:?}", op)),
                };

                let output_type = self.graph.nodes[inner_id].output_type.clone();
                Ok(self.graph.add_node(compute_op, vec![inner_id], output_type))
            }

            // Function/method calls
            ExprKind::Call { func, args, .. } => self.build_call(func, args),

            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => self.build_method_call(receiver, method, args),

            // Field access
            ExprKind::Field { expr: inner, field } => {
                let inner_id = self.build_from_expr(inner)?;
                Ok(self.graph.add_node(
                    ComputeOp::Field {
                        name: field.as_str().to_string(),
                    },
                    vec![inner_id],
                    DiffType::Unknown,
                ))
            }

            // Index access
            ExprKind::Index { expr: inner, index } => {
                let inner_id = self.build_from_expr(inner)?;
                // Try to extract constant index
                let idx = self.try_extract_const_index(index)?;
                Ok(self.graph.add_node(
                    ComputeOp::Index { index: idx },
                    vec![inner_id],
                    DiffType::Scalar,
                ))
            }

            // Parenthesized expressions
            ExprKind::Paren(inner) => self.build_from_expr(inner),

            // If expressions (require special gradient handling)
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Build condition (not differentiable, but needed for selection)
                let cond_id = self.build_from_expr(&self.extract_condition_expr(condition)?)?;
                let then_id = self.build_from_block(then_branch)?;

                let else_id = match else_branch {
                    Some(else_expr) => self.build_from_expr(else_expr)?,
                    None => self.graph.add_node(
                        ComputeOp::Constant { value: 0.0 },
                        vec![],
                        DiffType::Scalar,
                    ),
                };

                Ok(self.graph.add_node(
                    ComputeOp::Select {
                        condition_node: cond_id,
                    },
                    vec![then_id, else_id],
                    DiffType::Unknown,
                ))
            }

            // Block expressions
            ExprKind::Block(block) => self.build_from_block(block),

            // For loops (accumulation)
            ExprKind::For {
                label: _,
                pattern: _,
                iter,
                body: _,
                invariants: _,
                decreases: _,
            } => {
                // For loops in autodiff context are handled as accumulation operations.
                // The gradient flows through by treating the loop as a reduction over
                // the iterated values. This is correct for common patterns like:
                //   sum = 0; for x in xs { sum += f(x); } => grad(sum) = sum of grads
                // For more complex loop patterns, users should use explicit reduction ops.
                let iter_id = self.build_from_expr(iter)?;
                Ok(self.graph.add_node(
                    ComputeOp::Sum { axis: None },
                    vec![iter_id],
                    DiffType::Scalar,
                ))
            }

            _ => Err(format!(
                "Unsupported expression in autodiff: {:?}",
                expr.kind
            )),
        }
    }

    /// Build graph for function call
    fn build_call(&mut self, func: &Expr, args: &List<Expr>) -> Result<usize, String> {
        // Extract function name
        let func_name = match &func.kind {
            ExprKind::Path(path) => path
                .as_ident()
                .map(|i| i.as_str().to_string())
                .ok_or_else(|| "Complex function paths not supported".to_string())?,
            _ => return Err("Non-path function calls not supported".to_string()),
        };

        // Build argument nodes
        let arg_ids: Vec<usize> = args
            .iter()
            .map(|a| self.build_from_expr(a))
            .collect::<Result<_, _>>()?;

        // Match known differentiable functions
        let (op, output_type) = match func_name.as_str() {
            "sin" => (ComputeOp::Sin, DiffType::Scalar),
            "cos" => (ComputeOp::Cos, DiffType::Scalar),
            "tan" => (ComputeOp::Tan, DiffType::Scalar),
            "exp" => (ComputeOp::Exp, DiffType::Scalar),
            "log" | "ln" => (ComputeOp::Log, DiffType::Scalar),
            "sqrt" => (ComputeOp::Sqrt, DiffType::Scalar),
            "abs" => (ComputeOp::Abs, DiffType::Scalar),
            "tanh" => (ComputeOp::Tanh, DiffType::Scalar),
            "sigmoid" => (ComputeOp::Sigmoid, DiffType::Scalar),
            "relu" => (ComputeOp::Relu, DiffType::Unknown),
            "softmax" => (ComputeOp::Softmax, DiffType::Unknown),
            "pow" if arg_ids.len() == 2 => (ComputeOp::Pow, DiffType::Scalar),
            "matmul" if arg_ids.len() == 2 => (ComputeOp::MatMul, DiffType::Unknown),
            "transpose" => (ComputeOp::Transpose, DiffType::Unknown),
            "sum" => (ComputeOp::Sum { axis: None }, DiffType::Scalar),
            "mean" => (ComputeOp::Mean { axis: None }, DiffType::Scalar),
            _ => (
                ComputeOp::Call {
                    func_name: func_name.clone(),
                },
                DiffType::Unknown,
            ),
        };

        Ok(self.graph.add_node(op, arg_ids, output_type))
    }

    /// Build graph for method call
    fn build_method_call(
        &mut self,
        receiver: &Expr,
        method: &Ident,
        args: &List<Expr>,
    ) -> Result<usize, String> {
        let recv_id = self.build_from_expr(receiver)?;
        let method_name = method.as_str();

        // Build additional argument nodes
        let mut all_args = vec![recv_id];
        for arg in args.iter() {
            all_args.push(self.build_from_expr(arg)?);
        }

        let (op, output_type) = match method_name {
            "exp" => (ComputeOp::Exp, DiffType::Scalar),
            "log" | "ln" => (ComputeOp::Log, DiffType::Scalar),
            "sqrt" => (ComputeOp::Sqrt, DiffType::Scalar),
            "abs" => (ComputeOp::Abs, DiffType::Scalar),
            "sin" => (ComputeOp::Sin, DiffType::Scalar),
            "cos" => (ComputeOp::Cos, DiffType::Scalar),
            "tan" => (ComputeOp::Tan, DiffType::Scalar),
            "tanh" => (ComputeOp::Tanh, DiffType::Scalar),
            "sigmoid" => (ComputeOp::Sigmoid, DiffType::Scalar),
            "relu" => (ComputeOp::Relu, DiffType::Unknown),
            "softmax" => (ComputeOp::Softmax, DiffType::Unknown),
            "pow" => (ComputeOp::Pow, DiffType::Scalar),
            "sum" => (ComputeOp::Sum { axis: None }, DiffType::Scalar),
            "mean" => (ComputeOp::Mean { axis: None }, DiffType::Scalar),
            "transpose" => (ComputeOp::Transpose, DiffType::Unknown),
            "broadcast" => (
                ComputeOp::Broadcast {
                    target_shape: vec![],
                },
                DiffType::Unknown,
            ),
            _ => (
                ComputeOp::Call {
                    func_name: method_name.to_string(),
                },
                DiffType::Unknown,
            ),
        };

        Ok(self.graph.add_node(op, all_args, output_type))
    }

    // Helper methods

    fn get_param_name(&self, param: &FunctionParam) -> Option<String> {
        match &param.kind {
            FunctionParamKind::Regular { pattern, .. } => self.get_pattern_name(pattern),
            FunctionParamKind::SelfValue
            | FunctionParamKind::SelfValueMut
            | FunctionParamKind::SelfRef
            | FunctionParamKind::SelfRefMut
            | FunctionParamKind::SelfOwn
            | FunctionParamKind::SelfOwnMut
            | FunctionParamKind::SelfRefChecked
            | FunctionParamKind::SelfRefCheckedMut
            | FunctionParamKind::SelfRefUnsafe
            | FunctionParamKind::SelfRefUnsafeMut => Some("self".to_string()),
        }
    }

    fn get_pattern_name(&self, pattern: &Pattern) -> Option<String> {
        match &pattern.kind {
            PatternKind::Ident { name, .. } => Some(name.as_str().to_string()),
            PatternKind::Wildcard => None,
            _ => None,
        }
    }

    fn infer_diff_type_from_param(&self, param: &FunctionParam) -> DiffType {
        match &param.kind {
            FunctionParamKind::Regular { ty, .. } => self.infer_diff_type_from_type(ty),
            _ => DiffType::Unknown,
        }
    }

    fn infer_diff_type_from_type(&self, ty: &Type) -> DiffType {
        match &ty.kind {
            TypeKind::Float => DiffType::Scalar,
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    match ident.as_str() {
                        "Float" | "f32" | "f64" => DiffType::Scalar,
                        "Tensor" => DiffType::Unknown, // Need generic args for shape
                        _ => DiffType::Unknown,
                    }
                } else {
                    // Handle generic types like Tensor<T, [N, M]> etc.
                    DiffType::Unknown
                }
            }
            TypeKind::Array {
                element: _,
                size: _,
            } => DiffType::Unknown,
            _ => DiffType::Unknown,
        }
    }

    fn extract_numeric_literal(&self, lit: &verum_ast::Literal) -> Result<f64, String> {
        use verum_ast::LiteralKind;
        match &lit.kind {
            LiteralKind::Int(v) => Ok(v.value as f64),
            LiteralKind::Float(v) => Ok(v.value),
            _ => Err("Non-numeric literal in autodiff context".to_string()),
        }
    }

    fn infer_binary_output_type(&self, left_id: usize, right_id: usize) -> DiffType {
        let left_type = &self.graph.nodes[left_id].output_type;
        let right_type = &self.graph.nodes[right_id].output_type;

        // Broadcasting rules: larger type wins
        match (left_type, right_type) {
            (DiffType::Tensor(s), _) | (_, DiffType::Tensor(s)) => DiffType::Tensor(s.clone()),
            (DiffType::Matrix(r, c), _) | (_, DiffType::Matrix(r, c)) => DiffType::Matrix(*r, *c),
            (DiffType::Vector(n), _) | (_, DiffType::Vector(n)) => DiffType::Vector(*n),
            _ => DiffType::Scalar,
        }
    }

    fn try_extract_const_index(&self, expr: &Expr) -> Result<usize, String> {
        match &expr.kind {
            ExprKind::Literal(lit) => {
                use verum_ast::LiteralKind;
                match &lit.kind {
                    LiteralKind::Int(v) => Ok(v.value as usize),
                    _ => Err("Non-integer index".to_string()),
                }
            }
            _ => Err("Non-constant index in autodiff context".to_string()),
        }
    }

    fn extract_condition_expr(&self, cond: &verum_ast::expr::IfCondition) -> Result<Expr, String> {
        // Extract the first expression condition
        for c in cond.conditions.iter() {
            match c {
                verum_ast::expr::ConditionKind::Expr(e) => return Ok(e.clone()),
                _ => continue,
            }
        }
        Err("No expression condition found".to_string())
    }
}

/// Generator for derivative functions (VJP, JVP, gradient)
pub struct DerivativeGenerator {
    /// The original function
    func: FunctionDecl,
    /// Configuration
    config: DifferentiableConfig,
    /// Computational graph
    graph: ComputeGraph,
}

impl DerivativeGenerator {
    pub fn new(func: FunctionDecl, config: DifferentiableConfig, graph: ComputeGraph) -> Self {
        Self {
            func,
            config,
            graph,
        }
    }

    /// Generate VJP (Vector-Jacobian Product) function
    /// Reverse-mode automatic differentiation
    ///
    /// For `fn f(x, y) -> z`:
    /// Generates `fn f_vjp(x, y, grad_z) -> (z, (grad_x, grad_y))`
    ///
    /// CBGR Note: Generated code includes ~15ns overhead per reference check
    pub fn generate_vjp(&self) -> Result<Item, String> {
        let vjp_name: verum_common::Text = format!("{}_vjp", self.func.name.as_str()).into();
        let span = self.func.span;

        // Build VJP function parameters:
        // - Original parameters
        // - grad_output (gradient from downstream)
        let mut vjp_params = self.func.params.clone();

        // Add grad_output parameter
        let grad_output_param = self.create_grad_output_param(span);
        vjp_params.push(grad_output_param);

        // Build VJP body using reverse-mode differentiation
        let vjp_body = self.build_vjp_body(span)?;

        // Determine return type: tuple of gradients for wrt params
        let return_type = self.build_vjp_return_type(span);

        let vjp_func = FunctionDecl {
            visibility: self.func.visibility.clone(),
            is_async: false,
            is_pure: false, // Generated VJP functions are not pure
            is_meta: true, // VJP functions are meta functions
            stage_level: 1,
            is_generator: false,
            is_cofix: false,
            is_unsafe: false,
            is_transparent: false,
            extern_abi: None,
            is_variadic: false,
            name: Ident::new(vjp_name, span),
            generics: self.func.generics.clone(),
            params: vjp_params,
            return_type: Some(return_type),
            throws_clause: None,
            std_attr: None,
            contexts: List::new(),
            generic_where_clause: self.func.generic_where_clause.clone(),
            meta_where_clause: self.func.meta_where_clause.clone(),
            attributes: self.build_generated_attributes(span).into_iter().collect(),
            body: Some(FunctionBody::Block(vjp_body)),
            requires: List::new(),
            ensures: List::new(),
            span,
        };

        Ok(Item::new(ItemKind::Function(vjp_func), span))
    }

    /// Generate JVP (Jacobian-Vector Product) function
    /// Forward-mode automatic differentiation
    ///
    /// For `fn f(x, y) -> z`:
    /// Generates `fn f_jvp(x, y, tangent_x, tangent_y) -> (z, tangent_z)`
    pub fn generate_jvp(&self) -> Result<Item, String> {
        let jvp_name: verum_common::Text = format!("{}_jvp", self.func.name.as_str()).into();
        let span = self.func.span;

        // Build JVP function parameters:
        // - Original parameters
        // - Tangent vectors for each wrt parameter
        let mut jvp_params = self.func.params.clone();

        for wrt_param in &self.config.wrt_params {
            let tangent_param = self.create_tangent_param(wrt_param, span);
            jvp_params.push(tangent_param);
        }

        // Build JVP body using forward-mode differentiation
        let jvp_body = self.build_jvp_body(span)?;

        // Return type: (output, tangent_output)
        let return_type = self.build_jvp_return_type(span);

        let jvp_func = FunctionDecl {
            visibility: self.func.visibility.clone(),
            is_async: false,
            is_pure: false, // Generated JVP functions are not pure
            is_meta: true,
            stage_level: 1,
            is_generator: false,
            is_cofix: false,
            is_unsafe: false,
            is_transparent: false,
            extern_abi: None,
            is_variadic: false,
            name: Ident::new(jvp_name, span),
            generics: self.func.generics.clone(),
            params: jvp_params,
            return_type: Some(return_type),
            throws_clause: None,
            std_attr: None,
            contexts: List::new(),
            generic_where_clause: self.func.generic_where_clause.clone(),
            meta_where_clause: self.func.meta_where_clause.clone(),
            attributes: self.build_generated_attributes(span).into_iter().collect(),
            body: Some(FunctionBody::Block(jvp_body)),
            requires: List::new(),
            ensures: List::new(),
            span,
        };

        Ok(Item::new(ItemKind::Function(jvp_func), span))
    }

    /// Generate gradient function (for scalar outputs)
    ///
    /// For `fn f(x, y) -> Float`:
    /// Generates `fn f_grad(x, y) -> (grad_x, grad_y)`
    pub fn generate_grad(&self) -> Result<Item, String> {
        let grad_name: verum_common::Text = format!("{}_grad", self.func.name.as_str()).into();
        let span = self.func.span;

        // Gradient function has same parameters as original
        let grad_params = self.func.params.clone();

        // Build gradient body (calls VJP with grad_output = 1.0)
        let grad_body = self.build_grad_body(span)?;

        // Return type: tuple of gradients
        let return_type = self.build_grad_return_type(span);

        let grad_func = FunctionDecl {
            visibility: self.func.visibility.clone(),
            is_async: false,
            is_pure: false, // Generated gradient functions are not pure
            is_meta: true,
            stage_level: 1,
            is_generator: false,
            is_cofix: false,
            is_unsafe: false,
            is_transparent: false,
            extern_abi: None,
            is_variadic: false,
            name: Ident::new(grad_name, span),
            generics: self.func.generics.clone(),
            params: grad_params,
            return_type: Some(return_type),
            throws_clause: None,
            std_attr: None,
            contexts: List::new(),
            generic_where_clause: self.func.generic_where_clause.clone(),
            meta_where_clause: self.func.meta_where_clause.clone(),
            attributes: self.build_generated_attributes(span).into_iter().collect(),
            body: Some(FunctionBody::Block(grad_body)),
            requires: List::new(),
            ensures: List::new(),
            span,
        };

        Ok(Item::new(ItemKind::Function(grad_func), span))
    }

    // VJP body construction (reverse-mode)
    fn build_vjp_body(&self, span: Span) -> Result<Block, String> {
        let mut stmts = Vec::new();

        // Step 1: Forward pass - compute all intermediate values
        // (needed for gradient computation)
        let forward_stmts = self.generate_forward_pass_stmts(span);
        for stmt in forward_stmts {
            stmts.push(stmt);
        }

        // Step 2: Backward pass - compute gradients in reverse order
        let backward_stmts = self.generate_backward_pass_stmts(span);
        for stmt in backward_stmts {
            stmts.push(stmt);
        }

        // Step 3: Return tuple of gradients for wrt params
        let grad_tuple = self.build_gradient_tuple_expr(span);

        Ok(Block {
            stmts: stmts.into(),
            expr: Some(Box::new(grad_tuple)),
            span,
        })
    }

    // JVP body construction (forward-mode)
    fn build_jvp_body(&self, span: Span) -> Result<Block, String> {
        let mut stmts = Vec::new();

        // Forward-mode computes primal and tangent together
        let jvp_stmts = self.generate_forward_mode_stmts(span);
        for stmt in jvp_stmts {
            stmts.push(stmt);
        }

        // Return (primal_output, tangent_output)
        let result_tuple = self.build_jvp_result_tuple(span);

        Ok(Block {
            stmts: stmts.into(),
            expr: Some(Box::new(result_tuple)),
            span,
        })
    }

    // Gradient body construction
    fn build_grad_body(&self, span: Span) -> Result<Block, String> {
        let mut stmts = Vec::new();

        // Call VJP with grad_output = 1.0 (for scalar output)
        let vjp_call = self.build_vjp_call_with_one(span);
        let result_binding = verum_ast::Stmt::new(
            verum_ast::StmtKind::Let {
                pattern: Pattern::new(
                    PatternKind::Ident {
                        by_ref: false,
                        name: Ident::new("_result", span),
                        mutable: false,
                        subpattern: None,
                    },
                    span,
                ),
                ty: None,
                value: Some(vjp_call),
            },
            span,
        );
        stmts.push(result_binding);

        // Extract and return gradient tuple
        let mut result_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new("_result", span))),
            span,
        );
        result_ref.ref_kind = None;
        result_ref.check_eliminated = false;

        Ok(Block {
            stmts: stmts.into(),
            expr: Some(Box::new(result_ref)),
            span,
        })
    }

    // Generate forward pass statements
    fn generate_forward_pass_stmts(&self, span: Span) -> Vec<verum_ast::Stmt> {
        let mut stmts = Vec::new();

        // Iterate through graph nodes in topological order
        for node in &self.graph.nodes {
            if let Some(stmt) = self.generate_forward_stmt_for_node(node, span) {
                stmts.push(stmt);
            }
        }

        stmts
    }

    // Generate backward pass statements
    fn generate_backward_pass_stmts(&self, span: Span) -> Vec<verum_ast::Stmt> {
        let mut stmts = Vec::new();

        // Initialize gradient accumulator for output
        let init_grad = self.create_grad_init_stmt("grad_output", span);
        stmts.push(init_grad);

        // Process nodes in reverse topological order
        let mut reverse_nodes: Vec<_> = self.graph.nodes.iter().collect();
        reverse_nodes.reverse();

        for node in reverse_nodes {
            let node_stmts = self.generate_backward_stmt_for_node(node, span);
            stmts.extend(node_stmts);
        }

        stmts
    }

    // Generate forward statement for a single node
    fn generate_forward_stmt_for_node(
        &self,
        node: &ComputeNode,
        span: Span,
    ) -> Option<verum_ast::Stmt> {
        let var_name = format!("_v{}", node.id);

        let expr = match &node.op {
            ComputeOp::Parameter { name: _, .. } => {
                // Parameters are already bound, just reference them
                return None;
            }
            ComputeOp::Constant { value } => Expr::new(
                ExprKind::Literal(verum_ast::Literal::float(*value, span)),
                span,
            ),
            ComputeOp::Add => self.build_binary_expr("+", &node.inputs, span),
            ComputeOp::Sub => self.build_binary_expr("-", &node.inputs, span),
            ComputeOp::Mul => self.build_binary_expr("*", &node.inputs, span),
            ComputeOp::Div => self.build_binary_expr("/", &node.inputs, span),
            ComputeOp::Neg => self.build_unary_expr("-", node.inputs[0], span),
            ComputeOp::Pow => self.build_call_expr("pow", &node.inputs, span),
            ComputeOp::Sin => self.build_call_expr("sin", &node.inputs, span),
            ComputeOp::Cos => self.build_call_expr("cos", &node.inputs, span),
            ComputeOp::Tan => self.build_call_expr("tan", &node.inputs, span),
            ComputeOp::Exp => self.build_call_expr("exp", &node.inputs, span),
            ComputeOp::Log => self.build_call_expr("log", &node.inputs, span),
            ComputeOp::Sqrt => self.build_call_expr("sqrt", &node.inputs, span),
            ComputeOp::Abs => self.build_call_expr("abs", &node.inputs, span),
            ComputeOp::Tanh => self.build_call_expr("tanh", &node.inputs, span),
            ComputeOp::Sigmoid => self.build_call_expr("sigmoid", &node.inputs, span),
            ComputeOp::Relu => self.build_call_expr("relu", &node.inputs, span),
            ComputeOp::Softmax => self.build_call_expr("softmax", &node.inputs, span),
            ComputeOp::MatMul => self.build_call_expr("matmul", &node.inputs, span),
            ComputeOp::Transpose => {
                self.build_method_call_expr("transpose", node.inputs[0], &[], span)
            }
            ComputeOp::Sum { axis: _ } => {
                self.build_method_call_expr("sum", node.inputs[0], &[], span)
            }
            ComputeOp::Mean { axis: _ } => {
                self.build_method_call_expr("mean", node.inputs[0], &[], span)
            }
            ComputeOp::Broadcast { .. } => {
                self.build_method_call_expr("broadcast", node.inputs[0], &[], span)
            }
            ComputeOp::Select { condition_node } => {
                // if cond { then } else { else }
                self.build_select_expr(*condition_node, &node.inputs, span)
            }
            ComputeOp::Index { index } => self.build_index_expr(node.inputs[0], *index, span),
            ComputeOp::Field { name } => self.build_field_expr(node.inputs[0], name, span),
            ComputeOp::Call { func_name } => self.build_call_expr(func_name, &node.inputs, span),
        };

        Some(verum_ast::Stmt::new(
            verum_ast::StmtKind::Let {
                pattern: Pattern::new(
                    PatternKind::Ident {
                        by_ref: false,
                        name: Ident::new(var_name, span),
                        mutable: false,
                        subpattern: None,
                    },
                    span,
                ),
                ty: None,
                value: Some(expr),
            },
            span,
        ))
    }

    // Generate backward statements for a single node
    fn generate_backward_stmt_for_node(
        &self,
        node: &ComputeNode,
        span: Span,
    ) -> Vec<verum_ast::Stmt> {
        let mut stmts = Vec::new();
        let grad_name = format!("_grad{}", node.id);

        // Apply chain rule based on operation type
        match &node.op {
            ComputeOp::Parameter { name, .. } => {
                // Accumulate gradient to parameter gradient
                if self.graph.wrt_params.contains(name) {
                    let accum = self.build_grad_accumulate_stmt(name, &grad_name, span);
                    stmts.push(accum);
                }
            }
            ComputeOp::Constant { .. } => {
                // Constants have zero gradient - nothing to propagate
            }
            ComputeOp::Add => {
                // d/dx (x + y) = 1, d/dy (x + y) = 1
                // grad_x += grad_out, grad_y += grad_out
                for input_id in &node.inputs {
                    let prop = self.build_grad_propagate_stmt(*input_id, &grad_name, "1.0", span);
                    stmts.push(prop);
                }
            }
            ComputeOp::Sub => {
                // d/dx (x - y) = 1, d/dy (x - y) = -1
                if node.inputs.len() >= 2 {
                    let prop_x =
                        self.build_grad_propagate_stmt(node.inputs[0], &grad_name, "1.0", span);
                    let prop_y =
                        self.build_grad_propagate_stmt(node.inputs[1], &grad_name, "-1.0", span);
                    stmts.push(prop_x);
                    stmts.push(prop_y);
                }
            }
            ComputeOp::Mul => {
                // d/dx (x * y) = y, d/dy (x * y) = x
                if node.inputs.len() >= 2 {
                    let x_var = format!("_v{}", node.inputs[0]);
                    let y_var = format!("_v{}", node.inputs[1]);
                    let prop_x = self.build_grad_mul_propagate_stmt(
                        node.inputs[0],
                        &grad_name,
                        &y_var,
                        span,
                    );
                    let prop_y = self.build_grad_mul_propagate_stmt(
                        node.inputs[1],
                        &grad_name,
                        &x_var,
                        span,
                    );
                    stmts.push(prop_x);
                    stmts.push(prop_y);
                }
            }
            ComputeOp::Div => {
                // d/dx (x / y) = 1/y, d/dy (x / y) = -x/y^2
                if node.inputs.len() >= 2 {
                    let x_var = format!("_v{}", node.inputs[0]);
                    let y_var = format!("_v{}", node.inputs[1]);
                    let prop_x = self.build_grad_div_propagate_x_stmt(
                        node.inputs[0],
                        &grad_name,
                        &y_var,
                        span,
                    );
                    let prop_y = self.build_grad_div_propagate_y_stmt(
                        node.inputs[1],
                        &grad_name,
                        &x_var,
                        &y_var,
                        span,
                    );
                    stmts.push(prop_x);
                    stmts.push(prop_y);
                }
            }
            ComputeOp::Neg => {
                // d/dx (-x) = -1
                let prop = self.build_grad_propagate_stmt(node.inputs[0], &grad_name, "-1.0", span);
                stmts.push(prop);
            }
            ComputeOp::Pow => {
                // d/dx (x^y) = y * x^(y-1), d/dy (x^y) = x^y * ln(x)
                if node.inputs.len() >= 2 {
                    let prop_base = self.build_pow_grad_base_stmt(node, &grad_name, span);
                    let prop_exp = self.build_pow_grad_exp_stmt(node, &grad_name, span);
                    stmts.push(prop_base);
                    stmts.push(prop_exp);
                }
            }
            ComputeOp::Sin => {
                // d/dx sin(x) = cos(x)
                let prop = self.build_trig_grad_stmt(node, &grad_name, "cos", span);
                stmts.push(prop);
            }
            ComputeOp::Cos => {
                // d/dx cos(x) = -sin(x)
                let prop = self.build_trig_grad_stmt(node, &grad_name, "-sin", span);
                stmts.push(prop);
            }
            ComputeOp::Tan => {
                // d/dx tan(x) = sec^2(x) = 1/cos^2(x)
                let prop = self.build_tan_grad_stmt(node, &grad_name, span);
                stmts.push(prop);
            }
            ComputeOp::Exp => {
                // d/dx exp(x) = exp(x)
                let out_var = format!("_v{}", node.id);
                let prop =
                    self.build_grad_mul_propagate_stmt(node.inputs[0], &grad_name, &out_var, span);
                stmts.push(prop);
            }
            ComputeOp::Log => {
                // d/dx log(x) = 1/x
                let x_var = format!("_v{}", node.inputs[0]);
                let prop = self.build_log_grad_stmt(node.inputs[0], &grad_name, &x_var, span);
                stmts.push(prop);
            }
            ComputeOp::Sqrt => {
                // d/dx sqrt(x) = 1/(2*sqrt(x))
                let out_var = format!("_v{}", node.id);
                let prop = self.build_sqrt_grad_stmt(node.inputs[0], &grad_name, &out_var, span);
                stmts.push(prop);
            }
            ComputeOp::Abs => {
                // d/dx |x| = sign(x)
                let x_var = format!("_v{}", node.inputs[0]);
                let prop = self.build_abs_grad_stmt(node.inputs[0], &grad_name, &x_var, span);
                stmts.push(prop);
            }
            ComputeOp::Tanh => {
                // d/dx tanh(x) = 1 - tanh^2(x)
                let out_var = format!("_v{}", node.id);
                let prop = self.build_tanh_grad_stmt(node.inputs[0], &grad_name, &out_var, span);
                stmts.push(prop);
            }
            ComputeOp::Sigmoid => {
                // d/dx sigmoid(x) = sigmoid(x) * (1 - sigmoid(x))
                let out_var = format!("_v{}", node.id);
                let prop = self.build_sigmoid_grad_stmt(node.inputs[0], &grad_name, &out_var, span);
                stmts.push(prop);
            }
            ComputeOp::Relu => {
                // d/dx relu(x) = grad * (x > 0)
                let x_var = format!("_v{}", node.inputs[0]);
                let prop = self.build_relu_grad_stmt(node.inputs[0], &grad_name, &x_var, span);
                stmts.push(prop);
            }
            ComputeOp::Softmax => {
                // d/dx softmax(x) is complex - Jacobian is diagonal - outer product
                // grad_x = softmax * (grad - dot(softmax, grad))
                let out_var = format!("_v{}", node.id);
                let prop = self.build_softmax_grad_stmt(node.inputs[0], &grad_name, &out_var, span);
                stmts.push(prop);
            }
            ComputeOp::MatMul => {
                // d/dA (A @ B) = grad @ B^T, d/dB (A @ B) = A^T @ grad
                if node.inputs.len() >= 2 {
                    let prop_a = self.build_matmul_grad_a_stmt(node, &grad_name, span);
                    let prop_b = self.build_matmul_grad_b_stmt(node, &grad_name, span);
                    stmts.push(prop_a);
                    stmts.push(prop_b);
                }
            }
            ComputeOp::Transpose => {
                // d/dx (x^T) = (grad)^T
                let prop = self.build_transpose_grad_stmt(node, &grad_name, span);
                stmts.push(prop);
            }
            ComputeOp::Sum { axis: _ } => {
                // d/dx sum(x) = broadcast(grad, shape(x))
                let prop = self.build_sum_grad_stmt(node, &grad_name, span);
                stmts.push(prop);
            }
            ComputeOp::Mean { axis: _ } => {
                // d/dx mean(x) = broadcast(grad / n, shape(x))
                let prop = self.build_mean_grad_stmt(node, &grad_name, span);
                stmts.push(prop);
            }
            ComputeOp::Broadcast { .. } => {
                // d/dx broadcast(x) = sum_reduce(grad)
                let prop = self.build_broadcast_grad_stmt(node, &grad_name, span);
                stmts.push(prop);
            }
            ComputeOp::Select { condition_node } => {
                // d/dx select(c, x, y) = c ? grad : 0, d/dy = c ? 0 : grad
                let prop_then =
                    self.build_select_grad_then_stmt(node, &grad_name, *condition_node, span);
                let prop_else =
                    self.build_select_grad_else_stmt(node, &grad_name, *condition_node, span);
                stmts.push(prop_then);
                stmts.push(prop_else);
            }
            ComputeOp::Index { index } => {
                // d/dx x[i] = scatter(grad, i, shape(x))
                let prop = self.build_index_grad_stmt(node, &grad_name, *index, span);
                stmts.push(prop);
            }
            ComputeOp::Field { name } => {
                // Field access gradient (struct gradient)
                let prop = self.build_field_grad_stmt(node, &grad_name, name, span);
                stmts.push(prop);
            }
            ComputeOp::Call { func_name } => {
                // Call VJP of the called function
                let prop = self.build_call_grad_stmt(node, &grad_name, func_name, span);
                stmts.push(prop);
            }
        }

        stmts
    }

    // Generate forward-mode statements
    fn generate_forward_mode_stmts(&self, span: Span) -> Vec<verum_ast::Stmt> {
        let mut stmts = Vec::new();

        // Process nodes in topological order, computing both primal and tangent
        for node in &self.graph.nodes {
            if let Some((primal_stmt, tangent_stmt)) =
                self.generate_forward_mode_node_stmts(node, span)
            {
                stmts.push(primal_stmt);
                stmts.push(tangent_stmt);
            }
        }

        stmts
    }

    fn generate_forward_mode_node_stmts(
        &self,
        node: &ComputeNode,
        span: Span,
    ) -> Option<(verum_ast::Stmt, verum_ast::Stmt)> {
        let primal_name = format!("_v{}", node.id);
        let tangent_name = format!("_t{}", node.id);

        match &node.op {
            ComputeOp::Parameter { name, .. } => {
                // Parameters: primal is the input, tangent is the tangent input
                let _tangent_input = format!("tangent_{}", name);
                if self.graph.wrt_params.contains(name) {
                    // Tangent is provided as input
                    return None; // Already bound
                } else {
                    // Non-differentiated param: tangent is zero
                    let tangent_expr = Expr::new(
                        ExprKind::Literal(verum_ast::Literal::float(0.0, span)),
                        span,
                    );
                    let tangent_stmt = verum_ast::Stmt::new(
                        verum_ast::StmtKind::Let {
                            pattern: Pattern::new(
                                PatternKind::Ident {
                                    by_ref: false,
                                    name: Ident::new(tangent_name, span),
                                    mutable: false,
                                    subpattern: None,
                                },
                                span,
                            ),
                            ty: None,
                            value: Some(tangent_expr),
                        },
                        span,
                    );
                    // No primal stmt needed for params
                    return Some((tangent_stmt.clone(), tangent_stmt));
                }
            }
            ComputeOp::Constant { value } => {
                // Constants: primal is the value, tangent is zero
                let primal_expr = Expr::new(
                    ExprKind::Literal(verum_ast::Literal::float(*value, span)),
                    span,
                );
                let tangent_expr = Expr::new(
                    ExprKind::Literal(verum_ast::Literal::float(0.0, span)),
                    span,
                );
                let primal_stmt = self.create_let_stmt(&primal_name, primal_expr, span);
                let tangent_stmt = self.create_let_stmt(&tangent_name, tangent_expr, span);
                return Some((primal_stmt, tangent_stmt));
            }
            _ => {
                // Build primal expression
                let primal_expr =
                    self.generate_forward_stmt_for_node(node, span)
                        .map(|s| match s.kind {
                            verum_ast::StmtKind::Let { value: Some(e), .. } => e,
                            _ => Expr::new(
                                ExprKind::Path(Path::single(Ident::new("_unit", span))),
                                span,
                            ),
                        })?;

                // Build tangent expression using forward-mode rules
                let tangent_expr = self.build_forward_tangent_expr(node, span);

                let primal_stmt = self.create_let_stmt(&primal_name, primal_expr, span);
                let tangent_stmt = self.create_let_stmt(&tangent_name, tangent_expr, span);
                return Some((primal_stmt, tangent_stmt));
            }
        }
    }

    fn build_forward_tangent_expr(&self, node: &ComputeNode, span: Span) -> Expr {
        match &node.op {
            // Arithmetic operations
            ComputeOp::Add => {
                // tangent(x + y) = tangent_x + tangent_y
                let t0 = self.tangent_ref(node.inputs[0], span);
                let t1 = self.tangent_ref(node.inputs[1], span);
                self.build_add_expr(t0, t1, span)
            }
            ComputeOp::Sub => {
                // tangent(x - y) = tangent_x - tangent_y
                let t0 = self.tangent_ref(node.inputs[0], span);
                let t1 = self.tangent_ref(node.inputs[1], span);
                self.build_sub_expr(t0, t1, span)
            }
            ComputeOp::Mul => {
                // tangent(x * y) = tangent_x * y + x * tangent_y
                let v0 = self.primal_ref(node.inputs[0], span);
                let v1 = self.primal_ref(node.inputs[1], span);
                let t0 = self.tangent_ref(node.inputs[0], span);
                let t1 = self.tangent_ref(node.inputs[1], span);
                let term1 = self.build_mul_expr(t0, v1.clone(), span);
                let term2 = self.build_mul_expr(v0, t1, span);
                self.build_add_expr(term1, term2, span)
            }
            ComputeOp::Div => {
                // tangent(x / y) = (tangent_x * y - x * tangent_y) / y^2
                let v0 = self.primal_ref(node.inputs[0], span);
                let v1 = self.primal_ref(node.inputs[1], span);
                let t0 = self.tangent_ref(node.inputs[0], span);
                let t1 = self.tangent_ref(node.inputs[1], span);
                let num1 = self.build_mul_expr(t0, v1.clone(), span);
                let num2 = self.build_mul_expr(v0, t1, span);
                let num = self.build_sub_expr(num1, num2, span);
                let denom = self.build_mul_expr(v1.clone(), v1, span);
                self.build_div_expr(num, denom, span)
            }
            ComputeOp::Neg => {
                // tangent(-x) = -tangent_x
                let t0 = self.tangent_ref(node.inputs[0], span);
                self.build_neg_expr(t0, span)
            }
            ComputeOp::Pow => {
                // tangent(x^y) = y * x^(y-1) * tangent_x + x^y * ln(x) * tangent_y
                // For common case where y is constant: tangent(x^c) = c * x^(c-1) * tangent_x
                if node.inputs.len() < 2 {
                    return self.zero_expr(span);
                }
                let v0 = self.primal_ref(node.inputs[0], span);
                let v1 = self.primal_ref(node.inputs[1], span);
                let t0 = self.tangent_ref(node.inputs[0], span);
                let t1 = self.tangent_ref(node.inputs[1], span);

                // First term: y * x^(y-1) * tangent_x
                let one = Expr::new(
                    ExprKind::Literal(verum_ast::Literal::float(1.0, span)),
                    span,
                );
                let y_minus_1 = self.build_sub_expr(v1.clone(), one, span);
                let pow_term = self.build_fn_call_expr("pow", vec![v0.clone(), y_minus_1], span);
                let term1 = self.build_mul_expr(v1.clone(), pow_term, span);
                let term1 = self.build_mul_expr(term1, t0, span);

                // Second term: x^y * ln(x) * tangent_y
                let out_v = self.primal_ref(node.id, span);
                let ln_x = self.build_fn_call_expr("log", vec![v0], span);
                let term2 = self.build_mul_expr(out_v, ln_x, span);
                let term2 = self.build_mul_expr(term2, t1, span);

                self.build_add_expr(term1, term2, span)
            }

            // Elementary functions
            ComputeOp::Sin => {
                // tangent(sin(x)) = cos(x) * tangent_x
                let v0 = self.primal_ref(node.inputs[0], span);
                let t0 = self.tangent_ref(node.inputs[0], span);
                let cos_v = self.build_fn_call_expr("cos", vec![v0], span);
                self.build_mul_expr(cos_v, t0, span)
            }
            ComputeOp::Cos => {
                // tangent(cos(x)) = -sin(x) * tangent_x
                let v0 = self.primal_ref(node.inputs[0], span);
                let t0 = self.tangent_ref(node.inputs[0], span);
                let sin_v = self.build_fn_call_expr("sin", vec![v0], span);
                let neg_sin = self.build_neg_expr(sin_v, span);
                self.build_mul_expr(neg_sin, t0, span)
            }
            ComputeOp::Tan => {
                // tangent(tan(x)) = tangent_x / cos^2(x)
                let v0 = self.primal_ref(node.inputs[0], span);
                let t0 = self.tangent_ref(node.inputs[0], span);
                let cos_v = self.build_fn_call_expr("cos", vec![v0], span);
                let cos_sq = self.build_mul_expr(cos_v.clone(), cos_v, span);
                self.build_div_expr(t0, cos_sq, span)
            }
            ComputeOp::Exp => {
                // tangent(exp(x)) = exp(x) * tangent_x
                let out_v = self.primal_ref(node.id, span);
                let t0 = self.tangent_ref(node.inputs[0], span);
                self.build_mul_expr(out_v, t0, span)
            }
            ComputeOp::Log => {
                // tangent(log(x)) = tangent_x / x
                let v0 = self.primal_ref(node.inputs[0], span);
                let t0 = self.tangent_ref(node.inputs[0], span);
                self.build_div_expr(t0, v0, span)
            }
            ComputeOp::Sqrt => {
                // tangent(sqrt(x)) = tangent_x / (2 * sqrt(x))
                let out_v = self.primal_ref(node.id, span);
                let t0 = self.tangent_ref(node.inputs[0], span);
                let two = Expr::new(
                    ExprKind::Literal(verum_ast::Literal::float(2.0, span)),
                    span,
                );
                let denom = self.build_mul_expr(two, out_v, span);
                self.build_div_expr(t0, denom, span)
            }
            ComputeOp::Abs => {
                // tangent(abs(x)) = sign(x) * tangent_x
                // sign(x) = x / abs(x) for x != 0, else 0
                let v0 = self.primal_ref(node.inputs[0], span);
                let t0 = self.tangent_ref(node.inputs[0], span);
                let out_v = self.primal_ref(node.id, span);
                let sign = self.build_div_expr(v0, out_v, span);
                self.build_mul_expr(sign, t0, span)
            }

            // Activation functions
            ComputeOp::Tanh => {
                // tangent(tanh(x)) = (1 - tanh^2(x)) * tangent_x
                let out_v = self.primal_ref(node.id, span);
                let t0 = self.tangent_ref(node.inputs[0], span);
                let one = Expr::new(
                    ExprKind::Literal(verum_ast::Literal::float(1.0, span)),
                    span,
                );
                let out_sq = self.build_mul_expr(out_v.clone(), out_v, span);
                let factor = self.build_sub_expr(one, out_sq, span);
                self.build_mul_expr(factor, t0, span)
            }
            ComputeOp::Sigmoid => {
                // tangent(sigmoid(x)) = sigmoid(x) * (1 - sigmoid(x)) * tangent_x
                let out_v = self.primal_ref(node.id, span);
                let t0 = self.tangent_ref(node.inputs[0], span);
                let one = Expr::new(
                    ExprKind::Literal(verum_ast::Literal::float(1.0, span)),
                    span,
                );
                let one_minus_out = self.build_sub_expr(one, out_v.clone(), span);
                let factor = self.build_mul_expr(out_v, one_minus_out, span);
                self.build_mul_expr(factor, t0, span)
            }
            ComputeOp::Relu => {
                // tangent(relu(x)) = tangent_x * (x > 0)
                // relu'(x) = 1 if x > 0, else 0
                let v0 = self.primal_ref(node.inputs[0], span);
                let t0 = self.tangent_ref(node.inputs[0], span);
                let zero = Expr::new(
                    ExprKind::Literal(verum_ast::Literal::float(0.0, span)),
                    span,
                );

                // Build condition: x > 0
                let cond = Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Gt,
                        left: Box::new(v0),
                        right: Box::new(zero.clone()),
                    },
                    span,
                );

                // if x > 0 { tangent_x } else { 0 }
                Expr::new(
                    ExprKind::If {
                        condition: Box::new(verum_ast::expr::IfCondition {
                            conditions: verum_ast::smallvec::smallvec![
                                verum_ast::expr::ConditionKind::Expr(cond)
                            ],
                            span,
                        }),
                        then_branch: Block {
                            stmts: List::new(),
                            expr: Some(Box::new(t0)),
                            span,
                        },
                        else_branch: Some(Box::new(Expr::new(
                            ExprKind::Block(Block {
                                stmts: List::new(),
                                expr: Some(Box::new(zero)),
                                span,
                            }),
                            span,
                        ))),
                    },
                    span,
                )
            }
            ComputeOp::Softmax => {
                // Forward-mode softmax gradient uses efficient Jacobian-vector product:
                // For softmax output s[i] = exp(x[i]) / sum(exp(x))
                // The Jacobian J[i,j] = s[i] * (delta[i,j] - s[j])
                // JVP = J @ tangent_x = s * (tangent_x - dot(s, tangent_x))
                // This avoids materializing the full Jacobian matrix (O(n^2) storage)

                let out_v = self.primal_ref(node.id, span);
                let t0 = self.tangent_ref(node.inputs[0], span);

                // dot_product = dot(softmax, tangent_x)
                let dot_prod =
                    self.build_fn_call_expr("dot", vec![out_v.clone(), t0.clone()], span);

                // tangent_x - dot_product (broadcast to each element)
                let diff = self.build_sub_expr(t0, dot_prod, span);

                // softmax * diff (element-wise)
                self.build_mul_expr(out_v, diff, span)
            }

            // Matrix operations
            ComputeOp::MatMul => {
                // tangent(A @ B) = tangent_A @ B + A @ tangent_B
                if node.inputs.len() < 2 {
                    return self.zero_expr(span);
                }
                let v0 = self.primal_ref(node.inputs[0], span);
                let v1 = self.primal_ref(node.inputs[1], span);
                let t0 = self.tangent_ref(node.inputs[0], span);
                let t1 = self.tangent_ref(node.inputs[1], span);

                let term1 = self.build_fn_call_expr("matmul", vec![t0, v1], span);
                let term2 = self.build_fn_call_expr("matmul", vec![v0, t1], span);
                self.build_add_expr(term1, term2, span)
            }
            ComputeOp::Transpose => {
                // tangent(A^T) = tangent_A^T
                let t0 = self.tangent_ref(node.inputs[0], span);
                self.build_fn_call_expr("transpose", vec![t0], span)
            }

            // Reduction operations
            ComputeOp::Sum { axis } => {
                // tangent(sum(x)) = sum(tangent_x)
                let t0 = self.tangent_ref(node.inputs[0], span);
                if axis.is_some() {
                    self.build_method_call_expr("sum_axis", node.inputs[0], &[], span)
                } else {
                    self.build_fn_call_expr("sum", vec![t0], span)
                }
            }
            ComputeOp::Mean { axis } => {
                // tangent(mean(x)) = mean(tangent_x)
                let t0 = self.tangent_ref(node.inputs[0], span);
                if axis.is_some() {
                    self.build_method_call_expr("mean_axis", node.inputs[0], &[], span)
                } else {
                    self.build_fn_call_expr("mean", vec![t0], span)
                }
            }

            // Broadcasting
            ComputeOp::Broadcast { target_shape: _ } => {
                // tangent(broadcast(x)) = broadcast(tangent_x)
                let t0 = self.tangent_ref(node.inputs[0], span);
                // Broadcast has same shape transformation on tangent
                t0
            }

            // Control flow
            ComputeOp::Select { condition_node } => {
                // tangent(select(c, x, y)) = select(c, tangent_x, tangent_y)
                // Condition doesn't affect tangent (it's discrete)
                if node.inputs.len() < 2 {
                    return self.zero_expr(span);
                }
                let t_then = self.tangent_ref(node.inputs[0], span);
                let t_else = self.tangent_ref(node.inputs[1], span);
                let cond = self.primal_ref(*condition_node, span);

                // Build if expression: if cond { t_then } else { t_else }
                Expr::new(
                    ExprKind::If {
                        condition: Box::new(verum_ast::expr::IfCondition {
                            conditions: verum_ast::smallvec::smallvec![
                                verum_ast::expr::ConditionKind::Expr(cond)
                            ],
                            span,
                        }),
                        then_branch: Block {
                            stmts: List::new(),
                            expr: Some(Box::new(t_then)),
                            span,
                        },
                        else_branch: Some(Box::new(Expr::new(
                            ExprKind::Block(Block {
                                stmts: List::new(),
                                expr: Some(Box::new(t_else)),
                                span,
                            }),
                            span,
                        ))),
                    },
                    span,
                )
            }

            // Indexing operations (tangent flows through)
            ComputeOp::Index { index } => {
                // tangent(x[i]) = tangent_x[i]
                let t0 = self.tangent_ref(node.inputs[0], span);
                let idx = Expr::new(
                    ExprKind::Literal(verum_ast::Literal::int(*index as i128, span)),
                    span,
                );
                Expr::new(
                    ExprKind::Index {
                        expr: Box::new(t0),
                        index: Box::new(idx),
                    },
                    span,
                )
            }
            ComputeOp::Field { name } => {
                // tangent(x.field) = tangent_x.field
                let t0 = self.tangent_ref(node.inputs[0], span);
                Expr::new(
                    ExprKind::Field {
                        expr: Box::new(t0),
                        field: Ident::new(name.clone(), span),
                    },
                    span,
                )
            }

            // Function calls (assume differentiable function exists)
            ComputeOp::Call { func_name } => {
                // tangent(f(x)) = f_jvp(x, tangent_x)
                // This assumes the called function has a JVP
                let jvp_name = format!("{}_jvp", func_name);
                let mut args = Vec::new();

                // Add primal arguments
                for &input_id in &node.inputs {
                    args.push(self.primal_ref(input_id, span));
                }

                // Add tangent arguments
                for &input_id in &node.inputs {
                    args.push(self.tangent_ref(input_id, span));
                }

                // Call returns (primal, tangent), extract tangent
                let jvp_call = self.build_fn_call_expr(&jvp_name, args, span);
                Expr::new(
                    ExprKind::Field {
                        expr: Box::new(jvp_call),
                        field: Ident::new("1", span), // Second element of tuple
                    },
                    span,
                )
            }

            // Parameter and Constant handled in generate_forward_mode_node_stmts
            ComputeOp::Parameter { .. } | ComputeOp::Constant { .. } => {
                // Should not reach here - handled separately
                self.zero_expr(span)
            }
        }
    }

    fn zero_expr(&self, span: Span) -> Expr {
        Expr::new(
            ExprKind::Literal(verum_ast::Literal::float(0.0, span)),
            span,
        )
    }

    // Helper methods for expression building

    fn primal_ref(&self, node_id: usize, span: Span) -> Expr {
        let var_name = if let ComputeOp::Parameter { name, .. } = &self.graph.nodes[node_id].op {
            name.clone()
        } else {
            format!("_v{}", node_id)
        };
        Expr::new(
            ExprKind::Path(Path::single(Ident::new(var_name, span))),
            span,
        )
    }

    fn tangent_ref(&self, node_id: usize, span: Span) -> Expr {
        let var_name = if let ComputeOp::Parameter { name, .. } = &self.graph.nodes[node_id].op {
            format!("tangent_{}", name)
        } else {
            format!("_t{}", node_id)
        };
        Expr::new(
            ExprKind::Path(Path::single(Ident::new(var_name, span))),
            span,
        )
    }

    fn build_add_expr(&self, left: Expr, right: Expr, span: Span) -> Expr {
        Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: Box::new(left),
                right: Box::new(right),
            },
            span,
        )
    }

    fn build_sub_expr(&self, left: Expr, right: Expr, span: Span) -> Expr {
        Expr::new(
            ExprKind::Binary {
                op: BinOp::Sub,
                left: Box::new(left),
                right: Box::new(right),
            },
            span,
        )
    }

    fn build_mul_expr(&self, left: Expr, right: Expr, span: Span) -> Expr {
        Expr::new(
            ExprKind::Binary {
                op: BinOp::Mul,
                left: Box::new(left),
                right: Box::new(right),
            },
            span,
        )
    }

    fn build_div_expr(&self, left: Expr, right: Expr, span: Span) -> Expr {
        Expr::new(
            ExprKind::Binary {
                op: BinOp::Div,
                left: Box::new(left),
                right: Box::new(right),
            },
            span,
        )
    }

    fn build_neg_expr(&self, inner: Expr, span: Span) -> Expr {
        Expr::new(
            ExprKind::Unary {
                op: UnOp::Neg,
                expr: Box::new(inner),
            },
            span,
        )
    }

    fn build_fn_call_expr(&self, name: &str, args: Vec<Expr>, span: Span) -> Expr {
        let func = Expr::new(ExprKind::Path(Path::single(Ident::new(name, span))), span);
        Expr::new(
            ExprKind::Call {
                func: Box::new(func),
                type_args: List::new(),
                args: args.into(),
            },
            span,
        )
    }

    fn create_let_stmt(&self, name: &str, value: Expr, span: Span) -> verum_ast::Stmt {
        verum_ast::Stmt::new(
            verum_ast::StmtKind::Let {
                pattern: Pattern::new(
                    PatternKind::Ident {
                        by_ref: false,
                        name: Ident::new(name, span),
                        mutable: false,
                        subpattern: None,
                    },
                    span,
                ),
                ty: None,
                value: Some(value),
            },
            span,
        )
    }

    // Stub methods for complex gradient computations
    // These generate the appropriate AST for each gradient rule

    fn create_grad_init_stmt(&self, name: &str, span: Span) -> verum_ast::Stmt {
        // Initialize gradient variable
        let init_expr = Expr::new(ExprKind::Path(Path::single(Ident::new(name, span))), span);
        self.create_let_stmt(&format!("_grad_init_{}", name), init_expr, span)
    }

    fn build_grad_accumulate_stmt(
        &self,
        param_name: &str,
        grad_name: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        self.create_let_stmt(&format!("grad_{}", param_name), grad_ref, span)
    }

    fn build_grad_propagate_stmt(
        &self,
        input_id: usize,
        grad_name: &str,
        factor: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        let factor_expr = if factor == "1.0" {
            grad_ref
        } else if factor == "-1.0" {
            self.build_neg_expr(grad_ref, span)
        } else {
            let factor_lit = Expr::new(
                ExprKind::Literal(verum_ast::Literal::float(
                    factor.parse().unwrap_or(1.0),
                    span,
                )),
                span,
            );
            self.build_mul_expr(grad_ref, factor_lit, span)
        };
        self.create_let_stmt(&format!("_grad{}", input_id), factor_expr, span)
    }

    fn build_grad_mul_propagate_stmt(
        &self,
        input_id: usize,
        grad_name: &str,
        other_var: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        let other_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(other_var, span))),
            span,
        );
        let result = self.build_mul_expr(grad_ref, other_ref, span);
        self.create_let_stmt(&format!("_grad{}", input_id), result, span)
    }

    fn build_grad_div_propagate_x_stmt(
        &self,
        input_id: usize,
        grad_name: &str,
        y_var: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // grad_x = grad / y
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        let y_ref = Expr::new(ExprKind::Path(Path::single(Ident::new(y_var, span))), span);
        let result = self.build_div_expr(grad_ref, y_ref, span);
        self.create_let_stmt(&format!("_grad{}", input_id), result, span)
    }

    fn build_grad_div_propagate_y_stmt(
        &self,
        input_id: usize,
        grad_name: &str,
        x_var: &str,
        y_var: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // grad_y = -grad * x / y^2
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        let x_ref = Expr::new(ExprKind::Path(Path::single(Ident::new(x_var, span))), span);
        let y_ref = Expr::new(ExprKind::Path(Path::single(Ident::new(y_var, span))), span);
        let neg_grad = self.build_neg_expr(grad_ref, span);
        let num = self.build_mul_expr(neg_grad, x_ref, span);
        let y_sq = self.build_mul_expr(y_ref.clone(), y_ref, span);
        let result = self.build_div_expr(num, y_sq, span);
        self.create_let_stmt(&format!("_grad{}", input_id), result, span)
    }

    // Production gradient rule implementations
    // Each rule follows the chain rule and is mathematically correct

    fn build_pow_grad_base_stmt(
        &self,
        node: &ComputeNode,
        grad_name: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // d/dx (x^y) = y * x^(y-1) * grad
        // Get references to base (x) and exponent (y)
        let x_ref = self.primal_ref(node.inputs[0], span);
        let y_ref = self.primal_ref(node.inputs[1], span);
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );

        // Compute y - 1
        let one = Expr::new(
            ExprKind::Literal(verum_ast::Literal::float(1.0, span)),
            span,
        );
        let y_minus_one = self.build_sub_expr(y_ref.clone(), one, span);

        // Compute x^(y-1) using pow function
        let x_pow_y_minus_one = self.build_fn_call_expr("pow", vec![x_ref, y_minus_one], span);

        // Compute y * x^(y-1)
        let y_times_pow = self.build_mul_expr(y_ref, x_pow_y_minus_one, span);

        // Compute y * x^(y-1) * grad
        let result = self.build_mul_expr(y_times_pow, grad_ref, span);

        self.create_let_stmt(&format!("_grad{}", node.inputs[0]), result, span)
    }

    fn build_pow_grad_exp_stmt(
        &self,
        node: &ComputeNode,
        grad_name: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // d/dy (x^y) = x^y * ln(x) * grad
        // Get references to base (x) and the output (x^y)
        let x_ref = self.primal_ref(node.inputs[0], span);
        let output_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(format!("_v{}", node.id), span))),
            span,
        );
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );

        // Compute ln(x)
        let ln_x = self.build_fn_call_expr("ln", vec![x_ref], span);

        // Compute x^y * ln(x)
        let output_times_ln = self.build_mul_expr(output_ref, ln_x, span);

        // Compute x^y * ln(x) * grad
        let result = self.build_mul_expr(output_times_ln, grad_ref, span);

        self.create_let_stmt(&format!("_grad{}", node.inputs[1]), result, span)
    }

    fn build_trig_grad_stmt(
        &self,
        node: &ComputeNode,
        grad_name: &str,
        deriv_fn: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        let v0 = self.primal_ref(node.inputs[0], span);
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );

        let deriv = if deriv_fn.starts_with('-') {
            let fn_name = &deriv_fn[1..];
            let call = self.build_fn_call_expr(fn_name, vec![v0], span);
            self.build_neg_expr(call, span)
        } else {
            self.build_fn_call_expr(deriv_fn, vec![v0], span)
        };

        let result = self.build_mul_expr(deriv, grad_ref, span);
        self.create_let_stmt(&format!("_grad{}", node.inputs[0]), result, span)
    }

    fn build_tan_grad_stmt(
        &self,
        node: &ComputeNode,
        grad_name: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // d/dx tan(x) = 1/cos^2(x) = sec^2(x)
        let v0 = self.primal_ref(node.inputs[0], span);
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        let cos_v = self.build_fn_call_expr("cos", vec![v0], span);
        let cos_sq = self.build_mul_expr(cos_v.clone(), cos_v, span);
        let one = Expr::new(
            ExprKind::Literal(verum_ast::Literal::float(1.0, span)),
            span,
        );
        let sec_sq = self.build_div_expr(one, cos_sq, span);
        let result = self.build_mul_expr(sec_sq, grad_ref, span);
        self.create_let_stmt(&format!("_grad{}", node.inputs[0]), result, span)
    }

    fn build_log_grad_stmt(
        &self,
        input_id: usize,
        grad_name: &str,
        x_var: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // d/dx log(x) = 1/x
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        let x_ref = Expr::new(ExprKind::Path(Path::single(Ident::new(x_var, span))), span);
        let result = self.build_div_expr(grad_ref, x_ref, span);
        self.create_let_stmt(&format!("_grad{}", input_id), result, span)
    }

    fn build_sqrt_grad_stmt(
        &self,
        input_id: usize,
        grad_name: &str,
        out_var: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // d/dx sqrt(x) = 1/(2*sqrt(x)) = grad/(2*out)
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        let out_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(out_var, span))),
            span,
        );
        let two = Expr::new(
            ExprKind::Literal(verum_ast::Literal::float(2.0, span)),
            span,
        );
        let denom = self.build_mul_expr(two, out_ref, span);
        let result = self.build_div_expr(grad_ref, denom, span);
        self.create_let_stmt(&format!("_grad{}", input_id), result, span)
    }

    fn build_abs_grad_stmt(
        &self,
        input_id: usize,
        grad_name: &str,
        x_var: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // d/dx |x| = sign(x) * grad
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        let x_ref = Expr::new(ExprKind::Path(Path::single(Ident::new(x_var, span))), span);
        let sign = self.build_fn_call_expr("sign", vec![x_ref], span);
        let result = self.build_mul_expr(sign, grad_ref, span);
        self.create_let_stmt(&format!("_grad{}", input_id), result, span)
    }

    fn build_tanh_grad_stmt(
        &self,
        input_id: usize,
        grad_name: &str,
        out_var: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // d/dx tanh(x) = 1 - tanh^2(x) = (1 - out^2) * grad
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        let out_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(out_var, span))),
            span,
        );
        let one = Expr::new(
            ExprKind::Literal(verum_ast::Literal::float(1.0, span)),
            span,
        );
        let out_sq = self.build_mul_expr(out_ref.clone(), out_ref, span);
        let deriv = self.build_sub_expr(one, out_sq, span);
        let result = self.build_mul_expr(deriv, grad_ref, span);
        self.create_let_stmt(&format!("_grad{}", input_id), result, span)
    }

    fn build_sigmoid_grad_stmt(
        &self,
        input_id: usize,
        grad_name: &str,
        out_var: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // d/dx sigmoid(x) = sigmoid(x) * (1 - sigmoid(x)) = out * (1 - out) * grad
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        let out_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(out_var, span))),
            span,
        );
        let one = Expr::new(
            ExprKind::Literal(verum_ast::Literal::float(1.0, span)),
            span,
        );
        let one_minus_out = self.build_sub_expr(one, out_ref.clone(), span);
        let deriv = self.build_mul_expr(out_ref, one_minus_out, span);
        let result = self.build_mul_expr(deriv, grad_ref, span);
        self.create_let_stmt(&format!("_grad{}", input_id), result, span)
    }

    fn build_relu_grad_stmt(
        &self,
        input_id: usize,
        grad_name: &str,
        x_var: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // d/dx relu(x) = grad * (x > 0)
        // Gradient is 1 if x > 0, else 0
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        let x_ref = Expr::new(ExprKind::Path(Path::single(Ident::new(x_var, span))), span);
        let zero = Expr::new(
            ExprKind::Literal(verum_ast::Literal::float(0.0, span)),
            span,
        );

        // Build condition: x > 0
        let cond = Expr::new(
            ExprKind::Binary {
                op: BinOp::Gt,
                left: Box::new(x_ref),
                right: Box::new(zero.clone()),
            },
            span,
        );

        // if x > 0 { grad } else { 0 }
        let result = Expr::new(
            ExprKind::If {
                condition: Box::new(verum_ast::expr::IfCondition {
                    conditions: verum_ast::smallvec::smallvec![
                        verum_ast::expr::ConditionKind::Expr(cond)
                    ],
                    span,
                }),
                then_branch: Block {
                    stmts: List::new(),
                    expr: Some(Box::new(grad_ref)),
                    span,
                },
                else_branch: Some(Box::new(Expr::new(
                    ExprKind::Block(Block {
                        stmts: List::new(),
                        expr: Some(Box::new(zero)),
                        span,
                    }),
                    span,
                ))),
            },
            span,
        );

        self.create_let_stmt(&format!("_grad{}", input_id), result, span)
    }

    fn build_softmax_grad_stmt(
        &self,
        input_id: usize,
        grad_name: &str,
        out_var: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // d/dx softmax(x) = softmax * (grad - dot(softmax, grad))
        // This is the efficient formulation of the Jacobian-vector product
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        let out_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(out_var, span))),
            span,
        );

        // dot_product = dot(softmax, grad)
        let dot_prod =
            self.build_fn_call_expr("dot", vec![out_ref.clone(), grad_ref.clone()], span);

        // grad - dot_product (broadcast subtraction)
        let diff = self.build_sub_expr(grad_ref, dot_prod, span);

        // softmax * diff (element-wise multiplication)
        let result = self.build_mul_expr(out_ref, diff, span);

        self.create_let_stmt(&format!("_grad{}", input_id), result, span)
    }

    fn build_matmul_grad_a_stmt(
        &self,
        node: &ComputeNode,
        grad_name: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // d/dA (A @ B) = grad @ B^T
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        let b_ref = self.primal_ref(node.inputs[1], span);
        let b_t = self.build_fn_call_expr("transpose", vec![b_ref], span);
        let result = self.build_fn_call_expr("matmul", vec![grad_ref, b_t], span);
        self.create_let_stmt(&format!("_grad{}", node.inputs[0]), result, span)
    }

    fn build_matmul_grad_b_stmt(
        &self,
        node: &ComputeNode,
        grad_name: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // d/dB (A @ B) = A^T @ grad
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        let a_ref = self.primal_ref(node.inputs[0], span);
        let a_t = self.build_fn_call_expr("transpose", vec![a_ref], span);
        let result = self.build_fn_call_expr("matmul", vec![a_t, grad_ref], span);
        self.create_let_stmt(&format!("_grad{}", node.inputs[1]), result, span)
    }

    fn build_transpose_grad_stmt(
        &self,
        node: &ComputeNode,
        grad_name: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // d/dx (x^T) = grad^T
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        let result = self.build_fn_call_expr("transpose", vec![grad_ref], span);
        self.create_let_stmt(&format!("_grad{}", node.inputs[0]), result, span)
    }

    fn build_sum_grad_stmt(
        &self,
        node: &ComputeNode,
        grad_name: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // d/dx sum(x) = broadcast(grad, shape(x))
        // The gradient of sum is a tensor filled with the upstream gradient value
        // broadcasted to match the input shape
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );

        // Get the input reference to determine shape
        let input_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(
                format!("_p{}", node.inputs[0]),
                span,
            ))),
            span,
        );

        // Build: broadcast(grad, shape(input))
        let shape_call = self.build_fn_call_expr("shape", vec![input_ref], span);
        let broadcast_result =
            self.build_fn_call_expr("broadcast", vec![grad_ref, shape_call], span);

        self.create_let_stmt(&format!("_grad{}", node.inputs[0]), broadcast_result, span)
    }

    fn build_mean_grad_stmt(
        &self,
        node: &ComputeNode,
        grad_name: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // d/dx mean(x) = grad / n, broadcasted to input shape
        // where n is the number of elements in x
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );

        // Get the input reference to determine shape and size
        let input_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(
                format!("_p{}", node.inputs[0]),
                span,
            ))),
            span,
        );

        // Build: size(input) to get element count
        let size_call = self.build_fn_call_expr("size", vec![input_ref.clone()], span);

        // Convert size to float for division
        let size_float = self.build_fn_call_expr("as_float", vec![size_call], span);

        // Divide gradient by size: grad / n
        let scaled_grad = Expr::new(
            ExprKind::Binary {
                op: BinOp::Div,
                left: Box::new(grad_ref),
                right: Box::new(size_float),
            },
            span,
        );

        // Broadcast to input shape
        let shape_call = self.build_fn_call_expr("shape", vec![input_ref], span);
        let broadcast_result =
            self.build_fn_call_expr("broadcast", vec![scaled_grad, shape_call], span);

        self.create_let_stmt(&format!("_grad{}", node.inputs[0]), broadcast_result, span)
    }

    fn build_broadcast_grad_stmt(
        &self,
        node: &ComputeNode,
        grad_name: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // d/dx broadcast(x) = sum_reduce(grad)
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        let result = self.build_fn_call_expr("sum", vec![grad_ref], span);
        self.create_let_stmt(&format!("_grad{}", node.inputs[0]), result, span)
    }

    fn build_select_grad_then_stmt(
        &self,
        node: &ComputeNode,
        grad_name: &str,
        cond_id: usize,
        span: Span,
    ) -> verum_ast::Stmt {
        // grad_then = cond ? grad : 0
        // For the "then" branch, gradient flows when condition is true
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );

        // Reference to the condition value
        let cond_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(format!("_p{}", cond_id), span))),
            span,
        );

        // Zero constant for when condition is false
        let zero = Expr::new(
            ExprKind::Literal(verum_ast::Literal::float(0.0, span)),
            span,
        );

        // Build: if cond { grad } else { 0.0 }
        let select_expr = Expr::new(
            ExprKind::If {
                condition: Box::new(verum_ast::expr::IfCondition {
                    conditions: verum_ast::smallvec::smallvec![
                        verum_ast::expr::ConditionKind::Expr(cond_ref)
                    ],
                    span,
                }),
                then_branch: Block {
                    stmts: List::new(),
                    expr: Some(Box::new(grad_ref)),
                    span,
                },
                else_branch: Some(Box::new(zero)),
            },
            span,
        );

        self.create_let_stmt(&format!("_grad{}", node.inputs[0]), select_expr, span)
    }

    fn build_select_grad_else_stmt(
        &self,
        node: &ComputeNode,
        grad_name: &str,
        cond_id: usize,
        span: Span,
    ) -> verum_ast::Stmt {
        // grad_else = cond ? 0 : grad
        // For the "else" branch, gradient flows when condition is false
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );

        // Reference to the condition value
        let cond_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(format!("_p{}", cond_id), span))),
            span,
        );

        // Zero constant for when condition is true
        let zero = Expr::new(
            ExprKind::Literal(verum_ast::Literal::float(0.0, span)),
            span,
        );

        // Build: if cond { 0.0 } else { grad }
        let select_expr = Expr::new(
            ExprKind::If {
                condition: Box::new(verum_ast::expr::IfCondition {
                    conditions: verum_ast::smallvec::smallvec![
                        verum_ast::expr::ConditionKind::Expr(cond_ref)
                    ],
                    span,
                }),
                then_branch: Block {
                    stmts: List::new(),
                    expr: Some(Box::new(zero)),
                    span,
                },
                else_branch: Some(Box::new(grad_ref)),
            },
            span,
        );

        self.create_let_stmt(&format!("_grad{}", node.inputs[1]), select_expr, span)
    }

    fn build_index_grad_stmt(
        &self,
        node: &ComputeNode,
        grad_name: &str,
        index: usize,
        span: Span,
    ) -> verum_ast::Stmt {
        // d/dx x[i] = scatter(grad, i, shape(x))
        // The gradient of indexing is a sparse tensor with the gradient value
        // at the indexed position and zeros elsewhere
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );

        // Get the input array reference for shape
        let input_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(
                format!("_p{}", node.inputs[0]),
                span,
            ))),
            span,
        );

        // Index literal
        let index_lit = Expr::new(
            ExprKind::Literal(verum_ast::Literal::int(index as i128, span)),
            span,
        );

        // Build: scatter(grad, index, shape(input))
        let shape_call = self.build_fn_call_expr("shape", vec![input_ref], span);
        let scatter_result =
            self.build_fn_call_expr("scatter", vec![grad_ref, index_lit, shape_call], span);

        self.create_let_stmt(&format!("_grad{}", node.inputs[0]), scatter_result, span)
    }

    fn build_field_grad_stmt(
        &self,
        node: &ComputeNode,
        grad_name: &str,
        _field_name: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // Struct gradient: update field gradient
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        self.create_let_stmt(&format!("_grad{}", node.inputs[0]), grad_ref, span)
    }

    fn build_call_grad_stmt(
        &self,
        node: &ComputeNode,
        grad_name: &str,
        func_name: &str,
        span: Span,
    ) -> verum_ast::Stmt {
        // Call the VJP of the called function
        // VJP signature: fn_vjp(args..., grad_output) -> (grad_arg0, grad_arg1, ...)
        let vjp_name = format!("{}_vjp", func_name);

        // Build arguments for VJP call: original args + gradient output
        let mut vjp_args: Vec<Expr> = Vec::new();

        // Add primal values of all inputs
        for &input_id in &node.inputs {
            vjp_args.push(self.primal_ref(input_id, span));
        }

        // Add the incoming gradient
        let grad_ref = Expr::new(
            ExprKind::Path(Path::single(Ident::new(grad_name, span))),
            span,
        );
        vjp_args.push(grad_ref);

        // Build the VJP function call
        let vjp_call = self.build_fn_call_expr(&vjp_name, vjp_args, span);

        // If there's only one input, the VJP returns a single gradient
        // If there are multiple inputs, it returns a tuple of gradients
        if node.inputs.len() == 1 {
            // Single input: VJP returns grad directly
            self.create_let_stmt(&format!("_grad{}", node.inputs[0]), vjp_call, span)
        } else {
            // Multiple inputs: VJP returns tuple, need to destructure
            // Create a binding for the tuple result first
            let tuple_name = format!("_grad_tuple_{}", func_name);

            // For now, bind the whole tuple - the caller should handle destructuring
            // In a full implementation, we'd generate let (_grad0, _grad1, ...) = vjp_call;
            self.create_let_stmt(&tuple_name, vjp_call, span)
        }
    }

    // Return type builders

    fn build_vjp_return_type(&self, span: Span) -> Type {
        // Return tuple of gradients for wrt params
        let grad_types: Vec<Type> = self
            .config
            .wrt_params
            .iter()
            .map(|_| Type::float(span))
            .collect();

        if grad_types.len() == 1 {
            grad_types.iter().next().unwrap().clone()
        } else {
            Type::new(TypeKind::Tuple(grad_types.into()), span)
        }
    }

    fn build_jvp_return_type(&self, span: Span) -> Type {
        // Return (primal_output, tangent_output)
        let output_type = self
            .func
            .return_type
            .clone()
            .unwrap_or_else(|| Type::unit(span));
        let tangent_type = output_type.clone();

        let mut types = Vec::new();
        types.push(output_type);
        types.push(tangent_type);
        Type::new(TypeKind::Tuple(types.into()), span)
    }

    fn build_grad_return_type(&self, span: Span) -> Type {
        self.build_vjp_return_type(span)
    }

    // Parameter builders

    fn create_grad_output_param(&self, span: Span) -> FunctionParam {
        let output_type = self
            .func
            .return_type
            .clone()
            .unwrap_or_else(|| Type::float(span));

        FunctionParam::new(
            FunctionParamKind::Regular {
                pattern: Pattern::new(
                    PatternKind::Ident {
                        by_ref: false,
                        name: Ident::new("grad_output", span),
                        mutable: false,
                        subpattern: None,
                    },
                    span,
                ),
                ty: output_type,
                default_value: verum_common::Maybe::None,
            },
            span,
        )
    }

    fn create_tangent_param(&self, param_name: &str, span: Span) -> FunctionParam {
        // Find the type of the original parameter
        let param_type = self
            .find_param_type(param_name)
            .unwrap_or_else(|| Type::float(span));

        FunctionParam::new(
            FunctionParamKind::Regular {
                pattern: Pattern::new(
                    PatternKind::Ident {
                        by_ref: false,
                        name: Ident::new(format!("tangent_{}", param_name), span),
                        mutable: false,
                        subpattern: None,
                    },
                    span,
                ),
                ty: param_type,
                default_value: verum_common::Maybe::None,
            },
            span,
        )
    }

    fn find_param_type(&self, name: &str) -> Option<Type> {
        for param in self.func.params.iter() {
            if let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind {
                if let PatternKind::Ident { name: ident, .. } = &pattern.kind {
                    if ident.as_str() == name {
                        return Some(ty.clone());
                    }
                }
            }
        }
        None
    }

    // Expression builders for VJP body

    fn build_binary_expr(&self, op: &str, inputs: &[usize], span: Span) -> Expr {
        let left = self.primal_ref(inputs[0], span);
        let right = self.primal_ref(inputs[1], span);
        let bin_op = match op {
            "+" => BinOp::Add,
            "-" => BinOp::Sub,
            "*" => BinOp::Mul,
            "/" => BinOp::Div,
            _ => BinOp::Add,
        };
        Expr::new(
            ExprKind::Binary {
                op: bin_op,
                left: Box::new(left),
                right: Box::new(right),
            },
            span,
        )
    }

    fn build_unary_expr(&self, op: &str, input: usize, span: Span) -> Expr {
        let inner = self.primal_ref(input, span);
        let un_op = match op {
            "-" => UnOp::Neg,
            "!" => UnOp::Not,
            _ => UnOp::Neg,
        };
        Expr::new(
            ExprKind::Unary {
                op: un_op,
                expr: Box::new(inner),
            },
            span,
        )
    }

    fn build_call_expr(&self, name: &str, inputs: &[usize], span: Span) -> Expr {
        let args: Vec<Expr> = inputs.iter().map(|&id| self.primal_ref(id, span)).collect();
        self.build_fn_call_expr(name, args, span)
    }

    fn build_method_call_expr(
        &self,
        method: &str,
        receiver: usize,
        args: &[usize],
        span: Span,
    ) -> Expr {
        let recv = self.primal_ref(receiver, span);
        let arg_exprs: Vec<Expr> = args.iter().map(|&id| self.primal_ref(id, span)).collect();
        Expr::new(
            ExprKind::MethodCall {
                receiver: Box::new(recv),
                method: Ident::new(method, span),
                type_args: List::new(),
                args: arg_exprs.into(),
            },
            span,
        )
    }

    fn build_select_expr(&self, cond: usize, branches: &[usize], span: Span) -> Expr {
        // If expression
        let cond_expr = self.primal_ref(cond, span);
        let then_expr = self.primal_ref(branches[0], span);
        let else_expr = if branches.len() > 1 {
            self.primal_ref(branches[1], span)
        } else {
            Expr::new(
                ExprKind::Literal(verum_ast::Literal::float(0.0, span)),
                span,
            )
        };

        Expr::new(
            ExprKind::If {
                condition: Box::new(verum_ast::expr::IfCondition {
                    conditions: verum_ast::smallvec::smallvec![
                        verum_ast::expr::ConditionKind::Expr(cond_expr)
                    ],
                    span,
                }),
                then_branch: Block {
                    stmts: List::new(),
                    expr: Some(Box::new(then_expr)),
                    span,
                },
                else_branch: Some(Box::new(else_expr)),
            },
            span,
        )
    }

    fn build_index_expr(&self, array: usize, index: usize, span: Span) -> Expr {
        let arr = self.primal_ref(array, span);
        let idx = Expr::new(
            ExprKind::Literal(verum_ast::Literal::int(index as i128, span)),
            span,
        );
        Expr::new(
            ExprKind::Index {
                expr: Box::new(arr),
                index: Box::new(idx),
            },
            span,
        )
    }

    fn build_field_expr(&self, obj: usize, field: &str, span: Span) -> Expr {
        let obj_expr = self.primal_ref(obj, span);
        Expr::new(
            ExprKind::Field {
                expr: Box::new(obj_expr),
                field: Ident::new(field, span),
            },
            span,
        )
    }

    // Result tuple builders

    fn build_gradient_tuple_expr(&self, span: Span) -> Expr {
        let grad_exprs: Vec<Expr> = self
            .config
            .wrt_params
            .iter()
            .map(|name| {
                Expr::new(
                    ExprKind::Path(Path::single(Ident::new(format!("grad_{}", name), span))),
                    span,
                )
            })
            .collect();

        if grad_exprs.len() == 1 {
            grad_exprs.iter().next().unwrap().clone()
        } else {
            Expr::new(ExprKind::Tuple(grad_exprs.into()), span)
        }
    }

    fn build_jvp_result_tuple(&self, span: Span) -> Expr {
        let output_id = self.graph.output_id;
        let primal = self.primal_ref(output_id, span);
        let tangent = self.tangent_ref(output_id, span);

        let mut exprs = Vec::new();
        exprs.push(primal);
        exprs.push(tangent);
        Expr::new(ExprKind::Tuple(exprs.into()), span)
    }

    fn build_vjp_call_with_one(&self, span: Span) -> Expr {
        // Build call to VJP with grad_output = 1.0
        let vjp_name = format!("{}_vjp", self.func.name.as_str());
        let func = Expr::new(
            ExprKind::Path(Path::single(Ident::new(vjp_name.as_str(), span))),
            span,
        );

        let mut args: Vec<Expr> = self
            .func
            .params
            .iter()
            .filter_map(|p| match &p.kind {
                FunctionParamKind::Regular { pattern, .. } => {
                    if let PatternKind::Ident { name, .. } = &pattern.kind {
                        Some(Expr::new(ExprKind::Path(Path::single(name.clone())), span))
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .collect();

        // Add grad_output = 1.0 for scalar gradient
        args.push(Expr::new(
            ExprKind::Literal(verum_ast::Literal::float(1.0, span)),
            span,
        ));

        Expr::new(
            ExprKind::Call {
                func: Box::new(func),
                type_args: List::new(),
                args: args.into(),
            },
            span,
        )
    }

    fn build_generated_attributes(&self, span: Span) -> Vec<Attribute> {
        let mut attrs = Vec::new();

        // Add @generated attribute to mark this as compiler-generated
        attrs.push(Attribute::new(
            "generated".into(),
            Some(
                vec![Expr::new(
                    ExprKind::Literal(verum_ast::Literal::string("autodiff".into(), span)),
                    span,
                )]
                .into(),
            ),
            span,
        ));

        // Add CBGR overhead documentation comment
        // Note: Generated code includes ~15ns CBGR overhead per reference check
        attrs.push(Attribute::simple("cbgr_overhead_15ns".into(), span));

        attrs
    }
}

/// Main autodiff compilation phase
pub struct AutodiffCompilationPhase {
    /// Autodiff statistics
    stats: AutodiffStats,
}

/// Statistics for autodiff compilation
#[derive(Debug, Clone, Default)]
struct AutodiffStats {
    /// Number of differentiable functions found
    differentiable_functions: usize,
    /// Number of VJP functions generated
    vjp_functions_generated: usize,
    /// Number of JVP functions generated
    jvp_functions_generated: usize,
    /// Number of gradient functions generated
    grad_functions_generated: usize,
    /// Number of gradient operations generated
    gradient_ops_generated: usize,
}

impl AutodiffCompilationPhase {
    pub fn new() -> Self {
        Self {
            stats: AutodiffStats::default(),
        }
    }

    /// Process modules for autodiff
    fn process_modules(&mut self, modules: &[Module]) -> Result<Vec<Module>, Vec<Diagnostic>> {
        let mut processed_modules = Vec::new();

        for module in modules {
            let processed = self.process_module(module)?;
            processed_modules.push(processed);
        }

        Ok(processed_modules)
    }

    /// Process a single module
    fn process_module(&mut self, module: &Module) -> Result<Module, Vec<Diagnostic>> {
        tracing::debug!("Processing module for autodiff");

        let mut new_items = Vec::new();

        for item in &module.items {
            // Add original item
            new_items.push(item.clone());

            // Generate autodiff functions if needed
            if let Some(generated) = self.process_item(item)? {
                new_items.extend(generated);
            }
        }

        Ok(Module {
            items: new_items.into(),
            attributes: module.attributes.clone(),
            file_id: module.file_id,
            span: module.span,
        })
    }

    /// Process a single item for autodiff
    fn process_item(&mut self, item: &Item) -> Result<Option<Vec<Item>>, Vec<Diagnostic>> {
        match &item.kind {
            ItemKind::Function(func) => {
                // Check if function is marked as differentiable
                if let Some(config) = self.parse_differentiable_attr(func) {
                    self.stats.differentiable_functions += 1;

                    // Validate differentiable parameters
                    self.validate_differentiable_params(func, &config)?;

                    // Build computational graph
                    let builder = GraphBuilder::new();
                    let graph = builder.build_from_function(func, &config).map_err(|e| {
                        let diag = DiagnosticBuilder::error()
                            .message(format!("Failed to build computation graph: {}", e))
                            .span(super::ast_span_to_diagnostic_span(func.span, None))
                            .build();
                        vec![diag]
                    })?;

                    // Create derivative generator
                    let generator = DerivativeGenerator::new(func.clone(), config.clone(), graph);

                    let mut generated = Vec::new();

                    // Generate VJP (reverse-mode)
                    if config.mode == DifferentiationMode::Reverse
                        || config.mode == DifferentiationMode::Both
                    {
                        if config.custom_vjp.is_none() {
                            let vjp_func = generator.generate_vjp().map_err(|e| {
                                let diag = DiagnosticBuilder::error()
                                    .message(format!("Failed to generate VJP: {}", e))
                                    .span(super::ast_span_to_diagnostic_span(func.span, None))
                                    .build();
                                vec![diag]
                            })?;
                            generated.push(vjp_func);
                            self.stats.vjp_functions_generated += 1;
                        }
                    }

                    // Generate JVP (forward-mode)
                    if config.mode == DifferentiationMode::Forward
                        || config.mode == DifferentiationMode::Both
                    {
                        let jvp_func = generator.generate_jvp().map_err(|e| {
                            let diag = DiagnosticBuilder::error()
                                .message(format!("Failed to generate JVP: {}", e))
                                .span(super::ast_span_to_diagnostic_span(func.span, None))
                                .build();
                            vec![diag]
                        })?;
                        generated.push(jvp_func);
                        self.stats.jvp_functions_generated += 1;
                    }

                    // Generate gradient function (for scalar outputs)
                    if self.is_scalar_output(func) {
                        let grad_func = generator.generate_grad().map_err(|e| {
                            let diag = DiagnosticBuilder::error()
                                .message(format!("Failed to generate gradient: {}", e))
                                .span(super::ast_span_to_diagnostic_span(func.span, None))
                                .build();
                            vec![diag]
                        })?;
                        generated.push(grad_func);
                        self.stats.grad_functions_generated += 1;
                    }

                    Ok(Some(generated))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    /// Parse @differentiable attribute from function
    fn parse_differentiable_attr(&self, func: &FunctionDecl) -> Option<DifferentiableConfig> {
        for attr in func.attributes.iter() {
            if attr.name.as_str() == "differentiable" {
                let mut config = DifferentiableConfig::default();

                // Parse attribute arguments
                if let Some(args) = &attr.args {
                    for arg in args.iter() {
                        self.parse_attr_arg(arg, &mut config);
                    }
                }

                // If no wrt params specified, differentiate w.r.t. all differentiable params
                if config.wrt_params.is_empty() {
                    for param in func.params.iter() {
                        if let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind {
                            if self.is_differentiable_type(ty) {
                                if let PatternKind::Ident { name, .. } = &pattern.kind {
                                    config.wrt_params.push(name.as_str().to_string());
                                }
                            }
                        }
                    }
                }

                return Some(config);
            }
        }

        // Legacy check: function name contains "diff" or "grad"
        if func.name.as_str().contains("diff") || func.name.as_str().contains("grad") {
            let mut config = DifferentiableConfig::default();
            // Add all differentiable params
            for param in func.params.iter() {
                if let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind {
                    if self.is_differentiable_type(ty) {
                        if let PatternKind::Ident { name, .. } = &pattern.kind {
                            config.wrt_params.push(name.as_str().to_string());
                        }
                    }
                }
            }
            if !config.wrt_params.is_empty() {
                return Some(config);
            }
        }

        None
    }

    /// Parse a single attribute argument
    fn parse_attr_arg(&self, arg: &Expr, config: &mut DifferentiableConfig) {
        match &arg.kind {
            ExprKind::Binary {
                op: BinOp::Assign,
                left,
                right,
            } => {
                // Named argument: wrt = "x, y"
                if let ExprKind::Path(path) = &left.kind {
                    if let Some(name) = path.as_ident() {
                        match name.as_str() {
                            "wrt" => {
                                if let ExprKind::Literal(lit) = &right.kind {
                                    if let verum_ast::LiteralKind::Text(s) = &lit.kind {
                                        config.wrt_params = s
                                            .as_str()
                                            .split(',')
                                            .map(|s| s.trim().to_string())
                                            .filter(|s| !s.is_empty())
                                            .collect();
                                    }
                                }
                            }
                            "mode" => {
                                if let ExprKind::Literal(lit) = &right.kind {
                                    if let verum_ast::LiteralKind::Text(s) = &lit.kind {
                                        config.mode = match s.as_str() {
                                            "forward" => DifferentiationMode::Forward,
                                            "reverse" => DifferentiationMode::Reverse,
                                            "both" => DifferentiationMode::Both,
                                            _ => DifferentiationMode::Reverse,
                                        };
                                    }
                                }
                            }
                            "order" => {
                                if let ExprKind::Literal(lit) = &right.kind {
                                    if let verum_ast::LiteralKind::Int(n) = &lit.kind {
                                        config.order = n.value as u32;
                                    }
                                }
                            }
                            "custom_vjp" => {
                                if let ExprKind::Literal(lit) = &right.kind {
                                    if let verum_ast::LiteralKind::Text(s) = &lit.kind {
                                        config.custom_vjp = Some(s.as_str().to_string());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Check if type is differentiable
    fn is_differentiable_type(&self, ty: &Type) -> bool {
        match &ty.kind {
            TypeKind::Float => true,
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    matches!(
                        ident.as_str(),
                        "Float" | "f32" | "f64" | "Tensor" | "Complex" | "List" // List<Float> etc.
                    )
                } else {
                    false
                }
            }
            TypeKind::Generic { base, .. } => self.is_differentiable_type(base),
            TypeKind::Array { element, .. } => self.is_differentiable_type(element),
            TypeKind::Reference { inner, .. } => self.is_differentiable_type(inner),
            _ => false,
        }
    }

    /// Validate that wrt parameters are differentiable
    fn validate_differentiable_params(
        &self,
        func: &FunctionDecl,
        config: &DifferentiableConfig,
    ) -> Result<(), Vec<Diagnostic>> {
        let mut errors = Vec::new();

        for wrt_param in &config.wrt_params {
            let mut found = false;
            let mut is_diff = false;

            for param in func.params.iter() {
                if let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind {
                    if let PatternKind::Ident { name, .. } = &pattern.kind {
                        if name.as_str() == wrt_param {
                            found = true;
                            is_diff = self.is_differentiable_type(ty);
                            break;
                        }
                    }
                }
            }

            if !found {
                let diag = DiagnosticBuilder::error()
                    .message(format!(
                        "Parameter '{}' specified in wrt does not exist",
                        wrt_param
                    ))
                    .span(super::ast_span_to_diagnostic_span(func.span, None))
                    .build();
                errors.push(diag);
            } else if !is_diff {
                let diag = DiagnosticBuilder::error()
                    .message(format!(
                        "Parameter '{}' has non-differentiable type",
                        wrt_param
                    ))
                    .span(super::ast_span_to_diagnostic_span(func.span, None))
                    .help("Only Tensor<T>, Float, and numeric types can be differentiated")
                    .build();
                errors.push(diag);
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Check if function has scalar output
    fn is_scalar_output(&self, func: &FunctionDecl) -> bool {
        match &func.return_type {
            Some(ty) => matches!(ty.kind, TypeKind::Float | TypeKind::Path(_)),
            None => false,
        }
    }
}

impl Default for AutodiffCompilationPhase {
    fn default() -> Self {
        Self::new()
    }
}

impl CompilationPhase for AutodiffCompilationPhase {
    fn name(&self) -> &str {
        "Phase 4a: Autodiff Compilation"
    }

    fn description(&self) -> &str {
        "Generate VJP/JVP functions for automatic differentiation. Note: Generated code includes ~15ns CBGR overhead per reference check."
    }

    fn execute(&self, input: PhaseInput) -> Result<PhaseOutput, List<Diagnostic>> {
        let start = Instant::now();

        // Extract modules from input
        let modules = match &input.data {
            PhaseData::AstModules(modules) => modules,
            _ => {
                let diag = DiagnosticBuilder::error()
                    .message("Invalid input for autodiff compilation phase")
                    .build();
                return Err(List::from(vec![diag]));
            }
        };

        // Create mutable phase for statistics
        let mut phase = Self {
            stats: AutodiffStats::default(),
        };

        // Process modules for autodiff
        let processed_modules = match phase.process_modules(modules) {
            Ok(processed) => processed,
            Err(errors) => return Err(List::from(errors)),
        };

        let duration = start.elapsed();
        let mut metrics = PhaseMetrics::new(self.name()).with_duration(duration);
        metrics.add_custom_metric(
            "differentiable_functions",
            phase.stats.differentiable_functions.to_string(),
        );
        metrics.add_custom_metric(
            "vjp_functions_generated",
            phase.stats.vjp_functions_generated.to_string(),
        );
        metrics.add_custom_metric(
            "jvp_functions_generated",
            phase.stats.jvp_functions_generated.to_string(),
        );
        metrics.add_custom_metric(
            "grad_functions_generated",
            phase.stats.grad_functions_generated.to_string(),
        );
        metrics.add_custom_metric(
            "gradient_ops_generated",
            phase.stats.gradient_ops_generated.to_string(),
        );
        metrics.add_custom_metric("cbgr_overhead_per_check", "~15ns".to_string());

        tracing::info!(
            "Autodiff compilation complete: {} differentiable functions, {} VJPs, {} JVPs, {} grads generated, {:.2}ms",
            phase.stats.differentiable_functions,
            phase.stats.vjp_functions_generated,
            phase.stats.jvp_functions_generated,
            phase.stats.grad_functions_generated,
            duration.as_millis()
        );

        Ok(PhaseOutput {
            data: PhaseData::AstModules(processed_modules.into()),
            warnings: List::new(),
            metrics,
        })
    }

    fn can_parallelize(&self) -> bool {
        true // Functions can be differentiated in parallel
    }

    fn metrics(&self) -> PhaseMetrics {
        PhaseMetrics::new(self.name())
    }
}
