//! Execution State Sub-Context
//!
//! Manages variable bindings, call stack, and recursion tracking during
//! meta function execution.
//!
//! ## Responsibility
//!
//! - Variable bindings (name -> MetaValue)
//! - Call stack for debugging and recursion detection
//! - Recursion depth tracking
//! - Unique identifier generation
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

use verum_ast::{MetaValue, Span};
use verum_common::{List, Map, Maybe, Text};

/// Call frame for debugging and stack traces
#[derive(Debug, Clone)]
pub struct CallFrame {
    /// Function name being called
    pub function_name: Text,
    /// Span of the call site
    pub call_span: Span,
    /// Arguments passed to the function
    pub arguments: List<MetaValue>,
}

impl CallFrame {
    /// Create a new call frame
    pub fn new(function_name: Text, call_span: Span, arguments: List<MetaValue>) -> Self {
        Self {
            function_name,
            call_span,
            arguments,
        }
    }
}

/// Execution state for meta functions
///
/// Tracks variable bindings, call stack, and execution metadata during
/// compile-time evaluation.
#[derive(Debug, Clone)]
pub struct ExecutionState {
    /// Variable bindings (name -> value)
    bindings: Map<Text, MetaValue>,

    /// Current recursion depth
    recursion_depth: usize,

    /// Call stack for debugging
    call_stack: List<CallFrame>,

    /// Span of the current call site
    call_site_span: Span,

    /// Span of the macro/function definition site
    def_site_span: Span,

    /// Counter for generating unique identifiers
    unique_counter: u64,

    /// Current quote nesting depth
    quote_depth: u32,

    /// Trace markers for debugging staged execution
    trace_markers: List<MetaValue>,
}

impl Default for ExecutionState {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionState {
    /// Create a new empty execution state
    pub fn new() -> Self {
        Self {
            bindings: Map::new(),
            recursion_depth: 0,
            call_stack: List::new(),
            call_site_span: Span::dummy(),
            def_site_span: Span::dummy(),
            unique_counter: 0,
            quote_depth: 0,
            trace_markers: List::new(),
        }
    }

    // ======== Binding Operations ========

    /// Bind a variable to a value
    #[inline]
    pub fn bind(&mut self, name: Text, value: MetaValue) {
        self.bindings.insert(name, value);
    }

    /// Get a variable's value
    #[inline]
    pub fn get(&self, name: &Text) -> Option<MetaValue> {
        self.bindings.get(name).cloned()
    }

    /// Check if a variable is bound
    #[inline]
    pub fn has(&self, name: &Text) -> bool {
        self.bindings.contains_key(name)
    }

    /// Remove a binding
    #[inline]
    pub fn unbind(&mut self, name: &Text) -> Maybe<MetaValue> {
        self.bindings.remove(name).into()
    }

    /// Clear all bindings
    #[inline]
    pub fn clear_bindings(&mut self) {
        self.bindings.clear();
    }

    /// Get all binding names
    pub fn binding_names(&self) -> List<Text> {
        self.bindings.keys().cloned().collect()
    }

    /// Get all bindings as a map (for saving/restoring state)
    pub fn bindings(&self) -> &Map<Text, MetaValue> {
        &self.bindings
    }

    /// Get mutable access to bindings
    pub fn bindings_mut(&mut self) -> &mut Map<Text, MetaValue> {
        &mut self.bindings
    }

    // ======== Call Stack Operations ========

    /// Push a call frame onto the stack
    pub fn push_call(&mut self, frame: CallFrame) {
        self.call_stack.push(frame);
        self.recursion_depth += 1;
    }

    /// Pop a call frame from the stack
    pub fn pop_call(&mut self) -> Maybe<CallFrame> {
        if self.recursion_depth > 0 {
            self.recursion_depth -= 1;
        }
        self.call_stack.pop().into()
    }

    /// Get current recursion depth
    #[inline]
    pub fn recursion_depth(&self) -> usize {
        self.recursion_depth
    }

    /// Get the call stack
    pub fn call_stack(&self) -> &List<CallFrame> {
        &self.call_stack
    }

    /// Get the current function name (top of call stack)
    pub fn current_function(&self) -> Maybe<&Text> {
        self.call_stack.last().map(|f| &f.function_name).into()
    }

    // ======== Span Operations ========

    /// Get call site span
    #[inline]
    pub fn call_site_span(&self) -> Span {
        self.call_site_span
    }

    /// Set call site span
    #[inline]
    pub fn set_call_site_span(&mut self, span: Span) {
        self.call_site_span = span;
    }

    /// Get definition site span
    #[inline]
    pub fn def_site_span(&self) -> Span {
        self.def_site_span
    }

    /// Set definition site span
    #[inline]
    pub fn set_def_site_span(&mut self, span: Span) {
        self.def_site_span = span;
    }

    // ======== Unique ID Generation ========

    /// Generate a unique identifier
    #[inline]
    pub fn gen_unique_id(&mut self) -> u64 {
        let id = self.unique_counter;
        self.unique_counter += 1;
        id
    }

    /// Generate a unique identifier with prefix
    pub fn gen_unique_ident(&mut self, prefix: &str) -> Text {
        let id = self.gen_unique_id();
        Text::from(format!("{}_{}", prefix, id))
    }

    // ======== Quote Depth ========

    /// Get current quote nesting depth
    #[inline]
    pub fn quote_depth(&self) -> u32 {
        self.quote_depth
    }

    /// Increment quote depth
    #[inline]
    pub fn enter_quote(&mut self) {
        self.quote_depth += 1;
    }

    /// Decrement quote depth
    #[inline]
    pub fn exit_quote(&mut self) {
        if self.quote_depth > 0 {
            self.quote_depth -= 1;
        }
    }

    // ======== Trace Markers ========

    /// Add a trace marker for debugging
    pub fn add_trace_marker(&mut self, marker: MetaValue) {
        self.trace_markers.push(marker);
    }

    /// Get trace markers
    pub fn trace_markers(&self) -> &List<MetaValue> {
        &self.trace_markers
    }

    /// Clear trace markers
    pub fn clear_trace_markers(&mut self) {
        self.trace_markers.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binding_operations() {
        let mut state = ExecutionState::new();
        state.bind(Text::from("x"), MetaValue::Int(42));
        assert_eq!(state.get(&Text::from("x")), Some(MetaValue::Int(42)));
        assert!(state.has(&Text::from("x")));
        assert!(!state.has(&Text::from("y")));

        let removed = state.unbind(&Text::from("x"));
        assert_eq!(removed, Maybe::Some(MetaValue::Int(42)));
        assert!(!state.has(&Text::from("x")));
    }

    #[test]
    fn test_call_stack() {
        let mut state = ExecutionState::new();
        assert_eq!(state.recursion_depth(), 0);

        state.push_call(CallFrame::new(
            Text::from("foo"),
            Span::dummy(),
            List::new(),
        ));
        assert_eq!(state.recursion_depth(), 1);

        state.push_call(CallFrame::new(
            Text::from("bar"),
            Span::dummy(),
            List::new(),
        ));
        assert_eq!(state.recursion_depth(), 2);

        state.pop_call();
        assert_eq!(state.recursion_depth(), 1);
    }

    #[test]
    fn test_unique_id() {
        let mut state = ExecutionState::new();
        let id1 = state.gen_unique_id();
        let id2 = state.gen_unique_id();
        assert_ne!(id1, id2);
    }
}
