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

    // -----------------------------------------------------------------------
    // Resource-tagged variants — track *what* the function reads/writes/spawns.
    //

    // These exist so capability-audit (`@verify(static)` + `@permission(...)`)
    // can compare a function's effect set against a declared allow-list:
    //  property `Reads(FileSystem("/etc/*"))` matches `@permission(fs_read: ["/etc/*"])`.
    //

    // The plain `ReadsExternal` / `WritesExternal` / `Spawns` remain for
    // back-compat — passes that don't care about the tagged distinction
    // can use either form.
    // -----------------------------------------------------------------------

    /// Reads a tagged resource — FileSystem path / Network host / Env name / Stdin.
    Reads(ResourceKind),

    /// Writes a tagged resource — FileSystem path / Network host / Env name / Stdout / Stderr.
    Writes(ResourceKind),

    /// Spawns a tagged subordinate — process binary, async task, OS thread.
    SpawnsKind(SpawnKind),

    /// Custom user-defined computational property
    /// For extensibility and domain-specific properties
    Custom(Text),
}

/// Kinds of resource a `Reads` / `Writes` property can refer to.
///

/// Each variant carries enough detail for capability-audit to match against
/// a frontmatter `@permission(...)` allow-list. Glob patterns are kept as
/// plain `Text` — the matcher in `core.shell.permissions` handles glob
/// expansion.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceKind {
    /// Filesystem path (literal or glob pattern).
    FileSystem(Text),
    /// Network endpoint — `host`, `host:port`, `*.example.com`, CIDR, etc.
    Network(Text),
    /// Environment variable name.
    Env(Text),
    /// Standard input stream.
    Stdin,
    /// Standard output stream.
    Stdout,
    /// Standard error stream.
    Stderr,
    /// User-defined / domain-specific resource.
    Custom(Text),
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResourceKind::FileSystem(p) => write!(f, "FileSystem({})", p),
            ResourceKind::Network(h)    => write!(f, "Network({})", h),
            ResourceKind::Env(n)        => write!(f, "Env({})", n),
            ResourceKind::Stdin         => write!(f, "Stdin"),
            ResourceKind::Stdout        => write!(f, "Stdout"),
            ResourceKind::Stderr        => write!(f, "Stderr"),
            ResourceKind::Custom(s)     => write!(f, "Custom({})", s),
        }
    }
}

/// Kinds of subordinate a `SpawnsKind` property can refer to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SpawnKind {
    /// External program — `program` is the basename or absolute path.
    Process(Text),
    /// Async task on the executor.
    Task,
    /// OS thread.
    Thread,
    /// User-defined.
    Custom(Text),
}

impl fmt::Display for SpawnKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpawnKind::Process(p) => write!(f, "Process({})", p),
            SpawnKind::Task       => write!(f, "Task"),
            SpawnKind::Thread     => write!(f, "Thread"),
            SpawnKind::Custom(s)  => write!(f, "Custom({})", s),
        }
    }
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

/// Lift an FFI function declaration into a `PropertySet` so the
/// property-inference engine propagates its safety surface to every
/// caller.
///

/// Without this lift the FFI registration in `infer.rs` set
/// `properties: None`, dropping the four declared safety facts
/// (`memory_effects`, `thread_safe`, `error_protocol`, plus the
/// implicit "this is an FFI call" tag) at the type-system boundary.
/// A `pure fn` could then call an `Allocates` extern with no
/// diagnostic — the master-audit ranked this E-3 / S7 SOUNDNESS.
///

/// Mapping (each FFI declaration → one or more `ComputationalProperty`):
///

/// | FFI declaration | Properties added |
/// |---------------------------------|------------------------------|
/// | every FFI function | `FFI` |
/// | `thread_safe = false` | `Mutates` |
/// | `memory_effects = Pure` | (nothing — already pure) |
/// | `memory_effects = Reads(_)` | `IO`, `ReadsExternal` |
/// | `memory_effects = Writes(_)` | `IO`, `WritesExternal`, |
/// | | `Mutates` |
/// | `memory_effects = Allocates` | `Allocates` |
/// | `memory_effects = Deallocates(_)`| `Deallocates` |
/// | `memory_effects = Combined(xs)` | union of mapped(xs) |
/// | `error_protocol != None` | `Fallible` |
///

/// The result is `Some(PropertySet)` whenever ANY non-trivial
/// property would be set. A truly pure thread-safe FFI with
/// `error_protocol = None` still returns `Some({FFI})` — the FFI
/// tag is always added so capability audits can recognise the
/// boundary even on otherwise-pure externs.
pub fn lift_ffi_function_to_properties(
    ffi: &verum_ast::ffi::FFIFunction,
) -> Option<PropertySet> {
    use verum_ast::ffi::{ErrorProtocol, MemoryEffects};

    let mut props = Set::<ComputationalProperty>::new();
    props.insert(ComputationalProperty::FFI);

    if !ffi.thread_safe {
        props.insert(ComputationalProperty::Mutates);
    }

    fn collect(eff: &MemoryEffects, out: &mut Set<ComputationalProperty>) {
        match eff {
            MemoryEffects::Pure => {}
            MemoryEffects::Reads(_) => {
                out.insert(ComputationalProperty::IO);
                out.insert(ComputationalProperty::ReadsExternal);
            }
            MemoryEffects::Writes(_) => {
                out.insert(ComputationalProperty::IO);
                out.insert(ComputationalProperty::WritesExternal);
                out.insert(ComputationalProperty::Mutates);
            }
            MemoryEffects::Allocates => {
                out.insert(ComputationalProperty::Allocates);
            }
            MemoryEffects::Deallocates(_) => {
                out.insert(ComputationalProperty::Deallocates);
            }
            MemoryEffects::Combined(xs) => {
                for x in xs.iter() {
                    collect(x, out);
                }
            }
        }
    }
    collect(&ffi.memory_effects, &mut props);

    if !matches!(ffi.error_protocol, ErrorProtocol::None) {
        props.insert(ComputationalProperty::Fallible);
    }

    Some(PropertySet { properties: props })
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

    // ─── Resource-tagged accessors ──────────────────────────────────────────

    /// Iterate every `Reads(...)` resource in this set.
    pub fn iter_reads(&self) -> impl Iterator<Item = &ResourceKind> {
        self.properties.iter().filter_map(|p| match p {
            ComputationalProperty::Reads(r) => Some(r),
            _ => None,
        })
    }

    /// Iterate every `Writes(...)` resource.
    pub fn iter_writes(&self) -> impl Iterator<Item = &ResourceKind> {
        self.properties.iter().filter_map(|p| match p {
            ComputationalProperty::Writes(r) => Some(r),
            _ => None,
        })
    }

    /// Iterate every `Spawns(kind)` subordinate.
    pub fn iter_spawns(&self) -> impl Iterator<Item = &SpawnKind> {
        self.properties.iter().filter_map(|p| match p {
            ComputationalProperty::SpawnsKind(s) => Some(s),
            _ => None,
        })
    }

    /// True iff this set spawns any external process.
    pub fn spawns_processes(&self) -> bool {
        self.contains(&ComputationalProperty::Spawns)
            || self.properties.iter().any(|p| matches!(p, ComputationalProperty::SpawnsKind(SpawnKind::Process(_))))
    }

    /// Add a `Reads(resource)` property and ensure `IO` is also present.
    pub fn add_read(&mut self, resource: ResourceKind) {
        self.properties.insert(ComputationalProperty::Reads(resource));
        self.properties.insert(ComputationalProperty::IO);
        self.properties.remove(&ComputationalProperty::Pure);
    }

    /// Add a `Writes(resource)` property and ensure `IO` is also present.
    pub fn add_write(&mut self, resource: ResourceKind) {
        self.properties.insert(ComputationalProperty::Writes(resource));
        self.properties.insert(ComputationalProperty::IO);
        self.properties.remove(&ComputationalProperty::Pure);
    }

    /// Add a `Spawns(kind)` property and ensure both `IO` and the legacy
    /// `Spawns` are present.
    pub fn add_spawn(&mut self, kind: SpawnKind) {
        self.properties.insert(ComputationalProperty::SpawnsKind(kind));
        self.properties.insert(ComputationalProperty::Spawns);
        self.properties.insert(ComputationalProperty::IO);
        self.properties.remove(&ComputationalProperty::Pure);
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
            ComputationalProperty::Reads(r)       => write!(f, "Reads({})", r),
            ComputationalProperty::Writes(r)      => write!(f, "Writes({})", r),
            ComputationalProperty::SpawnsKind(s)  => write!(f, "Spawns({})", s),
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

// =============================================================================
// ComputationalSignature — unified function-level computational shape (#171)
// =============================================================================

/// **Computational signature** — a function's compile-time
/// computational shape: which contexts it depends on (DI) PLUS which
/// computational properties it has.
///

/// Verum's CLAUDE.md establishes a **clear architectural distinction**
/// between these two concepts:
///

///  - **Contexts** (DI): runtime dependency injection via
///  `using [Database, Logger]`. Resolved at call time; ~5–30ns.
///  - **Properties**: compile-time computational classification
///  (`Pure`, `IO`, `Async`, `Fallible`, `Mutates`, etc.).
///  Zero runtime cost.
///

/// The two are SEPARATE concepts ("Verum has no algebraic effects")
/// but consumers that need a function's full computational shape —
/// purity audits, capability checks, optimization decisions, FFI
/// boundary analysis — typically need BOTH. `ComputationalSignature`
/// is the unified handle.
///

/// **Architectural notes** (per CLAUDE.md):
///

///  - Properties MUST NOT be called "Effects". Verum doesn't have
///  algebraic effects; the property system is a compile-time
///  classification, not a runtime dispatch mechanism.
///  - Contexts and Properties are co-existent on a function type;
///  bundling them does NOT collapse the distinction.
///  - The `ComputationalSignature` is descriptive, not prescriptive
///  — it's a uniform read accessor; the underlying storage on
///  `Type::Function` keeps the two fields separate.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ComputationalSignature {
    /// Context names declared in the function's `using [...]` clause.
    /// Strings to keep the API serde-friendly without dragging in
    /// the full Context AST. Empty list = no context dependencies.
    pub contexts: List<Text>,
    /// Compile-time computational properties. Empty set = no
    /// declared / inferred properties (typically a fully pure
    /// function).
    pub properties: PropertySet,
}

impl ComputationalSignature {
    /// Construct a signature from raw context names + property set.
    pub fn new(contexts: List<Text>, properties: PropertySet) -> Self {
        Self {
            contexts,
            properties,
        }
    }

    /// Construct an empty signature — no contexts. Per Verum
    /// convention (`PropertySet::from_properties(empty)` defaults to
    /// `Pure`), this is the **pure baseline**: no context dependencies,
    /// only the `Pure` property. Architecturally equivalent to
    /// [`Self::pure`].
    pub fn empty() -> Self {
        Self::pure()
    }

    /// Construct a pure signature — no contexts, explicitly Pure
    /// property. Different from `empty()` in that the Pure property
    /// is asserted (not merely absent).
    pub fn pure() -> Self {
        Self {
            contexts: List::new(),
            properties: PropertySet::pure(),
        }
    }

    /// Whether this signature has any context dependencies.
    pub fn has_contexts(&self) -> bool {
        !self.contexts.is_empty()
    }

    /// Whether this signature has any declared properties.
    pub fn has_properties(&self) -> bool {
        !self.properties.is_empty()
    }

    /// Whether this signature is fully pure — no contexts AND
    /// either empty properties or only the `Pure` property.
    pub fn is_pure(&self) -> bool {
        if !self.contexts.is_empty() {
            return false;
        }
        self.properties.is_empty()
            || (self.properties.len() == 1
                && self.properties.contains(&ComputationalProperty::Pure))
    }

    /// Whether the function is async (carries the `Async` property
    /// or any context whose name suggests async — conservative
    /// detection by property only).
    pub fn is_async(&self) -> bool {
        self.properties.contains(&ComputationalProperty::Async)
    }

    /// Whether the function is fallible (carries the `Fallible`
    /// property).
    pub fn is_fallible(&self) -> bool {
        self.properties.contains(&ComputationalProperty::Fallible)
    }

    /// Whether the function performs IO.
    pub fn is_io(&self) -> bool {
        self.properties.contains(&ComputationalProperty::IO)
    }

    /// Whether the function mutates state.
    pub fn mutates(&self) -> bool {
        self.properties.contains(&ComputationalProperty::Mutates)
    }

    /// Whether the function is at the FFI boundary.
    pub fn is_ffi(&self) -> bool {
        self.properties.contains(&ComputationalProperty::FFI)
    }

    /// Number of context dependencies.
    pub fn context_count(&self) -> usize {
        self.contexts.len()
    }

    /// Number of declared properties.
    pub fn property_count(&self) -> usize {
        self.properties.len()
    }

    /// Whether this signature subsumes another — every context
    /// required by `other` is also required by `self`, AND every
    /// property in `other` is also in `self`. Used by call-site
    /// type-checking to verify the caller can supply everything
    /// the callee needs.
    pub fn subsumes(&self, other: &ComputationalSignature) -> bool {
        // Every context in `other` must also appear in `self`.
        let self_ctxs: std::collections::BTreeSet<&str> =
            self.contexts.iter().map(|c| c.as_str()).collect();
        for c in other.contexts.iter() {
            if !self_ctxs.contains(c.as_str()) {
                return false;
            }
        }
        // Every property in `other` must also appear in `self`.
        for p in other.properties.iter() {
            if !self.properties.contains(p) {
                return false;
            }
        }
        true
    }

    /// Union of two signatures — the minimum signature that
    /// accommodates both. Contexts are merged (deduplicated by
    /// string equality); properties are unioned via PropertySet.
    pub fn union(&self, other: &ComputationalSignature) -> ComputationalSignature {
        let mut merged_ctxs: std::collections::BTreeSet<Text> =
            self.contexts.iter().cloned().collect();
        for c in other.contexts.iter() {
            merged_ctxs.insert(c.clone());
        }
        let mut union_contexts = List::new();
        for c in merged_ctxs {
            union_contexts.push(c);
        }
        ComputationalSignature {
            contexts: union_contexts,
            properties: self.properties.union(&other.properties),
        }
    }

    /// Diagnostic-friendly classification tag.
    ///

    ///  - `"pure"` if `is_pure()`
    ///  - `"async"` if `is_async()`
    ///  - `"io"` if `is_io()`
    ///  - `"impure"` otherwise
    pub fn classify(&self) -> &'static str {
        if self.is_pure() {
            "pure"
        } else if self.is_async() {
            "async"
        } else if self.is_io() {
            "io"
        } else if self.mutates() {
            "mutating"
        } else if self.is_ffi() {
            "ffi"
        } else {
            "impure"
        }
    }
}

impl Default for ComputationalSignature {
    fn default() -> Self {
        Self::empty()
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

    // -----------------------------------------------------------------
    // S7 — FFI safety lift to PropertySet
    // -----------------------------------------------------------------

    fn ffi_fn_for(
        thread_safe: bool,
        memory_effects: verum_ast::ffi::MemoryEffects,
        error_protocol: verum_ast::ffi::ErrorProtocol,
    ) -> verum_ast::ffi::FFIFunction {
        use verum_ast::span::Span;
        verum_ast::ffi::FFIFunction {
            name: verum_ast::ty::Ident::new("f", Span::dummy()),
            signature: verum_ast::ffi::FFISignature {
                params: List::new(),
                return_type: verum_ast::ty::Type::unit(Span::dummy()),
                calling_convention: verum_ast::ffi::CallingConvention::C,
                is_variadic: false,
                span: Span::dummy(),
            },
            requires: List::new(),
            ensures: List::new(),
            memory_effects,
            thread_safe,
            error_protocol,
            ownership: verum_ast::ffi::Ownership::Borrow,
            span: Span::dummy(),
        }
    }

    #[test]
    fn lift_ffi_pure_thread_safe_no_error_only_marks_ffi() {
        let f = ffi_fn_for(
            true,
            verum_ast::ffi::MemoryEffects::Pure,
            verum_ast::ffi::ErrorProtocol::None,
        );
        let ps = lift_ffi_function_to_properties(&f).unwrap();
        assert!(ps.contains(&ComputationalProperty::FFI));
        assert!(!ps.contains(&ComputationalProperty::Mutates));
        assert!(!ps.contains(&ComputationalProperty::Fallible));
        assert!(!ps.contains(&ComputationalProperty::Allocates));
    }

    #[test]
    fn lift_ffi_thread_unsafe_adds_mutates() {
        let f = ffi_fn_for(
            false,
            verum_ast::ffi::MemoryEffects::Pure,
            verum_ast::ffi::ErrorProtocol::None,
        );
        let ps = lift_ffi_function_to_properties(&f).unwrap();
        assert!(ps.contains(&ComputationalProperty::Mutates));
        assert!(ps.contains(&ComputationalProperty::FFI));
    }

    #[test]
    fn lift_ffi_allocates_propagates() {
        let f = ffi_fn_for(
            true,
            verum_ast::ffi::MemoryEffects::Allocates,
            verum_ast::ffi::ErrorProtocol::None,
        );
        let ps = lift_ffi_function_to_properties(&f).unwrap();
        assert!(ps.contains(&ComputationalProperty::Allocates));
    }

    #[test]
    fn lift_ffi_writes_adds_io_writes_external_mutates() {
        let f = ffi_fn_for(
            true,
            verum_ast::ffi::MemoryEffects::Writes(verum_common::Maybe::None),
            verum_ast::ffi::ErrorProtocol::None,
        );
        let ps = lift_ffi_function_to_properties(&f).unwrap();
        assert!(ps.contains(&ComputationalProperty::IO));
        assert!(ps.contains(&ComputationalProperty::WritesExternal));
        assert!(ps.contains(&ComputationalProperty::Mutates));
    }

    #[test]
    fn lift_ffi_errno_propagates_fallible() {
        let f = ffi_fn_for(
            true,
            verum_ast::ffi::MemoryEffects::Pure,
            verum_ast::ffi::ErrorProtocol::Errno,
        );
        let ps = lift_ffi_function_to_properties(&f).unwrap();
        assert!(ps.contains(&ComputationalProperty::Fallible));
    }

    #[test]
    fn lift_ffi_combined_effects_unions_mappings() {
        let f = ffi_fn_for(
            true,
            verum_ast::ffi::MemoryEffects::Combined({
                let mut xs = List::new();
                xs.push(verum_ast::ffi::MemoryEffects::Allocates);
                xs.push(verum_ast::ffi::MemoryEffects::Reads(verum_common::Maybe::None));
                xs
            }),
            verum_ast::ffi::ErrorProtocol::None,
        );
        let ps = lift_ffi_function_to_properties(&f).unwrap();
        assert!(ps.contains(&ComputationalProperty::Allocates));
        assert!(ps.contains(&ComputationalProperty::IO));
        assert!(ps.contains(&ComputationalProperty::ReadsExternal));
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

#[cfg(test)]
mod computational_signature_tests {
    use super::*;

    #[test]
    fn empty_signature_has_no_contexts_and_pure_baseline() {
        let sig = ComputationalSignature::empty();
        assert!(!sig.has_contexts());
        assert_eq!(sig.context_count(), 0);
        // Per Verum convention, empty PropertySet defaults to Pure —
        // the pure baseline. `empty()` and `pure()` are equivalent.
        assert!(sig.is_pure());
        assert_eq!(sig, ComputationalSignature::pure());
    }

    #[test]
    fn pure_signature_has_pure_property() {
        let sig = ComputationalSignature::pure();
        assert!(!sig.has_contexts());
        assert!(sig.has_properties());
        assert!(sig.is_pure());
        assert_eq!(sig.classify(), "pure");
    }

    #[test]
    fn empty_signature_is_pure_classification() {
        let sig = ComputationalSignature::empty();
        // Empty signature = no contexts + no properties = pure baseline.
        assert!(sig.is_pure());
        assert_eq!(sig.classify(), "pure");
    }

    #[test]
    fn signature_with_io_property_is_io() {
        let sig = ComputationalSignature::new(
            List::new(),
            PropertySet::single(ComputationalProperty::IO),
        );
        assert!(sig.is_io());
        assert!(!sig.is_pure());
        assert_eq!(sig.classify(), "io");
    }

    #[test]
    fn signature_with_async_property_is_async() {
        let sig = ComputationalSignature::new(
            List::new(),
            PropertySet::single(ComputationalProperty::Async),
        );
        assert!(sig.is_async());
        assert_eq!(sig.classify(), "async");
    }

    #[test]
    fn signature_with_context_is_not_pure() {
        let mut ctxs = List::new();
        ctxs.push(Text::from("Database"));
        let sig = ComputationalSignature::new(ctxs, PropertySet::from_properties(Vec::<ComputationalProperty>::new()));
        assert!(sig.has_contexts());
        assert!(!sig.is_pure());
    }

    #[test]
    fn subsumes_requires_all_contexts() {
        let mut superset_ctxs = List::new();
        superset_ctxs.push(Text::from("Database"));
        superset_ctxs.push(Text::from("Logger"));
        let superset = ComputationalSignature::new(superset_ctxs, PropertySet::from_properties(Vec::<ComputationalProperty>::new()));

        let mut subset_ctxs = List::new();
        subset_ctxs.push(Text::from("Database"));
        let subset = ComputationalSignature::new(subset_ctxs, PropertySet::from_properties(Vec::<ComputationalProperty>::new()));

        assert!(superset.subsumes(&subset));
        assert!(!subset.subsumes(&superset));
    }

    #[test]
    fn subsumes_requires_all_properties() {
        let superset = ComputationalSignature::new(
            List::new(),
            PropertySet::from_properties(vec![
                ComputationalProperty::IO,
                ComputationalProperty::Async,
            ]),
        );
        let subset = ComputationalSignature::new(
            List::new(),
            PropertySet::single(ComputationalProperty::IO),
        );
        assert!(superset.subsumes(&subset));
        assert!(!subset.subsumes(&superset));
    }

    #[test]
    fn union_merges_contexts_and_properties() {
        let mut a_ctxs = List::new();
        a_ctxs.push(Text::from("Database"));
        let a = ComputationalSignature::new(
            a_ctxs,
            PropertySet::single(ComputationalProperty::IO),
        );

        let mut b_ctxs = List::new();
        b_ctxs.push(Text::from("Logger"));
        let b = ComputationalSignature::new(
            b_ctxs,
            PropertySet::single(ComputationalProperty::Async),
        );

        let merged = a.union(&b);
        assert_eq!(merged.context_count(), 2);
        assert_eq!(merged.property_count(), 2);
        assert!(merged.is_io());
        assert!(merged.is_async());
    }

    #[test]
    fn union_dedups_overlapping_contexts() {
        let mut a_ctxs = List::new();
        a_ctxs.push(Text::from("Database"));
        a_ctxs.push(Text::from("Logger"));
        let a = ComputationalSignature::new(a_ctxs, PropertySet::from_properties(Vec::<ComputationalProperty>::new()));

        let mut b_ctxs = List::new();
        b_ctxs.push(Text::from("Database"));
        b_ctxs.push(Text::from("Telemetry"));
        let b = ComputationalSignature::new(b_ctxs, PropertySet::from_properties(Vec::<ComputationalProperty>::new()));

        let merged = a.union(&b);
        assert_eq!(merged.context_count(), 3, "expect dedup of Database");
    }

    #[test]
    fn classify_distinguishes_known_categories() {
        assert_eq!(ComputationalSignature::pure().classify(), "pure");
        assert_eq!(
            ComputationalSignature::new(
                List::new(),
                PropertySet::single(ComputationalProperty::IO),
            )
            .classify(),
            "io",
        );
        assert_eq!(
            ComputationalSignature::new(
                List::new(),
                PropertySet::single(ComputationalProperty::Async),
            )
            .classify(),
            "async",
        );
        assert_eq!(
            ComputationalSignature::new(
                List::new(),
                PropertySet::single(ComputationalProperty::Mutates),
            )
            .classify(),
            "mutating",
        );
        assert_eq!(
            ComputationalSignature::new(
                List::new(),
                PropertySet::single(ComputationalProperty::FFI),
            )
            .classify(),
            "ffi",
        );
    }

    #[test]
    fn predicates_match_property_membership() {
        let sig_io = ComputationalSignature::new(
            List::new(),
            PropertySet::single(ComputationalProperty::IO),
        );
        assert!(sig_io.is_io());
        assert!(!sig_io.is_async());
        assert!(!sig_io.mutates());
        assert!(!sig_io.is_ffi());
        assert!(!sig_io.is_fallible());
    }

    #[test]
    fn signature_serde_round_trip() {
        let mut ctxs = List::new();
        ctxs.push(Text::from("Database"));
        let sig = ComputationalSignature::new(
            ctxs,
            PropertySet::from_properties(vec![
                ComputationalProperty::IO,
                ComputationalProperty::Fallible,
            ]),
        );
        let json = serde_json::to_string(&sig).unwrap();
        let restored: ComputationalSignature = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.contexts, sig.contexts);
        // PropertySet equality checks set membership regardless of order.
        assert_eq!(restored.properties, sig.properties);
    }

    #[test]
    fn default_is_empty_signature() {
        let default_sig = ComputationalSignature::default();
        let empty_sig = ComputationalSignature::empty();
        assert_eq!(default_sig, empty_sig);
    }
}
