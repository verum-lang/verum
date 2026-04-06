//! Execution context for cross-cell state preservation.
//!
//! The `ExecutionContext` maintains variable bindings and function definitions
//! across cell executions, enabling a notebook-like experience where later cells
//! can reference values defined in earlier cells.

use std::collections::HashMap;

use verum_common::Text;
use verum_vbc::module::FunctionId;
use verum_vbc::value::Value;
use verum_vbc::interpreter::InterpreterState;

use super::pipeline::CompiledCell;
use crate::CellId;

/// Information about a variable binding.
#[derive(Debug, Clone)]
pub struct BindingInfo {
    /// The variable name.
    pub name: Text,
    /// The current value.
    pub value: Value,
    /// Type information (as string for display).
    pub type_info: Text,
    /// The cell ID where this binding was defined.
    pub defined_in: CellId,
    /// Whether this binding is mutable.
    pub is_mutable: bool,
}

/// Information about a function defined in the session.
#[derive(Debug, Clone)]
pub struct FunctionInfo {
    /// The function name.
    pub name: Text,
    /// The function ID in the VBC module.
    pub func_id: FunctionId,
    /// The cell ID where this function was defined.
    pub defined_in: CellId,
    /// Parameter names and types (for display).
    pub params: Vec<(Text, Text)>,
    /// Return type (for display).
    pub return_type: Text,
}

/// Dependency graph for cells.
///
/// Tracks which cells depend on which bindings, enabling smart re-execution
/// when a binding changes.
#[derive(Debug, Default, Clone)]
pub struct DependencyGraph {
    /// Maps binding name to the cells that use it.
    uses: HashMap<Text, Vec<CellId>>,
    /// Maps cell ID to the bindings it defines.
    defines: HashMap<CellId, Vec<Text>>,
}

impl DependencyGraph {
    /// Creates a new empty dependency graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that a cell uses a binding.
    pub fn add_use(&mut self, binding: Text, cell_id: CellId) {
        self.uses.entry(binding).or_default().push(cell_id);
    }

    /// Records that a cell defines a binding.
    pub fn add_definition(&mut self, cell_id: CellId, binding: Text) {
        self.defines.entry(cell_id).or_default().push(binding);
    }

    /// Returns all cells that depend on a given binding.
    pub fn dependents(&self, binding: &Text) -> &[CellId] {
        self.uses.get(binding).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Clears all records for a cell (when it's re-executed).
    pub fn clear_cell(&mut self, cell_id: CellId) {
        // Remove from all use lists
        for uses in self.uses.values_mut() {
            uses.retain(|id| *id != cell_id);
        }
        // Remove definitions
        self.defines.remove(&cell_id);
    }
}

/// Execution context for a playground session.
///
/// Maintains state across cell executions including:
/// - Variable bindings (name → value + metadata)
/// - Function definitions
/// - Dependency tracking
/// - Context values (for `using [...]` clauses)
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    /// Variable bindings: name → (value, type, cell_id).
    pub bindings: HashMap<Text, BindingInfo>,

    /// Function definitions available in the session.
    pub functions: HashMap<Text, FunctionInfo>,

    /// Cell dependency graph.
    pub dependencies: DependencyGraph,

    /// Active context types (for `using [...]` clauses).
    pub active_contexts: Vec<ContextEntry>,

    /// Counter for generating unique IDs.
    next_id: u64,
}

/// An active context entry for dependency injection.
#[derive(Debug, Clone)]
pub struct ContextEntry {
    /// Context type name.
    pub type_name: Text,
    /// Context type ID (for VBC).
    pub type_id: u32,
    /// The context value.
    pub value: Value,
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionContext {
    /// Creates a new empty execution context.
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
            functions: HashMap::new(),
            dependencies: DependencyGraph::new(),
            active_contexts: Vec::new(),
            next_id: 0,
        }
    }

    /// Generates a unique ID.
    pub fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Adds or updates a variable binding.
    pub fn set_binding(&mut self, info: BindingInfo) {
        self.bindings.insert(info.name.clone(), info);
    }

    /// Gets a binding by name.
    pub fn get_binding(&self, name: &str) -> Option<&BindingInfo> {
        self.bindings.get(&Text::from(name))
    }

    /// Gets a mutable binding by name.
    pub fn get_binding_mut(&mut self, name: &str) -> Option<&mut BindingInfo> {
        self.bindings.get_mut(&Text::from(name))
    }

    /// Removes a binding.
    pub fn remove_binding(&mut self, name: &str) -> Option<BindingInfo> {
        self.bindings.remove(&Text::from(name))
    }

    /// Adds or updates a function definition.
    pub fn set_function(&mut self, info: FunctionInfo) {
        self.functions.insert(info.name.clone(), info);
    }

    /// Gets a function by name.
    pub fn get_function(&self, name: &str) -> Option<&FunctionInfo> {
        self.functions.get(&Text::from(name))
    }

    /// Returns all binding names.
    pub fn binding_names(&self) -> impl Iterator<Item = &Text> {
        self.bindings.keys()
    }

    /// Returns all function names.
    pub fn function_names(&self) -> impl Iterator<Item = &Text> {
        self.functions.keys()
    }

    /// Injects bindings into the interpreter state before execution.
    ///
    /// This transfers the current bindings from the execution context into
    /// the VBC interpreter's global state, making them available for the
    /// next cell execution.
    pub fn inject_bindings(&self, state: &mut InterpreterState) {
        // For now, we use a simple approach: store bindings in TLS slots
        // In a full implementation, we would inject them as global variables
        // or use a dedicated global variable table in the VBC module.

        // Each binding gets a TLS slot indexed by a hash of the name
        for (name, binding) in &self.bindings {
            let slot = hash_name(name.as_str());
            state.tls_set(slot, binding.value);
        }
    }

    /// Extracts bindings from the interpreter state after execution.
    ///
    /// This captures any new or modified bindings from the executed cell
    /// and stores them in the execution context for future cells.
    pub fn extract_bindings(
        &mut self,
        compiled: &CompiledCell,
        state: &InterpreterState,
        cell_id: CellId,
    ) {
        // Extract new bindings that were defined by this cell
        for (name, _type_info) in &compiled.new_bindings {
            let slot = hash_name(name.as_str());
            if let Some(value) = state.tls_get(slot) {
                self.set_binding(BindingInfo {
                    name: name.clone(),
                    value,
                    type_info: Text::from("<inferred>"),
                    defined_in: cell_id,
                    is_mutable: false,
                });

                // Track dependency
                self.dependencies.add_definition(cell_id, name.clone());
            }
        }
    }

    /// Clears all bindings defined by a specific cell.
    ///
    /// Used when a cell is re-executed to remove stale bindings.
    pub fn clear_cell_bindings(&mut self, cell_id: CellId) {
        // Remove bindings defined by this cell
        self.bindings.retain(|_, info| info.defined_in != cell_id);
        self.functions.retain(|_, info| info.defined_in != cell_id);
        self.dependencies.clear_cell(cell_id);
    }

    /// Provides a context value for dependency injection.
    pub fn provide_context(&mut self, type_name: Text, type_id: u32, value: Value) {
        // Remove existing context of this type
        self.active_contexts.retain(|c| c.type_id != type_id);

        self.active_contexts.push(ContextEntry {
            type_name,
            type_id,
            value,
        });
    }

    /// Gets a context value by type ID.
    pub fn get_context(&self, type_id: u32) -> Option<&ContextEntry> {
        self.active_contexts.iter().find(|c| c.type_id == type_id)
    }

    /// Injects contexts into interpreter state.
    pub fn inject_contexts(&self, state: &mut InterpreterState) {
        for context in &self.active_contexts {
            let depth = state.call_stack.depth();
            state.context_stack.provide(context.type_id, context.value, depth);
        }
    }

    /// Resets the execution context to empty state.
    pub fn reset(&mut self) {
        self.bindings.clear();
        self.functions.clear();
        self.dependencies = DependencyGraph::new();
        self.active_contexts.clear();
        self.next_id = 0;
    }
}

/// Simple hash function for binding name to slot mapping.
fn hash_name(name: &str) -> usize {
    let mut hash = 0usize;
    for byte in name.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(byte as usize);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binding_operations() {
        let mut ctx = ExecutionContext::new();

        let cell_id = CellId::new();
        ctx.set_binding(BindingInfo {
            name: Text::from("x"),
            value: Value::from_i64(42),
            type_info: Text::from("Int"),
            defined_in: cell_id,
            is_mutable: false,
        });

        assert!(ctx.get_binding("x").is_some());
        assert_eq!(ctx.get_binding("x").unwrap().value.as_i64(), 42);
        assert!(ctx.get_binding("y").is_none());

        ctx.remove_binding("x");
        assert!(ctx.get_binding("x").is_none());
    }

    #[test]
    fn test_dependency_tracking() {
        let mut ctx = ExecutionContext::new();
        let cell1 = CellId::new();
        let cell2 = CellId::new();

        ctx.dependencies.add_definition(cell1, Text::from("x"));
        ctx.dependencies.add_use(Text::from("x"), cell2);

        let deps = ctx.dependencies.dependents(&Text::from("x"));
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0], cell2);
    }

    #[test]
    fn test_clear_cell_bindings() {
        let mut ctx = ExecutionContext::new();
        let cell1 = CellId::new();
        let cell2 = CellId::new();

        ctx.set_binding(BindingInfo {
            name: Text::from("x"),
            value: Value::from_i64(1),
            type_info: Text::from("Int"),
            defined_in: cell1,
            is_mutable: false,
        });
        ctx.set_binding(BindingInfo {
            name: Text::from("y"),
            value: Value::from_i64(2),
            type_info: Text::from("Int"),
            defined_in: cell2,
            is_mutable: false,
        });

        ctx.clear_cell_bindings(cell1);

        assert!(ctx.get_binding("x").is_none());
        assert!(ctx.get_binding("y").is_some());
    }
}
