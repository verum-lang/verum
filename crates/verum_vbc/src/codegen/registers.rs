//! Register allocator for VBC codegen.
//!
//! Uses a simple linear scan allocator that assigns virtual registers
//! to variables and temporaries. The register file is unlimited during
//! codegen; the final register count is stored in the function descriptor.
//!
//! # Register Layout
//!
//! ```text
//! r0..rN-1    : Function parameters (N = param count)
//! rN..rM-1    : Local variables
//! rM..rK-1    : Temporaries
//! ```

use crate::instruction::Reg;
use std::collections::HashMap;

/// Maximum registers per function.
pub const MAX_REGISTERS: u16 = 16384;

/// Register allocator for VBC code generation.
///
/// Manages allocation of virtual registers for parameters, locals,
/// and temporaries within a function.
#[derive(Debug)]
pub struct RegisterAllocator {
    /// Next available register index.
    next_reg: u16,

    /// Map from variable names to registers.
    variables: HashMap<String, RegisterInfo>,

    /// Stack of scope boundaries for nested scopes.
    scope_stack: Vec<ScopeMarker>,

    /// Peak register usage (high water mark).
    peak_usage: u16,

    /// Free list for recycled temporaries.
    free_list: Vec<Reg>,

    /// Whether to recycle temporaries (optimization).
    recycle_temps: bool,
}

/// Information about a register binding.
#[derive(Debug, Clone)]
pub struct RegisterInfo {
    /// The allocated register.
    pub reg: Reg,

    /// Whether this binding is mutable.
    pub is_mutable: bool,

    /// Whether this binding has been initialized.
    ///
    /// `false` for `let x: T;` (no initializer) — allows one assignment
    /// as initialization even when `!is_mutable`. After the first assignment,
    /// this is set to `true`.
    pub is_initialized: bool,

    /// Scope level where this was defined.
    pub scope_level: usize,

    /// Kind of binding.
    pub kind: RegisterKind,

    /// Whether this variable is a heap-allocated cell for mutable closure captures.
    /// When true, the register holds a pointer to a 1-field object, and reads/writes
    /// must go through GetF/SetF at field index 0.
    pub is_cell: bool,
}

/// Kind of register binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterKind {
    /// Function parameter.
    Parameter,
    /// Local variable.
    Local,
    /// Temporary value.
    Temporary,
    /// Captured variable (from closure).
    Captured,
}

/// Marker for scope boundaries.
#[derive(Debug, Clone)]
struct ScopeMarker {
    /// Variables defined in this scope.
    scope_vars: Vec<String>,
    /// Shadowed variables that need to be restored on scope exit.
    /// Maps variable name to its previous RegisterInfo.
    shadowed_vars: Vec<(String, RegisterInfo)>,
}

impl Default for RegisterAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl RegisterAllocator {
    /// Creates a new register allocator.
    pub fn new() -> Self {
        Self {
            next_reg: 0,
            variables: HashMap::new(),
            scope_stack: vec![ScopeMarker {
                scope_vars: Vec::new(),
                shadowed_vars: Vec::new(),
            }],
            peak_usage: 0,
            free_list: Vec::new(),
            recycle_temps: true,
        }
    }

    /// Creates an allocator without temporary recycling.
    pub fn without_recycling() -> Self {
        let mut alloc = Self::new();
        alloc.recycle_temps = false;
        alloc
    }

    /// Allocates registers for function parameters.
    ///
    /// Parameters are allocated in order starting from r0.
    /// Each parameter is a tuple of (name, is_mutable).
    /// Returns the register assigned to each parameter.
    pub fn alloc_parameters(&mut self, params: &[(String, bool)]) -> Vec<Reg> {
        let mut regs = Vec::with_capacity(params.len());

        for (i, (name, is_mutable)) in params.iter().enumerate() {
            let reg = Reg(i as u16);
            self.variables.insert(
                name.clone(),
                RegisterInfo {
                    reg,
                    is_mutable: *is_mutable,
                    is_initialized: true,
                    scope_level: 0,
                    kind: RegisterKind::Parameter,
                    is_cell: false,
                },
            );
            regs.push(reg);
        }

        self.next_reg = params.len() as u16;
        self.peak_usage = self.peak_usage.max(self.next_reg);

        regs
    }

    /// Allocates a register for a local variable.
    ///
    /// The variable is bound in the current scope. If a variable with the same
    /// name already exists (shadowing), the old binding is saved and will be
    /// restored when the scope exits.
    pub fn alloc_local(&mut self, name: &str, is_mutable: bool) -> Reg {
        let reg = self.alloc_fresh();
        let scope_level = self.scope_stack.len() - 1;

        // If this variable shadows an existing one, save the old binding
        if let Some(scope) = self.scope_stack.last_mut()
            && let Some(old_info) = self.variables.get(name) {
                scope.shadowed_vars.push((name.to_string(), old_info.clone()));
            }

        self.variables.insert(
            name.to_string(),
            RegisterInfo {
                reg,
                is_mutable,
                is_initialized: true,
                scope_level,
                kind: RegisterKind::Local,
                is_cell: false,
            },
        );

        // Track in current scope for cleanup
        if let Some(scope) = self.scope_stack.last_mut() {
            scope.scope_vars.push(name.to_string());
        }

        reg
    }

    /// Allocates a register for a named variable (alias for alloc_local with default mutability).
    ///
    /// Returns a Result to match the expected API.
    pub fn alloc_named(&mut self, name: &str) -> Result<Reg, super::error::CodegenError> {
        Ok(self.alloc_local(name, true))
    }

    /// Allocates a fresh register (for temporary values).
    ///
    /// May recycle from free list if recycling is enabled.
    pub fn alloc_temp(&mut self) -> Reg {
        if self.recycle_temps
            && let Some(reg) = self.free_list.pop() {
                return reg;
            }
        self.alloc_fresh()
    }

    /// Releases a temporary register for reuse.
    pub fn free_temp(&mut self, reg: Reg) {
        if self.recycle_temps {
            // Only recycle if it's a temporary (not a named variable)
            let is_named = self.variables.values().any(|info| info.reg == reg);
            if !is_named {
                self.free_list.push(reg);
            }
        }
    }

    /// Allocates a fresh register (never recycled).
    ///
    /// Use this when you need to guarantee consecutive registers,
    /// such as for function call arguments.
    pub fn alloc_fresh(&mut self) -> Reg {
        let reg = Reg(self.next_reg);
        self.next_reg += 1;
        self.peak_usage = self.peak_usage.max(self.next_reg);
        reg
    }

    /// Looks up a variable's register.
    pub fn lookup(&self, name: &str) -> Option<&RegisterInfo> {
        self.variables.get(name)
    }

    /// Looks up a variable's register (mutable).
    pub fn lookup_mut(&mut self, name: &str) -> Option<&mut RegisterInfo> {
        self.variables.get_mut(name)
    }

    /// Checks if a variable exists.
    pub fn contains(&self, name: &str) -> bool {
        self.variables.contains_key(name)
    }

    /// Gets all variable names in the current scope.
    pub fn all_variable_names(&self) -> Vec<String> {
        self.variables.keys().cloned().collect()
    }

    /// Gets the register for a variable, if it exists.
    pub fn get_reg(&self, name: &str) -> Option<Reg> {
        self.variables.get(name).map(|info| info.reg)
    }

    /// Enters a new scope.
    ///
    /// Variables defined in the new scope will be removed when the scope exits.
    /// Any shadowed variables will be restored to their previous bindings.
    pub fn enter_scope(&mut self) {
        self.scope_stack.push(ScopeMarker {
            scope_vars: Vec::new(),
            shadowed_vars: Vec::new(),
        });
    }

    /// Exits the current scope.
    ///
    /// Removes all variables defined in this scope and restores any shadowed
    /// variable bindings to their previous values.
    /// Returns the variables that were removed (for potential drop calls).
    pub fn exit_scope(&mut self) -> Vec<(String, Reg)> {
        let mut removed = Vec::new();

        if let Some(scope) = self.scope_stack.pop() {
            // First, remove all variables defined in this scope
            for name in &scope.scope_vars {
                if let Some(info) = self.variables.remove(name) {
                    removed.push((name.clone(), info.reg));
                    // Recycle the register
                    if self.recycle_temps && info.kind == RegisterKind::Local {
                        self.free_list.push(info.reg);
                    }
                }
            }

            // Restore shadowed variables in reverse order (LIFO)
            for (name, info) in scope.shadowed_vars.into_iter().rev() {
                self.variables.insert(name, info);
            }
        }

        removed
    }

    /// Returns the current scope level.
    ///
    /// # Panics
    /// Panics if the scope stack is empty, which indicates a bug in scope management
    /// (either missing `begin_function`/`reset()` call, or improper restoration after
    /// closure compilation).
    pub fn scope_level(&self) -> usize {
        assert!(
            !self.scope_stack.is_empty(),
            "BUG: scope_stack is empty - missing begin_function/reset or improper restore after closure"
        );
        self.scope_stack.len() - 1
    }

    /// Returns the total number of registers used.
    ///
    /// This is the peak usage, which should be stored in the function descriptor.
    pub fn register_count(&self) -> u16 {
        self.peak_usage
    }

    /// Collects debug variable information for DWARF emission.
    ///
    /// Returns a list of (variable_name, register, is_parameter, arg_index) tuples
    /// for all named variables (excludes temporaries).
    pub fn collect_debug_variables(&self) -> Vec<(String, u16, bool, u16)> {
        let mut result = Vec::new();
        let mut param_idx = 0u16;

        for (name, info) in &self.variables {
            match info.kind {
                RegisterKind::Parameter => {
                    param_idx += 1;
                    result.push((name.clone(), info.reg.0, true, param_idx));
                }
                RegisterKind::Local | RegisterKind::Captured => {
                    result.push((name.clone(), info.reg.0, false, 0));
                }
                RegisterKind::Temporary => {
                    // Skip temporaries — they have no user-visible name
                }
            }
        }

        // Sort by register index for deterministic output
        result.sort_by_key(|&(_, reg, _, _)| reg);
        result
    }

    /// Returns the current next register index.
    pub fn current_reg(&self) -> u16 {
        self.next_reg
    }

    /// Reserves a specific number of additional registers.
    ///
    /// Useful for pre-allocating space for call arguments.
    pub fn reserve(&mut self, count: u16) -> Reg {
        let start = Reg(self.next_reg);
        self.next_reg += count;
        self.peak_usage = self.peak_usage.max(self.next_reg);
        start
    }

    /// Checks if allocation would exceed maximum registers.
    pub fn would_overflow(&self, additional: u16) -> bool {
        self.next_reg.saturating_add(additional) > MAX_REGISTERS
    }

    /// Marks a variable as mutable.
    pub fn mark_mutable(&mut self, name: &str) {
        if let Some(info) = self.variables.get_mut(name) {
            info.is_mutable = true;
        }
    }

    /// Gets all variables in the current scope.
    pub fn current_scope_vars(&self) -> impl Iterator<Item = (&str, &RegisterInfo)> {
        let current_level = self.scope_level();
        self.variables
            .iter()
            .filter(move |(_, info)| info.scope_level == current_level)
            .map(|(name, info)| (name.as_str(), info))
    }

    /// Creates a snapshot of current register state.
    ///
    /// This saves the full state including variables and scope_stack, which is needed
    /// for closure compilation where we need to restore the parent's
    /// variable bindings and scope state after the closure is compiled.
    pub fn snapshot(&self) -> RegisterSnapshot {
        RegisterSnapshot {
            next_reg: self.next_reg,
            variables: self.variables.clone(),
            scope_stack: self.scope_stack.clone(),
            peak_usage: self.peak_usage,
            free_list: self.free_list.clone(),
        }
    }

    /// Restores from a snapshot (full state restoration).
    ///
    /// This is used after closure compilation to restore the parent function's
    /// full state including variables and scope_stack that were cleared by
    /// `begin_function` -> `reset()`.
    pub fn restore_reg(&mut self, snapshot: &RegisterSnapshot) {
        self.next_reg = snapshot.next_reg;
        self.variables = snapshot.variables.clone();
        self.scope_stack = snapshot.scope_stack.clone();
        self.peak_usage = snapshot.peak_usage;
        self.free_list = snapshot.free_list.clone();
    }

    /// Resets the allocator for a new function.
    pub fn reset(&mut self) {
        self.next_reg = 0;
        self.variables.clear();
        self.scope_stack.clear();
        self.scope_stack.push(ScopeMarker {
            scope_vars: Vec::new(),
            shadowed_vars: Vec::new(),
        });
        self.peak_usage = 0;
        self.free_list.clear();
    }
}

/// Snapshot of register allocator state.
///
/// This is used to save/restore state when compiling closures.
/// The full state is saved including scope_stack, which is critical for
/// correct restoration after nested closure compilation.
#[derive(Debug, Clone)]
pub struct RegisterSnapshot {
    next_reg: u16,
    /// Saved variables map for full state restoration
    variables: std::collections::HashMap<String, RegisterInfo>,
    /// Saved scope stack - critical for correct scope tracking after closure compilation
    scope_stack: Vec<ScopeMarker>,
    /// Peak register usage at snapshot time
    peak_usage: u16,
    /// Free list for temporary recycling
    free_list: Vec<Reg>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_allocator() {
        let alloc = RegisterAllocator::new();
        assert_eq!(alloc.register_count(), 0);
        assert_eq!(alloc.scope_level(), 0);
    }

    #[test]
    fn test_alloc_parameters() {
        let mut alloc = RegisterAllocator::new();
        let params = vec![("a".to_string(), false), ("b".to_string(), true), ("c".to_string(), false)];
        let regs = alloc.alloc_parameters(&params);

        assert_eq!(regs.len(), 3);
        assert_eq!(regs[0], Reg(0));
        assert_eq!(regs[1], Reg(1));
        assert_eq!(regs[2], Reg(2));

        assert_eq!(alloc.get_reg("a"), Some(Reg(0)));
        assert_eq!(alloc.get_reg("b"), Some(Reg(1)));
        assert_eq!(alloc.get_reg("c"), Some(Reg(2)));
        assert_eq!(alloc.register_count(), 3);
    }

    #[test]
    fn test_alloc_locals() {
        let mut alloc = RegisterAllocator::new();

        let r0 = alloc.alloc_local("x", false);
        let r1 = alloc.alloc_local("y", true);

        assert_eq!(r0, Reg(0));
        assert_eq!(r1, Reg(1));

        let info_x = alloc.lookup("x").unwrap();
        assert!(!info_x.is_mutable);
        assert_eq!(info_x.kind, RegisterKind::Local);

        let info_y = alloc.lookup("y").unwrap();
        assert!(info_y.is_mutable);
    }

    #[test]
    fn test_alloc_temps() {
        let mut alloc = RegisterAllocator::new();

        let t0 = alloc.alloc_temp();
        let t1 = alloc.alloc_temp();
        let t2 = alloc.alloc_temp();

        assert_eq!(t0, Reg(0));
        assert_eq!(t1, Reg(1));
        assert_eq!(t2, Reg(2));

        // Free and reuse
        alloc.free_temp(t1);
        let t3 = alloc.alloc_temp();
        assert_eq!(t3, Reg(1)); // Recycled

        alloc.free_temp(t0);
        alloc.free_temp(t2);
        let t4 = alloc.alloc_temp();
        let t5 = alloc.alloc_temp();
        assert_eq!(t4, Reg(2)); // LIFO order
        assert_eq!(t5, Reg(0));
    }

    #[test]
    fn test_scope_management() {
        let mut alloc = RegisterAllocator::new();

        // Outer scope
        alloc.alloc_local("outer", false);
        assert!(alloc.contains("outer"));

        // Enter inner scope
        alloc.enter_scope();
        alloc.alloc_local("inner", false);
        assert!(alloc.contains("outer"));
        assert!(alloc.contains("inner"));

        // Exit inner scope
        let removed = alloc.exit_scope();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].0, "inner");

        assert!(alloc.contains("outer"));
        assert!(!alloc.contains("inner"));
    }

    #[test]
    fn test_nested_scopes() {
        let mut alloc = RegisterAllocator::new();

        alloc.alloc_local("a", false);
        alloc.enter_scope();
        alloc.alloc_local("b", false);
        alloc.enter_scope();
        alloc.alloc_local("c", false);

        assert_eq!(alloc.scope_level(), 2);
        assert!(alloc.contains("a"));
        assert!(alloc.contains("b"));
        assert!(alloc.contains("c"));

        alloc.exit_scope();
        assert!(!alloc.contains("c"));
        assert!(alloc.contains("b"));

        alloc.exit_scope();
        assert!(!alloc.contains("b"));
        assert!(alloc.contains("a"));
    }

    #[test]
    fn test_shadowing() {
        let mut alloc = RegisterAllocator::new();

        let r0 = alloc.alloc_local("x", false);
        assert_eq!(r0, Reg(0));

        alloc.enter_scope();
        let r1 = alloc.alloc_local("x", true); // Shadow
        assert_eq!(r1, Reg(1));
        assert_eq!(alloc.get_reg("x"), Some(Reg(1)));

        alloc.exit_scope();
        assert_eq!(alloc.get_reg("x"), Some(Reg(0))); // Restored
    }

    #[test]
    fn test_reserve() {
        let mut alloc = RegisterAllocator::new();

        alloc.alloc_local("x", false);
        let start = alloc.reserve(5);

        assert_eq!(start, Reg(1));
        assert_eq!(alloc.register_count(), 6);
    }

    #[test]
    fn test_peak_usage() {
        let mut alloc = RegisterAllocator::new();

        // Allocate some temps
        let t0 = alloc.alloc_temp();
        let t1 = alloc.alloc_temp();
        let t2 = alloc.alloc_temp();
        assert_eq!(alloc.register_count(), 3);

        // Free them
        alloc.free_temp(t0);
        alloc.free_temp(t1);
        alloc.free_temp(t2);

        // Peak should still be 3
        assert_eq!(alloc.register_count(), 3);

        // Allocate more (recycled)
        let _t3 = alloc.alloc_temp();
        let _t4 = alloc.alloc_temp();
        assert_eq!(alloc.register_count(), 3); // Still 3 (recycled)

        // Force new allocation
        let _t5 = alloc.alloc_temp();
        let _t6 = alloc.alloc_temp();
        assert_eq!(alloc.register_count(), 4); // Now 4
    }

    #[test]
    fn test_without_recycling() {
        let mut alloc = RegisterAllocator::without_recycling();

        let t0 = alloc.alloc_temp();
        let t1 = alloc.alloc_temp();
        alloc.free_temp(t0);
        alloc.free_temp(t1);

        // Should NOT recycle
        let t2 = alloc.alloc_temp();
        assert_eq!(t2, Reg(2)); // Fresh allocation
    }

    #[test]
    fn test_reset() {
        let mut alloc = RegisterAllocator::new();

        alloc.alloc_parameters(&[("a".to_string(), false), ("b".to_string(), false)]);
        alloc.alloc_local("x", false);
        alloc.enter_scope();

        assert!(alloc.register_count() > 0);

        alloc.reset();

        assert_eq!(alloc.register_count(), 0);
        assert_eq!(alloc.scope_level(), 0);
        assert!(!alloc.contains("a"));
        assert!(!alloc.contains("x"));
    }

    #[test]
    fn test_snapshot_restore() {
        let mut alloc = RegisterAllocator::new();

        alloc.alloc_local("x", false);
        let snap = alloc.snapshot();

        alloc.alloc_temp();
        alloc.alloc_temp();
        assert_eq!(alloc.current_reg(), 3);

        alloc.restore_reg(&snap);
        assert_eq!(alloc.current_reg(), 1);
    }
}
