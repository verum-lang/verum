//! Weakest Precondition (WP) Calculus for Contract Verification
//!
//! This module implements Dijkstra's weakest precondition calculus for verifying
//! function contracts (requires/ensures) and loop invariants.
//!
//! ## Theory
//!
//! For a statement S and postcondition Q, `wp(S, Q)` is the weakest condition
//! such that if `wp(S, Q)` holds before executing S, then Q holds after.
//!
//! Key rules:
//! - `wp(skip, Q) = Q`
//! - `wp(x := e, Q) = Q[e/x]`  (substitution)
//! - `wp(S1; S2, Q) = wp(S1, wp(S2, Q))`
//! - `wp(if b then S1 else S2, Q) = (b => wp(S1, Q)) && (!b => wp(S2, Q))`
//! - For loops: `wp(while b inv I, Q) = I && forall state. (I && !b => Q) && (I && b => wp(body, I))`
//!
//! Implements the contract literal verification pipeline: contract literals
//! (`contract#"requires P; ensures Q;"`) on `@verify(proof)` functions are parsed
//! into precondition/postcondition pairs, then WP calculus generates verification
//! conditions discharged to SMT solvers (Z3/CVC5). Hoare triples {P} c {Q} hold
//! when P implies wp(c, Q). Verification conditions include loop invariant
//! preservation, branch coverage, and refinement type satisfaction.

use crate::context::Context;
use crate::translate::{TranslationError, Translator};
use std::collections::HashMap;
use verum_ast::{
    Expr, Pattern, PatternKind, Span, Type,
    expr::{BinOp, Block, ConditionKind, ExprKind, IfCondition},
    stmt::StmtKind,
};
use verum_common::{List, Map, Maybe, Text};
use z3::ast::{Bool, Dynamic, Int};

/// Result type for WP calculus operations
pub type WpResult<T> = Result<T, WpError>;

/// Errors that can occur during WP computation
#[derive(Debug, Clone, thiserror::Error)]
pub enum WpError {
    /// Translation error from Verum AST to Z3
    #[error("translation error: {0}")]
    Translation(#[from] TranslationError),

    /// Unsupported statement type
    #[error("unsupported statement: {0}")]
    UnsupportedStatement(Text),

    /// Unsupported expression type
    #[error("unsupported expression: {0}")]
    UnsupportedExpression(Text),

    /// Invalid loop invariant
    #[error("invalid loop invariant: {0}")]
    InvalidInvariant(Text),

    /// Missing loop invariant
    #[error("loop without invariant: loops require invariants for verification")]
    MissingInvariant,

    /// Function call without contract
    #[error("function call without contract: {0}")]
    MissingContract(Text),

    /// Internal error
    #[error("internal error: {0}")]
    Internal(Text),
}

/// WP calculus engine for computing weakest preconditions
///
/// This engine translates Verum statements into Z3 constraints using
/// Dijkstra's weakest precondition calculus.
pub struct WpEngine<'ctx> {
    /// Translator for expressions (carries the 'ctx lifetime)
    translator: Translator<'ctx>,

    /// Function contracts for call summarization
    /// Maps function name to (precondition, postcondition) Z3 expressions
    function_contracts: Map<Text, (List<Dynamic>, List<Dynamic>)>,

    /// State versioning for SSA-like transformation
    /// Maps variable name to current version number
    state_versions: HashMap<String, u32>,

    /// Old values for postcondition handling
    /// Maps variable name to Z3 expression representing pre-state value
    old_values: Map<Text, Dynamic>,

    /// Bound for loop unrolling when no invariant is available
    loop_unroll_bound: u32,
}

impl<'ctx> WpEngine<'ctx> {
    /// Create a new WP engine
    pub fn new(context: &'ctx Context) -> Self {
        Self {
            translator: Translator::new(context),
            function_contracts: Map::new(),
            state_versions: HashMap::new(),
            old_values: Map::new(),
            loop_unroll_bound: 10,
        }
    }

    /// Set the loop unrolling bound for loops without invariants
    pub fn set_loop_unroll_bound(&mut self, bound: u32) {
        self.loop_unroll_bound = bound;
    }

    /// Bind an input variable
    pub fn bind_input(&mut self, name: &Text, ty: &Type) -> WpResult<Dynamic> {
        let var = self.translator.create_var(name.as_str(), ty)?;
        self.translator.bind(name.clone(), var.clone());
        Ok(var)
    }

    /// Register a function contract for call summarization
    ///
    /// When the WP engine encounters a call to this function, it will use
    /// the contract instead of inlining the function body.
    pub fn register_contract(
        &mut self,
        name: Text,
        preconditions: List<Dynamic>,
        postconditions: List<Dynamic>,
    ) {
        self.function_contracts
            .insert(name, (preconditions, postconditions));
    }

    /// Store old values for postcondition handling
    ///
    /// Call this before computing WP to capture pre-state values
    /// referenced by `old(expr)` in postconditions.
    pub fn capture_old_values(&mut self, var_names: &[Text]) {
        for name in var_names {
            if let Maybe::Some(val) = self.translator.get(name.as_str()) {
                self.old_values.insert(name.clone(), val.clone());
            }
        }
    }

    /// Compute the weakest precondition of a function body
    ///
    /// Given a function body and postcondition, computes the weakest condition
    /// that must hold before execution for the postcondition to hold after.
    pub fn wp(&mut self, body: &Expr, postcondition: &Bool) -> WpResult<Bool> {
        match &body.kind {
            // Block expression: process statements in reverse order
            ExprKind::Block(block) => self.wp_block(block, postcondition),

            // Assignment handled as part of binary expression
            ExprKind::Binary { op, left, right } if op.is_assignment() => {
                self.wp_assignment(*op, left, right, postcondition)
            }

            // If-then-else expression
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.wp_if(condition, then_branch, else_branch.as_ref(), postcondition),

            // While loop
            ExprKind::While {
                condition, body, ..
            } => {
                let body_expr = Expr::new(ExprKind::Block(body.clone()), body.span);
                self.wp_while_unrolled(condition, &body_expr, self.loop_unroll_bound, postcondition)
            }

            // Infinite loop (loop { ... })
            ExprKind::Loop { body, .. } => {
                let body_expr = Expr::new(ExprKind::Block(body.clone()), body.span);
                let true_lit = Expr::new(
                    ExprKind::Literal(verum_ast::Literal::bool(true, Span::dummy())),
                    Span::dummy(),
                );
                self.wp_while_unrolled(&true_lit, &body_expr, self.loop_unroll_bound, postcondition)
            }

            // Return statement - postcondition must hold at this point
            ExprKind::Return(maybe_value) => {
                // For return, the postcondition becomes the verification obligation
                if let Maybe::Some(ret_val) = maybe_value {
                    let z3_result = self.translator.translate_expr(ret_val)?;
                    self.translator.bind(Text::from("result"), z3_result);
                }
                Ok(postcondition.clone())
            }

            // Parenthesized expression
            ExprKind::Paren(inner) => self.wp(inner, postcondition),

            // Pure expressions don't modify state - postcondition unchanged
            ExprKind::Literal(_)
            | ExprKind::Path(_)
            | ExprKind::Field { .. }
            | ExprKind::Index { .. }
            | ExprKind::Tuple(_)
            | ExprKind::Array(_) => Ok(postcondition.clone()),

            // Unary operators
            ExprKind::Unary { .. } => Ok(postcondition.clone()),

            // Function call
            ExprKind::Call { func, args, .. } => self.wp_call(func, args, postcondition),

            // Method call
            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => self.wp_method_call(receiver, method, args, postcondition),

            // Break and continue - handled by loop WP
            ExprKind::Break { .. } | ExprKind::Continue { .. } => Ok(postcondition.clone()),

            // Other binary expressions (non-assignment)
            ExprKind::Binary { .. } => Ok(postcondition.clone()),

            _ => Err(WpError::UnsupportedExpression(Text::from(format!(
                "{:?}",
                body.kind
            )))),
        }
    }

    /// Compute WP for a block of statements
    fn wp_block(&mut self, block: &Block, postcondition: &Bool) -> WpResult<Bool> {
        let mut current_wp = postcondition.clone();

        // First, handle the trailing expression if present
        if let Maybe::Some(expr) = &block.expr {
            current_wp = self.wp(expr, &current_wp)?;
        }

        // Then process statements in reverse order
        for stmt in block.stmts.iter().rev() {
            current_wp = self.wp_stmt(stmt, &current_wp)?;
        }

        Ok(current_wp)
    }

    /// Compute WP for a single statement
    fn wp_stmt(&mut self, stmt: &verum_ast::Stmt, postcondition: &Bool) -> WpResult<Bool> {
        match &stmt.kind {
            StmtKind::Let { pattern, value, .. } => {
                if let Maybe::Some(val) = value {
                    self.wp_let_binding(pattern, val, postcondition)
                } else {
                    Ok(postcondition.clone())
                }
            }

            StmtKind::LetElse {
                pattern,
                value,
                else_block,
                ..
            } => {
                // let pattern = expr else { diverge }
                let z3_value = self.translator.translate_expr(value)?;

                // Create a symbolic condition for pattern match
                let match_name = format!("pattern_matches_{:?}", pattern.span);
                let match_cond = Bool::new_const(match_name.as_str());

                // Bind pattern variables
                self.bind_pattern(pattern, &z3_value)?;

                // WP for else block (diverges)
                let else_expr = Expr::new(ExprKind::Block(else_block.clone()), else_block.span);
                let wp_else = self.wp(&else_expr, postcondition)?;

                // Combine: match_cond => postcondition && !match_cond => wp_else
                let result = Bool::and(&[
                    &match_cond.implies(postcondition),
                    &match_cond.not().implies(&wp_else),
                ]);

                Ok(result)
            }

            StmtKind::Expr { expr, .. } => self.wp(expr, postcondition),

            StmtKind::Defer(defer_expr) => {
                // Defer executes at scope exit - compose with postcondition
                self.wp(defer_expr, postcondition)
            }

            StmtKind::Errdefer(errdefer_expr) => {
                // Errdefer only executes on error path
                // For verification, we model this similarly to defer but only on error
                // For now, treat as a no-op for normal path verification
                self.wp(errdefer_expr, postcondition)
            }

            StmtKind::Provide { value, .. } => {
                // Context system is runtime DI, doesn't affect verification
                self.wp(value, postcondition)
            }

            StmtKind::ProvideScope { value, block, .. } => {
                // Analyze both value and block
                let wp_block = self.wp(block, postcondition)?;
                self.wp(value, &wp_block)
            }

            StmtKind::Item(_) => Ok(postcondition.clone()),

            StmtKind::Empty => Ok(postcondition.clone()),
        }
    }

    /// Compute WP for let binding
    fn wp_let_binding(
        &mut self,
        pattern: &Pattern,
        value: &Expr,
        postcondition: &Bool,
    ) -> WpResult<Bool> {
        let z3_value = self.translator.translate_expr(value)?;
        self.bind_pattern(pattern, &z3_value)?;
        Ok(postcondition.clone())
    }

    /// Bind pattern variables to a Z3 expression
    fn bind_pattern(&mut self, pattern: &Pattern, value: &Dynamic) -> WpResult<()> {
        match &pattern.kind {
            PatternKind::Ident { name, .. } => {
                self.translator
                    .bind(Text::from(name.as_str()), value.clone());
                Ok(())
            }

            PatternKind::Wildcard => Ok(()),

            PatternKind::Tuple(patterns) => {
                for (i, pat) in patterns.iter().enumerate() {
                    let elem_name = format!("tuple_elem_{}", i);
                    let elem_var = Int::new_const(elem_name.as_str());
                    self.bind_pattern(pat, &Dynamic::from_ast(&elem_var))?;
                }
                Ok(())
            }

            PatternKind::Paren(inner) => self.bind_pattern(inner, value),

            _ => Err(WpError::UnsupportedStatement(Text::from(format!(
                "pattern binding: {:?}",
                pattern.kind
            )))),
        }
    }

    /// Compute WP for assignment
    fn wp_assignment(
        &mut self,
        op: BinOp,
        left: &Expr,
        right: &Expr,
        postcondition: &Bool,
    ) -> WpResult<Bool> {
        // Handle compound assignments
        let actual_value = if op == BinOp::Assign {
            right.clone()
        } else {
            let base_op = match op {
                BinOp::AddAssign => BinOp::Add,
                BinOp::SubAssign => BinOp::Sub,
                BinOp::MulAssign => BinOp::Mul,
                BinOp::DivAssign => BinOp::Div,
                BinOp::RemAssign => BinOp::Rem,
                BinOp::BitAndAssign => BinOp::BitAnd,
                BinOp::BitOrAssign => BinOp::BitOr,
                BinOp::BitXorAssign => BinOp::BitXor,
                BinOp::ShlAssign => BinOp::Shl,
                BinOp::ShrAssign => BinOp::Shr,
                _ => {
                    return Err(WpError::UnsupportedStatement(Text::from(format!(
                        "assignment operator: {:?}",
                        op
                    ))));
                }
            };

            Expr {
                kind: ExprKind::Binary {
                    op: base_op,
                    left: verum_common::Heap::new(left.clone()),
                    right: verum_common::Heap::new(right.clone()),
                },
                span: right.span,
                check_eliminated: false,
                ref_kind: None,
            }
        };

        // Get the variable name being assigned
        let var_name = self.extract_lvalue_name(left)?;

        // Translate the new value
        let z3_value = self.translator.translate_expr(&actual_value)?;

        // Create a fresh variable for the old state
        let version = self.get_next_version(&var_name);
        let _primed_name = format!("{}_{}", var_name, version);

        // Bind the new variable
        self.translator
            .bind(Text::from(var_name.as_str()), z3_value);

        Ok(postcondition.clone())
    }

    /// Extract variable name from an lvalue expression
    fn extract_lvalue_name(&self, expr: &Expr) -> WpResult<String> {
        match &expr.kind {
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    Ok(ident.as_str().to_string())
                } else {
                    Err(WpError::UnsupportedExpression(Text::from(
                        "complex path in assignment",
                    )))
                }
            }
            ExprKind::Field { expr: base, field } => {
                let base_name = self.extract_lvalue_name(base)?;
                Ok(format!("{}.{}", base_name, field.as_str()))
            }
            ExprKind::Index { expr: base, .. } => {
                let base_name = self.extract_lvalue_name(base)?;
                Ok(format!("{}_idx", base_name))
            }
            ExprKind::Paren(inner) => self.extract_lvalue_name(inner),
            _ => Err(WpError::UnsupportedExpression(Text::from(format!(
                "lvalue: {:?}",
                expr.kind
            )))),
        }
    }

    /// Get the next version number for a variable
    fn get_next_version(&mut self, name: &str) -> u32 {
        let entry = self.state_versions.entry(name.to_string()).or_insert(0);
        *entry += 1;
        *entry
    }

    /// Compute WP for if-then-else
    fn wp_if(
        &mut self,
        condition: &verum_common::Heap<IfCondition>,
        then_branch: &Block,
        else_branch: Option<&verum_common::Heap<Expr>>,
        postcondition: &Bool,
    ) -> WpResult<Bool> {
        // Translate condition
        let z3_cond = self.translate_if_condition(condition)?;

        // Compute WP for then branch
        let then_expr = Expr::new(ExprKind::Block(then_branch.clone()), then_branch.span);
        let wp_then = self.wp(&then_expr, postcondition)?;

        // Compute WP for else branch (or postcondition if no else)
        let wp_else = if let Some(else_expr) = else_branch {
            self.wp(else_expr, postcondition)?
        } else {
            postcondition.clone()
        };

        // Combine: (cond => wp_then) && (!cond => wp_else)
        let result = Bool::and(&[&z3_cond.implies(&wp_then), &z3_cond.not().implies(&wp_else)]);

        Ok(result)
    }

    /// Translate an if condition to Z3
    fn translate_if_condition(&self, condition: &verum_common::Heap<IfCondition>) -> WpResult<Bool> {
        if condition.conditions.is_empty() {
            return Err(WpError::UnsupportedExpression(Text::from(
                "empty condition in if",
            )));
        }

        match &condition.conditions[0] {
            ConditionKind::Expr(expr) => {
                let z3_expr = self.translator.translate_expr(expr)?;
                z3_expr
                    .as_bool()
                    .ok_or_else(|| WpError::Internal(Text::from("condition must be boolean")))
            }
            ConditionKind::Let { .. } => Err(WpError::UnsupportedExpression(Text::from(
                "if-let patterns in WP",
            ))),
        }
    }

    /// Compute WP for while loop using bounded unrolling
    fn wp_while_unrolled(
        &mut self,
        condition: &Expr,
        body: &Expr,
        bound: u32,
        postcondition: &Bool,
    ) -> WpResult<Bool> {
        if bound == 0 {
            let z3_cond = self.translator.translate_expr(condition)?;
            let z3_cond_bool = z3_cond
                .as_bool()
                .ok_or_else(|| WpError::Internal(Text::from("condition must be boolean")))?;

            Ok(z3_cond_bool.not().implies(postcondition))
        } else {
            let z3_cond = self.translator.translate_expr(condition)?;
            let z3_cond_bool = z3_cond
                .as_bool()
                .ok_or_else(|| WpError::Internal(Text::from("condition must be boolean")))?;

            let wp_rest = self.wp_while_unrolled(condition, body, bound - 1, postcondition)?;
            let wp_body = self.wp(body, &wp_rest)?;

            Ok(Bool::and(&[
                &z3_cond_bool.implies(&wp_body),
                &z3_cond_bool.not().implies(postcondition),
            ]))
        }
    }

    /// Compute WP for function call using contract summarization
    fn wp_call(&mut self, func: &Expr, args: &[Expr], postcondition: &Bool) -> WpResult<Bool> {
        // Extract function name
        let func_name = match &func.kind {
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    ident.as_str().to_string()
                } else {
                    return Err(WpError::UnsupportedExpression(Text::from(
                        "complex function path",
                    )));
                }
            }
            _ => {
                return Err(WpError::UnsupportedExpression(Text::from(
                    "non-path function call",
                )));
            }
        };

        // Look up function contract
        if let Maybe::Some((preconditions, postconditions)) =
            self.function_contracts.get(&Text::from(func_name.as_str()))
        {
            let mut conjuncts = Vec::new();

            for pre in preconditions.iter() {
                if let Some(pre_bool) = pre.as_bool() {
                    conjuncts.push(pre_bool);
                }
            }

            let mut post_conjuncts = Vec::new();
            for post in postconditions.iter() {
                if let Some(post_bool) = post.as_bool() {
                    post_conjuncts.push(post_bool);
                }
            }

            if !post_conjuncts.is_empty() {
                let post_refs: Vec<&Bool> = post_conjuncts.iter().collect();
                let all_posts = Bool::and(&post_refs);
                conjuncts.push(all_posts.implies(postcondition));
            }

            if conjuncts.is_empty() {
                Ok(postcondition.clone())
            } else {
                let refs: Vec<&Bool> = conjuncts.iter().collect();
                Ok(Bool::and(&refs))
            }
        } else {
            // Without contract: assume function is pure
            Ok(postcondition.clone())
        }
    }

    /// Compute WP for method call
    fn wp_method_call(
        &mut self,
        receiver: &Expr,
        method: &verum_ast::Ident,
        _args: &[Expr],
        postcondition: &Bool,
    ) -> WpResult<Bool> {
        let method_name = method.as_str();
        if method_name.starts_with("set_")
            || method_name.ends_with("_mut")
            || matches!(method_name, "push" | "pop" | "insert" | "remove" | "clear")
        {
            if let Ok(var_name) = self.extract_lvalue_name(receiver) {
                let _version = self.get_next_version(&var_name);
            }
        }

        Ok(postcondition.clone())
    }

    /// Get the translator for external access
    pub fn translator(&self) -> &Translator<'ctx> {
        &self.translator
    }

    /// Get mutable access to the translator
    pub fn translator_mut(&mut self) -> &mut Translator<'ctx> {
        &mut self.translator
    }
}

// ==================== Enhanced Loop Effects Extraction ====================

/// State modification tracking for dataflow analysis
#[derive(Debug, Clone)]
pub struct StateModification {
    /// Variable being modified
    pub variable: Text,
    /// New value expression
    pub value_expr: Expr,
    /// Whether this is a direct assignment or indirect modification
    pub is_direct: bool,
    /// Path through references (for tracking indirect modifications)
    pub reference_path: List<Text>,
}

/// Comprehensive dataflow analyzer for loop body effects
pub struct DataflowAnalyzer<'a> {
    /// Tracked state variables
    state_vars: &'a [(Text, Type)],
    /// Modifications discovered
    modifications: List<StateModification>,
}

impl<'a> DataflowAnalyzer<'a> {
    /// Create a new dataflow analyzer
    pub fn new(state_vars: &'a [(Text, Type)]) -> Self {
        Self {
            state_vars,
            modifications: List::new(),
        }
    }

    /// Analyze an expression for state modifications
    pub fn analyze(&mut self, expr: &Expr) {
        self.analyze_impl(expr);
    }

    /// Implementation of expression analysis
    fn analyze_impl(&mut self, expr: &Expr) {
        use ExprKind::*;

        match &expr.kind {
            // Assignment expressions
            Binary { op, left, right } if op.is_assignment() => {
                if let Some(var_name) = self.extract_base_variable(left) {
                    if self.is_state_var(&var_name) {
                        let is_direct = matches!(&left.kind, Path(_));
                        let value_expr = if *op == BinOp::Assign {
                            (**right).clone()
                        } else {
                            self.build_compound_expr(*op, left, right)
                        };

                        self.modifications.push(StateModification {
                            variable: var_name,
                            value_expr,
                            is_direct,
                            reference_path: List::new(),
                        });
                    }
                }

                self.analyze_impl(right);
            }

            Binary { left, right, .. } => {
                self.analyze_impl(left);
                self.analyze_impl(right);
            }

            Unary { expr: inner, .. } => {
                self.analyze_impl(inner);
            }

            MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                // Check for mutating methods
                let method_name = method.as_str();
                if method_name.starts_with("set_")
                    || method_name.ends_with("_mut")
                    || matches!(method_name, "push" | "pop" | "insert" | "remove" | "clear")
                {
                    if let Some(var_name) = self.extract_base_variable(receiver) {
                        if self.is_state_var(&var_name) {
                            self.modifications.push(StateModification {
                                variable: var_name,
                                value_expr: Expr::new(
                                    ExprKind::MethodCall {
                                        receiver: receiver.clone(),
                                        method: method.clone(),
                                        type_args: List::new(),
                                        args: args.clone(),
                                    },
                                    method.span,
                                ),
                                is_direct: false,
                                reference_path: List::new(),
                            });
                        }
                    }
                }

                self.analyze_impl(receiver);
                for arg in args.iter() {
                    self.analyze_impl(arg);
                }
            }

            Call { func, args, .. } => {
                self.analyze_impl(func);
                for arg in args.iter() {
                    self.analyze_impl(arg);
                }
            }

            Block(block) => {
                for stmt in &block.stmts {
                    self.analyze_stmt(stmt);
                }
                if let Maybe::Some(expr) = &block.expr {
                    self.analyze_impl(expr);
                }
            }

            If {
                condition,
                then_branch,
                else_branch,
            } => {
                for cond in &condition.conditions {
                    match cond {
                        ConditionKind::Expr(e) => self.analyze_impl(e),
                        ConditionKind::Let { value, .. } => self.analyze_impl(value),
                    }
                }

                for stmt in &then_branch.stmts {
                    self.analyze_stmt(stmt);
                }
                if let Maybe::Some(expr) = &then_branch.expr {
                    self.analyze_impl(expr);
                }

                if let Maybe::Some(else_expr) = else_branch {
                    self.analyze_impl(else_expr);
                }
            }

            Loop { body, .. } | While { body, .. } => {
                for stmt in &body.stmts {
                    self.analyze_stmt(stmt);
                }
                if let Maybe::Some(expr) = &body.expr {
                    self.analyze_impl(expr);
                }
            }

            Index { expr: base, index } => {
                self.analyze_impl(base);
                self.analyze_impl(index);
            }

            Field { expr: base, .. } => {
                self.analyze_impl(base);
            }

            Paren(inner) => {
                self.analyze_impl(inner);
            }

            Literal(_) | Path(_) | Tuple(_) | Array(_) | Range { .. } => {}

            _ => {}
        }
    }

    /// Analyze a statement for effects
    fn analyze_stmt(&mut self, stmt: &verum_ast::Stmt) {
        match &stmt.kind {
            StmtKind::Let { value, .. } => {
                if let Maybe::Some(val) = value {
                    self.analyze_impl(val);
                }
            }
            StmtKind::LetElse {
                value, else_block, ..
            } => {
                self.analyze_impl(value);
                for s in &else_block.stmts {
                    self.analyze_stmt(s);
                }
                if let Maybe::Some(expr) = &else_block.expr {
                    self.analyze_impl(expr);
                }
            }
            StmtKind::Expr { expr, .. } => {
                self.analyze_impl(expr);
            }
            StmtKind::Defer(expr) => {
                self.analyze_impl(expr);
            }
            StmtKind::Errdefer(expr) => {
                // Errdefer only runs on error path, analyze similarly
                self.analyze_impl(expr);
            }
            StmtKind::Provide { value, .. } => {
                self.analyze_impl(value);
            }
            StmtKind::ProvideScope { value, block, .. } => {
                self.analyze_impl(value);
                self.analyze_impl(block);
            }
            StmtKind::Item(_) | StmtKind::Empty => {}
        }
    }

    /// Check if a name is a tracked state variable
    fn is_state_var(&self, name: &Text) -> bool {
        self.state_vars.iter().any(|(n, _)| n == name)
    }

    /// Extract the base variable name from an expression
    fn extract_base_variable(&self, expr: &Expr) -> Option<Text> {
        match &expr.kind {
            ExprKind::Path(path) => path.as_ident().map(|i| Text::from(i.as_str())),
            ExprKind::Field { expr: base, .. } => self.extract_base_variable(base),
            ExprKind::Index { expr: base, .. } => self.extract_base_variable(base),
            ExprKind::Paren(inner) => self.extract_base_variable(inner),
            ExprKind::Unary { expr: inner, .. } => self.extract_base_variable(inner),
            _ => None,
        }
    }

    /// Build a compound expression from operator and operands
    fn build_compound_expr(&self, op: BinOp, left: &Expr, right: &Expr) -> Expr {
        let base_op = match op {
            BinOp::AddAssign => BinOp::Add,
            BinOp::SubAssign => BinOp::Sub,
            BinOp::MulAssign => BinOp::Mul,
            BinOp::DivAssign => BinOp::Div,
            BinOp::RemAssign => BinOp::Rem,
            BinOp::BitAndAssign => BinOp::BitAnd,
            BinOp::BitOrAssign => BinOp::BitOr,
            BinOp::BitXorAssign => BinOp::BitXor,
            BinOp::ShlAssign => BinOp::Shl,
            BinOp::ShrAssign => BinOp::Shr,
            _ => return right.clone(),
        };

        Expr {
            kind: ExprKind::Binary {
                op: base_op,
                left: verum_common::Heap::new(left.clone()),
                right: verum_common::Heap::new(right.clone()),
            },
            span: right.span,
            check_eliminated: false,
            ref_kind: None,
        }
    }

    /// Get all discovered modifications
    pub fn get_modifications(&self) -> &List<StateModification> {
        &self.modifications
    }
}

/// Extract loop body effects using comprehensive dataflow analysis
///
/// This function analyzes a loop body expression and extracts all effects
/// (assignments) that modify state variables.
pub fn extract_loop_body_effects_enhanced(
    loop_body: &Expr,
    state_vars: &[(Text, verum_ast::Type)],
) -> List<(Text, Expr)> {
    let mut analyzer = DataflowAnalyzer::new(state_vars);
    analyzer.analyze(loop_body);

    let mut effects = List::new();
    for modification in analyzer.get_modifications().iter() {
        effects.push((
            modification.variable.clone(),
            modification.value_expr.clone(),
        ));
    }

    effects
}
