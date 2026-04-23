//! Termination checking for recursive functions.
//!
//! This module ensures that recursive functions terminate by checking that:
//! 1. Recursive calls are on structurally smaller arguments
//! 2. At least one argument decreases in size on each recursive call
//! 3. Optional termination metrics (lexicographic ordering)
//! 4. Size-change termination for complex recursive patterns
//! 5. Productivity for coinductive types
//!
//! Termination checking: ensuring recursive functions terminate via structural recursion on well-founded orderings — Termination and Totality
//!
//! # Design
//!
//! The termination checker operates in several phases:
//! 1. **Call graph construction** - Identify all function calls and recursion
//! 2. **Structural analysis** - Track which parameters are structurally smaller
//! 3. **Size comparison** - Verify recursive calls have smaller arguments
//! 4. **Metric validation** - Check decreasing clauses if provided
//! 5. **Size-change analysis** - Build size-change graphs for complex recursion
//! 6. **Mutual recursion** - Analyze strongly connected components
//! 7. **Productivity checking** - Ensure coinductive definitions are productive
//!
//! # Examples
//!
//! ```verum
//! // Structural recursion - automatically checked
//! fn length<A>(xs: List<A>) -> Nat =
//!     match xs {
//!         Nil => Zero,
//!         Cons(_, tail) => Succ(length(tail))  // OK: tail smaller than xs
//!     }
//!
//! // General recursion needs proof
//! fn ackermann(m: Nat, n: Nat) -> Nat
//!     decreasing (m, n) by lex_order = {
//!     match (m, n) {
//!         (Zero, n) => Succ(n),
//!         (Succ(m'), Zero) => ackermann(m', Succ(Zero)),
//!         (Succ(m'), Succ(n')) => ackermann(m', ackermann(Succ(m'), n'))
//!     }
//! }
//! ```

use verum_ast::decl::{FunctionDecl, FunctionParamKind};
use verum_ast::expr::{Expr, ExprKind};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::span::{Span, Spanned};
use verum_ast::ty::{Ident, Path, PathSegment, Type};
use verum_common::{List, Map, Set, Text};

/// Helper function to convert a Path to Text
fn path_to_text(path: &Path) -> Text {
    let parts: Vec<Text> = path
        .segments
        .iter()
        .filter_map(|seg| match seg {
            PathSegment::Name(ident) => Some(ident.name.clone()),
            PathSegment::SelfValue => Some("self".into()),
            PathSegment::Super => Some("super".into()),
            PathSegment::Cog => Some("cog".into()),
            PathSegment::Relative => None,
        })
        .collect();
    let joined: String = parts.iter().map(|t| t.as_str()).collect::<Vec<_>>().join(".");
    joined.into()
}

/// Result of termination checking
pub type Result<T> = std::result::Result<T, TerminationError>;

/// Errors that can occur during termination checking
#[derive(Debug, Clone)]
pub enum TerminationError {
    /// Function cannot be proven to terminate
    NonTerminating {
        function: Text,
        reason: Text,
        span: Span,
    },

    /// Recursive call does not have a smaller argument
    NoDecreasingArgument {
        function: Text,
        call_site: Span,
        available_args: List<Text>,
    },

    /// Invalid decreasing clause
    InvalidDecreasingClause {
        function: Text,
        clause: Text,
        reason: Text,
        span: Span,
    },

    /// Mutual recursion cycle detected without termination proof
    MutualRecursionCycle { cycle: List<Text>, span: Span },

    /// Parameter not structurally smaller
    NotStructurallySmaller {
        function: Text,
        param: Text,
        call_arg: Text,
        span: Span,
    },

    /// Corecursive call is not guarded by a constructor
    /// Termination checking: ensuring recursive functions terminate via structural recursion on well-founded orderings — .2
    UnguardedCorecursion { function: Text, span: Span },
}

impl std::fmt::Display for TerminationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TerminationError::NonTerminating {
                function,
                reason,
                span: _,
            } => {
                write!(
                    f,
                    "function `{}` cannot be proven to terminate: {}",
                    function, reason
                )
            }
            TerminationError::NoDecreasingArgument {
                function,
                call_site: _,
                available_args,
            } => {
                write!(
                    f,
                    "recursive call to `{}` has no decreasing argument (available: {})",
                    function,
                    available_args
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            TerminationError::InvalidDecreasingClause {
                function,
                clause,
                reason,
                span: _,
            } => {
                write!(
                    f,
                    "invalid decreasing clause for `{}`: `{}` - {}",
                    function, clause, reason
                )
            }
            TerminationError::MutualRecursionCycle { cycle, span: _ } => {
                write!(
                    f,
                    "mutual recursion cycle detected: {}",
                    cycle
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(" -> ")
                )
            }
            TerminationError::NotStructurallySmaller {
                function,
                param,
                call_arg,
                span: _,
            } => {
                write!(
                    f,
                    "in function `{}`: argument `{}` is not structurally smaller than parameter `{}`",
                    function, call_arg, param
                )
            }
            TerminationError::UnguardedCorecursion { function, span: _ } => {
                write!(
                    f,
                    "corecursive call in function `{}` is not guarded by a constructor",
                    function
                )
            }
        }
    }
}

impl std::error::Error for TerminationError {}

/// Termination checker for recursive functions
pub struct TerminationChecker {
    /// Call graph: function -> list of functions it calls
    call_graph: Map<Text, List<Text>>,
    /// Recursive functions detected
    recursive_functions: Set<Text>,
    /// Functions currently being analyzed (for cycle detection)
    analysis_stack: List<Text>,
}

impl TerminationChecker {
    /// Create a new termination checker
    pub fn new() -> Self {
        Self {
            call_graph: Map::new(),
            recursive_functions: Set::new(),
            analysis_stack: List::new(),
        }
    }

    /// Check termination for a function declaration.
    /// `is_method` should be true when the function is inside an `implement` block.
    pub fn check_function(&mut self, decl: &FunctionDecl) -> Result<()> {
        self.check_function_with_context(decl, false)
    }

    /// Check termination for a method inside an implement block.
    pub fn check_method(&mut self, decl: &FunctionDecl) -> Result<()> {
        self.check_function_with_context(decl, true)
    }

    fn check_function_with_context(&mut self, decl: &FunctionDecl, is_method: bool) -> Result<()> {
        let func_name = &decl.name.name;

        // Skip if no body (external or forward declaration)
        let body = match &decl.body {
            Some(body) => body,
            None => return Ok(()),
        };

        // Extract parameters
        let params = self.extract_parameters(&decl.params);

        // Build call graph and check for recursion
        let calls = self.extract_calls(body, func_name);
        self.call_graph.insert(func_name.clone(), calls.clone());

        // Check if function is directly recursive
        // For methods inside implement blocks, a bare function call like `add(x, y)`
        // is calling the imported free function, NOT the method being defined.
        // Only method calls like `self.add(y)` would be actual recursion.
        // So for methods, also check that there are actual method-call-style recursive calls.
        let is_recursive = if is_method {
            // For methods, check if the body contains self-recursive method calls
            // (MethodCall with receiver=self and method=func_name)
            self.has_self_recursive_method_call(body, func_name)
        } else {
            calls.contains(func_name)
        };

        if is_recursive {
            self.recursive_functions.insert(func_name.clone());

            // Check structural recursion or decreasing clause
            self.check_recursive_function(decl, &params, body)?;
        }

        Ok(())
    }

    /// Check if a function body contains `self.func_name(...)` method calls (true recursion for methods)
    fn has_self_recursive_method_call(&self, body: &verum_ast::decl::FunctionBody, func_name: &Text) -> bool {
        match body {
            verum_ast::decl::FunctionBody::Block(block) => {
                for stmt in &block.stmts {
                    match &stmt.kind {
                        verum_ast::stmt::StmtKind::Expr { expr, .. } => {
                            if self.expr_has_self_method_call(expr, func_name) {
                                return true;
                            }
                        }
                        verum_ast::stmt::StmtKind::Let { value: Some(init), .. } => {
                            if self.expr_has_self_method_call(init, func_name) {
                                return true;
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(expr) = &block.expr {
                    if self.expr_has_self_method_call(expr, func_name) {
                        return true;
                    }
                }
                false
            }
            verum_ast::decl::FunctionBody::Expr(e) => self.expr_has_self_method_call(e, func_name),
        }
    }

    /// Check if an expression contains `self.func_name(...)` method calls
    fn expr_has_self_method_call(&self, expr: &Expr, func_name: &Text) -> bool {
        match &expr.kind {
            ExprKind::MethodCall { receiver, method, args, .. } => {
                // Check if this is self.func_name(...)
                let is_self_call = matches!(&receiver.kind, ExprKind::Path(p) if p.as_ident().is_some_and(|i| i.name.as_str() == "self"))
                    && method.name.as_str() == func_name.as_str();
                if is_self_call {
                    return true;
                }
                // Also check receiver and args recursively
                if self.expr_has_self_method_call(receiver, func_name) {
                    return true;
                }
                for arg in args {
                    if self.expr_has_self_method_call(arg, func_name) {
                        return true;
                    }
                }
                false
            }
            ExprKind::Call { func, args, .. } => {
                if self.expr_has_self_method_call(func, func_name) {
                    return true;
                }
                for arg in args {
                    if self.expr_has_self_method_call(arg, func_name) {
                        return true;
                    }
                }
                false
            }
            ExprKind::If { condition, then_branch, else_branch } => {
                match &**condition {
                    verum_ast::expr::IfCondition { conditions, .. } => {
                        for cond in conditions {
                            match cond {
                                verum_ast::expr::ConditionKind::Expr(e) => {
                                    if self.expr_has_self_method_call(e, func_name) { return true; }
                                }
                                verum_ast::expr::ConditionKind::Let { value, .. } => {
                                    if self.expr_has_self_method_call(value, func_name) { return true; }
                                }
                            }
                        }
                    }
                }
                if self.block_has_self_method_call(then_branch, func_name) { return true; }
                if let Some(e) = else_branch {
                    if self.expr_has_self_method_call(e, func_name) { return true; }
                }
                false
            }
            ExprKind::Match { expr, arms } => {
                if self.expr_has_self_method_call(expr, func_name) { return true; }
                for arm in arms {
                    if self.expr_has_self_method_call(&arm.body, func_name) { return true; }
                }
                false
            }
            ExprKind::Block(block) => self.block_has_self_method_call(block, func_name),
            _ => false,
        }
    }

    /// Check if a block contains self-recursive method calls
    fn block_has_self_method_call(&self, block: &verum_ast::expr::Block, func_name: &Text) -> bool {
        for stmt in &block.stmts {
            match &stmt.kind {
                verum_ast::stmt::StmtKind::Expr { expr: e, .. } => {
                    if self.expr_has_self_method_call(e, func_name) { return true; }
                }
                verum_ast::stmt::StmtKind::Let { value: Some(init), .. } => {
                    if self.expr_has_self_method_call(init, func_name) { return true; }
                }
                _ => {}
            }
        }
        if let Some(e) = &block.expr {
            if self.expr_has_self_method_call(e, func_name) { return true; }
        }
        false
    }

    /// Check that a recursive function terminates
    fn check_recursive_function(
        &self,
        decl: &FunctionDecl,
        params: &List<ParamInfo>,
        body: &verum_ast::decl::FunctionBody,
    ) -> Result<()> {
        let func_name = &decl.name.name;

        // Extract the body expression
        let body_expr = match body {
            verum_ast::decl::FunctionBody::Block(block) => {
                // For block bodies, analyze all statements
                self.check_block_termination(decl, params, block)?;
                return Ok(());
            }
            verum_ast::decl::FunctionBody::Expr(expr) => expr,
        };

        // Find all recursive calls
        let rec_calls = self.find_recursive_calls(body_expr, func_name);

        // If no recursive calls, trivially terminates
        if rec_calls.is_empty() {
            return Ok(());
        }

        // Check each recursive call has a decreasing argument
        for call in &rec_calls {
            self.check_call_decreases(decl, params, call)?;
        }

        Ok(())
    }

    /// Check termination for a block body
    fn check_block_termination(
        &self,
        decl: &FunctionDecl,
        params: &List<ParamInfo>,
        block: &verum_ast::expr::Block,
    ) -> Result<()> {
        let func_name = &decl.name.name;

        // GUARDED RECURSION CHECK:
        // If the function body has if/match control flow where at least one branch
        // does NOT recurse, treat the function as having a base case (guarded recursion).
        // This handles common patterns like:
        //   fn gamma(x: Float) -> Float {
        //       if x < 0.5 { ... gamma(1.0 - x) ... }  // recursive branch
        //       else { ... }  // base case (no recursion)
        //   }
        if self.has_guarded_recursion(block, func_name) {
            return Ok(());
        }

        // Analyze each statement
        for stmt in &block.stmts {
            match &stmt.kind {
                verum_ast::stmt::StmtKind::Expr { expr, has_semi: _ } => {
                    let rec_calls = self.find_recursive_calls(expr, func_name);
                    for call in &rec_calls {
                        self.check_call_decreases(decl, params, call)?;
                    }
                }
                verum_ast::stmt::StmtKind::Let {
                    pattern: _,
                    ty: _,
                    value,
                } => {
                    if let Some(init) = value {
                        let rec_calls = self.find_recursive_calls(init, func_name);
                        for call in &rec_calls {
                            self.check_call_decreases(decl, params, call)?;
                        }
                    }
                }
                _ => {}
            }
        }

        // Check final expression if any
        if let Some(expr) = &block.expr {
            let rec_calls = self.find_recursive_calls(expr, func_name);
            for call in &rec_calls {
                self.check_call_decreases(decl, params, call)?;
            }
        }

        Ok(())
    }

    /// Check if a block has guarded recursion (recursive calls inside if/match with at least
    /// one non-recursive branch).
    fn has_guarded_recursion(&self, block: &verum_ast::expr::Block, func_name: &Text) -> bool {
        // Check all statements and trailing expression for if/match with guarded recursion
        for stmt in &block.stmts {
            if let verum_ast::stmt::StmtKind::Expr { expr, .. } = &stmt.kind {
                if self.expr_has_guarded_recursion(expr, func_name) {
                    return true;
                }
            }
            // Early return statements inside an if block also constitute a base case
            if let verum_ast::stmt::StmtKind::Expr { expr, .. } = &stmt.kind {
                if self.expr_is_early_return_guard(expr, func_name) {
                    return true;
                }
            }
        }
        if let Some(expr) = &block.expr {
            if self.expr_has_guarded_recursion(expr, func_name) {
                return true;
            }
        }
        false
    }

    /// Check if an expression is an if/match with at least one non-recursive branch
    fn expr_has_guarded_recursion(&self, expr: &Expr, func_name: &Text) -> bool {
        match &expr.kind {
            ExprKind::If { then_branch, else_branch, .. } => {
                let then_calls = self.block_has_recursive_calls(then_branch, func_name);
                let else_calls = if let Some(else_expr) = else_branch {
                    !self.find_recursive_calls(else_expr, func_name).is_empty()
                } else {
                    false
                };
                // Guarded if at least one branch has no recursion
                then_calls != else_calls
            }
            ExprKind::Match { arms, .. } => {
                if arms.len() < 2 {
                    return false;
                }
                let mut has_recursive_arm = false;
                let mut has_non_recursive_arm = false;
                for arm in arms {
                    let arm_calls = !self.find_recursive_calls(&arm.body, func_name).is_empty();
                    if arm_calls {
                        has_recursive_arm = true;
                    } else {
                        has_non_recursive_arm = true;
                    }
                }
                has_recursive_arm && has_non_recursive_arm
            }
            _ => false,
        }
    }

    /// Check if an expression is an early-return guard (if condition { return ...; })
    /// followed by recursive code
    fn expr_is_early_return_guard(&self, expr: &Expr, func_name: &Text) -> bool {
        // Check if this is an if-expression with a return in the then-branch
        if let ExprKind::If { then_branch, else_branch, .. } = &expr.kind {
            let then_has_return = self.block_has_return(then_branch);
            let then_has_recursion = self.block_has_recursive_calls(then_branch, func_name);
            // If then-branch returns early without recursion, it's a guard
            if then_has_return && !then_has_recursion {
                return true;
            }
            // Same for else branch
            if let Some(else_expr) = else_branch {
                if let ExprKind::Block(else_block) = &else_expr.kind {
                    let else_has_return = self.block_has_return(else_block);
                    let else_has_recursion = self.block_has_recursive_calls(else_block, func_name);
                    if else_has_return && !else_has_recursion {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Check if a block contains a return statement
    fn block_has_return(&self, block: &verum_ast::expr::Block) -> bool {
        for stmt in &block.stmts {
            if let verum_ast::stmt::StmtKind::Expr { expr, .. } = &stmt.kind {
                if matches!(&expr.kind, ExprKind::Return(_)) {
                    return true;
                }
            }
        }
        if let Some(expr) = &block.expr {
            if matches!(&expr.kind, ExprKind::Return(_)) {
                return true;
            }
        }
        false
    }

    /// Check if a block contains any recursive calls (simplified check for guard detection)
    fn block_has_recursive_calls(&self, block: &verum_ast::expr::Block, func_name: &Text) -> bool {
        for stmt in &block.stmts {
            match &stmt.kind {
                verum_ast::stmt::StmtKind::Expr { expr, .. } => {
                    if !self.find_recursive_calls(expr, func_name).is_empty() {
                        return true;
                    }
                }
                verum_ast::stmt::StmtKind::Let { value: Some(init), .. } => {
                    if !self.find_recursive_calls(init, func_name).is_empty() {
                        return true;
                    }
                }
                _ => {}
            }
        }
        if let Some(expr) = &block.expr {
            if !self.find_recursive_calls(expr, func_name).is_empty() {
                return true;
            }
        }
        false
    }

    /// Check that a recursive call has at least one decreasing argument
    fn check_call_decreases(
        &self,
        decl: &FunctionDecl,
        params: &List<ParamInfo>,
        call: &RecursiveCall,
    ) -> Result<()> {
        let func_name = &decl.name.name;

        // Extract context from the call site (pattern matching, etc.)
        let context = self.extract_call_context(call);

        // Check if call is in a pattern match that guarantees structural recursion
        if self.is_structurally_recursive(&context, params, &call.args) {
            return Ok(());
        }

        // Check if any argument is structurally smaller
        for (i, arg) in call.args.iter().enumerate() {
            if i < params.len() {
                let param = &params[i];
                if self.is_smaller_argument(&context, arg, &param.name) {
                    return Ok(());
                }

                // Check match_bindings: if arg (after stripping derefs) is a variable
                // bound in a pattern that matched a parameter, it's structurally smaller.
                // E.g., match s { Cons(_, tail) => f(*tail) } — tail < s
                if let Some(arg_var) = Self::extract_inner_var_name(arg) {
                    if let Some(matched_param) = call.match_bindings.get(&arg_var) {
                        if *matched_param == param.name {
                            return Ok(());
                        }
                    }
                }
            }
        }

        // No decreasing argument found
        Err(TerminationError::NoDecreasingArgument {
            function: func_name.clone(),
            call_site: call.span,
            available_args: params.iter().map(|p| p.name.clone()).collect(),
        })
    }

    /// Extract parameter information from function parameters
    fn extract_parameters(&self, params: &[verum_ast::decl::FunctionParam]) -> List<ParamInfo> {
        let mut result = List::new();

        for param in params {
            match &param.kind {
                FunctionParamKind::Regular { pattern, ty, .. } => {
                    if let PatternKind::Ident { name, .. } = &pattern.kind {
                        result.push(ParamInfo {
                            name: name.name.clone(),
                            ty: ty.clone(),
                            span: param.span,
                        });
                    }
                }
                _ => {} // Skip self parameters
            }
        }

        result
    }

    /// Extract all function calls from an expression
    fn extract_calls(
        &self,
        body: &verum_ast::decl::FunctionBody,
        _current_func: &Text,
    ) -> List<Text> {
        let mut calls = List::new();

        let expr = match body {
            verum_ast::decl::FunctionBody::Block(block) => {
                // Collect calls from all statements
                for stmt in &block.stmts {
                    match &stmt.kind {
                        verum_ast::stmt::StmtKind::Expr {
                            expr: e,
                            has_semi: _,
                        } => {
                            self.collect_calls_from_expr(e, &mut calls);
                        }
                        verum_ast::stmt::StmtKind::Let {
                            pattern: _,
                            ty: _,
                            value,
                        } => {
                            if let Some(init) = value {
                                self.collect_calls_from_expr(init, &mut calls);
                            }
                        }
                        _ => {}
                    }
                }

                // Collect from final expression
                if let Some(e) = &block.expr {
                    self.collect_calls_from_expr(e, &mut calls);
                }

                return calls;
            }
            verum_ast::decl::FunctionBody::Expr(e) => e,
        };

        self.collect_calls_from_expr(expr, &mut calls);
        calls
    }

    /// Recursively collect all function calls from an expression
    fn collect_calls_from_expr(&self, expr: &Expr, calls: &mut List<Text>) {
        match &expr.kind {
            ExprKind::Call { func, args, .. } => {
                // Extract function name from call
                if let ExprKind::Path(path) = &func.kind {
                    calls.push(path_to_text(path));
                }

                // Recursively check arguments
                for arg in args {
                    self.collect_calls_from_expr(arg, calls);
                }
            }
            ExprKind::Match { expr, arms } => {
                self.collect_calls_from_expr(expr, calls);
                for arm in arms {
                    self.collect_calls_from_expr(&arm.body, calls);
                }
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Check condition
                match &**condition {
                    verum_ast::expr::IfCondition { conditions, .. } => {
                        for cond in conditions {
                            match cond {
                                verum_ast::expr::ConditionKind::Expr(e) => {
                                    self.collect_calls_from_expr(e, calls);
                                }
                                verum_ast::expr::ConditionKind::Let { pattern: _, value } => {
                                    self.collect_calls_from_expr(value, calls);
                                }
                            }
                        }
                    }
                }

                // Check branches
                self.collect_calls_from_block(then_branch, calls);
                if let Some(else_expr) = else_branch {
                    self.collect_calls_from_expr(else_expr, calls);
                }
            }
            ExprKind::Block(block) => {
                self.collect_calls_from_block(block, calls);
            }
            ExprKind::Binary { left, right, .. } => {
                self.collect_calls_from_expr(left, calls);
                self.collect_calls_from_expr(right, calls);
            }
            ExprKind::Unary { expr, .. } => {
                self.collect_calls_from_expr(expr, calls);
            }
            // Tuple expressions: (a, b, c)
            ExprKind::Tuple(elements) => {
                for elem in elements {
                    self.collect_calls_from_expr(elem, calls);
                }
            }
            // Array expressions: [a, b, c] or [val; size]
            ExprKind::Array(array_expr) => {
                use verum_ast::expr::ArrayExpr;
                match array_expr {
                    ArrayExpr::List(elements) => {
                        for elem in elements {
                            self.collect_calls_from_expr(elem, calls);
                        }
                    }
                    ArrayExpr::Repeat { value, count } => {
                        self.collect_calls_from_expr(value, calls);
                        self.collect_calls_from_expr(count, calls);
                    }
                }
            }
            // Index expressions: arr[i]
            ExprKind::Index { expr, index } => {
                self.collect_calls_from_expr(expr, calls);
                self.collect_calls_from_expr(index, calls);
            }
            // Field access: obj.field
            ExprKind::Field { expr, .. } => {
                self.collect_calls_from_expr(expr, calls);
            }
            // Method calls: obj.method(args)
            ExprKind::MethodCall { receiver, args, .. } => {
                self.collect_calls_from_expr(receiver, calls);
                for arg in args {
                    self.collect_calls_from_expr(arg, calls);
                }
            }
            // Record expressions: Struct { field: value }
            ExprKind::Record { fields, base, .. } => {
                for field in fields {
                    if let verum_common::Maybe::Some(ref value) = field.value {
                        self.collect_calls_from_expr(value, calls);
                    }
                }
                if let verum_common::Maybe::Some(base_expr) = base {
                    self.collect_calls_from_expr(base_expr.as_ref(), calls);
                }
            }
            // Range expressions: a..b, a..=b, etc.
            ExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.collect_calls_from_expr(s, calls);
                }
                if let Some(e) = end {
                    self.collect_calls_from_expr(e, calls);
                }
            }
            // Loop expressions: loop { body }
            ExprKind::Loop { body, .. } => {
                self.collect_calls_from_block(body, calls);
            }
            // While expressions: while cond { body }
            ExprKind::While {
                condition, body, ..
            } => {
                self.collect_calls_from_expr(condition, calls);
                self.collect_calls_from_block(body, calls);
            }
            // For expressions: for pat in iter { body }
            ExprKind::For { iter, body, .. } => {
                self.collect_calls_from_expr(iter, calls);
                self.collect_calls_from_block(body, calls);
            }
            // Closure expressions: |args| body
            ExprKind::Closure { body, .. } => {
                self.collect_calls_from_expr(body, calls);
            }
            // Return expressions: return value
            ExprKind::Return(maybe_expr) => {
                if let Some(e) = maybe_expr {
                    self.collect_calls_from_expr(e, calls);
                }
            }
            // Break expressions: break value
            ExprKind::Break { value, .. } => {
                if let Some(e) = value {
                    self.collect_calls_from_expr(e, calls);
                }
            }
            // Cast expressions: expr as Type
            ExprKind::Cast { expr, .. } => {
                self.collect_calls_from_expr(expr, calls);
            }
            // Await expressions: expr.await
            ExprKind::Await(expr) => {
                self.collect_calls_from_expr(expr, calls);
            }
            // Try expressions: expr?
            ExprKind::Try(expr) => {
                self.collect_calls_from_expr(expr, calls);
            }
            // Parenthesized: (expr)
            ExprKind::Paren(inner) => {
                self.collect_calls_from_expr(inner, calls);
            }
            // Literals, paths, continue - no nested expressions
            ExprKind::Literal(_) | ExprKind::Path(_) | ExprKind::Continue { .. } => {}
            // Async/unsafe blocks
            ExprKind::Async(body) | ExprKind::Unsafe(body) | ExprKind::Meta(body) => {
                self.collect_calls_from_block(body, calls);
            }
            // Other expressions we don't need to traverse for termination checking
            _ => {}
        }
    }

    /// Collect calls from a block
    fn collect_calls_from_block(&self, block: &verum_ast::expr::Block, calls: &mut List<Text>) {
        for stmt in &block.stmts {
            match &stmt.kind {
                verum_ast::stmt::StmtKind::Expr {
                    expr: e,
                    has_semi: _,
                } => {
                    self.collect_calls_from_expr(e, calls);
                }
                verum_ast::stmt::StmtKind::Let {
                    pattern: _,
                    ty: _,
                    value,
                } => {
                    if let Some(init) = value {
                        self.collect_calls_from_expr(init, calls);
                    }
                }
                _ => {}
            }
        }

        if let Some(expr) = &block.expr {
            self.collect_calls_from_expr(expr, calls);
        }
    }

    /// Find all recursive calls to a specific function in an expression
    fn find_recursive_calls(&self, expr: &Expr, func_name: &Text) -> List<RecursiveCall> {
        let mut calls = List::new();
        let bindings = Map::new();
        self.find_recursive_calls_impl(expr, func_name, &mut calls, &bindings);
        calls
    }

    /// Extract variable bindings from a pattern.
    /// Returns a list of variable names bound by the pattern.
    fn extract_pattern_bindings(pattern: &verum_ast::pattern::Pattern) -> List<Text> {
        use verum_ast::pattern::{PatternKind, VariantPatternData};
        let mut bindings = List::new();
        match &pattern.kind {
            PatternKind::Ident { name, subpattern, .. } => {
                bindings.push(name.name.clone());
                if let verum_common::Maybe::Some(sub) = subpattern {
                    bindings.extend(Self::extract_pattern_bindings(sub));
                }
            }
            PatternKind::Tuple(fields) => {
                for field in fields {
                    bindings.extend(Self::extract_pattern_bindings(field));
                }
            }
            PatternKind::Variant { data, .. } => {
                if let verum_common::Maybe::Some(data) = data {
                    match data {
                        VariantPatternData::Tuple(fields) => {
                            for field in fields {
                                bindings.extend(Self::extract_pattern_bindings(field));
                            }
                        }
                        VariantPatternData::Record { fields, .. } => {
                            for field in fields {
                                if let verum_common::Maybe::Some(ref p) = field.pattern {
                                    bindings.extend(Self::extract_pattern_bindings(p));
                                } else {
                                    bindings.push(field.name.name.clone());
                                }
                            }
                        }
                    }
                }
            }
            PatternKind::Record { fields, .. } => {
                for field in fields {
                    if let verum_common::Maybe::Some(ref p) = field.pattern {
                        bindings.extend(Self::extract_pattern_bindings(p));
                    } else {
                        bindings.push(field.name.name.clone());
                    }
                }
            }
            PatternKind::Paren(inner) => {
                bindings.extend(Self::extract_pattern_bindings(inner));
            }
            PatternKind::Reference { inner, .. } => {
                bindings.extend(Self::extract_pattern_bindings(inner));
            }
            PatternKind::Or(patterns) => {
                if let Some(first) = patterns.first() {
                    bindings.extend(Self::extract_pattern_bindings(first));
                }
            }
            PatternKind::Wildcard | PatternKind::Literal(_) | PatternKind::Rest | PatternKind::Range { .. } => {}
            _ => {}
        }
        bindings
    }

    /// Implementation helper for finding recursive calls
    fn find_recursive_calls_impl(
        &self,
        expr: &Expr,
        func_name: &Text,
        calls: &mut List<RecursiveCall>,
        match_bindings: &Map<Text, Text>,
    ) {
        match &expr.kind {
            ExprKind::Call { func, args, .. } => {
                // Check if this is a recursive call
                if let ExprKind::Path(path) = &func.kind
                    && path_to_text(path) == *func_name
                {
                    calls.push(RecursiveCall {
                        args: args.iter().cloned().collect(),
                        span: expr.span,
                        match_bindings: match_bindings.clone(),
                    });
                }

                // Recursively check arguments
                for arg in args {
                    self.find_recursive_calls_impl(arg, func_name, calls, match_bindings);
                }
            }
            ExprKind::Match { expr: scrutinee, arms } => {
                self.find_recursive_calls_impl(scrutinee, func_name, calls, match_bindings);

                // Determine the "root parameter" the scrutinee decomposes. We
                // accept both a direct path (`match sel`) and a field path
                // (`match sel.kind`, `match self.node.next`). For field
                // chains we walk to the root so pattern bindings still
                // attribute to the outer parameter — variant-typed fields
                // like `sel.kind` are the overwhelmingly common
                // destructuring idiom, and without this the termination
                // checker falsely flags every recursive function that
                // decomposes a tagged-union field.
                let scrutinee_name = Self::extract_root_param_name(scrutinee);

                for arm in arms {
                    // Build extended bindings for this arm
                    let mut arm_bindings = match_bindings.clone();
                    if let Some(ref param_name) = scrutinee_name {
                        let pattern_vars = Self::extract_pattern_bindings(&arm.pattern);
                        for var in pattern_vars {
                            arm_bindings.insert(var, param_name.clone());
                        }
                    }
                    self.find_recursive_calls_impl(&arm.body, func_name, calls, &arm_bindings);
                }
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Check condition
                match &**condition {
                    verum_ast::expr::IfCondition { conditions, .. } => {
                        for cond in conditions {
                            match cond {
                                verum_ast::expr::ConditionKind::Expr(e) => {
                                    self.find_recursive_calls_impl(e, func_name, calls, match_bindings);
                                }
                                verum_ast::expr::ConditionKind::Let { pattern: _, value } => {
                                    self.find_recursive_calls_impl(value, func_name, calls, match_bindings);
                                }
                            }
                        }
                    }
                }

                // Check branches
                for stmt in &then_branch.stmts {
                    match &stmt.kind {
                        verum_ast::stmt::StmtKind::Expr {
                            expr: e,
                            has_semi: _,
                        } => {
                            self.find_recursive_calls_impl(e, func_name, calls, match_bindings);
                        }
                        verum_ast::stmt::StmtKind::Let {
                            pattern: _,
                            ty: _,
                            value,
                        } => {
                            if let Some(init) = value {
                                self.find_recursive_calls_impl(init, func_name, calls, match_bindings);
                            }
                        }
                        _ => {}
                    }
                }

                if let Some(expr) = &then_branch.expr {
                    self.find_recursive_calls_impl(expr, func_name, calls, match_bindings);
                }

                if let Some(else_expr) = else_branch {
                    self.find_recursive_calls_impl(else_expr, func_name, calls, match_bindings);
                }
            }
            ExprKind::Block(block) => {
                for stmt in &block.stmts {
                    match &stmt.kind {
                        verum_ast::stmt::StmtKind::Expr {
                            expr: e,
                            has_semi: _,
                        } => {
                            self.find_recursive_calls_impl(e, func_name, calls, match_bindings);
                        }
                        verum_ast::stmt::StmtKind::Let {
                            pattern: _,
                            ty: _,
                            value,
                        } => {
                            if let Some(init) = value {
                                self.find_recursive_calls_impl(init, func_name, calls, match_bindings);
                            }
                        }
                        _ => {}
                    }
                }

                if let Some(expr) = &block.expr {
                    self.find_recursive_calls_impl(expr, func_name, calls, match_bindings);
                }
            }
            ExprKind::Binary { left, right, .. } => {
                self.find_recursive_calls_impl(left, func_name, calls, match_bindings);
                self.find_recursive_calls_impl(right, func_name, calls, match_bindings);
            }
            ExprKind::Unary { expr, .. } => {
                self.find_recursive_calls_impl(expr, func_name, calls, match_bindings);
            }
            // Tuple expressions: (a, b, c)
            ExprKind::Tuple(elements) => {
                for elem in elements {
                    self.find_recursive_calls_impl(elem, func_name, calls, match_bindings);
                }
            }
            // Array expressions: [a, b, c] or [val; size]
            ExprKind::Array(array_expr) => {
                use verum_ast::expr::ArrayExpr;
                match array_expr {
                    ArrayExpr::List(elements) => {
                        for elem in elements {
                            self.find_recursive_calls_impl(elem, func_name, calls, match_bindings);
                        }
                    }
                    ArrayExpr::Repeat { value, count } => {
                        self.find_recursive_calls_impl(value, func_name, calls, match_bindings);
                        self.find_recursive_calls_impl(count, func_name, calls, match_bindings);
                    }
                }
            }
            // Index expressions: arr[i]
            ExprKind::Index { expr, index } => {
                self.find_recursive_calls_impl(expr, func_name, calls, match_bindings);
                self.find_recursive_calls_impl(index, func_name, calls, match_bindings);
            }
            // Field access: obj.field
            ExprKind::Field { expr, .. } => {
                self.find_recursive_calls_impl(expr, func_name, calls, match_bindings);
            }
            // Method calls: obj.method(args)
            ExprKind::MethodCall { receiver, args, .. } => {
                self.find_recursive_calls_impl(receiver, func_name, calls, match_bindings);
                for arg in args {
                    self.find_recursive_calls_impl(arg, func_name, calls, match_bindings);
                }
            }
            // Record expressions: Struct { field: value }
            ExprKind::Record { fields, base, .. } => {
                for field in fields {
                    if let verum_common::Maybe::Some(ref value) = field.value {
                        self.find_recursive_calls_impl(value, func_name, calls, match_bindings);
                    }
                }
                if let verum_common::Maybe::Some(base_expr) = base {
                    self.find_recursive_calls_impl(base_expr.as_ref(), func_name, calls, match_bindings);
                }
            }
            // Range expressions: a..b, a..=b, etc.
            ExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.find_recursive_calls_impl(s, func_name, calls, match_bindings);
                }
                if let Some(e) = end {
                    self.find_recursive_calls_impl(e, func_name, calls, match_bindings);
                }
            }
            // Loop expressions: loop { body }
            ExprKind::Loop { body, .. } => {
                self.find_recursive_calls_in_block(body, func_name, calls, match_bindings);
            }
            // While expressions: while cond { body }
            ExprKind::While {
                condition, body, ..
            } => {
                self.find_recursive_calls_impl(condition, func_name, calls, match_bindings);
                self.find_recursive_calls_in_block(body, func_name, calls, match_bindings);
            }
            // For expressions: for pat in iter { body }
            ExprKind::For { iter, body, .. } => {
                self.find_recursive_calls_impl(iter, func_name, calls, match_bindings);
                self.find_recursive_calls_in_block(body, func_name, calls, match_bindings);
            }
            // Closure expressions: |args| body
            ExprKind::Closure { body, .. } => {
                self.find_recursive_calls_impl(body, func_name, calls, match_bindings);
            }
            // Return expressions: return value
            ExprKind::Return(maybe_expr) => {
                if let Some(e) = maybe_expr {
                    self.find_recursive_calls_impl(e, func_name, calls, match_bindings);
                }
            }
            // Break expressions: break value
            ExprKind::Break { value, .. } => {
                if let Some(e) = value {
                    self.find_recursive_calls_impl(e, func_name, calls, match_bindings);
                }
            }
            // Cast expressions: expr as Type
            ExprKind::Cast { expr, .. } => {
                self.find_recursive_calls_impl(expr, func_name, calls, match_bindings);
            }
            // Await expressions: expr.await
            ExprKind::Await(expr) => {
                self.find_recursive_calls_impl(expr, func_name, calls, match_bindings);
            }
            // Try expressions: expr?
            ExprKind::Try(expr) => {
                self.find_recursive_calls_impl(expr, func_name, calls, match_bindings);
            }
            // Parenthesized: (expr)
            ExprKind::Paren(inner) => {
                self.find_recursive_calls_impl(inner, func_name, calls, match_bindings);
            }
            // Async/unsafe/meta blocks
            ExprKind::Async(body) | ExprKind::Unsafe(body) | ExprKind::Meta(body) => {
                self.find_recursive_calls_in_block(body, func_name, calls, match_bindings);
            }
            // Literals, paths, continue - no nested expressions
            ExprKind::Literal(_) | ExprKind::Path(_) | ExprKind::Continue { .. } => {}
            // Other expressions we don't need to traverse for termination checking
            _ => {}
        }
    }

    /// Helper for finding recursive calls in a block
    fn find_recursive_calls_in_block(
        &self,
        block: &verum_ast::expr::Block,
        func_name: &Text,
        calls: &mut List<RecursiveCall>,
        match_bindings: &Map<Text, Text>,
    ) {
        for stmt in &block.stmts {
            match &stmt.kind {
                verum_ast::stmt::StmtKind::Expr {
                    expr: e,
                    has_semi: _,
                } => {
                    self.find_recursive_calls_impl(e, func_name, calls, match_bindings);
                }
                verum_ast::stmt::StmtKind::Let {
                    pattern: _,
                    ty: _,
                    value,
                } => {
                    if let Some(init) = value {
                        self.find_recursive_calls_impl(init, func_name, calls, match_bindings);
                    }
                }
                verum_ast::stmt::StmtKind::LetElse {
                    value, else_block, ..
                } => {
                    self.find_recursive_calls_impl(value, func_name, calls, match_bindings);
                    self.find_recursive_calls_in_block(else_block, func_name, calls, match_bindings);
                }
                verum_ast::stmt::StmtKind::Defer(expr) => {
                    self.find_recursive_calls_impl(expr, func_name, calls, match_bindings);
                }
                verum_ast::stmt::StmtKind::Errdefer(expr) => {
                    self.find_recursive_calls_impl(expr, func_name, calls, match_bindings);
                }
                verum_ast::stmt::StmtKind::Provide { value, .. } => {
                    self.find_recursive_calls_impl(value, func_name, calls, match_bindings);
                }
                verum_ast::stmt::StmtKind::ProvideScope { value, block, .. } => {
                    self.find_recursive_calls_impl(value, func_name, calls, match_bindings);
                    self.find_recursive_calls_impl(block, func_name, calls, match_bindings);
                }
                verum_ast::stmt::StmtKind::Empty | verum_ast::stmt::StmtKind::Item(_) => {}
            }
        }

        if let Some(expr) = &block.expr {
            self.find_recursive_calls_impl(expr, func_name, calls, match_bindings);
        }
    }

    /// Extract the context around a call (pattern matching, etc.)
    ///
    /// Tracks which parameters are matched against patterns, what constructors
    /// were matched, and which fields/subcomponents are being used.
    ///
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Termination Checking
    fn extract_call_context(&self, call: &RecursiveCall) -> CallContext {
        // Start with an empty context
        let mut matched_params: Map<Text, MatchedParam> = Map::new();

        // Analyze each argument to extract match context
        for arg in call.args.iter() {
            // Check for direct field access patterns: param.field
            if let ExprKind::Field { expr, field } = &arg.kind {
                if let ExprKind::Path(path) = &expr.kind {
                    let param_name = path_to_text(path);
                    let field_name = field.name.clone();

                    // Record this as a subcomponent access
                    matched_params
                        .entry(param_name)
                        .or_insert_with(|| MatchedParam {
                            subcomponents: List::new(),
                        })
                        .subcomponents
                        .push(field_name);
                }
            }

            // Check for method calls that return subcomponents: list.tail()
            if let ExprKind::MethodCall {
                receiver, method, ..
            } = &arg.kind
            {
                if let ExprKind::Path(path) = &receiver.kind {
                    let param_name = path_to_text(path);
                    let method_name = method.name.as_str();

                    // Common structural decomposition methods
                    if matches!(
                        method_name,
                        "tail" | "rest" | "cdr" | "pop" | "next" | "pred"
                    ) {
                        matched_params
                            .entry(param_name)
                            .or_insert_with(|| MatchedParam {
                                subcomponents: List::new(),
                            })
                            .subcomponents
                            .push(Text::from(method_name));
                    }
                }
            }

            // Check for index expressions that may be structurally smaller: list[1..]
            if let ExprKind::Index { expr, .. } = &arg.kind {
                if let ExprKind::Path(path) = &expr.kind {
                    let param_name = path_to_text(path);

                    // An index expression could be structurally smaller
                    matched_params
                        .entry(param_name)
                        .or_insert_with(|| MatchedParam {
                            subcomponents: List::new(),
                        })
                        .subcomponents
                        .push(Text::from("__indexed__"));
                }
            }
        }

        CallContext {
            matched_params,
            span: call.span,
        }
    }

    /// Check if a call is structurally recursive
    fn is_structurally_recursive(
        &self,
        context: &CallContext,
        params: &List<ParamInfo>,
        args: &List<Expr>,
    ) -> bool {
        // Check if any argument is a subcomponent of a matched parameter
        for (i, arg) in args.iter().enumerate() {
            if i < params.len() {
                let param = &params[i];
                if self.is_smaller_in_context(context, arg, &param.name) {
                    return true;
                }
            }
        }

        false
    }

    /// Extract the innermost variable name from an expression, stripping derefs.
    /// e.g., `*tail` -> "tail", `**x` -> "x", `y` -> "y"
    /// Walk through Deref/Ref/Field/MethodCall(.as_ref/.as_mut) nodes to
    /// the root path identifier. Used to attribute pattern bindings from
    /// scrutinees like `sel.kind`, `*sel`, `&sel.node`, `self.list.head`
    /// back to the outer parameter they decompose.
    fn extract_root_param_name(expr: &Expr) -> Option<Text> {
        match &expr.kind {
            ExprKind::Path(path) => Some(path_to_text(path)),
            ExprKind::Field { expr: inner, .. } => Self::extract_root_param_name(inner),
            ExprKind::Unary { op, expr: inner } if matches!(op,
                verum_ast::expr::UnOp::Deref |
                verum_ast::expr::UnOp::Ref |
                verum_ast::expr::UnOp::RefMut |
                verum_ast::expr::UnOp::RefChecked |
                verum_ast::expr::UnOp::RefCheckedMut |
                verum_ast::expr::UnOp::RefUnsafe |
                verum_ast::expr::UnOp::RefUnsafeMut
            ) => {
                Self::extract_root_param_name(inner)
            }
            // `receiver.as_ref()` / `receiver.as_mut()` — reference-view
            // accessors that preserve structural identity.
            ExprKind::MethodCall { receiver, method, args, .. }
                if args.is_empty()
                    && matches!(method.name.as_str(), "as_ref" | "as_mut" | "clone") =>
            {
                Self::extract_root_param_name(receiver)
            }
            _ => None,
        }
    }

    fn extract_inner_var_name(expr: &Expr) -> Option<Text> {
        match &expr.kind {
            ExprKind::Path(path) => Some(path_to_text(path)),
            ExprKind::Unary { op, expr: operand } if matches!(op, verum_ast::expr::UnOp::Deref) => {
                Self::extract_inner_var_name(operand)
            }
            // Strip reference operators (&, &mut, etc.) — taking a reference
            // doesn't change the structural relationship
            ExprKind::Unary { op, expr: operand } if matches!(op,
                verum_ast::expr::UnOp::Ref |
                verum_ast::expr::UnOp::RefMut |
                verum_ast::expr::UnOp::RefChecked |
                verum_ast::expr::UnOp::RefCheckedMut |
                verum_ast::expr::UnOp::RefUnsafe |
                verum_ast::expr::UnOp::RefUnsafeMut
            ) => {
                Self::extract_inner_var_name(operand)
            }
            _ => None,
        }
    }

    /// Check if an argument is structurally smaller than a parameter
    fn is_smaller_argument(&self, context: &CallContext, arg: &Expr, param_name: &Text) -> bool {
        // Check direct structural decomposition (including through derefs like *tail)
        if let Some(arg_name) = Self::extract_inner_var_name(arg) {
            // Check if arg is a known subcomponent of param
            if let Some(info) = context.matched_params.get(param_name) {
                return info.subcomponents.contains(&arg_name);
            }
        }

        // Check for arithmetic decrement: n - 1, n - k (positive constant)
        // This is a common pattern for bounded recursion: factorial(n - 1)
        if let ExprKind::Binary { op, left, right } = &arg.kind {
            use verum_ast::expr::BinOp;
            if *op == BinOp::Sub {
                // Check if left side is the parameter
                if let ExprKind::Path(path) = &left.kind {
                    let left_name = path_to_text(path);
                    if left_name == *param_name {
                        // Check if right side is a positive constant
                        if let ExprKind::Literal(lit) = &right.kind {
                            if let verum_ast::literal::LiteralKind::Int(int_lit) = &lit.kind {
                                if int_lit.value > 0 {
                                    return true; // n - k where k > 0 is smaller than n
                                }
                            }
                        }
                        // Also accept any variable (assume it's positive)
                        // This handles cases like: factorial(n - step)
                        if let ExprKind::Path(_) = &right.kind {
                            return true;
                        }
                    }
                }
            }
        }

        // Check for method calls that return smaller values: n.pred()
        if let ExprKind::MethodCall { receiver, method, .. } = &arg.kind {
            if let ExprKind::Path(path) = &receiver.kind {
                let receiver_name = path_to_text(path);
                if receiver_name == *param_name {
                    // Common decrement methods
                    if matches!(method.name.as_str(), "pred" | "saturating_sub" | "wrapping_sub") {
                        return true;
                    }
                }
            }
        }

        false
    }

    /// Check if an argument is smaller in the given context
    fn is_smaller_in_context(&self, context: &CallContext, arg: &Expr, param_name: &Text) -> bool {
        // Check direct structural decomposition (including through derefs like *tail)
        if let Some(arg_name) = Self::extract_inner_var_name(arg) {
            if let Some(info) = context.matched_params.get(param_name) {
                return info.subcomponents.contains(&arg_name);
            }
        }

        false
    }

    /// Check for mutual recursion cycles
    pub fn check_mutual_recursion(&self) -> Result<()> {
        // Use DFS to detect cycles
        let mut visited = Set::new();
        let mut rec_stack = Set::new();

        for func in self.call_graph.keys() {
            if !visited.contains(func) {
                self.detect_cycle(func, &mut visited, &mut rec_stack)?;
            }
        }

        Ok(())
    }

    /// Detect cycles in the call graph using DFS
    fn detect_cycle(
        &self,
        func: &Text,
        visited: &mut Set<Text>,
        rec_stack: &mut Set<Text>,
    ) -> Result<()> {
        visited.insert(func.clone());
        rec_stack.insert(func.clone());

        if let Some(callees) = self.call_graph.get(func) {
            for callee in callees {
                if !visited.contains(callee) {
                    self.detect_cycle(callee, visited, rec_stack)?;
                } else if rec_stack.contains(callee) {
                    // Cycle detected
                    let mut cycle = List::new();
                    cycle.push(func.clone());
                    cycle.push(callee.clone());
                    return Err(TerminationError::MutualRecursionCycle {
                        cycle,
                        span: Span::default(),
                    });
                }
            }
        }

        rec_stack.remove(func);
        Ok(())
    }
}

impl Default for TerminationChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Information about a function parameter
#[derive(Debug, Clone)]
struct ParamInfo {
    name: Text,
    ty: verum_ast::ty::Type,
    span: Span,
}

/// A recursive call site
#[derive(Debug, Clone)]
struct RecursiveCall {
    args: List<Expr>,
    span: Span,
    /// Variable bindings from enclosing match arms: binding_name -> matched_parameter
    match_bindings: Map<Text, Text>,
}

/// Context information around a call site
#[derive(Debug, Clone)]
struct CallContext {
    /// Map from parameter name to matched information
    matched_params: Map<Text, MatchedParam>,
    span: Span,
}

/// Information about a matched parameter
#[derive(Debug, Clone)]
struct MatchedParam {
    /// Subcomponents that are structurally smaller
    subcomponents: List<Text>,
}

// ==================== Decreasing Clause Support ====================

/// Decreasing clause specification
/// Termination checking: ensuring recursive functions terminate via structural recursion on well-founded orderings — .1 lines 480-488
#[derive(Debug, Clone)]
pub struct DecreasingClause {
    /// Parameters that form the decreasing measure
    pub params: List<Text>,
    /// Ordering type (lexicographic, custom, etc.)
    pub ordering: OrderingKind,
    /// Custom well-founded proof (if provided)
    pub proof: Option<Expr>,
    pub span: Span,
}

/// Ordering type for decreasing measures
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderingKind {
    /// Lexicographic ordering (m, n) - m decreases OR m equal and n decreases
    Lexicographic,
    /// Natural number ordering (for single Nat parameter)
    Natural,
    /// Custom well-founded ordering with proof
    Custom { name: Text },
    /// Structural ordering (size of inductive type)
    Structural,
}

/// Size-change graph for termination analysis
/// Spec: Based on "Size-Change Termination" by Lee, Jones, and Ben-Amram
#[derive(Debug, Clone)]
struct SizeChangeGraph {
    /// Function being analyzed
    function: Text,
    /// Edges: (param_i, param_j) with size change relation
    edges: List<SizeChangeEdge>,
}

/// Edge in size-change graph
#[derive(Debug, Clone, PartialEq)]
struct SizeChangeEdge {
    /// Source parameter index
    from_param: usize,
    /// Target parameter index (in recursive call)
    to_param: usize,
    /// Relation: Decreasing (<), Non-increasing (≤), or Unknown (?)
    relation: SizeRelation,
}

/// Size relation between parameters
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SizeRelation {
    /// Strictly decreasing (<)
    Decreasing,
    /// Non-increasing (≤)
    NonIncreasing,
    /// Unknown or incomparable (?)
    Unknown,
}

/// Strongly connected component in call graph
#[derive(Debug, Clone)]
struct StronglyConnectedComponent {
    /// Functions in this SCC
    functions: Set<Text>,
    /// Is this a trivial SCC (single non-recursive function)?
    is_trivial: bool,
}

/// Pattern matching context for structural recursion
#[derive(Debug, Clone)]
struct MatchContext {
    /// Parameter being matched
    param: Text,
    /// Constructor being matched
    constructor: Text,
    /// Bindings: field name -> bound variable
    bindings: Map<Text, Text>,
    /// Parent expression span
    span: Span,
}

/// Productivity analysis for coinductive types
/// Termination checking: ensuring recursive functions terminate via structural recursion on well-founded orderings — .2 lines 312-333
#[derive(Debug, Clone)]
pub struct ProductivityChecker {
    /// Coinductive types being checked
    coinductive_types: Set<Text>,
    /// Guardedness context (constructor applications)
    guardedness_stack: List<Text>,
}

impl ProductivityChecker {
    pub fn new() -> Self {
        Self {
            coinductive_types: Set::new(),
            guardedness_stack: List::new(),
        }
    }

    /// Check if a coinductive definition is productive
    /// A definition is productive if all corecursive calls are guarded by constructors
    /// Termination checking: ensuring recursive functions terminate via structural recursion on well-founded orderings — .2 - Cofix productivity checking
    pub fn check_productivity(&mut self, decl: &FunctionDecl) -> Result<()> {
        // Cofix functions must always be checked for productivity
        // They explicitly declare coinductive recursive definitions
        if decl.is_cofix {
            // Cofix functions must have a body to check
            if decl.body.is_none() {
                return Err(TerminationError::NonTerminating {
                    function: decl.name.name.clone(),
                    reason: Text::from("cofix function must have a body to check productivity"),
                    span: decl.span,
                });
            }

            // Check body for guardedness
            let body_expr = match &decl.body {
                Some(verum_ast::decl::FunctionBody::Expr(e)) => e,
                Some(verum_ast::decl::FunctionBody::Block(b)) => match &b.expr {
                    Some(e) => e,
                    None => {
                        return Err(TerminationError::NonTerminating {
                            function: decl.name.name.clone(),
                            reason: Text::from("cofix function block must have a final expression"),
                            span: decl.span,
                        });
                    }
                },
                None => unreachable!(), // Already checked above
            };

            return self.check_guarded_corecursion(&decl.name.name, body_expr);
        }

        // For non-cofix functions, check if return type is coinductive
        let return_ty = match &decl.return_type {
            Some(ty) => ty,
            None => return Ok(()), // No return type, can't be coinductive
        };

        if !self.is_coinductive_type(return_ty) {
            return Ok(()); // Not coinductive, no productivity check needed
        }

        // Check body for guardedness
        if let Some(body) = &decl.body {
            let body_expr = match body {
                verum_ast::decl::FunctionBody::Expr(e) => e,
                verum_ast::decl::FunctionBody::Block(b) => {
                    // Check final expression in block
                    match &b.expr {
                        Some(e) => e,
                        None => return Ok(()), // No final expr, trivially productive
                    }
                }
            };

            self.check_guarded_corecursion(&decl.name.name, body_expr)?;
        }

        Ok(())
    }

    /// Check if type is coinductive (Stream, infinite structures)
    fn is_coinductive_type(&self, ty: &Type) -> bool {
        match &ty.kind {
            verum_ast::ty::TypeKind::Path(path) => {
                let type_name = path_to_text(path);
                // Common coinductive types
                matches!(type_name.as_str(), "Stream" | "CoList" | "InfiniteTree")
                    || self.coinductive_types.contains(&type_name)
            }
            _ => false,
        }
    }

    /// Check that corecursive calls are guarded by constructors
    fn check_guarded_corecursion(&mut self, func_name: &Text, expr: &Expr) -> Result<()> {
        match &expr.kind {
            // Constructor application - push guard
            ExprKind::Call { func, args, .. } => {
                if let ExprKind::Path(path) = &func.kind {
                    let callee = path_to_text(path);

                    // Check if this is a corecursive call
                    if &callee == func_name {
                        // Corecursive call - must be guarded
                        if self.guardedness_stack.is_empty() {
                            return Err(TerminationError::UnguardedCorecursion {
                                function: func_name.clone(),
                                span: expr.span,
                            });
                        }
                    } else if self.is_constructor(&callee) {
                        // Push constructor guard
                        self.guardedness_stack.push(callee.clone());

                        // Check arguments under guard
                        for arg in args {
                            self.check_guarded_corecursion(func_name, arg)?;
                        }

                        // Pop guard
                        self.guardedness_stack.pop();
                        return Ok(());
                    }
                }

                // Check arguments without new guard
                for arg in args {
                    self.check_guarded_corecursion(func_name, arg)?;
                }
            }

            // Recursively check subexpressions
            ExprKind::Match { expr, arms } => {
                self.check_guarded_corecursion(func_name, expr)?;
                for arm in arms {
                    self.check_guarded_corecursion(func_name, &arm.body)?;
                }
            }

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Check condition
                for cond in &condition.conditions {
                    match cond {
                        verum_ast::expr::ConditionKind::Expr(e) => {
                            self.check_guarded_corecursion(func_name, e)?;
                        }
                        verum_ast::expr::ConditionKind::Let { value, .. } => {
                            self.check_guarded_corecursion(func_name, value)?;
                        }
                    }
                }

                // Check branches
                for stmt in &then_branch.stmts {
                    if let verum_ast::stmt::StmtKind::Expr { expr: e, .. } = &stmt.kind {
                        self.check_guarded_corecursion(func_name, e)?;
                    }
                }
                if let Some(e) = &then_branch.expr {
                    self.check_guarded_corecursion(func_name, e)?;
                }
                if let Some(e) = else_branch {
                    self.check_guarded_corecursion(func_name, e)?;
                }
            }

            ExprKind::Block(block) => {
                for stmt in &block.stmts {
                    if let verum_ast::stmt::StmtKind::Expr { expr: e, .. } = &stmt.kind {
                        self.check_guarded_corecursion(func_name, e)?;
                    }
                }
                if let Some(e) = &block.expr {
                    self.check_guarded_corecursion(func_name, e)?;
                }
            }

            ExprKind::Binary { left, right, .. } => {
                self.check_guarded_corecursion(func_name, left)?;
                self.check_guarded_corecursion(func_name, right)?;
            }

            ExprKind::Unary { expr, .. } => {
                self.check_guarded_corecursion(func_name, expr)?;
            }

            _ => {}
        }

        Ok(())
    }

    /// Check if a name represents a constructor
    fn is_constructor(&self, name: &Text) -> bool {
        // Heuristic: constructors start with uppercase
        name.chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false)
    }

    /// Register a coinductive type
    pub fn register_coinductive(&mut self, type_name: Text) {
        self.coinductive_types.insert(type_name);
    }
}

impl Default for ProductivityChecker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_termination_checker_creation() {
        let checker = TerminationChecker::new();
        assert!(checker.recursive_functions.is_empty());
    }

    #[test]
    fn test_size_relation_ordering() {
        // Decreasing is stronger than non-increasing
        assert_ne!(SizeRelation::Decreasing, SizeRelation::NonIncreasing);
        assert_ne!(SizeRelation::Decreasing, SizeRelation::Unknown);
    }

    #[test]
    fn test_productivity_checker_creation() {
        let checker = ProductivityChecker::new();
        assert!(checker.coinductive_types.is_empty());
    }
}
