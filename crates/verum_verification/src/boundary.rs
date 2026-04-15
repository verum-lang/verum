//! Verification Boundary Management with Call Graph Analysis
//!
//! Handles trusted/untrusted code boundaries and proof obligations using
//! full call graph analysis for boundary detection.
//!
//! Boundaries occur where code transitions between verification levels (e.g.,
//! @verify(proof) code calling @verify(runtime) code). At each boundary, proof
//! obligations are generated to ensure safety is maintained: the callee's
//! preconditions must be verified at the caller's level or above.

use crate::context::ProofObligationId;
use crate::level::VerificationLevel;
use serde::{Deserialize, Serialize};
use verum_ast::decl::{FunctionBody, FunctionDecl, ImplDecl, ImplItemKind};
use verum_ast::span::Span;
use verum_ast::{Attribute, Block, Expr, ExprKind, Item, ItemKind, Module};
use verum_common::{List, Map, Maybe, Set, Text};

// Re-export from context for convenience
pub use crate::context::BoundaryKind;
pub use crate::context::ObligationKind;

/// Proof obligation structure
pub type ProofObligation = crate::context::ProofObligation;

/// Unique identifier for a function in the call graph
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FunctionId(u64);

impl FunctionId {
    /// Create a new function ID
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    /// Get the raw ID value
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for FunctionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "fn_{}", self.0)
    }
}

/// Source location for tracking call sites
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SourceLocation {
    /// File ID
    pub file_id: u32,
    /// Start byte offset
    pub start: u32,
    /// End byte offset
    pub end: u32,
}

impl SourceLocation {
    /// Create from a span
    pub fn from_span(span: Span) -> Self {
        Self {
            file_id: span.file_id.raw(),
            start: span.start,
            end: span.end,
        }
    }
}

/// Trusted code boundary marker
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedBoundary {
    /// Verification level of trusted code
    pub level: VerificationLevel,

    /// Description
    pub description: Text,
}

/// Untrusted code boundary marker
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UntrustedBoundary {
    /// Required validation at boundary
    pub validation_required: bool,

    /// Description
    pub description: Text,
}

/// A node in the call graph representing a function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallGraphNode {
    /// Unique identifier for this function
    pub function_id: FunctionId,

    /// Fully qualified name of the function
    pub name: Text,

    /// Verification level of this function
    pub verification_level: VerificationLevel,

    /// Functions that call this function
    pub callers: List<FunctionId>,

    /// Functions that this function calls
    pub callees: List<FunctionId>,

    /// Source location of the function definition
    pub location: SourceLocation,

    /// Module path containing this function
    pub module_path: Text,

    /// Whether this function is public
    pub is_public: bool,

    /// Whether this function is async
    pub is_async: bool,

    /// Whether this is a trusted function (marked with @trusted)
    pub is_trusted: bool,

    /// Whether this is an external/FFI function
    pub is_external: bool,
}

impl CallGraphNode {
    /// Create a new call graph node
    pub fn new(
        function_id: FunctionId,
        name: Text,
        verification_level: VerificationLevel,
        location: SourceLocation,
    ) -> Self {
        Self {
            function_id,
            name,
            verification_level,
            callers: List::new(),
            callees: List::new(),
            location,
            module_path: Text::from(""),
            is_public: false,
            is_async: false,
            is_trusted: false,
            is_external: false,
        }
    }

    /// Add a caller to this function
    pub fn add_caller(&mut self, caller_id: FunctionId) {
        if !self.callers.iter().any(|id| *id == caller_id) {
            self.callers.push(caller_id);
        }
    }

    /// Add a callee to this function
    pub fn add_callee(&mut self, callee_id: FunctionId) {
        if !self.callees.iter().any(|id| *id == callee_id) {
            self.callees.push(callee_id);
        }
    }
}

/// An edge in the call graph representing a function call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEdge {
    /// The calling function
    pub caller: FunctionId,

    /// The called function
    pub callee: FunctionId,

    /// Location of the call site
    pub call_site: SourceLocation,

    /// Kind of boundary crossing, if any
    pub boundary_kind: Maybe<BoundaryKind>,

    /// Direction of the boundary crossing
    pub boundary_direction: Maybe<BoundaryDirection>,

    /// Whether this edge represents a recursive call
    pub is_recursive: bool,

    /// Whether this edge represents an indirect call (via function pointer or closure)
    pub is_indirect: bool,
}

impl CallEdge {
    /// Create a new call edge
    pub fn new(caller: FunctionId, callee: FunctionId, call_site: SourceLocation) -> Self {
        Self {
            caller,
            callee,
            call_site,
            boundary_kind: Maybe::None,
            boundary_direction: Maybe::None,
            is_recursive: false,
            is_indirect: false,
        }
    }

    /// Check if this edge crosses a verification boundary
    pub fn crosses_boundary(&self) -> bool {
        self.boundary_kind.is_some()
    }
}

/// Direction of verification level change at a boundary
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoundaryDirection {
    /// Moving to more restrictive verification (e.g., Runtime -> Static)
    MoreRestrictive,
    /// Moving to less restrictive verification (e.g., Static -> Runtime)
    LessRestrictive,
    /// Same verification level
    Same,
}

impl BoundaryDirection {
    /// Determine direction from two verification levels
    pub fn from_levels(from: VerificationLevel, to: VerificationLevel) -> Self {
        use VerificationLevel::*;
        match (from, to) {
            (Runtime, Static) | (Runtime, Proof) | (Static, Proof) => {
                BoundaryDirection::MoreRestrictive
            }
            (Static, Runtime) | (Proof, Runtime) | (Proof, Static) => {
                BoundaryDirection::LessRestrictive
            }
            _ => BoundaryDirection::Same,
        }
    }
}

/// The complete call graph for a module or compilation unit
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallGraph {
    /// All functions indexed by ID
    nodes: Map<FunctionId, CallGraphNode>,

    /// All call edges
    edges: List<CallEdge>,

    /// Map from function name to ID for quick lookup
    name_to_id: Map<Text, FunctionId>,

    /// Next function ID to allocate
    next_id: u64,

    /// Detected boundary crossings
    boundaries: List<DetectedBoundary>,

    /// Statistics about the call graph
    stats: CallGraphStats,
}

impl CallGraph {
    /// Create a new empty call graph
    pub fn new() -> Self {
        Self {
            nodes: Map::new(),
            edges: List::new(),
            name_to_id: Map::new(),
            next_id: 0,
            boundaries: List::new(),
            stats: CallGraphStats::default(),
        }
    }

    /// Build a call graph from an AST module
    pub fn from_module(module: &Module) -> Self {
        let mut builder = CallGraphBuilder::new();
        builder.build_from_module(module);
        builder.into_call_graph()
    }

    /// Add a function node to the graph
    pub fn add_node(&mut self, node: CallGraphNode) -> FunctionId {
        let id = node.function_id;
        self.name_to_id.insert(node.name.clone(), id);

        // Update functions_by_level stats
        match node.verification_level {
            VerificationLevel::Runtime => self.stats.functions_by_level.runtime += 1,
            VerificationLevel::Static => self.stats.functions_by_level.static_ += 1,
            VerificationLevel::Proof => self.stats.functions_by_level.proof += 1,
        }

        self.nodes.insert(id, node);
        self.stats.total_functions += 1;
        id
    }

    /// Allocate a new function ID
    pub fn allocate_id(&mut self) -> FunctionId {
        let id = FunctionId::new(self.next_id);
        self.next_id += 1;
        id
    }

    /// Add a call edge to the graph
    pub fn add_edge(&mut self, mut edge: CallEdge) {
        // Update caller's callees
        if let Some(caller_node) = self.nodes.get_mut(&edge.caller) {
            caller_node.add_callee(edge.callee);
        }

        // Update callee's callers
        if let Some(callee_node) = self.nodes.get_mut(&edge.callee) {
            callee_node.add_caller(edge.caller);
        }

        // Detect boundary crossing
        if let (Some(caller), Some(callee)) =
            (self.nodes.get(&edge.caller), self.nodes.get(&edge.callee))
        {
            let caller_level = caller.verification_level;
            let callee_level = callee.verification_level;

            if caller_level != callee_level {
                edge.boundary_kind = Maybe::Some(BoundaryKind::FunctionCall);
                edge.boundary_direction =
                    Maybe::Some(BoundaryDirection::from_levels(caller_level, callee_level));
                self.stats.boundary_crossings += 1;
            }
        }

        // Check for recursion
        if edge.caller == edge.callee {
            edge.is_recursive = true;
            self.stats.recursive_calls += 1;
        }

        self.edges.push(edge);
        self.stats.total_calls += 1;
    }

    /// Get a function by ID
    pub fn get_node(&self, id: FunctionId) -> Maybe<&CallGraphNode> {
        match self.nodes.get(&id) {
            Some(node) => Maybe::Some(node),
            None => Maybe::None,
        }
    }

    /// Get a function by name
    pub fn get_node_by_name(&self, name: &str) -> Maybe<&CallGraphNode> {
        match self.name_to_id.get(&Text::from(name)) {
            Some(id) => self.get_node(*id),
            None => Maybe::None,
        }
    }

    /// Get all nodes
    pub fn nodes(&self) -> impl Iterator<Item = &CallGraphNode> {
        self.nodes.values()
    }

    /// Get all edges
    pub fn edges(&self) -> &List<CallEdge> {
        &self.edges
    }

    /// Get edges that cross verification boundaries
    pub fn boundary_edges(&self) -> impl Iterator<Item = &CallEdge> {
        self.edges.iter().filter(|e| e.crosses_boundary())
    }

    /// Detect all verification boundaries in the graph
    pub fn detect_boundaries(&mut self) -> &List<DetectedBoundary> {
        self.boundaries.clear();

        for edge in &self.edges {
            if let (Maybe::Some(kind), Maybe::Some(direction)) =
                (edge.boundary_kind, edge.boundary_direction)
                && let (Some(caller), Some(callee)) =
                    (self.nodes.get(&edge.caller), self.nodes.get(&edge.callee))
            {
                let boundary = DetectedBoundary {
                    caller_id: edge.caller,
                    callee_id: edge.callee,
                    caller_name: caller.name.clone(),
                    callee_name: callee.name.clone(),
                    caller_level: caller.verification_level,
                    callee_level: callee.verification_level,
                    call_site: edge.call_site,
                    boundary_kind: kind,
                    direction,
                    required_obligations: List::new(),
                };
                self.boundaries.push(boundary);
            }
        }

        &self.boundaries
    }

    /// Get all detected boundaries
    pub fn get_boundaries(&self) -> &List<DetectedBoundary> {
        &self.boundaries
    }

    /// Get statistics about the call graph
    pub fn stats(&self) -> &CallGraphStats {
        &self.stats
    }

    /// Find all functions reachable from a given function
    pub fn reachable_from(&self, start: FunctionId) -> Set<FunctionId> {
        let mut visited = Set::new();
        let mut stack = vec![start];

        while let Some(current) = stack.pop() {
            if visited.contains(&current) {
                continue;
            }
            visited.insert(current);

            if let Some(node) = self.nodes.get(&current) {
                for callee in &node.callees {
                    if !visited.contains(callee) {
                        stack.push(*callee);
                    }
                }
            }
        }

        visited
    }

    /// Find all functions that can reach a given function
    pub fn callers_of(&self, target: FunctionId) -> Set<FunctionId> {
        let mut visited = Set::new();
        let mut stack = vec![target];

        while let Some(current) = stack.pop() {
            if visited.contains(&current) {
                continue;
            }
            visited.insert(current);

            if let Some(node) = self.nodes.get(&current) {
                for caller in &node.callers {
                    if !visited.contains(caller) {
                        stack.push(*caller);
                    }
                }
            }
        }

        visited
    }

    /// Detect cycles in the call graph (mutual recursion)
    pub fn find_cycles(&self) -> List<List<FunctionId>> {
        let mut cycles = List::new();
        let mut visited = Set::new();
        let mut rec_stack = Set::new();
        let mut path = List::new();

        for node_id in self.nodes.keys() {
            if !visited.contains(node_id) {
                self.find_cycles_dfs(
                    *node_id,
                    &mut visited,
                    &mut rec_stack,
                    &mut path,
                    &mut cycles,
                );
            }
        }

        cycles
    }

    fn find_cycles_dfs(
        &self,
        current: FunctionId,
        visited: &mut Set<FunctionId>,
        rec_stack: &mut Set<FunctionId>,
        path: &mut List<FunctionId>,
        cycles: &mut List<List<FunctionId>>,
    ) {
        visited.insert(current);
        rec_stack.insert(current);
        path.push(current);

        if let Some(node) = self.nodes.get(&current) {
            for callee in &node.callees {
                if !visited.contains(callee) {
                    self.find_cycles_dfs(*callee, visited, rec_stack, path, cycles);
                } else if rec_stack.contains(callee) {
                    // Found a cycle - extract it from the path
                    let mut cycle = List::new();
                    let mut in_cycle = false;
                    for id in path.iter() {
                        if *id == *callee {
                            in_cycle = true;
                        }
                        if in_cycle {
                            cycle.push(*id);
                        }
                    }
                    if !cycle.is_empty() {
                        cycles.push(cycle);
                    }
                }
            }
        }

        path.pop();
        rec_stack.remove(&current);
    }
}

impl Default for CallGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// A detected verification boundary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedBoundary {
    /// ID of the calling function
    pub caller_id: FunctionId,

    /// ID of the called function
    pub callee_id: FunctionId,

    /// Name of the calling function
    pub caller_name: Text,

    /// Name of the called function
    pub callee_name: Text,

    /// Verification level of the caller
    pub caller_level: VerificationLevel,

    /// Verification level of the callee
    pub callee_level: VerificationLevel,

    /// Location of the call site
    pub call_site: SourceLocation,

    /// Kind of boundary
    pub boundary_kind: BoundaryKind,

    /// Direction of the boundary crossing
    pub direction: BoundaryDirection,

    /// Required proof obligations at this boundary
    pub required_obligations: List<RequiredObligation>,
}

impl DetectedBoundary {
    /// Check if this boundary requires runtime checks
    pub fn requires_runtime_checks(&self) -> bool {
        // When calling from higher to lower verification level
        matches!(self.direction, BoundaryDirection::LessRestrictive)
    }

    /// Check if this boundary requires proof obligations
    pub fn requires_proof_obligations(&self) -> bool {
        // When calling from lower to higher verification level
        matches!(self.direction, BoundaryDirection::MoreRestrictive)
    }
}

/// A required proof obligation at a boundary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequiredObligation {
    /// Kind of obligation
    pub kind: ObligationKind,

    /// Description of what needs to be proven
    pub description: Text,

    /// Whether this obligation has been fulfilled
    pub fulfilled: bool,
}

impl RequiredObligation {
    /// Create a new required obligation
    pub fn new(kind: ObligationKind, description: Text) -> Self {
        Self {
            kind,
            description,
            fulfilled: false,
        }
    }
}

/// Statistics about a call graph
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallGraphStats {
    /// Total number of functions
    pub total_functions: usize,

    /// Total number of call edges
    pub total_calls: usize,

    /// Number of boundary crossings
    pub boundary_crossings: usize,

    /// Number of recursive calls
    pub recursive_calls: usize,

    /// Number of functions at each verification level
    pub functions_by_level: FunctionsByLevel,
}

/// Count of functions by verification level
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FunctionsByLevel {
    /// Functions with runtime verification
    pub runtime: usize,

    /// Functions with static verification
    pub static_: usize,

    /// Functions with proof verification
    pub proof: usize,
}

/// Builder for constructing call graphs from AST
#[derive(Debug)]
pub struct CallGraphBuilder {
    graph: CallGraph,
    current_function: Maybe<FunctionId>,
    current_module_path: Text,
}

impl CallGraphBuilder {
    /// Create a new call graph builder
    pub fn new() -> Self {
        Self {
            graph: CallGraph::new(),
            current_function: Maybe::None,
            current_module_path: Text::from(""),
        }
    }

    /// Build from a module
    pub fn build_from_module(&mut self, module: &Module) {
        // First pass: collect all function declarations
        for item in &module.items {
            self.collect_function(item, &self.current_module_path.clone());
        }

        // Second pass: collect all call edges
        for item in &module.items {
            self.collect_calls(item);
        }
    }

    /// Collect a function declaration
    fn collect_function(&mut self, item: &Item, module_path: &Text) {
        match &item.kind {
            ItemKind::Function(func) => {
                // Convert Vec<Attribute> to List<Attribute> (parser outputs Vec)
                let attrs_list: List<Attribute> = item.attributes.iter().cloned().collect();
                self.add_function(func, module_path, &attrs_list);
            }
            ItemKind::Impl(impl_decl) => {
                self.collect_impl_functions(impl_decl, module_path);
            }
            ItemKind::Module(mod_decl) => {
                if let Some(items) = &mod_decl.items {
                    let new_path = if module_path.is_empty() {
                        Text::from(mod_decl.name.as_str())
                    } else {
                        Text::from(format!("{}.{}", module_path, mod_decl.name.as_str()))
                    };
                    for inner_item in items {
                        self.collect_function(inner_item, &new_path);
                    }
                }
            }
            _ => {}
        }
    }

    /// Collect functions from an impl block
    fn collect_impl_functions(&mut self, impl_decl: &ImplDecl, module_path: &Text) {
        for impl_item in &impl_decl.items {
            if let ImplItemKind::Function(func) = &impl_item.kind {
                self.add_function(func, module_path, &List::new());
            }
        }
    }

    /// Add a function to the graph
    fn add_function(&mut self, func: &FunctionDecl, module_path: &Text, attrs: &List<Attribute>) {
        let id = self.graph.allocate_id();
        let verification_level = extract_verification_level(attrs, func);

        let qualified_name = if module_path.is_empty() {
            Text::from(func.name.as_str())
        } else {
            Text::from(format!("{}.{}", module_path, func.name.as_str()))
        };

        let mut node = CallGraphNode::new(
            id,
            qualified_name,
            verification_level,
            SourceLocation::from_span(func.span),
        );

        node.module_path = module_path.clone();
        node.is_public = func.visibility.is_public();
        node.is_async = func.is_async;
        // Convert func.attributes from Vec to List
        let func_attrs_list: List<Attribute> = func.attributes.iter().cloned().collect();
        node.is_trusted =
            has_attribute(attrs, "trusted") || has_attribute(&func_attrs_list, "trusted");
        node.is_external = func.body.is_none();

        // Update stats
        match verification_level {
            VerificationLevel::Runtime => self.graph.stats.functions_by_level.runtime += 1,
            VerificationLevel::Static => self.graph.stats.functions_by_level.static_ += 1,
            VerificationLevel::Proof => self.graph.stats.functions_by_level.proof += 1,
        }

        self.graph.add_node(node);
    }

    /// Collect call edges from items
    fn collect_calls(&mut self, item: &Item) {
        match &item.kind {
            ItemKind::Function(func) => {
                let func_name = Text::from(func.name.as_str());
                if let Some(id) = self.graph.name_to_id.get(&func_name).copied() {
                    self.current_function = Maybe::Some(id);
                    if let Some(body) = &func.body {
                        self.visit_function_body(body);
                    }
                    self.current_function = Maybe::None;
                }
            }
            ItemKind::Impl(impl_decl) => {
                for impl_item in &impl_decl.items {
                    if let ImplItemKind::Function(func) = &impl_item.kind {
                        let func_name = Text::from(func.name.as_str());
                        if let Some(id) = self.graph.name_to_id.get(&func_name).copied() {
                            self.current_function = Maybe::Some(id);
                            if let Some(body) = &func.body {
                                self.visit_function_body(body);
                            }
                            self.current_function = Maybe::None;
                        }
                    }
                }
            }
            ItemKind::Module(mod_decl) => {
                if let Some(items) = &mod_decl.items {
                    for inner_item in items {
                        self.collect_calls(inner_item);
                    }
                }
            }
            _ => {}
        }
    }

    /// Visit a function body to find calls
    fn visit_function_body(&mut self, body: &FunctionBody) {
        match body {
            FunctionBody::Block(block) => self.visit_block(block),
            FunctionBody::Expr(expr) => self.visit_expr(expr),
        }
    }

    /// Visit a block
    fn visit_block(&mut self, block: &Block) {
        for stmt in &block.stmts {
            self.visit_stmt(stmt);
        }
        if let Some(expr) = &block.expr {
            self.visit_expr(expr);
        }
    }

    /// Visit a statement
    fn visit_stmt(&mut self, stmt: &verum_ast::Stmt) {
        use verum_ast::StmtKind;
        match &stmt.kind {
            StmtKind::Expr { expr, .. } => self.visit_expr(expr),
            StmtKind::Let { value, .. } => {
                if let Some(val) = value {
                    self.visit_expr(val);
                }
            }
            StmtKind::LetElse {
                value, else_block, ..
            } => {
                self.visit_expr(value);
                self.visit_block(else_block);
            }
            StmtKind::Item(item) => self.collect_calls(item),
            StmtKind::Defer(expr) => self.visit_expr(expr),
            StmtKind::Errdefer(expr) => self.visit_expr(expr),
            StmtKind::Provide { value, .. } => self.visit_expr(value),
            StmtKind::ProvideScope { value, block, .. } => {
                self.visit_expr(value);
                self.visit_expr(block);
            }
            StmtKind::Empty => {}
        }
    }

    /// Visit an expression to find calls
    fn visit_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Call { func, args, .. } => {
                // Record the call
                if let Maybe::Some(caller_id) = self.current_function {
                    let callee_name = extract_function_name(func);
                    if let Some(callee_id) = self.graph.name_to_id.get(&callee_name).copied() {
                        let edge = CallEdge::new(
                            caller_id,
                            callee_id,
                            SourceLocation::from_span(expr.span),
                        );
                        self.graph.add_edge(edge);
                    }
                }

                // Visit arguments
                self.visit_expr(func);
                for arg in args {
                    self.visit_expr(arg);
                }
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                self.visit_expr(receiver);
                for arg in args {
                    self.visit_expr(arg);
                }
            }
            ExprKind::Binary { left, right, .. } => {
                self.visit_expr(left);
                self.visit_expr(right);
            }
            ExprKind::Unary { expr, .. } => {
                self.visit_expr(expr);
            }
            ExprKind::Block(block) => {
                self.visit_block(block);
            }
            ExprKind::If {
                condition: _,
                then_branch,
                else_branch,
            } => {
                self.visit_block(then_branch);
                if let Some(else_expr) = else_branch {
                    self.visit_expr(else_expr);
                }
            }
            ExprKind::Match { expr, arms } => {
                self.visit_expr(expr);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.visit_expr(guard);
                    }
                    self.visit_expr(&arm.body);
                }
            }
            ExprKind::Loop {
                label: _,
                body: block,
                invariants: _,
            }
            | ExprKind::Async(block)
            | ExprKind::Unsafe(block)
            | ExprKind::Meta(block) => {
                self.visit_block(block);
            }
            ExprKind::While {
                label: _,
                condition,
                body,
                invariants: _,
                decreases: _,
            } => {
                self.visit_expr(condition);
                self.visit_block(body);
            }
            ExprKind::For {
                label: _,
                iter,
                body,
                ..
            } => {
                self.visit_expr(iter);
                self.visit_block(body);
            }
            ExprKind::ForAwait {
                label: _,
                async_iterable,
                body,
                ..
            } => {
                // for-await loop desugars to: loop { match iter.next().await { ... } }
                //
                // Call graph traversal for for-await:
                // 1. The async_iterable expression may contain calls to record
                // 2. The implicit .next() method call is on the AsyncIterator protocol
                //    - Method resolution happens during type checking, not here
                //    - The actual callee depends on the concrete AsyncIterator impl
                // 3. The body may contain calls that cross verification boundaries
                //
                // We traverse both the iterable and body to capture all explicit calls.
                // The implicit next() method is a protocol method and will be resolved
                // when the concrete type is known during type checking. At that point,
                // boundary analysis can be performed on the resolved method.
                //
                // Note: If the AsyncIterator implementation has different verification
                // levels, those boundaries will be detected when the method is resolved.
                self.visit_expr(async_iterable);
                self.visit_block(body);
            }
            ExprKind::Closure { body, .. } => {
                self.visit_expr(body);
            }
            ExprKind::Try(expr)
            | ExprKind::TryBlock(expr)
            | ExprKind::Await(expr)
            | ExprKind::Yield(expr)
            | ExprKind::Paren(expr) => {
                self.visit_expr(expr);
            }
            ExprKind::Return(maybe_expr) => {
                if let Some(e) = maybe_expr {
                    self.visit_expr(e);
                }
            }
            ExprKind::Break { label: _, value } => {
                if let Some(e) = value {
                    self.visit_expr(e);
                }
            }
            ExprKind::Tuple(exprs) | ExprKind::SetLiteral { elements: exprs } => {
                for e in exprs {
                    self.visit_expr(e);
                }
            }
            ExprKind::Array(array_expr) => match array_expr {
                verum_ast::expr::ArrayExpr::List(exprs) => {
                    for e in exprs {
                        self.visit_expr(e);
                    }
                }
                verum_ast::expr::ArrayExpr::Repeat { value, count } => {
                    self.visit_expr(value);
                    self.visit_expr(count);
                }
            },
            ExprKind::Record { fields, base, .. } => {
                for field in fields {
                    if let Some(val) = &field.value {
                        self.visit_expr(val);
                    }
                }
                if let Some(b) = base {
                    self.visit_expr(b);
                }
            }
            ExprKind::Field { expr, .. }
            | ExprKind::OptionalChain { expr, .. }
            | ExprKind::TupleIndex { expr, .. } => {
                self.visit_expr(expr);
            }
            ExprKind::Index { expr, index } => {
                self.visit_expr(expr);
                self.visit_expr(index);
            }
            ExprKind::Pipeline { left, right } | ExprKind::NullCoalesce { left, right } => {
                self.visit_expr(left);
                self.visit_expr(right);
            }
            ExprKind::Cast { expr, .. } => {
                self.visit_expr(expr);
            }
            ExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.visit_expr(s);
                }
                if let Some(e) = end {
                    self.visit_expr(e);
                }
            }
            ExprKind::Comprehension { expr, clauses }
            | ExprKind::StreamComprehension { expr, clauses }
            | ExprKind::SetComprehension { expr, clauses }
            | ExprKind::GeneratorComprehension { expr, clauses } => {
                self.visit_expr(expr);
                for clause in clauses {
                    match &clause.kind {
                        verum_ast::expr::ComprehensionClauseKind::For { iter, .. } => {
                            self.visit_expr(iter);
                        }
                        verum_ast::expr::ComprehensionClauseKind::If(e) => {
                            self.visit_expr(e);
                        }
                        verum_ast::expr::ComprehensionClauseKind::Let { value, .. } => {
                            self.visit_expr(value);
                        }
                    }
                }
            }
            ExprKind::MapComprehension {
                key_expr,
                value_expr,
                clauses,
            } => {
                self.visit_expr(key_expr);
                self.visit_expr(value_expr);
                for clause in clauses {
                    match &clause.kind {
                        verum_ast::expr::ComprehensionClauseKind::For { iter, .. } => {
                            self.visit_expr(iter);
                        }
                        verum_ast::expr::ComprehensionClauseKind::If(e) => {
                            self.visit_expr(e);
                        }
                        verum_ast::expr::ComprehensionClauseKind::Let { value, .. } => {
                            self.visit_expr(value);
                        }
                    }
                }
            }
            ExprKind::InterpolatedString { exprs, .. } => {
                for e in exprs {
                    self.visit_expr(e);
                }
            }
            ExprKind::TensorLiteral { data, .. } => {
                self.visit_expr(data);
            }
            ExprKind::MapLiteral { entries } => {
                for (k, v) in entries {
                    self.visit_expr(k);
                    self.visit_expr(v);
                }
            }
            ExprKind::TryRecover {
                try_block,
                recover,
            } => {
                self.visit_expr(try_block);
                self.visit_recover_body(recover);
            }
            ExprKind::TryFinally {
                try_block,
                finally_block,
            } => {
                self.visit_expr(try_block);
                self.visit_expr(finally_block);
            }
            ExprKind::TryRecoverFinally {
                try_block,
                recover,
                finally_block,
            } => {
                self.visit_expr(try_block);
                self.visit_recover_body(recover);
                self.visit_expr(finally_block);
            }
            ExprKind::Spawn { expr, .. } => {
                self.visit_expr(expr);
            }
            ExprKind::Inject { .. } => {}
            ExprKind::UseContext { handler, body, .. } => {
                self.visit_expr(handler);
                self.visit_expr(body);
            }
            ExprKind::Forall { body, .. } | ExprKind::Exists { body, .. } => {
                self.visit_expr(body);
            }
            ExprKind::Attenuate { context, .. } => {
                // Visit the context expression being attenuated
                self.visit_expr(context);
            }
            ExprKind::MacroCall { .. } => {
                // Macro calls are expanded during parsing/typechecking
                // Skip them during boundary analysis
            }
            ExprKind::TypeProperty { .. } => {
                // Type properties are compile-time constants (T.size, T.alignment, etc.)
                // No expressions to visit - the type is static
            }
            ExprKind::Throw(inner) => {
                // Visit the thrown expression
                self.visit_expr(inner);
            }
            ExprKind::Select { arms, .. } => {
                // Visit all select arms
                for arm in arms.iter() {
                    if let Some(future) = &arm.future {
                        self.visit_expr(future);
                    }
                    self.visit_expr(&arm.body);
                    if let Some(guard) = &arm.guard {
                        self.visit_expr(guard);
                    }
                }
            }
            ExprKind::Is { expr: inner, .. } => {
                // Visit the expression being tested (pattern doesn't contain expressions)
                self.visit_expr(inner);
            }
            ExprKind::Nursery {
                body,
                on_cancel,
                recover,
                options,
                ..
            } => {
                // Nursery creates a structured concurrency scope
                // Visit all parts that may contain function calls
                self.visit_block(body);
                if let Some(cancel_block) = on_cancel {
                    self.visit_block(cancel_block);
                }
                if let Some(recover_body) = recover {
                    self.visit_recover_body(recover_body);
                }
                // Visit timeout and max_tasks expressions
                if let Some(timeout_expr) = &options.timeout {
                    self.visit_expr(timeout_expr);
                }
                if let Some(max_tasks_expr) = &options.max_tasks {
                    self.visit_expr(max_tasks_expr);
                }
            }
            ExprKind::StreamLiteral(stream_lit) => {
                // Stream literals: stream[1, 2, 3, ...] or stream[0..100]
                // Visit contained expressions to find any function calls
                match &stream_lit.kind {
                    verum_ast::expr::StreamLiteralKind::Elements { elements, .. } => {
                        for elem in elements.iter() {
                            self.visit_expr(elem);
                        }
                    }
                    verum_ast::expr::StreamLiteralKind::Range { start, end, .. } => {
                        self.visit_expr(start);
                        if let Some(end_expr) = end {
                            self.visit_expr(end_expr);
                        }
                    }
                }
            }
            ExprKind::InlineAsm { operands, .. } => {
                // Inline assembly operands may contain expressions
                for operand in operands.iter() {
                    match &operand.kind {
                        verum_ast::expr::AsmOperandKind::In { expr, .. } => {
                            self.visit_expr(expr);
                        }
                        verum_ast::expr::AsmOperandKind::Out { place, .. } => {
                            self.visit_expr(place);
                        }
                        verum_ast::expr::AsmOperandKind::InOut { place, .. } => {
                            self.visit_expr(place);
                        }
                        verum_ast::expr::AsmOperandKind::InLateOut { in_expr, out_place, .. } => {
                            self.visit_expr(in_expr);
                            self.visit_expr(out_place);
                        }
                        verum_ast::expr::AsmOperandKind::Const { expr } => {
                            self.visit_expr(expr);
                        }
                        verum_ast::expr::AsmOperandKind::Sym { .. }
                        | verum_ast::expr::AsmOperandKind::Clobber { .. } => {}
                    }
                }
            }
            ExprKind::Literal(_)
            | ExprKind::Path(_)
            | ExprKind::Continue { label: _ }
            | ExprKind::TypeExpr(_)
            | ExprKind::TypeBound { .. }
            | ExprKind::MetaFunction { .. }
            | ExprKind::Typeof(_)
            | ExprKind::Quote { .. }
            | ExprKind::StageEscape { .. }
            | ExprKind::Lift { .. } => {}

            // Destructuring assignment: visit the value expression for call analysis
            ExprKind::DestructuringAssign { value, .. } => {
                self.visit_expr(value);
            }
            // Named arguments: visit the value expression
            ExprKind::NamedArg { value, .. } => {
                self.visit_expr(value);
            }
            ExprKind::CalcBlock(_) => {
                // Calc blocks are proof constructs - no boundary checking needed
            }
            ExprKind::CopatternBody { arms, .. } => {
                // Copattern body: visit each arm body for call-graph boundary analysis
                for arm in arms.iter() {
                    self.visit_expr(&arm.body);
                }
            }
        }
    }

    /// Visit a recover body (either match arms or closure)
    fn visit_recover_body(&mut self, recover: &verum_ast::expr::RecoverBody) {
        match recover {
            verum_ast::expr::RecoverBody::MatchArms { arms, .. } => {
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.visit_expr(guard);
                    }
                    self.visit_expr(&arm.body);
                }
            }
            verum_ast::expr::RecoverBody::Closure { body, .. } => {
                self.visit_expr(body);
            }
        }
    }

    /// Convert the builder into a call graph
    pub fn into_call_graph(mut self) -> CallGraph {
        // Detect all boundaries
        self.graph.detect_boundaries();
        self.graph
    }
}

impl Default for CallGraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract the verification level from function attributes
fn extract_verification_level(attrs: &List<Attribute>, func: &FunctionDecl) -> VerificationLevel {
    // Check item-level attributes first
    for attr in attrs {
        if attr.name.as_str() == "verify" {
            // attr.args is Option<Vec<Expr>> from parser
            if let Some(args) = &attr.args
                && let Some(first_arg) = args.first()
                && let ExprKind::Path(path) = &first_arg.kind
            {
                let level_str = path
                    .segments
                    .last()
                    .and_then(|s| match s {
                        verum_ast::PathSegment::Name(ident) => Some(ident.as_str()),
                        verum_ast::PathSegment::Relative => Some("."),
                        _ => None,
                    })
                    .unwrap_or("");
                if let Some(level) = VerificationLevel::from_annotation(level_str) {
                    return level;
                }
            }
        }
    }

    // Check function-level attributes
    for attr in &func.attributes {
        if attr.name.as_str() == "verify" {
            // attr.args is Option<Vec<Expr>> from parser
            if let Some(args) = &attr.args
                && let Some(first_arg) = args.first()
                && let ExprKind::Path(path) = &first_arg.kind
            {
                let level_str = path
                    .segments
                    .last()
                    .and_then(|s| match s {
                        verum_ast::PathSegment::Name(ident) => Some(ident.as_str()),
                        verum_ast::PathSegment::Relative => Some("."),
                        _ => None,
                    })
                    .unwrap_or("");
                if let Some(level) = VerificationLevel::from_annotation(level_str) {
                    return level;
                }
            }
        }
    }

    // Default to runtime
    VerificationLevel::Runtime
}

/// Check if an attribute with the given name exists
fn has_attribute(attrs: &List<Attribute>, name: &str) -> bool {
    attrs.iter().any(|a| a.name.as_str() == name)
}

/// Extract function name from a call expression
fn extract_function_name(expr: &Expr) -> Text {
    match &expr.kind {
        ExprKind::Path(path) => path
            .segments
            .iter()
            .filter_map(|s| match s {
                verum_ast::PathSegment::Name(ident) => Some(ident.as_str()),
                verum_ast::PathSegment::SelfValue => Some("self"),
                verum_ast::PathSegment::Super => Some("super"),
                verum_ast::PathSegment::Cog => Some("cog"),
                verum_ast::PathSegment::Relative => Some("."),
            })
            .collect::<List<_>>()
            .join("."),
        _ => Text::from(""),
    }
}

/// Proof obligation generator for boundaries
#[derive(Debug)]
pub struct ObligationGenerator {
    next_id: u64,
}

impl ObligationGenerator {
    /// Create a new obligation generator
    pub fn new() -> Self {
        Self { next_id: 0 }
    }

    /// Allocate a new obligation ID
    fn allocate_id(&mut self) -> ProofObligationId {
        let id = ProofObligationId::new(self.next_id);
        self.next_id += 1;
        id
    }

    /// Generate obligations for a boundary
    pub fn generate_obligations(&mut self, boundary: &mut DetectedBoundary) {
        boundary.required_obligations.clear();

        match boundary.direction {
            BoundaryDirection::MoreRestrictive => {
                // Moving to higher verification: need to prove callee's requirements
                self.generate_more_restrictive_obligations(boundary);
            }
            BoundaryDirection::LessRestrictive => {
                // Moving to lower verification: need runtime checks
                self.generate_less_restrictive_obligations(boundary);
            }
            BoundaryDirection::Same => {
                // No obligations needed
            }
        }
    }

    /// Generate obligations when moving to more restrictive verification
    fn generate_more_restrictive_obligations(&mut self, boundary: &mut DetectedBoundary) {
        // Precondition validation
        boundary.required_obligations.push(RequiredObligation::new(
            ObligationKind::Precondition,
            Text::from(format!(
                "Verify preconditions of '{}' are satisfied at call site in '{}'",
                boundary.callee_name, boundary.caller_name
            )),
        ));

        // If moving to Proof level, require formal proof
        if boundary.callee_level == VerificationLevel::Proof {
            boundary.required_obligations.push(RequiredObligation::new(
                ObligationKind::Custom,
                Text::from(format!(
                    "Provide formal proof for call from '{}' to '{}'",
                    boundary.caller_name, boundary.callee_name
                )),
            ));
        }

        // Memory safety obligation for Static/Proof
        if boundary.callee_level != VerificationLevel::Runtime {
            boundary.required_obligations.push(RequiredObligation::new(
                ObligationKind::MemorySafety,
                Text::from(format!(
                    "Verify memory safety at boundary from '{}' to '{}'",
                    boundary.caller_name, boundary.callee_name
                )),
            ));
        }
    }

    /// Generate obligations when moving to less restrictive verification
    fn generate_less_restrictive_obligations(&mut self, boundary: &mut DetectedBoundary) {
        // Runtime checks required
        boundary.required_obligations.push(RequiredObligation::new(
            ObligationKind::RefinementConstraint,
            Text::from(format!(
                "Insert runtime checks for arguments passed from '{}' to '{}'",
                boundary.caller_name, boundary.callee_name
            )),
        ));

        // Postcondition validation on return
        boundary.required_obligations.push(RequiredObligation::new(
            ObligationKind::Postcondition,
            Text::from(format!(
                "Validate postconditions of '{}' at return to '{}'",
                boundary.callee_name, boundary.caller_name
            )),
        ));
    }

    /// Generate all obligations for a call graph
    pub fn generate_all_obligations(&mut self, graph: &mut CallGraph) {
        // First detect boundaries if not already done
        let boundaries: List<_> = graph.boundaries.iter().cloned().collect();
        graph.boundaries.clear();

        for mut boundary in boundaries {
            self.generate_obligations(&mut boundary);
            graph.boundaries.push(boundary);
        }
    }
}

impl Default for ObligationGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Boundary diagnostic information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundaryDiagnostic {
    /// The boundary that triggered this diagnostic
    pub boundary: DetectedBoundary,

    /// Severity of the diagnostic
    pub severity: DiagnosticSeverity,

    /// Message describing the issue
    pub message: Text,

    /// Suggested fix, if any
    pub suggestion: Maybe<Text>,
}

/// Severity levels for diagnostics
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    /// Informational
    Info,
    /// Warning
    Warning,
    /// Error
    Error,
}

/// Generate diagnostics for detected boundaries
pub fn generate_boundary_diagnostics(graph: &CallGraph) -> List<BoundaryDiagnostic> {
    let mut diagnostics = List::new();

    for boundary in &graph.boundaries {
        match boundary.direction {
            BoundaryDirection::LessRestrictive => {
                // Warning: potential safety gap
                let diag = BoundaryDiagnostic {
                    boundary: boundary.clone(),
                    severity: DiagnosticSeverity::Warning,
                    message: Text::from(format!(
                        "Call from verified function '{}' ({:?}) to less verified '{}' ({:?}) may reduce safety guarantees",
                        boundary.caller_name,
                        boundary.caller_level,
                        boundary.callee_name,
                        boundary.callee_level
                    )),
                    suggestion: Maybe::Some(Text::from(format!(
                        "Consider upgrading '{}' to {:?} verification level",
                        boundary.callee_name, boundary.caller_level
                    ))),
                };
                diagnostics.push(diag);
            }
            BoundaryDirection::MoreRestrictive => {
                // Info: verification boundary detected
                let diag = BoundaryDiagnostic {
                    boundary: boundary.clone(),
                    severity: DiagnosticSeverity::Info,
                    message: Text::from(format!(
                        "Call from '{}' ({:?}) to '{}' ({:?}) crosses verification boundary",
                        boundary.caller_name,
                        boundary.caller_level,
                        boundary.callee_name,
                        boundary.callee_level
                    )),
                    suggestion: Maybe::None,
                };
                diagnostics.push(diag);
            }
            BoundaryDirection::Same => {}
        }

        // Check for unfulfilled obligations
        for obligation in &boundary.required_obligations {
            if !obligation.fulfilled {
                let diag = BoundaryDiagnostic {
                    boundary: boundary.clone(),
                    severity: DiagnosticSeverity::Error,
                    message: Text::from(format!(
                        "Unfulfilled proof obligation: {}",
                        obligation.description
                    )),
                    suggestion: Maybe::Some(Text::from(format!(
                        "Provide proof or add runtime check for: {:?}",
                        obligation.kind
                    ))),
                };
                diagnostics.push(diag);
            }
        }
    }

    diagnostics
}
