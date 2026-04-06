//! Computational Properties Tracking for Verum
//!
//! Tracks side effects and purity of functions: Pure/IO/Async/Divergent
//!
//! ⚠️ NOTE: This is NOT the Context System (DI)!
//! - Context System = Dependency Injection (provide/using) - see context_check.rs module
//! - This module = Purity and effect tracking for optimization
//!
//! Computational properties: compile-time tracking of Pure, IO, Async, Fallible, Mutates effects
//! Context type system integration: context requirements tracked in function types, checked at call sites — (distinguishes from DI contexts)
//!
//! This module tracks the computational properties of functions and expressions
//! for optimization and safety:
//! - Pure: No side effects, always terminates
//! - IO: Performs I/O operations
//! - Async: Asynchronous computation
//! - Fallible: May fail (returns Result)
//! - Divergent: May not terminate (loop, panic)

use serde::{Deserialize, Serialize};
use std::fmt;
use verum_common::{List, Set, Text};

/// A computational property in the Verum type system.
///
/// Computational properties track side effects and runtime behavior of functions.
/// The property system enables:
/// - Safe optimization (pure functions can be memoized/reordered)
/// - Async/await type checking
/// - Error handling verification
/// - Divergence analysis
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ComputationalProperty {
    /// Pure computation - no side effects, always terminates
    /// Can be freely reordered, memoized, and optimized
    Pure,

    /// I/O operations (file, network, console)
    /// Prevents reordering and speculative execution
    IO,

    /// Asynchronous computation
    /// Requires async context to execute
    Async,

    /// Fallible operation - may return an error
    /// Represented as Result<T, E> in return type
    Fallible,

    /// Divergent - may not terminate
    /// Examples: infinite loops, panic, unreachable
    Divergent,

    /// Memory allocation
    /// Tracks heap allocations for memory analysis
    Allocates,

    /// Memory deallocation
    /// Tracks explicit deallocation (drop, free)
    Deallocates,

    /// State mutation
    /// Tracks mutable references and state changes
    Mutates,

    /// Reads from external state
    /// Examples: reading environment variables, system time
    ReadsExternal,

    /// Writes to external state
    /// Examples: writing environment variables, setting global state
    WritesExternal,

    /// Foreign function call
    /// Calling into C or other language via FFI
    FFI,

    /// Spawns concurrent task
    /// Creates new thread or async task
    Spawns,

    /// Custom user-defined computational property
    /// For extensibility and domain-specific properties
    Custom(Text),
}

/// Property set - collection of computational properties for a function or expression.
///
/// Property sets combine multiple properties and provide lattice operations:
/// - Union: Combining properties from multiple operations
/// - Subsumption: Checking if one property set is a subset of another
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertySet {
    /// Set of computational properties
    properties: Set<ComputationalProperty>,
}

// Custom Serialize/Deserialize for PropertySet because Set doesn't implement these traits
impl serde::Serialize for PropertySet {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Convert to List for serialization
        let list: List<ComputationalProperty> = self.properties.iter().cloned().collect();
        list.serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for PropertySet {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let list: List<ComputationalProperty> = serde::Deserialize::deserialize(deserializer)?;
        let mut properties = Set::new();
        for prop in list {
            properties.insert(prop);
        }
        Ok(PropertySet { properties })
    }
}

impl PropertySet {
    /// Create an empty property set (pure computation)
    pub fn pure() -> Self {
        let mut properties = Set::new();
        properties.insert(ComputationalProperty::Pure);
        PropertySet { properties }
    }

    /// Create property set with a single property
    pub fn single(property: ComputationalProperty) -> Self {
        let mut properties = Set::new();
        properties.insert(property);
        PropertySet { properties }
    }

    /// Create property set from multiple properties
    pub fn from_properties(properties: impl IntoIterator<Item = ComputationalProperty>) -> Self {
        let mut property_set = Set::new();
        for property in properties {
            property_set.insert(property);
        }

        // If Pure is explicitly present and other properties exist, remove Pure
        if property_set.len() > 1 && property_set.contains(&ComputationalProperty::Pure) {
            property_set.remove(&ComputationalProperty::Pure);
        }

        // If no properties, default to Pure
        if property_set.is_empty() {
            property_set.insert(ComputationalProperty::Pure);
        }

        PropertySet {
            properties: property_set,
        }
    }

    /// Check if this property set is pure
    pub fn is_pure(&self) -> bool {
        self.properties.len() == 1 && self.properties.contains(&ComputationalProperty::Pure)
    }

    /// Check if this property set contains a specific property
    pub fn contains(&self, property: &ComputationalProperty) -> bool {
        self.properties.contains(property)
    }

    /// Check if this property set contains IO
    pub fn has_io(&self) -> bool {
        self.contains(&ComputationalProperty::IO)
    }

    /// Check if this property set contains Async
    pub fn is_async(&self) -> bool {
        self.contains(&ComputationalProperty::Async)
    }

    /// Check if this property set is fallible
    pub fn is_fallible(&self) -> bool {
        self.contains(&ComputationalProperty::Fallible)
    }

    /// Check if this property set may diverge
    pub fn is_divergent(&self) -> bool {
        self.contains(&ComputationalProperty::Divergent)
    }

    /// Union of two property sets
    pub fn union(&self, other: &PropertySet) -> PropertySet {
        let mut combined = self.properties.clone();
        for property in other.properties.iter() {
            combined.insert(property.clone());
        }

        // If Pure is present with other properties, remove it
        if combined.len() > 1 && combined.contains(&ComputationalProperty::Pure) {
            combined.remove(&ComputationalProperty::Pure);
        }

        PropertySet {
            properties: combined,
        }
    }

    /// Check if this property set is a subset of another (subsumption)
    /// Returns true if all properties in self are also in other
    pub fn is_subset_of(&self, other: &PropertySet) -> bool {
        // Pure is a subset of any property set
        if self.is_pure() {
            return true;
        }

        // Check if all our properties are in other
        self.properties.iter().all(|e| other.properties.contains(e))
    }

    /// Iterate over properties
    pub fn iter(&self) -> impl Iterator<Item = &ComputationalProperty> {
        self.properties.iter()
    }

    /// Get number of properties
    pub fn len(&self) -> usize {
        self.properties.len()
    }

    /// Check if property set is empty (should always be false - at least Pure)
    pub fn is_empty(&self) -> bool {
        self.properties.is_empty()
    }

    /// Check if this property set is valid for a meta function.
    ///
    /// Meta functions must be pure - they run at compile-time and cannot have
    /// side effects. This method returns the impure properties if any exist.
    ///
    /// Note: `Fallible` is allowed in meta functions because:
    /// - Meta system purity rules: arithmetic operations (including %) are classified as pure
    /// - Fallible at compile-time means compile-time error, not runtime side effect
    /// - The operation is still deterministic (same inputs -> same result/error)
    ///
    /// Note: `Divergent` is allowed in meta functions because:
    /// - Meta system control flow: loops are allowed in meta functions (with iteration limits)
    /// - Meta system safety: iteration limits (default 1M) prevent infinite loops in compile-time evaluation
    /// - Meta evaluation has a step limit, so potential non-termination is bounded
    /// - Loops like while/for are common in meta functions for code generation
    ///
    /// Meta function purity: meta functions are implicitly pure (no IO, no mutation of non-meta state) — Meta functions are implicitly pure
    pub fn validate_for_meta_fn(&self) -> Result<(), List<ComputationalProperty>> {
        if self.is_pure() {
            return Ok(());
        }

        // Collect impure properties, excluding:
        // - Pure (obviously allowed)
        // - Fallible (compile-time errors are acceptable in meta functions)
        // - Divergent (loops are allowed with step limits per spec lines 393-396, 431)
        let impure: List<ComputationalProperty> = self
            .properties
            .iter()
            .filter(|p| {
                !matches!(
                    p,
                    ComputationalProperty::Pure
                        | ComputationalProperty::Fallible
                        | ComputationalProperty::Divergent
                )
            })
            .cloned()
            .collect();

        if impure.is_empty() {
            Ok(())
        } else {
            Err(impure)
        }
    }

    /// Check if this property set is valid for a pure function.
    ///
    /// Pure functions (`pure fn`) must have no side effects:
    /// - No IO operations
    /// - No mutation (Mutates)
    /// - No async computation
    /// - No external state access (ReadsExternal, WritesExternal)
    /// - No FFI calls
    /// - No spawning concurrent tasks
    /// - No allocation (debatable, but allowed for now)
    ///
    /// Allowed in pure functions:
    /// - Pure (obviously)
    /// - Fallible (returning errors is deterministic, no side effect)
    /// - Divergent (panic/unreachable are allowed per spec)
    /// - Allocates (heap allocation is allowed in pure functions)
    ///
    /// Returns Ok(()) if valid, Err(impure_properties) if violated.
    pub fn validate_for_pure_fn(&self) -> Result<(), List<ComputationalProperty>> {
        if self.is_pure() {
            return Ok(());
        }

        // Collect impure properties that violate pure function contract
        let impure: List<ComputationalProperty> = self
            .properties
            .iter()
            .filter(|p| {
                matches!(
                    p,
                    ComputationalProperty::IO
                        | ComputationalProperty::Async
                        | ComputationalProperty::Mutates
                        | ComputationalProperty::ReadsExternal
                        | ComputationalProperty::WritesExternal
                        | ComputationalProperty::FFI
                        | ComputationalProperty::Spawns
                )
            })
            .cloned()
            .collect();

        if impure.is_empty() {
            Ok(())
        } else {
            Err(impure)
        }
    }

    /// Convert to list for serialization
    pub fn to_list(&self) -> List<ComputationalProperty> {
        self.properties.iter().cloned().collect()
    }
}

impl Default for PropertySet {
    fn default() -> Self {
        Self::pure()
    }
}

impl fmt::Display for ComputationalProperty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComputationalProperty::Pure => write!(f, "Pure"),
            ComputationalProperty::IO => write!(f, "IO"),
            ComputationalProperty::Async => write!(f, "Async"),
            ComputationalProperty::Fallible => write!(f, "Fallible"),
            ComputationalProperty::Divergent => write!(f, "Divergent"),
            ComputationalProperty::Allocates => write!(f, "Allocates"),
            ComputationalProperty::Deallocates => write!(f, "Deallocates"),
            ComputationalProperty::Mutates => write!(f, "Mutates"),
            ComputationalProperty::ReadsExternal => write!(f, "ReadsExternal"),
            ComputationalProperty::WritesExternal => write!(f, "WritesExternal"),
            ComputationalProperty::FFI => write!(f, "FFI"),
            ComputationalProperty::Spawns => write!(f, "Spawns"),
            ComputationalProperty::Custom(name) => write!(f, "Custom({})", name),
        }
    }
}

impl fmt::Display for PropertySet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_pure() {
            return write!(f, "Pure");
        }

        let properties: List<String> = self.properties.iter().map(|e| e.to_string()).collect();

        write!(f, "{{{}}}", properties.join(", "))
    }
}

/// Property inference context
///
/// Tracks computational properties during type checking and inference.
/// Properties are inferred bottom-up from expressions and combined.
pub struct PropertyInferenceContext {
    /// Current property set being inferred
    current_properties: PropertySet,

    /// Known function properties (for lookup during inference)
    known_functions: std::collections::HashMap<Text, PropertySet>,

    /// Stack of nested scopes for property tracking
    scope_stack: List<PropertySet>,
}

impl PropertyInferenceContext {
    /// Create new property inference context
    pub fn new() -> Self {
        PropertyInferenceContext {
            current_properties: PropertySet::pure(),
            known_functions: std::collections::HashMap::new(),
            scope_stack: List::new(),
        }
    }

    /// Add a property to the current property set
    pub fn add_property(&mut self, property: ComputationalProperty) {
        let new_set = PropertySet::single(property);
        self.current_properties = self.current_properties.union(&new_set);
    }

    /// Add multiple properties
    pub fn add_properties(&mut self, properties: PropertySet) {
        self.current_properties = self.current_properties.union(&properties);
    }

    /// Get current property set
    pub fn get_properties(&self) -> &PropertySet {
        &self.current_properties
    }

    /// Take current property set and reset to pure
    pub fn take_properties(&mut self) -> PropertySet {
        std::mem::take(&mut self.current_properties)
    }

    /// Reset to pure
    pub fn reset(&mut self) {
        self.current_properties = PropertySet::pure();
    }

    /// Register a function with its known properties
    pub fn register_function(&mut self, name: Text, properties: PropertySet) {
        self.known_functions.insert(name, properties);
    }

    /// Get properties of a known function
    pub fn get_function_properties(&self, name: &Text) -> Option<&PropertySet> {
        self.known_functions.get(name)
    }

    /// Enter a new scope (e.g., for function body, block)
    pub fn enter_scope(&mut self) {
        self.scope_stack.push(self.current_properties.clone());
        self.current_properties = PropertySet::pure();
    }

    /// Exit scope and merge properties into parent
    pub fn exit_scope(&mut self) -> PropertySet {
        let scope_properties = self.take_properties();
        if let Some(parent) = self.scope_stack.pop() {
            self.current_properties = parent.union(&scope_properties);
        }
        scope_properties
    }

    /// Discard scope without merging (for branches that don't execute)
    pub fn discard_scope(&mut self) {
        let _ = self.take_properties();
        if let Some(parent) = self.scope_stack.pop() {
            self.current_properties = parent;
        }
    }
}

impl Default for PropertyInferenceContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Expression-based property inference.
///
/// Analyzes expressions to infer their computational properties bottom-up.
/// This is used during type inference to automatically determine:
/// - Whether a function is pure or has side effects
/// - Whether an expression is async
/// - Whether an expression may fail (Fallible)
/// - Whether an expression may diverge
pub struct PropertyInferrer {
    /// Context for tracking properties
    context: PropertyInferenceContext,
}

impl PropertyInferrer {
    /// Create a new property inferrer
    pub fn new() -> Self {
        Self {
            context: PropertyInferenceContext::new(),
        }
    }

    /// Infer properties from an expression.
    ///
    /// This recursively traverses the expression and combines properties
    /// from sub-expressions using the union operation.
    ///
    /// # Property Inference Rules
    ///
    /// - **Literals**: Pure
    /// - **Variables**: Pure (unless externally mutable)
    /// - **Binary/Unary ops**: Union of operand properties
    /// - **Function calls**: Lookup function properties + union with arg properties
    /// - **Method calls**: Union of receiver + args + method properties
    /// - **Await expressions**: Add Async property
    /// - **Try (?) expressions**: Add Fallible property
    /// - **Loops**: May add Divergent if unbounded
    /// - **Blocks**: Union of all statements
    pub fn infer_expr(&mut self, expr: &verum_ast::expr::Expr) -> PropertySet {
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            // Literals are pure
            ExprKind::Literal(_) => PropertySet::pure(),

            // Path/variable references are pure
            ExprKind::Path(_) => PropertySet::pure(),

            // Binary operations: union of operand properties
            ExprKind::Binary {
                left, right, op, ..
            } => {
                let left_props = self.infer_expr(left);
                let right_props = self.infer_expr(right);
                let mut combined = left_props.union(&right_props);

                // Division may be fallible (division by zero)
                if matches!(
                    op,
                    verum_ast::expr::BinOp::Div | verum_ast::expr::BinOp::Rem
                ) {
                    combined =
                        combined.union(&PropertySet::single(ComputationalProperty::Fallible));
                }

                combined
            }

            // Unary operations: just propagate properties
            ExprKind::Unary { expr: inner, .. } => self.infer_expr(inner),

            // Try operator adds Fallible
            ExprKind::Try(inner) => {
                let inner_props = self.infer_expr(inner);
                inner_props.union(&PropertySet::single(ComputationalProperty::Fallible))
            }

            // Await adds Async
            ExprKind::Await(inner) => {
                let inner_props = self.infer_expr(inner);
                inner_props.union(&PropertySet::single(ComputationalProperty::Async))
            }

            // Function calls: look up function properties + arg properties
            ExprKind::Call { func, args, .. } => {
                let mut combined = self.infer_expr(func);

                // Infer properties from arguments
                for arg in args {
                    combined = combined.union(&self.infer_expr(arg));
                }

                // Try to look up function properties if it's a simple path
                if let ExprKind::Path(path) = &func.kind {
                    if let Some(name) = path.as_ident() {
                        if let Some(func_props) = self
                            .context
                            .get_function_properties(&name.name.clone())
                        {
                            combined = combined.union(func_props);
                        }
                    }
                }

                combined
            }

            // Method calls
            ExprKind::MethodCall {
                receiver,
                args,
                method,
                ..
            } => {
                let mut combined = self.infer_expr(receiver);

                for arg in args {
                    combined = combined.union(&self.infer_expr(arg));
                }

                // Look up method properties
                if let Some(method_props) = self
                    .context
                    .get_function_properties(&method.name.clone())
                {
                    combined = combined.union(method_props);
                }

                combined
            }

            // If expressions: union of all branches
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Infer properties from conditions
                let mut combined = PropertySet::pure();
                for cond in &condition.conditions {
                    match cond {
                        verum_ast::expr::ConditionKind::Expr(e) => {
                            combined = combined.union(&self.infer_expr(e));
                        }
                        verum_ast::expr::ConditionKind::Let { value, .. } => {
                            combined = combined.union(&self.infer_expr(value));
                        }
                    }
                }

                if let Some(expr) = &then_branch.expr {
                    combined = combined.union(&self.infer_expr(expr));
                }
                for stmt in &then_branch.stmts {
                    combined = combined.union(&self.infer_stmt(stmt));
                }

                if let Some(else_expr) = else_branch {
                    combined = combined.union(&self.infer_expr(else_expr));
                }

                combined
            }

            // Match expressions: union of all arms
            ExprKind::Match { expr, arms, .. } => {
                let mut combined = self.infer_expr(expr);

                for arm in arms {
                    combined = combined.union(&self.infer_expr(&arm.body));
                    if let Some(guard) = &arm.guard {
                        combined = combined.union(&self.infer_expr(guard));
                    }
                }

                combined
            }

            // Blocks: union of all statements
            ExprKind::Block(block) => {
                let mut combined = PropertySet::pure();

                for stmt in &block.stmts {
                    combined = combined.union(&self.infer_stmt(stmt));
                }

                if let Some(expr) = &block.expr {
                    combined = combined.union(&self.infer_expr(expr));
                }

                combined
            }

            // Loops may diverge
            ExprKind::Loop { body, .. } => {
                // Loop bodies run repeatedly - add Divergent since they may not terminate
                let body_props = if let Some(expr) = &body.expr {
                    self.infer_expr(expr)
                } else {
                    PropertySet::pure()
                };

                // Conservative: loops may diverge
                body_props.union(&PropertySet::single(ComputationalProperty::Divergent))
            }

            ExprKind::While {
                condition, body, ..
            } => {
                let cond_props = self.infer_expr(condition);
                let body_props = if let Some(expr) = &body.expr {
                    self.infer_expr(expr)
                } else {
                    PropertySet::pure()
                };

                cond_props
                    .union(&body_props)
                    .union(&PropertySet::single(ComputationalProperty::Divergent))
            }

            ExprKind::For { iter, body, .. } => {
                let iter_props = self.infer_expr(iter);
                let body_props = if let Some(expr) = &body.expr {
                    self.infer_expr(expr)
                } else {
                    PropertySet::pure()
                };

                iter_props.union(&body_props)
            }

            // Closures capture their body properties
            ExprKind::Closure { body, .. } => self.infer_expr(body),

            // Async blocks are async
            ExprKind::Async(body) => {
                let body_props = if let Some(expr) = &body.expr {
                    self.infer_expr(expr)
                } else {
                    PropertySet::pure()
                };
                body_props.union(&PropertySet::single(ComputationalProperty::Async))
            }

            // Unsafe blocks mark as FFI (conservative - using unsafe code)
            ExprKind::Unsafe(body) => {
                let body_props = if let Some(expr) = &body.expr {
                    self.infer_expr(expr)
                } else {
                    PropertySet::pure()
                };
                body_props.union(&PropertySet::single(ComputationalProperty::FFI))
            }

            // Tuple: union of element properties
            ExprKind::Tuple(elems) => {
                let mut combined = PropertySet::pure();
                for elem in elems {
                    combined = combined.union(&self.infer_expr(elem));
                }
                combined
            }

            // Array: union of element properties
            ExprKind::Array(array_expr) => match array_expr {
                verum_ast::expr::ArrayExpr::List(elems) => {
                    let mut combined = PropertySet::pure();
                    for elem in elems {
                        combined = combined.union(&self.infer_expr(elem));
                    }
                    combined
                }
                verum_ast::expr::ArrayExpr::Repeat { value, count } => {
                    self.infer_expr(value).union(&self.infer_expr(count))
                }
            },

            ExprKind::Record { fields, base, .. } => {
                let mut combined = PropertySet::pure();
                for field in fields {
                    if let verum_common::Maybe::Some(ref value) = field.value {
                        combined = combined.union(&self.infer_expr(value));
                    }
                }
                if let verum_common::Maybe::Some(base_expr) = base {
                    combined = combined.union(&self.infer_expr(base_expr));
                }
                combined
            }

            // Index, field access: union of operands
            ExprKind::Index { expr, index } => self.infer_expr(expr).union(&self.infer_expr(index)),

            ExprKind::Field { expr, .. } => self.infer_expr(expr),

            // Return, break, continue: check inner expression
            ExprKind::Return(maybe_expr) => {
                if let Some(expr) = maybe_expr {
                    self.infer_expr(expr)
                } else {
                    PropertySet::pure()
                }
            }

            ExprKind::Break { value, .. } => {
                if let Some(expr) = value {
                    self.infer_expr(expr)
                } else {
                    PropertySet::pure()
                }
            }

            ExprKind::Continue { .. } => PropertySet::pure(),

            // Spawn adds Spawns property
            ExprKind::Spawn { expr, .. } => {
                let inner_props = self.infer_expr(expr);
                inner_props.union(&PropertySet::single(ComputationalProperty::Spawns))
            }

            // Nursery adds Spawns and Async properties (structured concurrency)
            // Nursery blocks are inherently async and spawn tasks
            ExprKind::Nursery {
                body,
                on_cancel,
                recover,
                options,
                ..
            } => {
                let mut combined = PropertySet::from_properties(vec![
                    ComputationalProperty::Spawns,
                    ComputationalProperty::Async,
                ]);

                // Infer properties from body
                for stmt in &body.stmts {
                    combined = combined.union(&self.infer_stmt(stmt));
                }
                if let Some(expr) = &body.expr {
                    combined = combined.union(&self.infer_expr(expr));
                }

                // Infer properties from on_cancel if present
                if let Some(cancel_block) = on_cancel {
                    for stmt in &cancel_block.stmts {
                        combined = combined.union(&self.infer_stmt(stmt));
                    }
                    if let Some(expr) = &cancel_block.expr {
                        combined = combined.union(&self.infer_expr(expr));
                    }
                }

                // Infer properties from recover if present (adds Fallible)
                if let Some(recover_body) = recover {
                    combined = combined.union(&PropertySet::single(ComputationalProperty::Fallible));
                    match recover_body {
                        verum_ast::expr::RecoverBody::MatchArms { arms, .. } => {
                            for arm in arms.iter() {
                                combined = combined.union(&self.infer_expr(&arm.body));
                                if let Some(guard) = &arm.guard {
                                    combined = combined.union(&self.infer_expr(guard));
                                }
                            }
                        }
                        verum_ast::expr::RecoverBody::Closure { body, .. } => {
                            combined = combined.union(&self.infer_expr(body));
                        }
                    }
                }

                // Infer properties from options
                if let Some(timeout) = &options.timeout {
                    combined = combined.union(&self.infer_expr(timeout));
                }
                if let Some(max_tasks) = &options.max_tasks {
                    combined = combined.union(&self.infer_expr(max_tasks));
                }

                combined
            }

            // Select expression adds Async property
            ExprKind::Select { arms, .. } => {
                let mut combined = PropertySet::single(ComputationalProperty::Async);
                for arm in arms.iter() {
                    if let Some(future) = &arm.future {
                        combined = combined.union(&self.infer_expr(future));
                    }
                    combined = combined.union(&self.infer_expr(&arm.body));
                    if let Some(guard) = &arm.guard {
                        combined = combined.union(&self.infer_expr(guard));
                    }
                }
                combined
            }

            // Default: pure for unhandled cases
            _ => PropertySet::pure(),
        }
    }

    /// Infer properties from a statement
    fn infer_stmt(&mut self, stmt: &verum_ast::stmt::Stmt) -> PropertySet {
        use verum_ast::stmt::StmtKind;

        match &stmt.kind {
            StmtKind::Expr { expr, .. } => self.infer_expr(expr),

            StmtKind::Let { value, .. } => {
                if let Some(expr) = value {
                    self.infer_expr(expr)
                } else {
                    PropertySet::pure()
                }
            }

            StmtKind::Defer(expr) => self.infer_expr(expr),

            // Errdefer is similar to Defer - analyze the deferred expression
            StmtKind::Errdefer(expr) => self.infer_expr(expr),

            _ => PropertySet::pure(),
        }
    }

    /// Infer properties from a function declaration.
    ///
    /// This analyzes the function signature and body to determine its computational properties:
    /// - If `is_async` is true, adds `Async` property
    /// - If `throws_clause` is present, adds `Fallible` property
    /// - If body is present, infers properties from the body expressions
    ///
    /// # Property Inference Rules for Functions
    ///
    /// - **Async functions**: `is_async: true` implies `Async` property
    /// - **Throws clause**: `throws_clause: Some(_)` implies `Fallible` property
    /// - **Body expressions**: Properties are inferred recursively from the function body
    ///
    /// # Example
    /// ```text
    /// async fn fetch(url: Text) throws(NetworkError) -> Data { ... }
    /// // Inferred properties: {Async, Fallible}
    /// ```
    pub fn infer_function_decl(&mut self, func: &verum_ast::decl::FunctionDecl) -> PropertySet {
        let mut properties = PropertySet::pure();

        // Async functions have the Async property
        if func.is_async {
            properties = properties.union(&PropertySet::single(ComputationalProperty::Async));
        }

        // Functions with throws clause are Fallible
        if func.throws_clause.is_some() {
            properties = properties.union(&PropertySet::single(ComputationalProperty::Fallible));
        }

        // Infer properties from the function body
        if let verum_common::Maybe::Some(ref body) = func.body {
            let body_props = match body {
                verum_ast::decl::FunctionBody::Block(block) => {
                    let mut block_props = PropertySet::pure();
                    for stmt in &block.stmts {
                        block_props = block_props.union(&self.infer_stmt(stmt));
                    }
                    if let Some(expr) = &block.expr {
                        block_props = block_props.union(&self.infer_expr(expr));
                    }
                    block_props
                }
                verum_ast::decl::FunctionBody::Expr(expr) => self.infer_expr(expr),
            };
            properties = properties.union(&body_props);
        }

        properties
    }

    /// Get the context for registering known function properties
    pub fn context_mut(&mut self) -> &mut PropertyInferenceContext {
        &mut self.context
    }

    /// Get the context for reading registered properties
    pub fn context(&self) -> &PropertyInferenceContext {
        &self.context
    }
}

impl Default for PropertyInferrer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pure_property_set() {
        let pure = PropertySet::pure();
        assert!(pure.is_pure());
        assert_eq!(pure.len(), 1);
        assert!(pure.contains(&ComputationalProperty::Pure));
    }

    #[test]
    fn test_single_property() {
        let io = PropertySet::single(ComputationalProperty::IO);
        assert!(!io.is_pure());
        assert!(io.has_io());
        assert_eq!(io.len(), 1);
    }

    #[test]
    fn test_multiple_properties() {
        let properties = PropertySet::from_properties(vec![
            ComputationalProperty::IO,
            ComputationalProperty::Async,
        ]);
        assert!(!properties.is_pure());
        assert!(properties.has_io());
        assert!(properties.is_async());
        assert_eq!(properties.len(), 2);
    }

    #[test]
    fn test_pure_removed_with_other_properties() {
        let properties = PropertySet::from_properties(vec![
            ComputationalProperty::Pure,
            ComputationalProperty::IO,
        ]);
        assert!(!properties.is_pure());
        assert!(properties.has_io());
        assert_eq!(properties.len(), 1);
    }

    #[test]
    fn test_property_union() {
        let io = PropertySet::single(ComputationalProperty::IO);
        let async_prop = PropertySet::single(ComputationalProperty::Async);
        let combined = io.union(&async_prop);

        assert!(combined.has_io());
        assert!(combined.is_async());
        assert_eq!(combined.len(), 2);
    }

    #[test]
    fn test_subsumption() {
        let pure = PropertySet::pure();
        let io = PropertySet::single(ComputationalProperty::IO);
        let io_async = PropertySet::from_properties(vec![
            ComputationalProperty::IO,
            ComputationalProperty::Async,
        ]);

        // Pure is subset of everything
        assert!(pure.is_subset_of(&io));
        assert!(pure.is_subset_of(&io_async));

        // IO is subset of {IO, Async}
        assert!(io.is_subset_of(&io_async));

        // {IO, Async} is not subset of {IO}
        assert!(!io_async.is_subset_of(&io));
    }

    #[test]
    fn test_property_inference_context() {
        let mut ctx = PropertyInferenceContext::new();
        assert!(ctx.get_properties().is_pure());

        ctx.add_property(ComputationalProperty::IO);
        assert!(ctx.get_properties().has_io());

        ctx.add_property(ComputationalProperty::Async);
        assert!(ctx.get_properties().is_async());

        let properties = ctx.take_properties();
        assert!(properties.has_io());
        assert!(properties.is_async());

        // After take, should be pure again
        assert!(ctx.get_properties().is_pure());
    }

    /// Helper to create a test function declaration
    fn create_test_function(
        is_async: bool,
        throws_clause: verum_common::Maybe<verum_ast::decl::ThrowsClause>,
    ) -> verum_ast::decl::FunctionDecl {
        use verum_ast::span::Span;
        use verum_ast::ty::Ident;

        verum_ast::decl::FunctionDecl {
            visibility: verum_ast::decl::Visibility::Private,
            is_async,
            is_meta: false,
            stage_level: 0,
            is_pure: false,
            is_generator: false,
            is_cofix: false,
            is_unsafe: false,
            is_transparent: false,
            is_variadic: false,
            extern_abi: verum_common::Maybe::None,
            name: Ident::new("test_fn", Span::dummy()),
            generics: verum_common::List::new(),
            params: verum_common::List::new(),
            throws_clause,
            return_type: verum_common::Maybe::None,
            std_attr: verum_common::Maybe::None,
            contexts: verum_common::List::new(),
            generic_where_clause: verum_common::Maybe::None,
            meta_where_clause: verum_common::Maybe::None,
            requires: verum_common::List::new(),
            ensures: verum_common::List::new(),
            attributes: verum_common::List::new(),
            body: verum_common::Maybe::None,
            span: Span::dummy(),
        }
    }

    #[test]
    fn test_infer_function_decl_pure() {
        let mut inferrer = PropertyInferrer::new();
        let func = create_test_function(false, verum_common::Maybe::None);

        let properties = inferrer.infer_function_decl(&func);

        assert!(properties.is_pure());
        assert!(!properties.is_fallible());
        assert!(!properties.is_async());
    }

    #[test]
    fn test_infer_function_decl_async() {
        let mut inferrer = PropertyInferrer::new();
        let func = create_test_function(true, verum_common::Maybe::None);

        let properties = inferrer.infer_function_decl(&func);

        assert!(properties.is_async());
        assert!(!properties.is_fallible());
        assert!(!properties.is_pure());
    }

    #[test]
    fn test_infer_function_decl_throws_clause_implies_fallible() {
        use verum_ast::span::Span;

        let mut inferrer = PropertyInferrer::new();

        // Create a function with a throws clause
        let throws = verum_ast::decl::ThrowsClause {
            error_types: verum_common::List::new(), // Empty list is fine for testing
            span: Span::dummy(),
        };
        let func = create_test_function(false, verum_common::Maybe::Some(throws));

        let properties = inferrer.infer_function_decl(&func);

        // Key assertion: throws clause implies Fallible
        assert!(
            properties.is_fallible(),
            "Function with throws clause should have Fallible property"
        );
        assert!(!properties.is_pure());
        assert!(!properties.is_async());
    }

    #[test]
    fn test_infer_function_decl_async_and_throws() {
        use verum_ast::span::Span;

        let mut inferrer = PropertyInferrer::new();

        // Create an async function with a throws clause
        let throws = verum_ast::decl::ThrowsClause {
            error_types: verum_common::List::new(),
            span: Span::dummy(),
        };
        let func = create_test_function(true, verum_common::Maybe::Some(throws));

        let properties = inferrer.infer_function_decl(&func);

        // Both Async and Fallible should be present
        assert!(
            properties.is_async(),
            "Async function should have Async property"
        );
        assert!(
            properties.is_fallible(),
            "Function with throws clause should have Fallible property"
        );
        assert!(!properties.is_pure());
        assert_eq!(properties.len(), 2);
    }

    // ===== Meta Function Purity Validation Tests =====

    #[test]
    fn test_validate_for_meta_fn_pure() {
        let pure = PropertySet::pure();
        assert!(pure.validate_for_meta_fn().is_ok());
    }

    #[test]
    fn test_validate_for_meta_fn_with_io() {
        let io = PropertySet::single(ComputationalProperty::IO);
        let result = io.validate_for_meta_fn();
        assert!(result.is_err());
        let impure = result.unwrap_err();
        assert_eq!(impure.len(), 1);
        assert!(impure.contains(&ComputationalProperty::IO));
    }

    #[test]
    fn test_validate_for_meta_fn_with_multiple_impure() {
        let properties = PropertySet::from_properties(vec![
            ComputationalProperty::IO,
            ComputationalProperty::Mutates,
            ComputationalProperty::WritesExternal,
        ]);
        let result = properties.validate_for_meta_fn();
        assert!(result.is_err());
        let impure = result.unwrap_err();
        assert_eq!(impure.len(), 3);
    }

    #[test]
    fn test_validate_for_meta_fn_with_async() {
        // Async is not allowed in meta functions
        let async_prop = PropertySet::single(ComputationalProperty::Async);
        assert!(async_prop.validate_for_meta_fn().is_err());
    }

    #[test]
    fn test_validate_for_meta_fn_with_ffi() {
        // FFI is not allowed in meta functions
        let ffi = PropertySet::single(ComputationalProperty::FFI);
        assert!(ffi.validate_for_meta_fn().is_err());
    }

    #[test]
    fn test_validate_for_meta_fn_with_fallible() {
        // Fallible IS allowed in meta functions because:
        // - Arithmetic ops like % are pure per spec line 428
        // - Compile-time errors are acceptable
        // - Operations are still deterministic
        let fallible = PropertySet::single(ComputationalProperty::Fallible);
        assert!(
            fallible.validate_for_meta_fn().is_ok(),
            "Fallible should be allowed in meta functions"
        );
    }

    #[test]
    fn test_validate_for_meta_fn_with_fallible_and_io() {
        // Fallible + IO should fail (IO is not allowed)
        let properties = PropertySet::from_properties(vec![
            ComputationalProperty::Fallible,
            ComputationalProperty::IO,
        ]);
        let result = properties.validate_for_meta_fn();
        assert!(result.is_err());
        let impure = result.unwrap_err();
        // Only IO should be reported as impure, not Fallible
        assert_eq!(impure.len(), 1);
        assert!(impure.contains(&ComputationalProperty::IO));
        assert!(!impure.contains(&ComputationalProperty::Fallible));
    }

    #[test]
    fn test_validate_for_meta_fn_with_divergent() {
        // Divergent IS allowed in meta functions because:
        // - Meta system control flow: loops are allowed in meta functions (with iteration limits)
        // - Spec lines 393-396 specify iteration limits to catch infinite loops
        // - Meta evaluation has step limits, so non-termination is bounded
        let divergent = PropertySet::single(ComputationalProperty::Divergent);
        assert!(
            divergent.validate_for_meta_fn().is_ok(),
            "Divergent should be allowed in meta functions (loops with step limits)"
        );
    }

    #[test]
    fn test_validate_for_meta_fn_with_divergent_and_fallible() {
        // Divergent + Fallible should both be allowed
        let properties = PropertySet::from_properties(vec![
            ComputationalProperty::Divergent,
            ComputationalProperty::Fallible,
        ]);
        assert!(
            properties.validate_for_meta_fn().is_ok(),
            "Divergent + Fallible should both be allowed in meta functions"
        );
    }

    #[test]
    fn test_validate_for_meta_fn_with_divergent_and_io() {
        // Divergent + IO should fail (IO is not allowed)
        let properties = PropertySet::from_properties(vec![
            ComputationalProperty::Divergent,
            ComputationalProperty::IO,
        ]);
        let result = properties.validate_for_meta_fn();
        assert!(result.is_err());
        let impure = result.unwrap_err();
        // Only IO should be reported as impure
        assert_eq!(impure.len(), 1);
        assert!(impure.contains(&ComputationalProperty::IO));
        assert!(!impure.contains(&ComputationalProperty::Divergent));
    }

    // ===== Pure Function Purity Validation Tests =====

    #[test]
    fn test_validate_for_pure_fn_pure() {
        let pure = PropertySet::pure();
        assert!(pure.validate_for_pure_fn().is_ok());
    }

    #[test]
    fn test_validate_for_pure_fn_with_io() {
        let io = PropertySet::single(ComputationalProperty::IO);
        let result = io.validate_for_pure_fn();
        assert!(result.is_err());
        let impure = result.unwrap_err();
        assert_eq!(impure.len(), 1);
        assert!(impure.contains(&ComputationalProperty::IO));
    }

    #[test]
    fn test_validate_for_pure_fn_with_mutates() {
        let mutates = PropertySet::single(ComputationalProperty::Mutates);
        let result = mutates.validate_for_pure_fn();
        assert!(result.is_err());
        let impure = result.unwrap_err();
        assert!(impure.contains(&ComputationalProperty::Mutates));
    }

    #[test]
    fn test_validate_for_pure_fn_with_async() {
        let async_prop = PropertySet::single(ComputationalProperty::Async);
        let result = async_prop.validate_for_pure_fn();
        assert!(result.is_err());
        let impure = result.unwrap_err();
        assert!(impure.contains(&ComputationalProperty::Async));
    }

    #[test]
    fn test_validate_for_pure_fn_with_fallible() {
        // Fallible is allowed in pure functions (errors are deterministic)
        let fallible = PropertySet::single(ComputationalProperty::Fallible);
        assert!(
            fallible.validate_for_pure_fn().is_ok(),
            "Fallible should be allowed in pure functions"
        );
    }

    #[test]
    fn test_validate_for_pure_fn_with_divergent() {
        // Divergent is allowed in pure functions (panic/unreachable per spec)
        let divergent = PropertySet::single(ComputationalProperty::Divergent);
        assert!(
            divergent.validate_for_pure_fn().is_ok(),
            "Divergent should be allowed in pure functions"
        );
    }

    #[test]
    fn test_validate_for_pure_fn_with_allocates() {
        // Allocates is allowed in pure functions (heap allocation is a side effect
        // but widely accepted in pure functional languages)
        let allocates = PropertySet::single(ComputationalProperty::Allocates);
        assert!(
            allocates.validate_for_pure_fn().is_ok(),
            "Allocates should be allowed in pure functions"
        );
    }

    #[test]
    fn test_validate_for_pure_fn_with_multiple_violations() {
        let properties = PropertySet::from_properties(vec![
            ComputationalProperty::IO,
            ComputationalProperty::Mutates,
            ComputationalProperty::Spawns,
        ]);
        let result = properties.validate_for_pure_fn();
        assert!(result.is_err());
        let impure = result.unwrap_err();
        assert_eq!(impure.len(), 3);
    }

    #[test]
    fn test_validate_for_pure_fn_fallible_and_divergent_ok() {
        // Fallible + Divergent are both allowed in pure functions
        let properties = PropertySet::from_properties(vec![
            ComputationalProperty::Fallible,
            ComputationalProperty::Divergent,
        ]);
        assert!(
            properties.validate_for_pure_fn().is_ok(),
            "Fallible + Divergent should both be allowed in pure functions"
        );
    }

    #[test]
    fn test_validate_for_pure_fn_fallible_and_io_fails() {
        // Fallible + IO should fail (IO is not allowed)
        let properties = PropertySet::from_properties(vec![
            ComputationalProperty::Fallible,
            ComputationalProperty::IO,
        ]);
        let result = properties.validate_for_pure_fn();
        assert!(result.is_err());
        let impure = result.unwrap_err();
        assert_eq!(impure.len(), 1);
        assert!(impure.contains(&ComputationalProperty::IO));
        assert!(!impure.contains(&ComputationalProperty::Fallible));
    }
}
