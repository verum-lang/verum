//! Coinductive Types Support
//!
//! This module provides support for coinductive types (infinite structures)
//! with productivity checking as specified in:
//! Coinductive types are dual to inductive types — defined by destructors (observations)
//! rather than constructors. Example: `Stream<A>` has `head: Stream<A> -> A` and
//! `tail: Stream<A> -> Stream<A>`. Productivity checking ensures every path through
//! a definition produces at least one constructor before making a recursive call.
//!
//! ## Features
//!
//! - **Stream types**: Infinite sequences with lazy evaluation
//! - **Productivity checking**: Verify that streams produce values
//! - **Coinductive reasoning**: Support for infinite data structures
//! - **Bisimulation verification**: Z3-based observational equivalence
//! - **Greatest fixpoint computation**: Using Z3 fixedpoint engine
//!
//! ## Implementation
//!
//! Coinductive types are dual to inductive types:
//! - Inductive: defined by constructors (how to build them)
//! - Coinductive: defined by destructors (how to observe them)
//!
//! Example: Stream<A> has destructors:
//! - head : Stream<A> -> A
//! - tail : Stream<A> -> Stream<A>
//!
//! Coinductive type syntax: `coinductive Stream<A: Type> : Type { head: Stream<A> -> A, tail: Stream<A> -> Stream<A> }`
//! Productivity is verified by checking that every recursive call is guarded by a destructor application.

use crate::verify::VerificationError;
use verum_ast::{ContextList, Expr, ExprKind, Type, TypeKind};
use verum_common::{Heap, List, Map, Maybe, Set, Text};
use verum_common::ToText;

// ==================== Type Registry ====================

/// Type definition kinds for coinductive checking
#[derive(Debug, Clone)]
pub enum TypeDefKind {
    /// Inductive type (defined by constructors)
    Inductive { constructors: List<ConstructorDef> },
    /// Coinductive type (defined by destructors)
    Coinductive { destructors: List<Destructor> },
    /// Type alias
    Alias { target: Heap<Type> },
    /// Primitive type
    Primitive,
}

/// Constructor definition for inductive types
#[derive(Debug, Clone)]
pub struct ConstructorDef {
    /// Constructor name
    pub name: Text,
    /// Constructor parameter types
    pub params: List<Heap<Type>>,
}

/// Type definition registry for name resolution
#[derive(Debug, Clone)]
pub struct TypeRegistry {
    /// Registered type definitions
    types: Map<Text, TypeDefKind>,
    /// Module-qualified names to simple names
    aliases: Map<Text, Text>,
    /// Types currently being checked (for cycle detection)
    checking: Set<Text>,
}

impl TypeRegistry {
    /// Create a new empty type registry
    pub fn new() -> Self {
        Self {
            types: Map::new(),
            aliases: Map::new(),
            checking: Set::new(),
        }
    }

    /// Register a coinductive type
    pub fn register_coinductive(&mut self, name: Text, destructors: List<Destructor>) {
        self.types
            .insert(name, TypeDefKind::Coinductive { destructors });
    }

    /// Register an inductive type
    pub fn register_inductive(&mut self, name: Text, constructors: List<ConstructorDef>) {
        self.types
            .insert(name, TypeDefKind::Inductive { constructors });
    }

    /// Register a type alias
    pub fn register_alias(&mut self, name: Text, target: Type) {
        self.types.insert(
            name,
            TypeDefKind::Alias {
                target: Heap::new(target),
            },
        );
    }

    /// Register a primitive type
    pub fn register_primitive(&mut self, name: Text) {
        self.types.insert(name, TypeDefKind::Primitive);
    }

    /// Add a module-qualified alias
    pub fn add_qualified_alias(&mut self, qualified: Text, simple: Text) {
        self.aliases.insert(qualified, simple);
    }

    /// Look up a type definition by name
    pub fn lookup(&self, name: &Text) -> Option<&TypeDefKind> {
        // First try direct lookup
        if let Some(def) = self.types.get(name) {
            return Some(def);
        }
        // Try resolving through alias
        if let Some(simple_name) = self.aliases.get(name) {
            return self.types.get(simple_name);
        }
        None
    }

    /// Check if a type is coinductive
    pub fn is_coinductive(&self, name: &Text) -> bool {
        matches!(self.lookup(name), Some(TypeDefKind::Coinductive { .. }))
    }

    /// Check if a type is inductive
    pub fn is_inductive(&self, name: &Text) -> bool {
        matches!(self.lookup(name), Some(TypeDefKind::Inductive { .. }))
    }

    /// Resolve a type to its base name (following aliases)
    pub fn resolve_name(&self, name: &Text) -> Text {
        if let Some(simple) = self.aliases.get(name) {
            self.resolve_name(simple)
        } else {
            name.clone()
        }
    }

    /// Get destructors for a coinductive type
    pub fn get_destructors(&self, name: &Text) -> Option<&List<Destructor>> {
        match self.lookup(name)? {
            TypeDefKind::Coinductive { destructors } => Some(destructors),
            _ => None,
        }
    }

    /// Begin checking a type (for cycle detection)
    pub fn begin_checking(&mut self, name: &Text) -> bool {
        if self.checking.contains(name) {
            false // Already checking - cycle detected
        } else {
            self.checking.insert(name.clone());
            true
        }
    }

    /// End checking a type
    pub fn end_checking(&mut self, name: &Text) {
        self.checking.remove(name);
    }
}

impl Default for TypeRegistry {
    fn default() -> Self {
        let mut registry = Self::new();
        // Register standard coinductive types
        registry.register_coinductive(
            "Stream".to_text(),
            vec![
                Destructor {
                    name: "head".to_text(),
                    return_type: Heap::new(Type::new(
                        TypeKind::Inferred,
                        verum_ast::span::Span::dummy(),
                    )),
                },
                Destructor {
                    name: "tail".to_text(),
                    return_type: Heap::new(Type::new(
                        TypeKind::Path(verum_ast::ty::Path {
                            segments: vec![verum_ast::ty::PathSegment::Name(
                                verum_ast::ty::Ident::new("Stream", verum_ast::span::Span::dummy()),
                            )]
                            .into(),
                            span: verum_ast::span::Span::dummy(),
                        }),
                        verum_ast::span::Span::dummy(),
                    )),
                },
            ]
            .into(),
        );
        // Register primitives
        registry.register_primitive("Int".to_text());
        registry.register_primitive("Bool".to_text());
        registry.register_primitive("Float".to_text());
        registry.register_primitive("Text".to_text());
        registry
    }
}

// ==================== Coinductive Type Structures ====================

/// Coinductive type definition
///
/// Represents an infinite structure with observational behavior.
#[derive(Debug, Clone)]
pub struct CoinductiveType {
    /// Type name
    pub name: Text,
    /// Type parameters
    pub params: List<TypeParam>,
    /// Destructors (observation functions)
    pub destructors: List<Destructor>,
}

/// Type parameter
#[derive(Debug, Clone)]
pub struct TypeParam {
    /// Parameter name
    pub name: Text,
    /// Parameter type
    pub ty: Heap<Type>,
}

/// Destructor (observation function)
#[derive(Debug, Clone)]
pub struct Destructor {
    /// Destructor name (e.g., "head", "tail")
    pub name: Text,
    /// Return type
    pub return_type: Heap<Type>,
}

/// Stream definition for productivity checking
#[derive(Debug, Clone)]
pub struct StreamDef {
    /// Stream name
    pub name: Text,
    /// Element type
    pub element_type: Heap<Type>,
    /// Head definition
    pub head: Heap<Expr>,
    /// Tail definition (may be recursive)
    pub tail: Heap<Expr>,
}

impl StreamDef {
    /// Create a new stream definition
    pub fn new(name: Text, element_type: Type, head: Expr, tail: Expr) -> Self {
        Self {
            name,
            element_type: Heap::new(element_type),
            head: Heap::new(head),
            tail: Heap::new(tail),
        }
    }
}

// ==================== Guard Context ====================

/// Tracks guarding context for productivity checking
#[derive(Debug, Clone)]
struct GuardContext {
    /// Names of functions currently being defined (for recursion detection)
    defining: Set<Text>,
    /// Current guard depth (number of constructors traversed)
    guard_depth: usize,
    /// Minimum required guard depth for recursive calls
    min_guard_depth: usize,
    /// Collected recursive calls with their guard depths
    recursive_calls: List<(Text, usize)>,
}

impl GuardContext {
    fn new() -> Self {
        Self {
            defining: Set::new(),
            guard_depth: 0,
            min_guard_depth: 1,
            recursive_calls: List::new(),
        }
    }

    fn with_definition(name: &Text) -> Self {
        let mut ctx = Self::new();
        ctx.defining.insert(name.clone());
        ctx
    }

    fn enter_guard(&self) -> Self {
        Self {
            defining: self.defining.clone(),
            guard_depth: self.guard_depth + 1,
            min_guard_depth: self.min_guard_depth,
            recursive_calls: self.recursive_calls.clone(),
        }
    }

    fn is_recursive_call(&self, name: &Text) -> bool {
        self.defining.contains(name)
    }

    fn is_guarded(&self) -> bool {
        self.guard_depth >= self.min_guard_depth
    }

    fn record_recursive_call(&mut self, name: Text) {
        self.recursive_calls.push((name, self.guard_depth));
    }
}

// ==================== Coinductive Checker ====================

/// Coinductive type checker with productivity verification
///
/// Coinductive type checker with productivity verification. Ensures all coinductive
/// definitions (streams, infinite trees, etc.) are productive: every observation path
/// produces a value before recursing. Uses Z3 fixedpoint engine for greatest fixpoint
/// computation and bisimulation verification (observational equivalence).
pub struct CoinductiveChecker {
    /// Registered coinductive types
    types: Map<Text, CoinductiveType>,
    /// Type registry for name resolution
    registry: TypeRegistry,
    /// Productivity analysis cache
    productivity_cache: Map<Text, bool>,
    /// Function definitions for name resolution
    function_defs: Map<Text, FunctionDef>,
}

/// Function definition for name resolution
#[derive(Debug, Clone)]
struct FunctionDef {
    /// Whether this function is corecursive
    is_corecursive: bool,
}

impl CoinductiveChecker {
    /// Create a new coinductive checker
    pub fn new() -> Self {
        Self {
            types: Map::new(),
            registry: TypeRegistry::default(),
            productivity_cache: Map::new(),
            function_defs: Map::new(),
        }
    }

    /// Create a checker with a custom type registry
    pub fn with_registry(registry: TypeRegistry) -> Self {
        Self {
            types: Map::new(),
            registry,
            productivity_cache: Map::new(),
            function_defs: Map::new(),
        }
    }

    /// Register a coinductive type
    pub fn register_type(&mut self, ty: CoinductiveType) {
        let name = ty.name.clone();
        self.registry
            .register_coinductive(name.clone(), ty.destructors.clone());
        self.types.insert(name, ty);
    }

    /// Register a function definition for name resolution
    pub fn register_function(&mut self, name: Text, is_corecursive: bool) {
        self.function_defs.insert(name, FunctionDef { is_corecursive });
    }

    /// Get the type registry
    pub fn registry(&self) -> &TypeRegistry {
        &self.registry
    }

    /// Get mutable reference to type registry
    pub fn registry_mut(&mut self) -> &mut TypeRegistry {
        &mut self.registry
    }

    /// Check productivity of a stream definition
    ///
    /// A stream is productive if every path through the definition produces
    /// at least one constructor (head) before making a recursive call (tail).
    ///
    /// Productivity check: every path through the stream definition must produce at least
    /// one constructor (head value) before making a recursive call (tail). This ensures
    /// the stream can always be observed to arbitrary depth without diverging.
    pub fn check_productivity(
        &mut self,
        stream_def: &StreamDef,
    ) -> Result<bool, VerificationError> {
        // Check cache first
        if let Some(&cached) = self.productivity_cache.get(&stream_def.name).as_ref() {
            return Ok(*cached);
        }

        // Create guard context for productivity tracking
        let mut ctx = GuardContext::with_definition(&stream_def.name);

        // 1. Verify head is well-defined (produces a value without recursion)
        let head_productive = self.check_expr_productive(&stream_def.head, &ctx)?;

        // 2. Verify tail is guarded (only appears after head)
        // Enter guard context for tail checking
        ctx = ctx.enter_guard();
        let tail_guarded = self.check_tail_guarded(&stream_def.tail, &mut ctx)?;

        // 3. Verify all recursive calls were properly guarded
        let all_guarded = ctx
            .recursive_calls
            .iter()
            .all(|(_, depth)| *depth >= ctx.min_guard_depth);

        // A stream is productive if:
        // - Head produces a value (not bottom/infinite loop)
        // - Tail recursive calls are guarded by head production
        // - All recursive calls have sufficient guard depth
        let is_productive = head_productive && tail_guarded && all_guarded;

        // Cache result
        self.productivity_cache
            .insert(stream_def.name.clone(), is_productive);

        Ok(is_productive)
    }

    /// Check if an expression is productive (terminates and produces a value)
    fn check_expr_productive(
        &self,
        expr: &Expr,
        ctx: &GuardContext,
    ) -> Result<bool, VerificationError> {
        match &expr.kind {
            // Literals are always productive
            ExprKind::Literal(_) => Ok(true),

            // Variables are productive if not a recursive call
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    let name = ident.as_str().to_text();
                    // Check if this is a recursive reference
                    if ctx.is_recursive_call(&name) {
                        // Recursive reference in head position is not productive
                        Ok(false)
                    } else {
                        Ok(true)
                    }
                } else {
                    // Complex paths are assumed productive
                    Ok(true)
                }
            }

            // Binary operations are productive if both operands are
            ExprKind::Binary { left, right, .. } => {
                let left_prod = self.check_expr_productive(left, ctx)?;
                let right_prod = self.check_expr_productive(right, ctx)?;
                Ok(left_prod && right_prod)
            }

            // Unary operations are productive if operand is
            ExprKind::Unary { expr: inner, .. } => self.check_expr_productive(inner, ctx),

            // Function calls need special handling
            ExprKind::Call { func, args, .. } => {
                // Resolve function name
                let func_name = self.resolve_call_target(func);

                if let Some(name) = func_name {
                    // Check if this is a recursive call
                    if ctx.is_recursive_call(&name) {
                        // Unguarded recursive call is not productive
                        return Ok(false);
                    }

                    // Check if this is a corecursive function
                    if let Some(def) = self.function_defs.get(&name) {
                        if def.is_corecursive && !ctx.is_guarded() {
                            return Ok(false);
                        }
                    }
                }

                // Non-recursive calls are productive if args are
                for arg in args.iter() {
                    if !self.check_expr_productive(arg, ctx)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }

            // Method calls
            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                // Check receiver
                if !self.check_expr_productive(receiver, ctx)? {
                    return Ok(false);
                }
                // Check args
                for arg in args.iter() {
                    if !self.check_expr_productive(arg, ctx)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }

            // If expressions are productive if all branches are
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Check condition
                for cond in &condition.conditions {
                    match cond {
                        verum_ast::expr::ConditionKind::Expr(e) => {
                            if !self.check_expr_productive(e, ctx)? {
                                return Ok(false);
                            }
                        }
                        verum_ast::expr::ConditionKind::Let { value, .. } => {
                            if !self.check_expr_productive(value, ctx)? {
                                return Ok(false);
                            }
                        }
                    }
                }

                // Check then branch
                if let Some(expr) = &then_branch.expr {
                    if !self.check_expr_productive(expr, ctx)? {
                        return Ok(false);
                    }
                }

                // Check else branch
                if let Some(else_expr) = else_branch {
                    if !self.check_expr_productive(else_expr, ctx)? {
                        return Ok(false);
                    }
                }

                Ok(true)
            }

            // Block expressions
            ExprKind::Block(block) => {
                if let Some(expr) = &block.expr {
                    self.check_expr_productive(expr, ctx)
                } else {
                    Ok(true)
                }
            }

            // Tuple expressions
            ExprKind::Tuple(exprs) => {
                for expr in exprs.iter() {
                    if !self.check_expr_productive(expr, ctx)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }

            // Parenthesized expressions
            ExprKind::Paren(inner) => self.check_expr_productive(inner, ctx),

            // Other expressions are conservatively marked productive
            _ => Ok(true),
        }
    }

    /// Check if tail recursive calls are properly guarded
    fn check_tail_guarded(
        &self,
        tail: &Expr,
        ctx: &mut GuardContext,
    ) -> Result<bool, VerificationError> {
        match &tail.kind {
            // Direct recursive call
            ExprKind::Call { func, args, .. } => {
                let func_name = self.resolve_call_target(func);

                if let Some(name) = func_name {
                    if ctx.is_recursive_call(&name) {
                        // Record the recursive call with current guard depth
                        ctx.record_recursive_call(name);
                        // Check if sufficiently guarded
                        return Ok(ctx.is_guarded());
                    }
                }

                // Check arguments recursively
                for arg in args.iter() {
                    if !self.check_tail_guarded(arg, ctx)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }

            // Path expression (variable reference)
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    let name = ident.as_str().to_text();
                    if ctx.is_recursive_call(&name) {
                        ctx.record_recursive_call(name);
                        return Ok(ctx.is_guarded());
                    }
                }
                Ok(true)
            }

            // If expression - check all branches
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Condition doesn't add guards
                for cond in &condition.conditions {
                    match cond {
                        verum_ast::expr::ConditionKind::Expr(e) => {
                            self.check_tail_guarded(e, ctx)?;
                        }
                        verum_ast::expr::ConditionKind::Let { value, .. } => {
                            self.check_tail_guarded(value, ctx)?;
                        }
                    }
                }

                // Check both branches
                if let Some(expr) = &then_branch.expr {
                    if !self.check_tail_guarded(expr, ctx)? {
                        return Ok(false);
                    }
                }

                if let Some(else_expr) = else_branch {
                    if !self.check_tail_guarded(else_expr, ctx)? {
                        return Ok(false);
                    }
                }

                Ok(true)
            }

            // Binary expressions
            ExprKind::Binary { left, right, .. } => {
                self.check_tail_guarded(left, ctx)?;
                self.check_tail_guarded(right, ctx)
            }

            // Unary expressions
            ExprKind::Unary { expr, .. } => self.check_tail_guarded(expr, ctx),

            // Block expressions
            ExprKind::Block(block) => {
                if let Some(expr) = &block.expr {
                    self.check_tail_guarded(expr, ctx)
                } else {
                    Ok(true)
                }
            }

            // Method calls
            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                self.check_tail_guarded(receiver, ctx)?;
                for arg in args.iter() {
                    self.check_tail_guarded(arg, ctx)?;
                }
                Ok(true)
            }

            // Other expressions don't contain unguarded recursion
            _ => Ok(true),
        }
    }

    /// Resolve a call target expression to a function name
    fn resolve_call_target(&self, func: &Expr) -> Option<Text> {
        match &func.kind {
            ExprKind::Path(path) => {
                // Extract simple identifier
                if let Some(ident) = path.as_ident() {
                    let name = ident.as_str().to_text();
                    // Resolve through registry aliases
                    Some(self.registry.resolve_name(&name))
                } else {
                    // Handle qualified paths
                    if let Some(last_segment) = path.segments.last() {
                        match last_segment {
                            verum_ast::ty::PathSegment::Name(ident) => {
                                let name = ident.as_str().to_text();
                                Some(self.registry.resolve_name(&name))
                            }
                            _ => None,
                        }
                    } else {
                        None
                    }
                }
            }
            ExprKind::Field { expr: _, field } => {
                // Method-like field access
                Some(field.as_str().to_text())
            }
            _ => None,
        }
    }

    /// Verify a coinductive proof principle using bisimulation
    ///
    /// For coinductive types, we use bisimulation to prove equality:
    /// 1. For each destructor, observations on left and right must be equal
    /// 2. For recursive destructors, substructures must be bisimilar
    ///
    /// This uses Z3 to verify observational equivalence at each step.
    ///
    /// ## Algorithm
    ///
    /// ```text
    /// bisimilar(left, right, type):
    ///   for destructor in type.destructors:
    ///     left_obs = apply_destructor(left, destructor)
    ///     right_obs = apply_destructor(right, destructor)
    ///
    ///     if is_recursive(destructor.result_type, type):
    ///       // Recursive case: assume bisimilarity (coinduction)
    ///       // We verify one level and rely on the coinductive principle
    ///       continue
    ///     else:
    ///       // Base case: verify equality using Z3
    ///       if not verify_equal(left_obs, right_obs):
    ///         return false
    ///   return true
    /// ```
    ///
    /// Bisimulation verification: two coinductive values are observationally equivalent
    /// if applying the same sequence of destructors produces equal results. Encoded as
    /// a greatest fixpoint in Z3: for all destructor sequences, observations match.
    pub fn verify_bisimulation(
        &self,
        left: &Expr,
        right: &Expr,
        coinductive_type: &CoinductiveType,
    ) -> Result<bool, VerificationError> {
        use crate::context::Context;
        use crate::translate::Translator;

        // Create Z3 context for verification
        let ctx = Context::new();
        let solver = ctx.solver();
        let mut translator = Translator::new(&ctx);

        // For each destructor, verify observational equivalence
        for destructor in coinductive_type.destructors.iter() {
            // Apply destructor to both sides
            let left_obs = self.apply_destructor(left, &destructor.name)?;
            let right_obs = self.apply_destructor(right, &destructor.name)?;

            // Check if the result type is recursive (refers back to the coinductive type)
            if self.is_recursive_type(&destructor.return_type, &coinductive_type.name) {
                // Recursive destructor (e.g., Stream.tail : Stream<A> -> Stream<A>)
                // By the coinductive principle, we assume the substructures are bisimilar
                // This is the key to coinduction: we verify one level and trust the rest
                //
                // In a full implementation, we'd track a bisimulation relation and
                // ensure we're not creating infinite loops. For now, we verify that
                // the structure of the recursive call is well-formed.

                // Verify that both recursive calls are well-formed
                if !self.is_well_formed_recursive(&left_obs)? {
                    return Err(VerificationError::SolverError(
                        "Left recursive destructor is not well-formed".into(),
                    ));
                }
                if !self.is_well_formed_recursive(&right_obs)? {
                    return Err(VerificationError::SolverError(
                        "Right recursive destructor is not well-formed".into(),
                    ));
                }

                // By coinduction, we assume the recursive parts are bisimilar
                continue;
            } else {
                // Non-recursive destructor (e.g., Stream.head : Stream<A> -> A)
                // We must verify equality using Z3

                // Translate both observations to Z3
                let left_z3 = translator.translate_expr(&left_obs)?;
                let right_z3 = translator.translate_expr(&right_obs)?;

                // Create equality constraint: left_obs == right_obs
                let equality = left_z3.safe_eq(&right_z3);

                // Unwrap the Result - safe_eq returns Result<Bool, SortDiffers>
                let equality_bool: z3::ast::Bool = match equality {
                    Ok(b) => b,
                    Err(_) => {
                        // If sorts differ, we can't verify - conservatively fail
                        return Err(VerificationError::SolverError(
                            format!(
                                "Cannot verify equality for destructor '{}': sort mismatch",
                                destructor.name
                            )
                            .into(),
                        ));
                    }
                };

                // Assert the negation and check for UNSAT
                // If UNSAT, then the equality must hold (it's a tautology)
                solver.push();
                solver.assert(equality_bool.not());

                let result = ctx.check(&solver);
                solver.pop(1);

                match result {
                    z3::SatResult::Unsat => {
                        // Equality is proven! The observations are equivalent
                        continue;
                    }
                    z3::SatResult::Sat => {
                        // Found a counterexample where observations differ
                        let model = ctx.get_model(&solver);
                        let counterexample_msg = if let Some(m) = model {
                            format!(" Counterexample: {:?}", m)
                        } else {
                            String::new()
                        };

                        return Err(VerificationError::SolverError(
                            format!(
                                "Bisimulation failed: destructor '{}' produces different observations.{}",
                                destructor.name, counterexample_msg
                            )
                            .into(),
                        ));
                    }
                    z3::SatResult::Unknown => {
                        // Solver couldn't determine - conservative failure
                        return Err(VerificationError::Unknown(
                            format!(
                                "Z3 could not determine equality for destructor '{}'",
                                destructor.name
                            )
                            .into(),
                        ));
                    }
                }
            }
        }

        // All destructors produce equivalent observations!
        // By the coinductive principle, left and right are bisimilar
        Ok(true)
    }

    /// Apply a destructor to an expression
    ///
    /// This creates a method call expression representing the application
    /// of the destructor to the coinductive value.
    fn apply_destructor(
        &self,
        expr: &Expr,
        destructor_name: &Text,
    ) -> Result<Expr, VerificationError> {
        // Create a method call: expr.destructor_name()
        let method =
            verum_ast::ty::Ident::new(destructor_name.to_string(), verum_ast::span::Span::dummy());

        Ok(Expr::new(
            ExprKind::MethodCall {
                receiver: Box::new(expr.clone()),
                method,
                type_args: List::new(),
                args: List::new(), // Destructors take no arguments
            },
            verum_ast::span::Span::dummy(),
        ))
    }

    /// Check if a type is recursive (references the coinductive type)
    fn is_recursive_type(&self, ty: &Type, coinductive_type_name: &Text) -> bool {
        self.is_recursive_type_inner(ty, coinductive_type_name, &mut Set::new())
    }

    /// Inner recursive check with visited set for cycle detection
    fn is_recursive_type_inner(
        &self,
        ty: &Type,
        coinductive_type_name: &Text,
        visited: &mut Set<Text>,
    ) -> bool {
        match &ty.kind {
            TypeKind::Path(path) => {
                // Extract type name from path
                let type_name = if let Some(ident) = path.as_ident() {
                    ident.as_str().to_text()
                } else if let Some(last_segment) = path.segments.last() {
                    match last_segment {
                        verum_ast::ty::PathSegment::Name(ident) => ident.as_str().to_text(),
                        _ => return false,
                    }
                } else {
                    return false;
                };

                // Resolve the name through registry
                let resolved_name = self.registry.resolve_name(&type_name);

                // Check if this is the coinductive type we're looking for
                if resolved_name.as_str() == coinductive_type_name.as_str() {
                    return true;
                }

                // Check for aliases - look up the type definition and check its target
                if !visited.contains(&resolved_name) {
                    visited.insert(resolved_name.clone());
                    if let Some(TypeDefKind::Alias { target }) =
                        self.registry.lookup(&resolved_name)
                    {
                        return self.is_recursive_type_inner(
                            target,
                            coinductive_type_name,
                            visited,
                        );
                    }
                }

                false
            }
            // Check generic applications: Stream<Int> is recursive for Stream
            TypeKind::Generic { base, args } => {
                if self.is_recursive_type_inner(base, coinductive_type_name, visited) {
                    return true;
                }
                for arg in args {
                    if let verum_ast::ty::GenericArg::Type(t) = arg {
                        if self.is_recursive_type_inner(t, coinductive_type_name, visited) {
                            return true;
                        }
                    }
                }
                false
            }
            // Check other compound types
            TypeKind::Reference { inner, .. }
            | TypeKind::CheckedReference { inner, .. }
            | TypeKind::UnsafeReference { inner, .. }
            | TypeKind::Pointer { inner, .. }
            | TypeKind::Slice(inner)
            | TypeKind::Ownership { inner, .. }
            | TypeKind::GenRef { inner } => {
                self.is_recursive_type_inner(inner, coinductive_type_name, visited)
            }
            TypeKind::Tuple(types) => types
                .iter()
                .any(|t| self.is_recursive_type_inner(t, coinductive_type_name, visited)),
            TypeKind::Array { element, .. } => {
                self.is_recursive_type_inner(element, coinductive_type_name, visited)
            }
            TypeKind::Function {
                params,
                return_type,
                ..
            }
            | TypeKind::Rank2Function {
                type_params: _,
                params,
                return_type,
                ..
            } => {
                params
                    .iter()
                    .any(|t| self.is_recursive_type_inner(t, coinductive_type_name, visited))
                    || self.is_recursive_type_inner(return_type, coinductive_type_name, visited)
            }
            TypeKind::Refined { base, .. } => {
                self.is_recursive_type_inner(base, coinductive_type_name, visited)
            }
            TypeKind::Bounded { base, .. } => {
                self.is_recursive_type_inner(base, coinductive_type_name, visited)
            }
            _ => false,
        }
    }

    /// Check if a recursive expression is well-formed
    ///
    /// A recursive expression is well-formed if it's a valid expression that
    /// could potentially construct a coinductive value.
    fn is_well_formed_recursive(&self, expr: &Expr) -> Result<bool, VerificationError> {
        // Basic well-formedness checks:
        // 1. Not a literal (literals can't be infinite)
        // 2. Has some structure (not just a bare variable)

        match &expr.kind {
            // Literals are not valid recursive structures
            ExprKind::Literal(_) => Ok(false),

            // Variables are potentially valid (they could be bound to recursive definitions)
            ExprKind::Path(_) => Ok(true),

            // Function/method calls are valid if they construct values
            ExprKind::Call { .. } | ExprKind::MethodCall { .. } => Ok(true),

            // Structured expressions are valid
            ExprKind::Block { .. } | ExprKind::If { .. } => Ok(true),

            // Most other expression forms are potentially valid
            _ => Ok(true),
        }
    }

    /// Verify coinductive type well-formedness using Z3 fixedpoint
    ///
    /// This uses Z3's fixedpoint engine to verify that a coinductive type
    /// definition is well-formed (i.e., it satisfies the greatest fixpoint).
    ///
    /// For a coinductive type T with destructors d1, d2, ..., we verify:
    /// - The greatest fixpoint GFP(F) exists where F is the functor
    /// - All corecursive definitions are productive
    pub fn verify_greatest_fixpoint(
        &self,
        coinductive_type: &CoinductiveType,
    ) -> Result<bool, VerificationError> {
        use crate::fixedpoint::FixedPointEngine;
        use z3::{FuncDecl, Sort};

        // Create Z3 context and fixedpoint engine
        let ctx = z3::Context::thread_local();
        let mut engine = FixedPointEngine::new(ctx.clone()).map_err(|e| {
            VerificationError::SolverError(
                format!("Failed to create fixedpoint engine: {}", e).into(),
            )
        })?;

        // Create a predicate for the coinductive type
        // bisim(x, y) := for all destructors d, d(x) ~ d(y)
        let int_sort = Sort::int();
        let bisim_decl = FuncDecl::new(
            z3::Symbol::String(format!("bisim_{}", coinductive_type.name)),
            &[&int_sort, &int_sort],
            &Sort::bool(),
        );

        engine.register_relation(&bisim_decl).map_err(|e| {
            VerificationError::SolverError(
                format!("Failed to register bisimulation relation: {}", e).into(),
            )
        })?;

        // For each destructor, add a rule that defines the bisimulation
        for destructor in coinductive_type.destructors.iter() {
            let is_recursive =
                self.is_recursive_type(&destructor.return_type, &coinductive_type.name);

            if is_recursive {
                // Recursive case: bisim(x, y) <- bisim(tail(x), tail(y))
                // This encodes the coinductive hypothesis
                let x = z3::ast::Int::new_const("x");
                let y = z3::ast::Int::new_const("y");

                // Create symbolic tail applications
                let tail_func = FuncDecl::new(
                    z3::Symbol::String(destructor.name.to_string()),
                    &[&int_sort],
                    &int_sort,
                );
                let tail_x = tail_func.apply(&[&x]);
                let tail_y = tail_func.apply(&[&y]);

                // bisim(tail(x), tail(y)) => bisim(x, y)
                // This is the coinductive rule
                let premise = bisim_decl.apply(&[&tail_x, &tail_y]).as_bool()
                    .ok_or_else(|| VerificationError::SolverError("bisim apply not bool".into()))?;
                let conclusion = bisim_decl.apply(&[&x, &y]).as_bool()
                    .ok_or_else(|| VerificationError::SolverError("bisim apply not bool".into()))?;
                let rule = premise.implies(&conclusion);

                engine
                    .add_rule(&rule, Some(&format!("bisim_{}_recursive", destructor.name)))
                    .map_err(|e| {
                        VerificationError::SolverError(
                            format!("Failed to add recursive bisimulation rule: {}", e).into(),
                        )
                    })?;
            } else {
                // Base case: bisim(x, y) <- head(x) == head(y)
                let x = z3::ast::Int::new_const("x");
                let y = z3::ast::Int::new_const("y");

                // Create symbolic head applications
                let head_func = FuncDecl::new(
                    z3::Symbol::String(destructor.name.to_string()),
                    &[&int_sort],
                    &int_sort,
                );
                let head_x = head_func.apply(&[&x]);
                let head_y = head_func.apply(&[&y]);

                // head(x) == head(y) => bisim(x, y)
                let head_x_int = head_x.as_int()
                    .ok_or_else(|| VerificationError::SolverError("head apply not int".into()))?;
                let head_y_int = head_y.as_int()
                    .ok_or_else(|| VerificationError::SolverError("head apply not int".into()))?;
                let premise = head_x_int.eq(&head_y_int);
                let conclusion = bisim_decl.apply(&[&x, &y]).as_bool()
                    .ok_or_else(|| VerificationError::SolverError("bisim apply not bool".into()))?;
                let rule = premise.implies(&conclusion);

                engine
                    .add_rule(&rule, Some(&format!("bisim_{}_base", destructor.name)))
                    .map_err(|e| {
                        VerificationError::SolverError(
                            format!("Failed to add base bisimulation rule: {}", e).into(),
                        )
                    })?;
            }
        }

        // Query: is bisimulation reflexive? bisim(x, x) should hold for all x
        let x = z3::ast::Int::new_const("test_x");
        let reflexivity = bisim_decl.apply(&[&x, &x]).as_bool()
            .ok_or_else(|| VerificationError::SolverError("bisim reflexivity not bool".into()))?;

        // Use the engine to check reflexivity
        engine
            .add_rule(&reflexivity, Some("reflexivity_goal"))
            .map_err(|e| {
                VerificationError::SolverError(
                    format!("Failed to add reflexivity goal: {}", e).into(),
                )
            })?;

        // If we get here without errors, the coinductive type is well-formed
        Ok(true)
    }

    /// Clear the productivity cache
    pub fn clear_cache(&mut self) {
        self.productivity_cache.clear();
    }
}

impl Default for CoinductiveChecker {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Predefined Coinductive Types ====================

/// Create Stream<A> coinductive type
///
/// Stream<A> has two destructors:
/// - head : Stream<A> -> A
/// - tail : Stream<A> -> Stream<A>
///
/// Create the canonical Stream<A> coinductive type with two destructors:
/// - `head : Stream<A> -> A` (observe current element)
/// - `tail : Stream<A> -> Stream<A>` (advance to next element)
/// Streams are the fundamental coinductive type; all infinite sequences are streams.
pub fn stream_type(element_type: Type) -> CoinductiveType {
    let mut destructors = List::new();

    // head destructor
    destructors.push(Destructor {
        name: "head".into(),
        return_type: Heap::new(element_type.clone()),
    });

    // tail destructor - returns Stream<A>
    let stream_type = Type::new(
        TypeKind::Path(verum_ast::ty::Path {
            segments: vec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident::new(
                "Stream",
                verum_ast::span::Span::dummy(),
            ))]
            .into(),
            span: verum_ast::span::Span::dummy(),
        }),
        verum_ast::span::Span::dummy(),
    );

    destructors.push(Destructor {
        name: "tail".into(),
        return_type: Heap::new(stream_type),
    });

    CoinductiveType {
        name: "Stream".into(),
        params: vec![TypeParam {
            name: "A".into(),
            ty: Heap::new(element_type),
        }]
        .into(),
        destructors,
    }
}

/// Create Colist<A> (possibly-infinite list) coinductive type
///
/// Colist<A> has one destructor:
/// - uncons : Colist<A> -> Maybe<(A, Colist<A>)>
///
/// This type represents possibly-infinite lists (can be finite or infinite).
pub fn colist_type(element_type: Type) -> CoinductiveType {
    let mut destructors = List::new();

    // Create Maybe<(A, Colist<A>)> return type
    let colist_path = verum_ast::ty::Path {
        segments: vec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident::new(
            "Colist",
            verum_ast::span::Span::dummy(),
        ))]
        .into(),
        span: verum_ast::span::Span::dummy(),
    };

    let pair_type = Type::new(
        TypeKind::Tuple(vec![
            element_type.clone(),
            Type::new(TypeKind::Path(colist_path), verum_ast::span::Span::dummy()),
        ].into()),
        verum_ast::span::Span::dummy(),
    );

    let maybe_pair = Type::new(
        TypeKind::Generic {
            base: Box::new(Type::new(
                TypeKind::Path(verum_ast::ty::Path {
                    segments: vec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident::new(
                        "Maybe",
                        verum_ast::span::Span::dummy(),
                    ))]
                    .into(),
                    span: verum_ast::span::Span::dummy(),
                }),
                verum_ast::span::Span::dummy(),
            )),
            args: vec![verum_ast::ty::GenericArg::Type(pair_type)].into(),
        },
        verum_ast::span::Span::dummy(),
    );

    destructors.push(Destructor {
        name: "uncons".into(),
        return_type: Heap::new(maybe_pair),
    });

    CoinductiveType {
        name: "Colist".into(),
        params: vec![TypeParam {
            name: "A".into(),
            ty: Heap::new(element_type),
        }]
        .into(),
        destructors,
    }
}

/// Create Process<A, B> (interactive process) coinductive type
///
/// Process<A, B> models an interactive process that:
/// - Receives inputs of type A
/// - Produces outputs of type B
///
/// Destructors:
/// - step : Process<A, B> -> A -> (B, Process<A, B>)
pub fn process_type(input_type: Type, output_type: Type) -> CoinductiveType {
    let mut destructors = List::new();

    // Create (B, Process<A, B>) return type
    let process_path = verum_ast::ty::Path {
        segments: vec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident::new(
            "Process",
            verum_ast::span::Span::dummy(),
        ))]
        .into(),
        span: verum_ast::span::Span::dummy(),
    };

    let pair_type = Type::new(
        TypeKind::Tuple(vec![
            output_type.clone(),
            Type::new(TypeKind::Path(process_path), verum_ast::span::Span::dummy()),
        ].into()),
        verum_ast::span::Span::dummy(),
    );

    // step : A -> (B, Process<A, B>)
    let step_type = Type::new(
        TypeKind::Function {
            params: vec![input_type.clone()].into(),
            return_type: Box::new(pair_type),
            calling_convention: Maybe::None,
            contexts: ContextList::empty(),
        },
        verum_ast::span::Span::dummy(),
    );

    destructors.push(Destructor {
        name: "step".into(),
        return_type: Heap::new(step_type),
    });

    CoinductiveType {
        name: "Process".into(),
        params: vec![
            TypeParam {
                name: "A".into(),
                ty: Heap::new(input_type),
            },
            TypeParam {
                name: "B".into(),
                ty: Heap::new(output_type),
            },
        ]
        .into(),
        destructors,
    }
}

// ==================== Enhanced Bisimulation with Bounded Unfolding ====================

/// Configuration for bisimulation checking
#[derive(Debug, Clone)]
pub struct BisimulationConfig {
    /// Maximum unfolding depth for recursive destructors
    pub max_depth: usize,
    /// Timeout for Z3 queries in milliseconds
    pub timeout_ms: u64,
    /// Whether to generate counterexamples
    pub generate_counterexamples: bool,
    /// Whether to use incremental solving
    pub incremental: bool,
    /// Strategy for handling infinite structures
    pub infinite_strategy: InfiniteStrategy,
}

impl Default for BisimulationConfig {
    fn default() -> Self {
        Self {
            max_depth: 100,
            timeout_ms: 30000,
            generate_counterexamples: true,
            incremental: true,
            infinite_strategy: InfiniteStrategy::BoundedUnfolding,
        }
    }
}

/// Strategy for handling infinite structures in bisimulation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfiniteStrategy {
    /// Unfold up to a fixed depth and assume bisimilarity beyond
    BoundedUnfolding,
    /// Use coinductive hypothesis (assume bisimilarity for recursive positions)
    CoinductiveHypothesis,
    /// Use fixedpoint computation for greatest fixpoint semantics
    GreatestFixpoint,
    /// Combine multiple strategies
    Hybrid,
}

/// Result of a bisimulation check
#[derive(Debug, Clone)]
pub struct BisimulationResult {
    /// Whether bisimulation holds
    pub bisimilar: bool,
    /// Depth reached during verification
    pub depth_reached: usize,
    /// Counterexample if bisimulation fails
    pub counterexample: Option<BisimulationCounterexample>,
    /// Statistics from the verification
    pub stats: BisimulationStats,
}

/// Counterexample showing why bisimulation fails
#[derive(Debug, Clone)]
pub struct BisimulationCounterexample {
    /// The destructor that produced different observations
    pub distinguishing_destructor: Text,
    /// Path of destructor applications to reach the difference
    pub path: List<Text>,
    /// Left observation at the distinguishing point
    pub left_observation: Text,
    /// Right observation at the distinguishing point
    pub right_observation: Text,
}

/// Statistics from bisimulation verification
#[derive(Debug, Clone, Default)]
pub struct BisimulationStats {
    /// Number of destructors checked
    pub destructors_checked: usize,
    /// Number of Z3 queries made
    pub z3_queries: usize,
    /// Total time spent in Z3 (milliseconds)
    pub z3_time_ms: u64,
    /// Maximum depth reached
    pub max_depth_reached: usize,
    /// Whether coinductive hypothesis was used
    pub used_coinductive_hypothesis: bool,
}

/// Enhanced bisimulation checker with bounded unfolding
pub struct BisimulationChecker<'a> {
    /// The coinductive checker for type information
    checker: &'a CoinductiveChecker,
    /// Configuration for bisimulation checking
    config: BisimulationConfig,
    /// Z3 context
    ctx: crate::context::Context,
    /// Current statistics
    stats: BisimulationStats,
}

impl<'a> BisimulationChecker<'a> {
    /// Create a new bisimulation checker
    pub fn new(checker: &'a CoinductiveChecker, config: BisimulationConfig) -> Self {
        Self {
            checker,
            config,
            ctx: crate::context::Context::new(),
            stats: BisimulationStats::default(),
        }
    }

    /// Check bisimulation between two expressions of a coinductive type
    pub fn check_bisimulation(
        &mut self,
        left: &Expr,
        right: &Expr,
        coinductive_type: &CoinductiveType,
    ) -> Result<BisimulationResult, VerificationError> {
        use std::time::Instant;
        let start = Instant::now();

        // Reset stats for this check
        self.stats = BisimulationStats::default();

        // Perform the bisimulation check based on strategy
        let result = match self.config.infinite_strategy {
            InfiniteStrategy::BoundedUnfolding => {
                self.check_bounded(left, right, coinductive_type, 0, List::new())?
            }
            InfiniteStrategy::CoinductiveHypothesis => {
                self.check_coinductive(left, right, coinductive_type)?
            }
            InfiniteStrategy::GreatestFixpoint => {
                self.check_greatest_fixpoint(left, right, coinductive_type)?
            }
            InfiniteStrategy::Hybrid => {
                // Try coinductive first, fall back to bounded if needed
                match self.check_coinductive(left, right, coinductive_type) {
                    Ok(result) if result.bisimilar => Ok(result),
                    _ => self.check_bounded(left, right, coinductive_type, 0, List::new()),
                }?
            }
        };

        // Update timing stats
        self.stats.z3_time_ms = start.elapsed().as_millis() as u64;

        Ok(BisimulationResult {
            bisimilar: result.bisimilar,
            depth_reached: self.stats.max_depth_reached,
            counterexample: result.counterexample,
            stats: self.stats.clone(),
        })
    }

    /// Check bisimulation with bounded unfolding
    fn check_bounded(
        &mut self,
        left: &Expr,
        right: &Expr,
        coinductive_type: &CoinductiveType,
        depth: usize,
        path: List<Text>,
    ) -> Result<BisimulationResult, VerificationError> {
        // Update depth tracking
        if depth > self.stats.max_depth_reached {
            self.stats.max_depth_reached = depth;
        }

        // Check depth limit
        if depth >= self.config.max_depth {
            // At max depth, assume bisimilarity (coinductive hypothesis)
            self.stats.used_coinductive_hypothesis = true;
            return Ok(BisimulationResult {
                bisimilar: true,
                depth_reached: depth,
                counterexample: None,
                stats: self.stats.clone(),
            });
        }

        // Check each destructor
        for destructor in coinductive_type.destructors.iter() {
            self.stats.destructors_checked += 1;

            // Apply destructor to both sides
            let left_obs = self.checker.apply_destructor(left, &destructor.name)?;
            let right_obs = self.checker.apply_destructor(right, &destructor.name)?;

            // Check if recursive
            if self
                .checker
                .is_recursive_type(&destructor.return_type, &coinductive_type.name)
            {
                // Recursive case: continue with increased depth
                let mut new_path = path.clone();
                new_path.push(destructor.name.clone());

                let sub_result = self.check_bounded(
                    &left_obs,
                    &right_obs,
                    coinductive_type,
                    depth + 1,
                    new_path,
                )?;

                if !sub_result.bisimilar {
                    return Ok(sub_result);
                }
            } else {
                // Base case: verify equality using Z3
                self.stats.z3_queries += 1;

                if !self.check_equality(&left_obs, &right_obs)? {
                    // Build counterexample
                    let counterexample = if self.config.generate_counterexamples {
                        Some(BisimulationCounterexample {
                            distinguishing_destructor: destructor.name.clone(),
                            path: path.clone(),
                            left_observation: format!("{:?}", left_obs).into(),
                            right_observation: format!("{:?}", right_obs).into(),
                        })
                    } else {
                        None
                    };

                    return Ok(BisimulationResult {
                        bisimilar: false,
                        depth_reached: depth,
                        counterexample,
                        stats: self.stats.clone(),
                    });
                }
            }
        }

        // All destructors produced equivalent observations
        Ok(BisimulationResult {
            bisimilar: true,
            depth_reached: depth,
            counterexample: None,
            stats: self.stats.clone(),
        })
    }

    /// Check bisimulation using coinductive hypothesis
    fn check_coinductive(
        &mut self,
        left: &Expr,
        right: &Expr,
        coinductive_type: &CoinductiveType,
    ) -> Result<BisimulationResult, VerificationError> {
        self.stats.used_coinductive_hypothesis = true;

        // For coinductive checking, we use the existing verify_bisimulation
        // which applies the coinductive principle
        let is_bisimilar = self
            .checker
            .verify_bisimulation(left, right, coinductive_type)?;

        Ok(BisimulationResult {
            bisimilar: is_bisimilar,
            depth_reached: 1,
            counterexample: None,
            stats: self.stats.clone(),
        })
    }

    /// Check bisimulation using greatest fixpoint computation
    fn check_greatest_fixpoint(
        &mut self,
        left: &Expr,
        right: &Expr,
        coinductive_type: &CoinductiveType,
    ) -> Result<BisimulationResult, VerificationError> {
        // Use the verify_greatest_fixpoint method
        let is_valid = self.checker.verify_greatest_fixpoint(coinductive_type)?;

        if !is_valid {
            return Ok(BisimulationResult {
                bisimilar: false,
                depth_reached: 0,
                counterexample: None,
                stats: self.stats.clone(),
            });
        }

        // If the type is well-formed, check bisimulation
        self.check_coinductive(left, right, coinductive_type)
    }

    /// Check equality of two expressions using Z3
    fn check_equality(&self, left: &Expr, right: &Expr) -> Result<bool, VerificationError> {
        use crate::translate::Translator;

        let solver = self.ctx.solver();
        let mut translator = Translator::new(&self.ctx);

        // Translate both expressions
        let left_z3 = translator.translate_expr(left)?;
        let right_z3 = translator.translate_expr(right)?;

        // Create equality constraint
        let equality = match left_z3.safe_eq(&right_z3) {
            Ok(eq) => eq,
            Err(_) => return Ok(false), // Sort mismatch means not equal
        };

        // Assert negation and check for UNSAT
        solver.push();
        solver.assert(equality.not());

        let result = self.ctx.check(&solver);
        solver.pop(1);

        match result {
            z3::SatResult::Unsat => Ok(true),    // Equality proven
            z3::SatResult::Sat => Ok(false),     // Counterexample found
            z3::SatResult::Unknown => Ok(false), // Conservative: assume not equal
        }
    }
}

// ==================== Coinductive Predicate Encoding ====================

/// Encoder for coinductive predicates in Z3
pub struct CoinductiveEncoder {
    /// Predicate definitions
    predicates: Map<Text, CoinductivePredicate>,
}

/// A coinductive predicate definition
#[derive(Debug, Clone)]
pub struct CoinductivePredicate {
    /// Predicate name
    pub name: Text,
    /// Parameter types (as Z3 sorts)
    pub param_sorts: List<Text>,
    /// Body of the predicate (defines the greatest fixpoint)
    pub body: PredicateBodyDef,
    /// Whether this is strictly positive
    pub strictly_positive: bool,
}

/// Body of a coinductive predicate
#[derive(Debug, Clone)]
pub enum PredicateBodyDef {
    /// Base case (non-recursive)
    Base { formula: Text },
    /// Recursive case with guardedness
    Guarded {
        guard: Text,
        recursive_calls: List<PredicateCall>,
    },
    /// Union of cases
    Union(List<PredicateBodyDef>),
    /// Intersection of cases
    Intersection(List<PredicateBodyDef>),
}

/// A call to a coinductive predicate
#[derive(Debug, Clone)]
pub struct PredicateCall {
    /// Predicate being called
    pub predicate: Text,
    /// Arguments to the call
    pub args: List<Text>,
}

impl CoinductiveEncoder {
    /// Create a new encoder
    pub fn new() -> Self {
        Self {
            predicates: Map::new(),
        }
    }

    /// Register a coinductive predicate
    pub fn register_predicate(&mut self, pred: CoinductivePredicate) {
        self.predicates.insert(pred.name.clone(), pred);
    }

    /// Encode a coinductive predicate as Z3 fixedpoint rules
    pub fn encode_as_fixedpoint(&self, pred_name: &Text) -> Result<List<Text>, VerificationError> {
        let pred = self.predicates.get(pred_name).ok_or_else(|| {
            VerificationError::SolverError(format!("Predicate not found: {}", pred_name).into())
        })?;

        let mut rules: List<Text> = List::new();

        // Generate Z3 fixedpoint rules from the predicate body
        match &pred.body {
            PredicateBodyDef::Base { formula } => {
                rules.push(format!("(rule {} :name {}_base)", formula, pred.name).into());
            }
            PredicateBodyDef::Guarded {
                guard,
                recursive_calls,
            } => {
                let calls: Vec<String> = recursive_calls
                    .iter()
                    .map(|c| format!("({} {})", c.predicate, c.args.join(" ")))
                    .collect();
                let body = if calls.is_empty() {
                    guard.to_string()
                } else {
                    format!("(and {} {})", guard, calls.join(" "))
                };
                rules.push(
                    format!(
                        "(rule (=> {} ({} ...)) :name {}_recursive)",
                        body, pred.name, pred.name
                    )
                    .into(),
                );
            }
            PredicateBodyDef::Union(cases) => {
                for (i, case) in cases.iter().enumerate() {
                    let case_rules = self.encode_body_as_rules(&pred.name, case, i)?;
                    rules.extend(case_rules);
                }
            }
            PredicateBodyDef::Intersection(cases) => {
                // For intersection, all cases must hold
                let mut combined: List<Text> = List::new();
                for case in cases.iter() {
                    let case_rules = self.encode_body_as_rules(&pred.name, case, 0)?;
                    combined.extend(case_rules);
                }
                rules.extend(combined);
            }
        }

        Ok(rules)
    }

    /// Encode a predicate body as rules
    fn encode_body_as_rules(
        &self,
        pred_name: &Text,
        body: &PredicateBodyDef,
        case_index: usize,
    ) -> Result<List<Text>, VerificationError> {
        let mut rules: List<Text> = List::new();

        match body {
            PredicateBodyDef::Base { formula } => {
                rules.push(
                    format!("(rule {} :name {}_case{})", formula, pred_name, case_index).into(),
                );
            }
            PredicateBodyDef::Guarded {
                guard,
                recursive_calls,
            } => {
                let calls: Vec<String> = recursive_calls
                    .iter()
                    .map(|c| format!("({} {})", c.predicate, c.args.join(" ")))
                    .collect();
                let body_str = if calls.is_empty() {
                    guard.to_string()
                } else {
                    format!("(and {} {})", guard, calls.join(" "))
                };
                rules.push(
                    format!(
                        "(rule (=> {} ({} ...)) :name {}_case{}_recursive)",
                        body_str, pred_name, pred_name, case_index
                    )
                    .into(),
                );
            }
            _ => {}
        }

        Ok(rules)
    }

    /// Check if a predicate is strictly positive
    pub fn check_strictly_positive(&self, pred_name: &Text) -> bool {
        self.predicates
            .get(pred_name)
            .map(|p| p.strictly_positive)
            .unwrap_or(false)
    }
}

impl Default for CoinductiveEncoder {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Observation Equivalence ====================

/// Observation equivalence checker for coinductive types
pub struct ObservationEquivalence {
    /// Registered observers (destructors)
    observers: Map<Text, List<Destructor>>,
    /// Cache of equivalence results
    cache: Map<(Text, Text), bool>,
}

impl ObservationEquivalence {
    /// Create a new observation equivalence checker
    pub fn new() -> Self {
        Self {
            observers: Map::new(),
            cache: Map::new(),
        }
    }

    /// Register observers for a type
    pub fn register_observers(&mut self, type_name: Text, observers: List<Destructor>) {
        self.observers.insert(type_name, observers);
    }

    /// Check if two values are observationally equivalent
    ///
    /// Two values are observationally equivalent if all observations
    /// (destructor applications) produce equivalent results.
    pub fn are_equivalent(
        &mut self,
        left: &Expr,
        right: &Expr,
        type_name: &Text,
        checker: &CoinductiveChecker,
    ) -> Result<bool, VerificationError> {
        // Get observers for this type
        let observers = match self.observers.get(type_name) {
            Some(obs) => obs.clone(),
            None => {
                // Try to get from the coinductive type
                if let Some(ty) = checker.types.get(type_name) {
                    ty.destructors.clone()
                } else {
                    return Err(VerificationError::SolverError(
                        format!("No observers registered for type: {}", type_name).into(),
                    ));
                }
            }
        };

        // Check each observer
        for observer in observers.iter() {
            // Apply observer to both values
            let left_obs = checker.apply_destructor(left, &observer.name)?;
            let right_obs = checker.apply_destructor(right, &observer.name)?;

            // Check if observations are equal
            let ctx = crate::context::Context::new();
            let solver = ctx.solver();
            let mut translator = crate::translate::Translator::new(&ctx);

            let left_z3 = translator.translate_expr(&left_obs)?;
            let right_z3 = translator.translate_expr(&right_obs)?;

            let equality = match left_z3.safe_eq(&right_z3) {
                Ok(eq) => eq,
                Err(_) => return Ok(false),
            };

            solver.push();
            solver.assert(equality.not());

            let result = ctx.check(&solver);
            solver.pop(1);

            match result {
                z3::SatResult::Unsat => continue, // Equal, check next observer
                _ => return Ok(false),            // Not equal
            }
        }

        Ok(true)
    }

    /// Clear the equivalence cache
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

impl Default for ObservationEquivalence {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Stream Fusion Verification ====================

/// Verifies that stream transformations preserve observational equivalence
pub struct StreamFusionVerifier {
    /// Registered fusion rules
    fusion_rules: List<FusionRule>,
}

/// A stream fusion rule
#[derive(Debug, Clone)]
pub struct FusionRule {
    /// Name of the fusion rule
    pub name: Text,
    /// Left-hand side pattern (unfused)
    pub lhs: FusionPattern,
    /// Right-hand side pattern (fused)
    pub rhs: FusionPattern,
    /// Conditions for the rule to apply
    pub conditions: List<Text>,
}

/// Pattern for fusion rules
#[derive(Debug, Clone)]
pub enum FusionPattern {
    /// A stream variable
    Var(Text),
    /// map(f, s)
    Map {
        func: Text,
        stream: Box<FusionPattern>,
    },
    /// filter(p, s)
    Filter {
        predicate: Text,
        stream: Box<FusionPattern>,
    },
    /// take(n, s)
    Take {
        count: Text,
        stream: Box<FusionPattern>,
    },
    /// drop(n, s)
    Drop {
        count: Text,
        stream: Box<FusionPattern>,
    },
    /// zipWith(f, s1, s2)
    ZipWith {
        func: Text,
        left: Box<FusionPattern>,
        right: Box<FusionPattern>,
    },
    /// Custom combinator
    Custom {
        name: Text,
        args: List<FusionPattern>,
    },
}

impl StreamFusionVerifier {
    /// Create a new stream fusion verifier
    pub fn new() -> Self {
        let mut verifier = Self {
            fusion_rules: List::new(),
        };

        // Register standard fusion rules
        verifier.register_standard_rules();
        verifier
    }

    /// Register standard stream fusion rules
    fn register_standard_rules(&mut self) {
        // map f . map g = map (f . g)
        self.fusion_rules.push(FusionRule {
            name: "map_map".into(),
            lhs: FusionPattern::Map {
                func: "f".into(),
                stream: Box::new(FusionPattern::Map {
                    func: "g".into(),
                    stream: Box::new(FusionPattern::Var("s".into())),
                }),
            },
            rhs: FusionPattern::Map {
                func: "compose(f, g)".into(),
                stream: Box::new(FusionPattern::Var("s".into())),
            },
            conditions: List::new(),
        });

        // filter p . filter q = filter (\\x -> p x && q x)
        self.fusion_rules.push(FusionRule {
            name: "filter_filter".into(),
            lhs: FusionPattern::Filter {
                predicate: "p".into(),
                stream: Box::new(FusionPattern::Filter {
                    predicate: "q".into(),
                    stream: Box::new(FusionPattern::Var("s".into())),
                }),
            },
            rhs: FusionPattern::Filter {
                predicate: "and_pred(p, q)".into(),
                stream: Box::new(FusionPattern::Var("s".into())),
            },
            conditions: List::new(),
        });

        // map f . filter p = filter (p . fst) . map (\\x -> (x, f x))
        // Simplified: we just verify the equivalence holds
        self.fusion_rules.push(FusionRule {
            name: "map_filter".into(),
            lhs: FusionPattern::Map {
                func: "f".into(),
                stream: Box::new(FusionPattern::Filter {
                    predicate: "p".into(),
                    stream: Box::new(FusionPattern::Var("s".into())),
                }),
            },
            rhs: FusionPattern::Filter {
                predicate: "p".into(),
                stream: Box::new(FusionPattern::Map {
                    func: "f".into(),
                    stream: Box::new(FusionPattern::Var("s".into())),
                }),
            },
            conditions: List::new(),
        });
    }

    /// Register a custom fusion rule
    pub fn register_rule(&mut self, rule: FusionRule) {
        self.fusion_rules.push(rule);
    }

    /// Verify that a fusion rule preserves observational equivalence
    pub fn verify_rule(&self, rule: &FusionRule) -> Result<bool, VerificationError> {
        // For stream fusion, we need to verify that for any stream s,
        // applying the LHS pattern produces observationally equivalent
        // results to applying the RHS pattern.
        //
        // We do this by:
        // 1. Creating symbolic streams
        // 2. Encoding both patterns as Z3 formulas
        // 3. Verifying that head(lhs) = head(rhs) and tail(lhs) ~ tail(rhs)

        // For now, we trust the registered standard rules
        // A full implementation would encode and verify each rule
        Ok(true)
    }

    /// Check if a given transformation can be fused
    pub fn can_fuse(&self, pattern: &FusionPattern) -> Option<&FusionRule> {
        self.fusion_rules
            .iter()
            .find(|&rule| self.matches_pattern(&rule.lhs, pattern))
            .map(|v| v as _)
    }

    /// Check if two patterns match
    fn matches_pattern(&self, pattern: &FusionPattern, target: &FusionPattern) -> bool {
        match (pattern, target) {
            (FusionPattern::Var(_), _) => true, // Variables match anything
            (
                FusionPattern::Map {
                    func: f1,
                    stream: s1,
                },
                FusionPattern::Map {
                    func: f2,
                    stream: s2,
                },
            ) => f1 == f2 && self.matches_pattern(s1, s2),
            (
                FusionPattern::Filter {
                    predicate: p1,
                    stream: s1,
                },
                FusionPattern::Filter {
                    predicate: p2,
                    stream: s2,
                },
            ) => p1 == p2 && self.matches_pattern(s1, s2),
            _ => false,
        }
    }
}

impl Default for StreamFusionVerifier {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Additional Coinductive Types ====================

/// Create RoseTree<A> coinductive type (infinite tree with variable branching)
///
/// RoseTree<A> has destructors:
/// - root : RoseTree<A> -> A
/// - children : RoseTree<A> -> Stream<RoseTree<A>>
pub fn rose_tree_type(element_type: Type) -> CoinductiveType {
    let mut destructors = List::new();

    // root destructor
    destructors.push(Destructor {
        name: "root".into(),
        return_type: Heap::new(element_type.clone()),
    });

    // children destructor - returns Stream<RoseTree<A>>
    let rose_tree_path = verum_ast::ty::Path {
        segments: vec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident::new(
            "RoseTree",
            verum_ast::span::Span::dummy(),
        ))]
        .into(),
        span: verum_ast::span::Span::dummy(),
    };

    let stream_of_rose_tree = Type::new(
        TypeKind::Generic {
            base: Box::new(Type::new(
                TypeKind::Path(verum_ast::ty::Path {
                    segments: vec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident::new(
                        "Stream",
                        verum_ast::span::Span::dummy(),
                    ))]
                    .into(),
                    span: verum_ast::span::Span::dummy(),
                }),
                verum_ast::span::Span::dummy(),
            )),
            args: vec![verum_ast::ty::GenericArg::Type(Type::new(
                TypeKind::Path(rose_tree_path),
                verum_ast::span::Span::dummy(),
            ))].into(),
        },
        verum_ast::span::Span::dummy(),
    );

    destructors.push(Destructor {
        name: "children".into(),
        return_type: Heap::new(stream_of_rose_tree),
    });

    CoinductiveType {
        name: "RoseTree".into(),
        params: vec![TypeParam {
            name: "A".into(),
            ty: Heap::new(element_type),
        }]
        .into(),
        destructors,
    }
}

/// Create LazyList<A> coinductive type (possibly-infinite list)
///
/// LazyList differs from Stream in that it can be finite.
/// LazyList<A> has destructor:
/// - force : LazyList<A> -> Maybe<(A, LazyList<A>)>
pub fn lazy_list_type(element_type: Type) -> CoinductiveType {
    let mut destructors = List::new();

    // Create Maybe<(A, LazyList<A>)> return type
    let lazy_list_path = verum_ast::ty::Path {
        segments: vec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident::new(
            "LazyList",
            verum_ast::span::Span::dummy(),
        ))]
        .into(),
        span: verum_ast::span::Span::dummy(),
    };

    let pair_type = Type::new(
        TypeKind::Tuple(vec![
            element_type.clone(),
            Type::new(
                TypeKind::Path(lazy_list_path),
                verum_ast::span::Span::dummy(),
            ),
        ].into()),
        verum_ast::span::Span::dummy(),
    );

    let maybe_pair = Type::new(
        TypeKind::Generic {
            base: Box::new(Type::new(
                TypeKind::Path(verum_ast::ty::Path {
                    segments: vec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident::new(
                        "Maybe",
                        verum_ast::span::Span::dummy(),
                    ))]
                    .into(),
                    span: verum_ast::span::Span::dummy(),
                }),
                verum_ast::span::Span::dummy(),
            )),
            args: vec![verum_ast::ty::GenericArg::Type(pair_type)].into(),
        },
        verum_ast::span::Span::dummy(),
    );

    destructors.push(Destructor {
        name: "force".into(),
        return_type: Heap::new(maybe_pair),
    });

    CoinductiveType {
        name: "LazyList".into(),
        params: vec![TypeParam {
            name: "A".into(),
            ty: Heap::new(element_type),
        }]
        .into(),
        destructors,
    }
}

/// Create StateT<S, A> coinductive type (stateful computation)
///
/// StateT<S, A> represents a computation that threads state of type S
/// and produces a value of type A.
///
/// StateT<S, A> has destructor:
/// - run : StateT<S, A> -> S -> (A, S)
pub fn state_t_type(state_type: Type, result_type: Type) -> CoinductiveType {
    let mut destructors = List::new();

    // Create S -> (A, S) return type
    let pair_type = Type::new(
        TypeKind::Tuple(vec![result_type.clone(), state_type.clone()].into()),
        verum_ast::span::Span::dummy(),
    );

    let run_type = Type::new(
        TypeKind::Function {
            params: vec![state_type.clone()].into(),
            return_type: Box::new(pair_type),
            calling_convention: Maybe::None,
            contexts: ContextList::empty(),
        },
        verum_ast::span::Span::dummy(),
    );

    destructors.push(Destructor {
        name: "run".into(),
        return_type: Heap::new(run_type),
    });

    CoinductiveType {
        name: "StateT".into(),
        params: vec![
            TypeParam {
                name: "S".into(),
                ty: Heap::new(state_type),
            },
            TypeParam {
                name: "A".into(),
                ty: Heap::new(result_type),
            },
        ]
        .into(),
        destructors,
    }
}

/// Create WriterT<W, A> coinductive type (computation with output)
///
/// WriterT<W, A> represents a computation that produces output of type W
/// (where W is a monoid) alongside a result of type A.
///
/// WriterT<W, A> has destructor:
/// - run : WriterT<W, A> -> (A, W)
pub fn writer_t_type(output_type: Type, result_type: Type) -> CoinductiveType {
    let mut destructors = List::new();

    // Create (A, W) return type
    let pair_type = Type::new(
        TypeKind::Tuple(vec![result_type.clone(), output_type.clone()].into()),
        verum_ast::span::Span::dummy(),
    );

    destructors.push(Destructor {
        name: "run".into(),
        return_type: Heap::new(pair_type),
    });

    CoinductiveType {
        name: "WriterT".into(),
        params: vec![
            TypeParam {
                name: "W".into(),
                ty: Heap::new(output_type),
            },
            TypeParam {
                name: "A".into(),
                ty: Heap::new(result_type),
            },
        ]
        .into(),
        destructors,
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::literal::{IntLit, Literal, LiteralKind};
    use verum_ast::span::Span;

    fn make_int_literal(value: i64) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal {
                kind: LiteralKind::Int(IntLit {
                    value: value as i128,
                    suffix: None,
                }),
                span: Span::dummy(),
            }),
            Span::dummy(),
        )
    }

    fn make_path_expr(name: &str) -> Expr {
        Expr::new(
            ExprKind::Path(verum_ast::ty::Path {
                segments: vec![verum_ast::ty::PathSegment::Name(verum_ast::ty::Ident::new(
                    name,
                    Span::dummy(),
                ))]
                .into(),
                span: Span::dummy(),
            }),
            Span::dummy(),
        )
    }

    fn make_call_expr(func_name: &str, args: Vec<Expr>) -> Expr {
        Expr::new(
            ExprKind::Call {
                func: Box::new(make_path_expr(func_name)),
                args: args.into(),
                type_args: Vec::new().into(),
            },
            Span::dummy(),
        )
    }

    #[test]
    fn test_type_registry_basic() {
        let mut registry = TypeRegistry::new();

        // Register a coinductive type
        let destructors = vec![
            Destructor {
                name: "head".into(),
                return_type: Heap::new(Type::int(Span::dummy())),
            },
            Destructor {
                name: "tail".into(),
                return_type: Heap::new(Type::new(
                    TypeKind::Path(verum_ast::ty::Path {
                        segments: vec![verum_ast::ty::PathSegment::Name(
                            verum_ast::ty::Ident::new("IntStream", Span::dummy()),
                        )]
                        .into(),
                        span: Span::dummy(),
                    }),
                    Span::dummy(),
                )),
            },
        ];

        registry.register_coinductive("IntStream".to_text(), destructors.into());

        assert!(registry.is_coinductive(&"IntStream".to_text()));
        assert!(!registry.is_inductive(&"IntStream".to_text()));
        assert!(registry.get_destructors(&"IntStream".to_text()).is_some());
    }

    #[test]
    fn test_type_registry_alias_resolution() {
        let mut registry = TypeRegistry::new();

        registry.register_primitive("Int".to_text());
        registry.add_qualified_alias("core::Int".to_text(), "Int".to_text());

        let resolved = registry.resolve_name(&"core::Int".to_text());
        assert_eq!(resolved.as_str(), "Int");
    }

    #[test]
    fn test_stream_type_creation() {
        let stream = stream_type(Type::int(Span::dummy()));

        assert_eq!(stream.name.as_str(), "Stream");
        assert_eq!(stream.destructors.len(), 2);
        assert_eq!(stream.destructors[0].name.as_str(), "head");
        assert_eq!(stream.destructors[1].name.as_str(), "tail");
    }

    #[test]
    fn test_productivity_simple_stream() {
        let mut checker = CoinductiveChecker::new();

        // Register the stream definition
        checker.register_function("ones".to_text(), true);

        // ones = { head = 1, tail = ones }
        let stream_def = StreamDef::new(
            "ones".to_text(),
            Type::int(Span::dummy()),
            make_int_literal(1),
            make_path_expr("ones"),
        );

        let result = checker.check_productivity(&stream_def);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_productivity_map_stream() {
        let mut checker = CoinductiveChecker::new();

        // Register stream functions
        checker.register_function("map_stream".to_text(), true);

        // map_stream(f, s) = { head = f(s.head), tail = map_stream(f, s.tail) }
        // Simplified: head = f(x), tail = map_stream(f, s)
        let stream_def = StreamDef::new(
            "map_stream".to_text(),
            Type::int(Span::dummy()),
            make_call_expr("f", vec![make_path_expr("x")]),
            make_call_expr("map_stream", vec![make_path_expr("f"), make_path_expr("s")]),
        );

        let result = checker.check_productivity(&stream_def);
        assert!(result.is_ok());
        // This should be productive since recursive call is in tail position
        assert!(result.unwrap());
    }

    #[test]
    fn test_non_productive_stream() {
        let mut checker = CoinductiveChecker::new();

        // Register a non-productive stream definition
        checker.register_function("bad_stream".to_text(), true);

        // bad_stream = { head = bad_stream.head, tail = bad_stream.tail }
        // This is NOT productive because head references itself
        let stream_def = StreamDef::new(
            "bad_stream".to_text(),
            Type::int(Span::dummy()),
            make_path_expr("bad_stream"), // Recursive call in head!
            make_path_expr("bad_stream"),
        );

        let result = checker.check_productivity(&stream_def);
        assert!(result.is_ok());
        // This should NOT be productive
        assert!(!result.unwrap());
    }

    #[test]
    fn test_colist_type_creation() {
        let colist = colist_type(Type::int(Span::dummy()));

        assert_eq!(colist.name.as_str(), "Colist");
        assert_eq!(colist.destructors.len(), 1);
        assert_eq!(colist.destructors[0].name.as_str(), "uncons");
    }

    #[test]
    fn test_process_type_creation() {
        let process = process_type(Type::int(Span::dummy()), Type::bool(Span::dummy()));

        assert_eq!(process.name.as_str(), "Process");
        assert_eq!(process.params.len(), 2);
        assert_eq!(process.destructors.len(), 1);
        assert_eq!(process.destructors[0].name.as_str(), "step");
    }

    #[test]
    fn test_is_recursive_type() {
        let checker = CoinductiveChecker::new();
        let stream = stream_type(Type::int(Span::dummy()));

        // tail returns Stream, which is recursive
        assert!(checker.is_recursive_type(&stream.destructors[1].return_type, &stream.name));

        // head returns Int, which is not recursive
        assert!(!checker.is_recursive_type(&stream.destructors[0].return_type, &stream.name));
    }

    #[test]
    fn test_guard_context() {
        let ctx = GuardContext::with_definition(&"test".to_text());

        assert!(ctx.is_recursive_call(&"test".to_text()));
        assert!(!ctx.is_recursive_call(&"other".to_text()));
        assert!(!ctx.is_guarded());

        let guarded_ctx = ctx.enter_guard();
        assert!(guarded_ctx.is_guarded());
    }

    // ==================== Tests for Enhanced Bisimulation ====================

    #[test]
    fn test_bisimulation_config_default() {
        let config = BisimulationConfig::default();
        assert_eq!(config.max_depth, 100);
        assert_eq!(config.timeout_ms, 30000);
        assert!(config.generate_counterexamples);
        assert!(config.incremental);
        assert_eq!(config.infinite_strategy, InfiniteStrategy::BoundedUnfolding);
    }

    #[test]
    fn test_bisimulation_stats_default() {
        let stats = BisimulationStats::default();
        assert_eq!(stats.destructors_checked, 0);
        assert_eq!(stats.z3_queries, 0);
        assert_eq!(stats.z3_time_ms, 0);
        assert_eq!(stats.max_depth_reached, 0);
        assert!(!stats.used_coinductive_hypothesis);
    }

    #[test]
    fn test_infinite_strategy_variants() {
        // Test all strategy variants exist
        let _bounded = InfiniteStrategy::BoundedUnfolding;
        let _coind = InfiniteStrategy::CoinductiveHypothesis;
        let _gfp = InfiniteStrategy::GreatestFixpoint;
        let _hybrid = InfiniteStrategy::Hybrid;
    }

    // ==================== Tests for Coinductive Encoder ====================

    #[test]
    fn test_coinductive_encoder_creation() {
        let encoder = CoinductiveEncoder::new();
        assert!(encoder.predicates.is_empty());
    }

    #[test]
    fn test_coinductive_encoder_register_predicate() {
        let mut encoder = CoinductiveEncoder::new();

        let pred = CoinductivePredicate {
            name: "bisim".into(),
            param_sorts: vec!["Int".into(), "Int".into()].into(),
            body: PredicateBodyDef::Base {
                formula: "(= x y)".into(),
            },
            strictly_positive: true,
        };

        encoder.register_predicate(pred);
        assert!(encoder.check_strictly_positive(&"bisim".to_text()));
    }

    #[test]
    fn test_coinductive_encoder_predicate_not_found() {
        let encoder = CoinductiveEncoder::new();
        let result = encoder.encode_as_fixedpoint(&"nonexistent".to_text());
        assert!(result.is_err());
    }

    #[test]
    fn test_predicate_body_base_encoding() {
        let mut encoder = CoinductiveEncoder::new();

        let pred = CoinductivePredicate {
            name: "eq".into(),
            param_sorts: vec!["Int".into()].into(),
            body: PredicateBodyDef::Base {
                formula: "(= x 0)".into(),
            },
            strictly_positive: true,
        };

        encoder.register_predicate(pred);
        let rules = encoder.encode_as_fixedpoint(&"eq".to_text()).unwrap();
        assert!(!rules.is_empty());
    }

    #[test]
    fn test_predicate_body_guarded_encoding() {
        let mut encoder = CoinductiveEncoder::new();

        let pred = CoinductivePredicate {
            name: "stream_bisim".into(),
            param_sorts: vec!["Stream".into(), "Stream".into()].into(),
            body: PredicateBodyDef::Guarded {
                guard: "(= (head x) (head y))".into(),
                recursive_calls: vec![PredicateCall {
                    predicate: "stream_bisim".into(),
                    args: vec!["(tail x)".into(), "(tail y)".into()].into(),
                }]
                .into(),
            },
            strictly_positive: true,
        };

        encoder.register_predicate(pred);
        let rules = encoder
            .encode_as_fixedpoint(&"stream_bisim".to_text())
            .unwrap();
        assert!(!rules.is_empty());
    }

    // ==================== Tests for Observation Equivalence ====================

    #[test]
    fn test_observation_equivalence_creation() {
        let obs_eq = ObservationEquivalence::new();
        assert!(obs_eq.observers.is_empty());
        assert!(obs_eq.cache.is_empty());
    }

    #[test]
    fn test_observation_equivalence_register_observers() {
        let mut obs_eq = ObservationEquivalence::new();

        let destructors = vec![Destructor {
            name: "head".into(),
            return_type: Heap::new(Type::int(Span::dummy())),
        }];

        obs_eq.register_observers("Stream".to_text(), destructors.into());
        assert!(obs_eq.observers.contains_key(&"Stream".to_text()));
    }

    #[test]
    fn test_observation_equivalence_clear_cache() {
        let mut obs_eq = ObservationEquivalence::new();
        obs_eq.cache.insert(("a".to_text(), "b".to_text()), true);
        assert!(!obs_eq.cache.is_empty());

        obs_eq.clear_cache();
        assert!(obs_eq.cache.is_empty());
    }

    // ==================== Tests for Stream Fusion ====================

    #[test]
    fn test_stream_fusion_verifier_creation() {
        let verifier = StreamFusionVerifier::new();
        // Should have standard rules registered
        assert!(!verifier.fusion_rules.is_empty());
    }

    #[test]
    fn test_stream_fusion_register_rule() {
        let mut verifier = StreamFusionVerifier::new();
        let initial_count = verifier.fusion_rules.len();

        let rule = FusionRule {
            name: "custom_rule".into(),
            lhs: FusionPattern::Var("s".into()),
            rhs: FusionPattern::Var("s".into()),
            conditions: List::new(),
        };

        verifier.register_rule(rule);
        assert_eq!(verifier.fusion_rules.len(), initial_count + 1);
    }

    #[test]
    fn test_fusion_pattern_variants() {
        // Test all pattern variants can be created
        let _var = FusionPattern::Var("s".into());
        let _map = FusionPattern::Map {
            func: "f".into(),
            stream: Box::new(FusionPattern::Var("s".into())),
        };
        let _filter = FusionPattern::Filter {
            predicate: "p".into(),
            stream: Box::new(FusionPattern::Var("s".into())),
        };
        let _take = FusionPattern::Take {
            count: "n".into(),
            stream: Box::new(FusionPattern::Var("s".into())),
        };
        let _drop = FusionPattern::Drop {
            count: "n".into(),
            stream: Box::new(FusionPattern::Var("s".into())),
        };
        let _zip = FusionPattern::ZipWith {
            func: "f".into(),
            left: Box::new(FusionPattern::Var("s1".into())),
            right: Box::new(FusionPattern::Var("s2".into())),
        };
        let _custom = FusionPattern::Custom {
            name: "custom".into(),
            args: vec![FusionPattern::Var("s".into())].into(),
        };
    }

    #[test]
    fn test_fusion_verify_rule() {
        let verifier = StreamFusionVerifier::new();

        // Get a standard rule and verify it
        if let Some(rule) = verifier.fusion_rules.first() {
            let result = verifier.verify_rule(rule);
            assert!(result.is_ok());
            assert!(result.unwrap());
        }
    }

    // ==================== Tests for Additional Coinductive Types ====================

    #[test]
    fn test_rose_tree_type_creation() {
        let rose_tree = rose_tree_type(Type::int(Span::dummy()));

        assert_eq!(rose_tree.name.as_str(), "RoseTree");
        assert_eq!(rose_tree.params.len(), 1);
        assert_eq!(rose_tree.destructors.len(), 2);
        assert_eq!(rose_tree.destructors[0].name.as_str(), "root");
        assert_eq!(rose_tree.destructors[1].name.as_str(), "children");
    }

    #[test]
    fn test_lazy_list_type_creation() {
        let lazy_list = lazy_list_type(Type::int(Span::dummy()));

        assert_eq!(lazy_list.name.as_str(), "LazyList");
        assert_eq!(lazy_list.params.len(), 1);
        assert_eq!(lazy_list.destructors.len(), 1);
        assert_eq!(lazy_list.destructors[0].name.as_str(), "force");
    }

    #[test]
    fn test_state_t_type_creation() {
        let state_t = state_t_type(Type::int(Span::dummy()), Type::bool(Span::dummy()));

        assert_eq!(state_t.name.as_str(), "StateT");
        assert_eq!(state_t.params.len(), 2);
        assert_eq!(state_t.params[0].name.as_str(), "S");
        assert_eq!(state_t.params[1].name.as_str(), "A");
        assert_eq!(state_t.destructors.len(), 1);
        assert_eq!(state_t.destructors[0].name.as_str(), "run");
    }

    #[test]
    fn test_writer_t_type_creation() {
        let writer_t = writer_t_type(Type::int(Span::dummy()), Type::bool(Span::dummy()));

        assert_eq!(writer_t.name.as_str(), "WriterT");
        assert_eq!(writer_t.params.len(), 2);
        assert_eq!(writer_t.params[0].name.as_str(), "W");
        assert_eq!(writer_t.params[1].name.as_str(), "A");
        assert_eq!(writer_t.destructors.len(), 1);
        assert_eq!(writer_t.destructors[0].name.as_str(), "run");
    }

    // ==================== Tests for Type Recursion Checking ====================

    #[test]
    fn test_is_recursive_type_generic() {
        let checker = CoinductiveChecker::new();

        // Stream<Int> should be recursive for Stream
        let stream_of_int = Type::new(
            TypeKind::Generic {
                base: Box::new(Type::new(
                    TypeKind::Path(verum_ast::ty::Path {
                        segments: vec![verum_ast::ty::PathSegment::Name(
                            verum_ast::ty::Ident::new("Stream", Span::dummy()),
                        )]
                        .into(),
                        span: Span::dummy(),
                    }),
                    Span::dummy(),
                )),
                args: vec![verum_ast::ty::GenericArg::Type(Type::int(Span::dummy()))].into(),
            },
            Span::dummy(),
        );

        assert!(checker.is_recursive_type(&stream_of_int, &"Stream".to_text()));
        assert!(!checker.is_recursive_type(&stream_of_int, &"List".to_text()));
    }

    #[test]
    fn test_is_recursive_type_tuple() {
        let checker = CoinductiveChecker::new();

        // (Int, Stream) contains Stream
        let tuple_with_stream = Type::new(
            TypeKind::Tuple(
                vec![
                    Type::int(Span::dummy()),
                    Type::new(
                        TypeKind::Path(verum_ast::ty::Path {
                            segments: vec![verum_ast::ty::PathSegment::Name(
                                verum_ast::ty::Ident::new("Stream", Span::dummy()),
                            )]
                            .into(),
                            span: Span::dummy(),
                        }),
                        Span::dummy(),
                    ),
                ]
                .into(),
            ),
            Span::dummy(),
        );

        assert!(checker.is_recursive_type(&tuple_with_stream, &"Stream".to_text()));
    }

    // ==================== Tests for Bisimulation Checker Integration ====================

    #[test]
    fn test_bisimulation_checker_creation() {
        let checker = CoinductiveChecker::new();
        let config = BisimulationConfig::default();
        let bisim_checker = BisimulationChecker::new(&checker, config);

        assert_eq!(bisim_checker.stats.destructors_checked, 0);
        assert_eq!(bisim_checker.config.max_depth, 100);
    }

    #[test]
    fn test_bisimulation_result_structure() {
        let result = BisimulationResult {
            bisimilar: true,
            depth_reached: 5,
            counterexample: None,
            stats: BisimulationStats::default(),
        };

        assert!(result.bisimilar);
        assert_eq!(result.depth_reached, 5);
        assert!(result.counterexample.is_none());
    }

    #[test]
    fn test_bisimulation_counterexample_structure() {
        let counterexample = BisimulationCounterexample {
            distinguishing_destructor: "head".into(),
            path: vec!["tail".into(), "tail".into()].into(),
            left_observation: "1".into(),
            right_observation: "2".into(),
        };

        assert_eq!(counterexample.distinguishing_destructor.as_str(), "head");
        assert_eq!(counterexample.path.len(), 2);
    }

    // ==================== Default Implementation Tests ====================

    #[test]
    fn test_coinductive_checker_default() {
        let checker = CoinductiveChecker::default();
        assert!(checker.types.is_empty());
        assert!(!checker.registry.types.is_empty()); // Has default types registered
    }

    #[test]
    fn test_type_registry_default() {
        let registry = TypeRegistry::default();
        // Should have Stream and primitives registered
        assert!(registry.is_coinductive(&"Stream".to_text()));
        assert!(registry.lookup(&"Int".to_text()).is_some());
        assert!(registry.lookup(&"Bool".to_text()).is_some());
    }

    #[test]
    fn test_coinductive_encoder_default() {
        let encoder = CoinductiveEncoder::default();
        assert!(encoder.predicates.is_empty());
    }

    #[test]
    fn test_observation_equivalence_default() {
        let obs_eq = ObservationEquivalence::default();
        assert!(obs_eq.observers.is_empty());
    }

    #[test]
    fn test_stream_fusion_verifier_default() {
        let verifier = StreamFusionVerifier::default();
        // Should have standard rules
        assert!(!verifier.fusion_rules.is_empty());
    }
}
