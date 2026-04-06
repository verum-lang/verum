//! Call Graph Construction and Analysis
//!
//! Builds interprocedural analysis data structures for escape analysis.
//! Interprocedural escape analysis requires tracking reference flow across function
//! boundaries. A reference escapes if passed to a function that may retain it (store
//! in heap, return to its caller, or capture in a closure). The call graph enables
//! whole-program analysis: can_promote_to_checked() checks (1) no escape, (2) exclusive
//! access, (3) lifetime dominance, and (4) no conflicting mutations across all callees.
//!
//! This module provides:
//! - Call graph data structures for interprocedural analysis
//! - Reference flow analysis between caller and callee
//! - Known safe function tracking
//! - Thread spawn detection for escape analysis
//!
//! Note: AST processing methods are provided separately in `verum_parser` to avoid
//! circular dependencies. Use `CallGraphBuilder`'s programmatic API to build call
//! graphs manually, or use the parser's integration module.

use crate::analysis::{BlockId, FunctionId, RefId};
use std::sync::atomic::{AtomicU64, Ordering};
use verum_common::{List, Map, Maybe, Set, Text};

/// Counter for generating unique function IDs
static FUNCTION_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a new unique function ID
pub fn new_function_id() -> FunctionId {
    FunctionId(FUNCTION_ID_COUNTER.fetch_add(1, Ordering::SeqCst))
}

/// Reference flow information between caller and callee
///
/// Tracks how references flow through function calls to determine
/// if escape analysis can safely promote references.
///
/// Formal escape analysis tracks per-parameter escape status: a parameter escapes if
/// the callee returns it, stores it in heap, captures it in a closure, or passes it
/// to another function that may retain it. This is the core data structure for
/// interprocedural promotion decisions (can_promote_to_checked).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefFlow {
    /// Which parameters may escape (by index)
    pub parameter_escapes: List<bool>,
    /// Whether the return value may contain escaped reference
    pub return_escapes: bool,
    /// Whether function may store reference to heap
    pub may_store_heap: bool,
    /// Whether function may spawn threads with reference
    pub may_spawn_thread: bool,
}

impl RefFlow {
    /// Conservative: assume everything escapes
    ///
    /// Used when analyzing unknown or external functions where we cannot
    /// determine the actual behavior statically.
    #[must_use]
    pub fn conservative(param_count: usize) -> Self {
        Self {
            parameter_escapes: vec![true; param_count].into(),
            return_escapes: true,
            may_store_heap: true,
            may_spawn_thread: true,
        }
    }

    /// Safe: nothing escapes (for known safe functions)
    ///
    /// Used for functions known to not retain references, such as
    /// pure functions and standard library accessors.
    #[must_use]
    pub fn safe(param_count: usize) -> Self {
        Self {
            parameter_escapes: vec![false; param_count].into(),
            return_escapes: false,
            may_store_heap: false,
            may_spawn_thread: false,
        }
    }

    /// Create a new `RefFlow` with specific parameter escape behavior
    #[must_use]
    pub fn with_params(param_escapes: List<bool>) -> Self {
        Self {
            parameter_escapes: param_escapes,
            return_escapes: false,
            may_store_heap: false,
            may_spawn_thread: false,
        }
    }

    /// Check if any parameter escapes
    #[must_use]
    pub fn any_param_escapes(&self) -> bool {
        self.parameter_escapes.iter().any(|&e| e)
    }

    /// Check if a specific parameter escapes
    #[must_use]
    pub fn param_escapes(&self, idx: usize) -> bool {
        self.parameter_escapes.get(idx).copied().unwrap_or(true)
    }

    /// Merge two `RefFlows` (union - conservative)
    ///
    /// Used when a function has multiple call sites and we need to
    /// track the combined escape behavior.
    #[must_use]
    pub fn merge(&self, other: &RefFlow) -> Self {
        let max_len = self
            .parameter_escapes
            .len()
            .max(other.parameter_escapes.len());
        let mut merged_params = vec![false; max_len];

        for (i, &escape) in self.parameter_escapes.iter().enumerate() {
            merged_params[i] = escape;
        }
        for (i, &escape) in other.parameter_escapes.iter().enumerate() {
            merged_params[i] = merged_params[i] || escape;
        }

        Self {
            parameter_escapes: merged_params.into(),
            return_escapes: self.return_escapes || other.return_escapes,
            may_store_heap: self.may_store_heap || other.may_store_heap,
            may_spawn_thread: self.may_spawn_thread || other.may_spawn_thread,
        }
    }
}

impl Default for RefFlow {
    fn default() -> Self {
        Self::safe(0)
    }
}

/// Function signature information for call graph analysis
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSignature {
    /// Fully qualified function name
    pub name: Text,
    /// Number of parameters
    pub param_count: usize,
    /// Whether the function is pure (no side effects)
    pub is_pure: bool,
    /// Whether this function spawns threads
    pub is_thread_spawn: bool,
    /// Whether this function is async
    pub is_async: bool,
    /// Whether this function is a known safe function
    pub is_safe: bool,
}

impl FunctionSignature {
    /// Create a new function signature
    pub fn new(name: impl Into<Text>, param_count: usize) -> Self {
        Self {
            name: name.into(),
            param_count,
            is_pure: false,
            is_thread_spawn: false,
            is_async: false,
            is_safe: false,
        }
    }

    /// Create a pure function signature (no side effects)
    pub fn pure(name: impl Into<Text>, param_count: usize) -> Self {
        Self {
            name: name.into(),
            param_count,
            is_pure: true,
            is_thread_spawn: false,
            is_async: false,
            is_safe: true,
        }
    }

    /// Create a thread-spawning function signature
    pub fn thread_spawn(name: impl Into<Text>, param_count: usize) -> Self {
        Self {
            name: name.into(),
            param_count,
            is_pure: false,
            is_thread_spawn: true,
            is_async: false,
            is_safe: false,
        }
    }

    /// Mark as safe (doesn't retain references)
    #[must_use]
    pub fn with_safe(mut self) -> Self {
        self.is_safe = true;
        self
    }

    /// Mark as async
    #[must_use]
    pub fn with_async(mut self) -> Self {
        self.is_async = true;
        self
    }
}

/// Call site information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSite {
    /// The block containing this call
    pub block: BlockId,
    /// Arguments passed to the call (as `RefIds` if they are references)
    pub ref_args: List<Maybe<RefId>>,
    /// Whether the result is stored
    pub result_stored: bool,
    /// Whether the result is returned from caller
    pub result_returned: bool,
}

impl CallSite {
    /// Create a new call site
    #[must_use]
    pub fn new(block: BlockId) -> Self {
        Self {
            block,
            ref_args: List::new(),
            result_stored: false,
            result_returned: false,
        }
    }

    /// Add a reference argument
    #[must_use]
    pub fn with_ref_arg(mut self, ref_id: Maybe<RefId>) -> Self {
        self.ref_args.push(ref_id);
        self
    }

    /// Mark result as stored
    #[must_use]
    pub fn with_stored(mut self) -> Self {
        self.result_stored = true;
        self
    }

    /// Mark result as returned
    #[must_use]
    pub fn with_returned(mut self) -> Self {
        self.result_returned = true;
        self
    }
}

/// Call edge with metadata
#[derive(Debug, Clone)]
pub struct CallEdge {
    /// Caller function
    pub caller: FunctionId,
    /// Callee function
    pub callee: FunctionId,
    /// Reference flow for this edge
    pub flow: RefFlow,
    /// Call sites where this edge occurs
    pub sites: List<CallSite>,
}

impl CallEdge {
    /// Create a new call edge
    #[must_use]
    pub fn new(caller: FunctionId, callee: FunctionId, flow: RefFlow) -> Self {
        Self {
            caller,
            callee,
            flow,
            sites: List::new(),
        }
    }

    /// Add a call site
    pub fn add_site(&mut self, site: CallSite) {
        self.sites.push(site);
    }
}

/// Call graph for interprocedural analysis
///
/// Provides the foundation for escape analysis by tracking how
/// functions call each other and how references flow between them.
///
/// Enables whole-program escape analysis: for each reference, we traverse the call
/// graph to determine if any callee in the transitive closure may retain it. Known-safe
/// functions (pure functions, standard library functions that don't store references)
/// are tracked separately to avoid conservative over-approximation. Thread spawn
/// detection marks references that cross thread boundaries as ThreadEscape.
#[derive(Debug)]
pub struct CallGraph {
    /// Direct calls: caller -> set of callees
    pub calls: Map<FunctionId, Set<FunctionId>>,
    /// Inverse: callee -> set of callers
    pub callers: Map<FunctionId, Set<FunctionId>>,
    /// Reference flow information for each edge
    pub flows: Map<(FunctionId, FunctionId), RefFlow>,
    /// Known safe functions (stdlib, builtins)
    pub safe_functions: Set<Text>,
    /// Function signatures (for parameter counting)
    pub signatures: Map<FunctionId, FunctionSignature>,
    /// Thread-spawning function names
    pub thread_spawn_functions: Set<Text>,
    /// Function name to ID mapping
    pub name_to_id: Map<Text, FunctionId>,
    /// Detailed call edges
    pub edges: List<CallEdge>,
}

impl CallGraph {
    /// Create a new empty call graph
    #[must_use]
    pub fn new() -> Self {
        let mut graph = Self {
            calls: Map::new(),
            callers: Map::new(),
            flows: Map::new(),
            safe_functions: Set::new(),
            signatures: Map::new(),
            thread_spawn_functions: Set::new(),
            name_to_id: Map::new(),
            edges: List::new(),
        };

        // Register known thread-spawning functions
        graph.register_thread_spawn_function("std.thread.spawn");
        graph.register_thread_spawn_function("std.thread.spawn_blocking");
        graph.register_thread_spawn_function("std.async.spawn");
        graph.register_thread_spawn_function("tokio.spawn");
        graph.register_thread_spawn_function("Thread.spawn");
        graph.register_thread_spawn_function("spawn");

        graph
    }

    /// Check if function may retain a reference passed as parameter
    ///
    /// This is the key query for escape analysis. Returns true if the
    /// function may keep the reference alive after returning.
    ///
    /// Checks whether a callee may store, return, or otherwise retain a reference
    /// passed at `param_idx`. If the function is in the known-safe set, returns false.
    /// Otherwise checks RefFlow data for per-parameter escape status. If no flow
    /// information is available, conservatively returns true (reference may escape).
    #[must_use]
    pub fn may_retain(&self, func: FunctionId, param_idx: usize) -> bool {
        // Check if function is known safe
        if let Maybe::Some(sig) = self.signatures.get(&func) {
            if self.safe_functions.contains(&sig.name) {
                return false;
            }
            if sig.is_safe || sig.is_pure {
                return false;
            }
        }

        // Check stored flow information
        for ((caller, callee), flow) in &self.flows {
            if *callee == func {
                if flow.param_escapes(param_idx) {
                    return true;
                }
                // Recursively check if caller might retain
                if flow.return_escapes && self.may_retain(*caller, 0) {
                    return true;
                }
            }
        }

        // If we have explicit flow info for this function, use it
        // If unknown, return true (conservative)
        true
    }

    /// Check if function may spawn threads
    ///
    /// Used by escape analysis to determine if references might
    /// escape to another thread.
    #[must_use]
    pub fn may_spawn_thread(&self, func: FunctionId) -> bool {
        if let Maybe::Some(sig) = self.signatures.get(&func) {
            if sig.is_thread_spawn {
                return true;
            }
            if self.thread_spawn_functions.contains(&sig.name) {
                return true;
            }
        }

        // Check if any callee may spawn threads
        if let Maybe::Some(callees) = self.calls.get(&func) {
            for &callee in callees {
                if self.may_spawn_thread(callee) {
                    return true;
                }
            }
        }

        false
    }

    /// Check if function may spawn threads with a specific reference
    #[must_use]
    pub fn may_spawn_thread_with_ref(&self, func: FunctionId, param_idx: usize) -> bool {
        if !self.may_spawn_thread(func) {
            return false;
        }

        // Check if the parameter flows to a thread spawn
        for ((_, callee), flow) in &self.flows {
            if *callee == func && flow.may_spawn_thread && flow.param_escapes(param_idx) {
                return true;
            }
        }

        // Conservative: if function spawns threads, assume parameter might be used
        true
    }

    /// Register a known safe function
    ///
    /// Safe functions are known to not retain references to their parameters
    /// beyond the duration of the call.
    pub fn register_safe_function(&mut self, name: impl Into<Text>) {
        self.safe_functions.insert(name.into());
    }

    /// Register a thread-spawning function
    pub fn register_thread_spawn_function(&mut self, name: impl Into<Text>) {
        self.thread_spawn_functions.insert(name.into());
    }

    /// Add a call edge with reference flow
    pub fn add_call(&mut self, caller: FunctionId, callee: FunctionId, flow: RefFlow) {
        // Add to calls map
        self.calls.entry(caller).or_default().insert(callee);

        // Add to callers map (inverse)
        self.callers.entry(callee).or_default().insert(caller);

        // Merge flow information
        let key = (caller, callee);
        if let Maybe::Some(existing) = self.flows.get(&key) {
            let merged = existing.merge(&flow);
            self.flows.insert(key, merged);
        } else {
            self.flows.insert(key, flow.clone());
        }

        // Add edge
        self.edges.push(CallEdge::new(caller, callee, flow));
    }

    /// Add a function signature
    pub fn add_function(&mut self, id: FunctionId, signature: FunctionSignature) {
        self.name_to_id.insert(signature.name.clone(), id);
        self.signatures.insert(id, signature);
    }

    /// Get function ID by name
    #[must_use]
    pub fn get_function_id(&self, name: &str) -> Maybe<FunctionId> {
        let name_text: Text = name.to_string().into();
        self.name_to_id.get(&name_text).copied()
    }

    /// Get all functions that a function calls
    #[must_use]
    pub fn callees(&self, func: FunctionId) -> Maybe<&Set<FunctionId>> {
        self.calls.get(&func)
    }

    /// Get all functions that call a function
    #[must_use]
    pub fn callers_of(&self, func: FunctionId) -> Maybe<&Set<FunctionId>> {
        self.callers.get(&func)
    }

    /// Get reference flow between two functions
    #[must_use]
    pub fn get_flow(&self, caller: FunctionId, callee: FunctionId) -> Maybe<&RefFlow> {
        self.flows.get(&(caller, callee))
    }

    /// Check if a function is known safe
    #[must_use]
    pub fn is_safe_function(&self, func: FunctionId) -> bool {
        if let Maybe::Some(sig) = self.signatures.get(&func) {
            self.safe_functions.contains(&sig.name) || sig.is_safe
        } else {
            false
        }
    }

    /// Check if a function is pure (no side effects)
    #[must_use]
    pub fn is_pure(&self, func: FunctionId) -> bool {
        if let Maybe::Some(sig) = self.signatures.get(&func) {
            sig.is_pure
        } else {
            false
        }
    }

    /// Compute transitive closure of calls
    ///
    /// Returns all functions reachable from the given function.
    #[must_use]
    pub fn reachable_from(&self, func: FunctionId) -> Set<FunctionId> {
        let mut reachable = Set::new();
        let mut worklist = vec![func];

        while let Some(current) = worklist.pop() {
            if reachable.contains(&current) {
                continue;
            }
            reachable.insert(current);

            if let Maybe::Some(callees) = self.calls.get(&current) {
                for &callee in callees {
                    if !reachable.contains(&callee) {
                        worklist.push(callee);
                    }
                }
            }
        }

        reachable
    }

    /// Compute strongly connected components (for recursive call analysis)
    ///
    /// Uses Tarjan's algorithm to find cycles in the call graph.
    #[must_use]
    pub fn compute_sccs(&self) -> List<Set<FunctionId>> {
        let mut sccs = List::new();
        let mut index_counter = 0u64;
        let mut stack: List<FunctionId> = List::new();
        let mut on_stack: Set<FunctionId> = Set::new();
        let mut indices: Map<FunctionId, u64> = Map::new();
        let mut lowlinks: Map<FunctionId, u64> = Map::new();

        fn strongconnect(
            v: FunctionId,
            graph: &CallGraph,
            index_counter: &mut u64,
            stack: &mut List<FunctionId>,
            on_stack: &mut Set<FunctionId>,
            indices: &mut Map<FunctionId, u64>,
            lowlinks: &mut Map<FunctionId, u64>,
            sccs: &mut List<Set<FunctionId>>,
        ) {
            indices.insert(v, *index_counter);
            lowlinks.insert(v, *index_counter);
            *index_counter += 1;
            stack.push(v);
            on_stack.insert(v);

            if let Maybe::Some(successors) = graph.calls.get(&v) {
                for &w in successors {
                    if indices.get(&w).is_none() {
                        strongconnect(
                            w,
                            graph,
                            index_counter,
                            stack,
                            on_stack,
                            indices,
                            lowlinks,
                            sccs,
                        );
                        let w_lowlink = *lowlinks.get(&w).unwrap_or(&u64::MAX);
                        let v_lowlink = *lowlinks.get(&v).unwrap_or(&u64::MAX);
                        lowlinks.insert(v, v_lowlink.min(w_lowlink));
                    } else if on_stack.contains(&w) {
                        let w_index = *indices.get(&w).unwrap_or(&u64::MAX);
                        let v_lowlink = *lowlinks.get(&v).unwrap_or(&u64::MAX);
                        lowlinks.insert(v, v_lowlink.min(w_index));
                    }
                }
            }

            let v_index = indices.get(&v).copied();
            let v_lowlink = lowlinks.get(&v).copied();
            if v_index == v_lowlink {
                let mut scc = Set::new();
                loop {
                    if let Some(w) = stack.pop() {
                        on_stack.remove(&w);
                        scc.insert(w);
                        if w == v {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                if !scc.is_empty() {
                    sccs.push(scc);
                }
            }
        }

        for &func in self.signatures.keys() {
            if indices.get(&func).is_none() {
                strongconnect(
                    func,
                    self,
                    &mut index_counter,
                    &mut stack,
                    &mut on_stack,
                    &mut indices,
                    &mut lowlinks,
                    &mut sccs,
                );
            }
        }

        sccs
    }

    /// Check if function is recursive (directly or indirectly)
    #[must_use]
    pub fn is_recursive(&self, func: FunctionId) -> bool {
        let sccs = self.compute_sccs();
        for scc in &sccs {
            if scc.contains(&func) && scc.len() > 1 {
                return true;
            }
            // Check for direct self-recursion
            if scc.contains(&func)
                && let Maybe::Some(callees) = self.calls.get(&func)
                && callees.contains(&func)
            {
                return true;
            }
        }
        false
    }

    /// Get total number of functions
    #[must_use]
    pub fn function_count(&self) -> usize {
        self.signatures.len()
    }

    /// Get total number of call edges
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.flows.len()
    }
}

impl Default for CallGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Call graph builder
///
/// Provides an incremental API for building call graphs from AST
/// or other sources.
pub struct CallGraphBuilder {
    graph: CallGraph,
    current_function: Maybe<FunctionId>,
}

impl CallGraphBuilder {
    /// Create a new call graph builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            graph: CallGraph::new(),
            current_function: Maybe::None,
        }
    }

    /// Register standard library safe functions
    ///
    /// These functions are known to not retain references to their parameters.
    pub fn register_stdlib_safe_functions(&mut self) {
        // List iteration and access (read-only, don't retain)
        self.graph.register_safe_function("List::iter");
        self.graph.register_safe_function("List::len");
        self.graph.register_safe_function("List::is_empty");
        self.graph.register_safe_function("List::get");
        self.graph.register_safe_function("List::first");
        self.graph.register_safe_function("List::last");
        self.graph.register_safe_function("List::contains");

        // Map access (read-only)
        self.graph.register_safe_function("Map::get");
        self.graph.register_safe_function("Map::len");
        self.graph.register_safe_function("Map::is_empty");
        self.graph.register_safe_function("Map::contains_key");
        self.graph.register_safe_function("Map::keys");
        self.graph.register_safe_function("Map::values");

        // Set access (read-only)
        self.graph.register_safe_function("Set::contains");
        self.graph.register_safe_function("Set::len");
        self.graph.register_safe_function("Set::is_empty");

        // Text operations (don't retain)
        self.graph.register_safe_function("Text::len");
        self.graph.register_safe_function("Text::is_empty");
        self.graph.register_safe_function("Text::chars");
        self.graph.register_safe_function("Text::bytes");
        self.graph.register_safe_function("Text::contains");
        self.graph.register_safe_function("Text::starts_with");
        self.graph.register_safe_function("Text::ends_with");

        // Pure functions
        self.graph.register_safe_function("clone");
        self.graph.register_safe_function("copy");
        self.graph.register_safe_function("eq");
        self.graph.register_safe_function("cmp");
        self.graph.register_safe_function("hash");
        self.graph.register_safe_function("fmt");
        self.graph.register_safe_function("debug");
        self.graph.register_safe_function("display");

        // Math functions (pure)
        self.graph.register_safe_function("abs");
        self.graph.register_safe_function("min");
        self.graph.register_safe_function("max");
        self.graph.register_safe_function("sqrt");
        self.graph.register_safe_function("pow");
        self.graph.register_safe_function("sin");
        self.graph.register_safe_function("cos");
        self.graph.register_safe_function("tan");

        // Type conversions (consume input, don't retain)
        self.graph.register_safe_function("into");
        self.graph.register_safe_function("from");
        self.graph.register_safe_function("try_into");
        self.graph.register_safe_function("try_from");
        self.graph.register_safe_function("as_ref");
        self.graph.register_safe_function("as_mut");

        // Iterator adaptors (don't retain beyond iteration)
        self.graph.register_safe_function("map");
        self.graph.register_safe_function("filter");
        self.graph.register_safe_function("fold");
        self.graph.register_safe_function("reduce");
        self.graph.register_safe_function("take");
        self.graph.register_safe_function("skip");
        self.graph.register_safe_function("enumerate");
        self.graph.register_safe_function("zip");

        // Option/Maybe operations
        self.graph.register_safe_function("unwrap");
        self.graph.register_safe_function("unwrap_or");
        self.graph.register_safe_function("unwrap_or_else");
        self.graph.register_safe_function("is_some");
        self.graph.register_safe_function("is_none");
        self.graph.register_safe_function("ok_or");

        // Result operations
        self.graph.register_safe_function("is_ok");
        self.graph.register_safe_function("is_err");
        self.graph.register_safe_function("ok");
        self.graph.register_safe_function("err");
        self.graph.register_safe_function("expect");
    }

    /// Begin processing a function
    pub fn begin_function(&mut self, id: FunctionId, signature: FunctionSignature) {
        self.graph.add_function(id, signature);
        self.current_function = Maybe::Some(id);
    }

    /// End processing the current function
    pub fn end_function(&mut self) {
        self.current_function = Maybe::None;
    }

    /// Record a call from the current function to another
    pub fn record_call(&mut self, callee: FunctionId, flow: RefFlow) {
        if let Maybe::Some(caller) = self.current_function {
            self.graph.add_call(caller, callee, flow);
        }
    }

    /// Record a call by name (will be resolved later)
    pub fn record_call_by_name(&mut self, callee_name: &str, param_count: usize) {
        if let Maybe::Some(caller) = self.current_function {
            let callee_name_text: Text = callee_name.to_string().into();
            // Check if callee is already known
            if let Maybe::Some(callee_id) = self.graph.get_function_id(callee_name) {
                let flow = if self.graph.safe_functions.contains(&callee_name_text) {
                    RefFlow::safe(param_count)
                } else {
                    RefFlow::conservative(param_count)
                };
                self.graph.add_call(caller, callee_id, flow);
            } else {
                // Create a placeholder function
                let callee_id = new_function_id();
                let is_thread_spawn = self.graph.thread_spawn_functions.contains(&callee_name_text);
                let signature = FunctionSignature {
                    name: callee_name_text.clone(),
                    param_count,
                    is_pure: false,
                    is_thread_spawn,
                    is_async: false,
                    is_safe: self.graph.safe_functions.contains(&callee_name_text),
                };
                self.graph.add_function(callee_id, signature);

                let flow = if self.graph.safe_functions.contains(&callee_name_text) {
                    RefFlow::safe(param_count)
                } else if is_thread_spawn {
                    RefFlow {
                        parameter_escapes: vec![true; param_count].into(),
                        return_escapes: true,
                        may_store_heap: true,
                        may_spawn_thread: true,
                    }
                } else {
                    RefFlow::conservative(param_count)
                };
                self.graph.add_call(caller, callee_id, flow);
            }
        }
    }

    /// Finalize and return the call graph
    #[must_use]
    pub fn build(self) -> CallGraph {
        self.graph
    }
}

impl Default for CallGraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Analysis result for interprocedural escape analysis
#[derive(Debug, Clone)]
pub struct InterproceduralEscapeResult {
    /// Function being analyzed
    pub function: FunctionId,
    /// Which parameters may escape
    pub escaping_params: Set<usize>,
    /// Whether return value may contain escaping references
    pub return_escapes: bool,
    /// Functions that may be called with escaping references
    pub escape_callees: Set<FunctionId>,
}

impl InterproceduralEscapeResult {
    /// Create a new result
    #[must_use]
    pub fn new(function: FunctionId) -> Self {
        Self {
            function,
            escaping_params: Set::new(),
            return_escapes: false,
            escape_callees: Set::new(),
        }
    }

    /// Check if any parameter escapes
    #[must_use]
    pub fn has_escaping_params(&self) -> bool {
        !self.escaping_params.is_empty()
    }

    /// Mark a parameter as escaping
    pub fn mark_param_escapes(&mut self, param_idx: usize) {
        self.escaping_params.insert(param_idx);
    }
}

/// Perform interprocedural escape analysis using the call graph
#[must_use]
pub fn analyze_interprocedural_escapes(
    graph: &CallGraph,
    function: FunctionId,
) -> InterproceduralEscapeResult {
    let mut result = InterproceduralEscapeResult::new(function);

    // Get all functions reachable from this function
    let reachable = graph.reachable_from(function);

    // For each callee, check if parameters might escape
    if let Maybe::Some(callees) = graph.callees(function) {
        for &callee in callees {
            if let Maybe::Some(flow) = graph.get_flow(function, callee) {
                // Check each parameter
                for (idx, &escapes) in flow.parameter_escapes.iter().enumerate() {
                    if escapes {
                        result.mark_param_escapes(idx);
                        result.escape_callees.insert(callee);
                    }
                }

                // Check return escape
                if flow.return_escapes {
                    result.return_escapes = true;
                }
            }

            // Check if callee spawns threads
            if graph.may_spawn_thread(callee) {
                result.escape_callees.insert(callee);
            }
        }
    }

    // Check for recursive escapes through SCCs
    let sccs = graph.compute_sccs();
    for scc in &sccs {
        if scc.contains(&function) && scc.len() > 1 {
            // Function is in a recursive cycle - conservatively mark all params as escaping
            if let Maybe::Some(sig) = graph.signatures.get(&function) {
                for i in 0..sig.param_count {
                    result.mark_param_escapes(i);
                }
            }
            result.return_escapes = true;
        }
    }

    // Check transitive thread spawning
    for &reachable_func in &reachable {
        if graph.may_spawn_thread(reachable_func) {
            result.escape_callees.insert(reachable_func);
        }
    }

    result
}
