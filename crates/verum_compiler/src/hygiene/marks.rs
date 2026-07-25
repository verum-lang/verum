//! Scope Marks for Macro Expansion Phases
//!
//! Marks track macro expansion phases to distinguish between different
//! expansions of the same macro.
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

use verum_common::Text;

use super::scope::ScopeId;

/// A scope mark identifies a particular macro expansion
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeMark {
    /// Unique identifier for this mark
    pub id: u64,
    /// The expansion phase when this mark was created
    pub phase: u32,
    /// The kind of mark
    pub kind: MarkKind,
}

/// Kind of scope mark
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarkKind {
    /// Mark added when entering a macro definition
    MacroDefinition,
    /// Mark added when expanding a macro use
    MacroExpansion,
    /// Mark added when entering a quote
    Quote,
    /// Mark removed when processing an unquote
    Unquote,
    /// Mark for built-in macro expansions
    BuiltinExpansion,
}

impl ScopeMark {
    /// Create a new scope mark
    pub fn new(id: u64, phase: u32, kind: MarkKind) -> Self {
        Self { id, phase, kind }
    }

    /// Create a macro definition mark
    pub fn macro_def(id: u64, phase: u32) -> Self {
        Self::new(id, phase, MarkKind::MacroDefinition)
    }

    /// Create a macro expansion mark
    pub fn macro_expansion(id: u64, phase: u32) -> Self {
        Self::new(id, phase, MarkKind::MacroExpansion)
    }

    /// Create a quote mark
    pub fn quote(id: u64, phase: u32) -> Self {
        Self::new(id, phase, MarkKind::Quote)
    }

    /// Create an unquote mark
    pub fn unquote(id: u64, phase: u32) -> Self {
        Self::new(id, phase, MarkKind::Unquote)
    }

    /// Create a builtin expansion mark
    pub fn builtin(id: u64, phase: u32) -> Self {
        Self::new(id, phase, MarkKind::BuiltinExpansion)
    }

    /// Check if this mark adds scopes
    pub fn adds_scope(&self) -> bool {
        matches!(
            self.kind,
            MarkKind::MacroDefinition | MarkKind::MacroExpansion | MarkKind::Quote
        )
    }

    /// Check if this mark removes scopes
    pub fn removes_scope(&self) -> bool {
        matches!(self.kind, MarkKind::Unquote)
    }
}

/// Mark stack for tracking nested macro expansions
#[derive(Debug, Clone)]
pub struct MarkStack {
    /// Stack of marks
    marks: Vec<ScopeMark>,
    /// Current expansion phase
    current_phase: u32,
    /// Counter for generating unique mark IDs
    counter: u64,
}

impl MarkStack {
    /// Create a new mark stack
    pub fn new() -> Self {
        Self {
            marks: Vec::new(),
            current_phase: 0,
            counter: 0,
        }
    }

    /// Push a new mark onto the stack
    pub fn push(&mut self, kind: MarkKind) -> ScopeMark {
        let mark = ScopeMark::new(self.counter, self.current_phase, kind);
        self.counter += 1;
        self.marks.push(mark);
        mark
    }

    /// Pop the top mark from the stack
    pub fn pop(&mut self) -> Option<ScopeMark> {
        self.marks.pop()
    }

    /// Get the current mark (top of stack)
    pub fn current(&self) -> Option<&ScopeMark> {
        self.marks.last()
    }

    /// Get the current expansion phase
    pub fn current_phase(&self) -> u32 {
        self.current_phase
    }

    /// Increment the expansion phase
    pub fn next_phase(&mut self) -> u32 {
        self.current_phase += 1;
        self.current_phase
    }

    /// Get the depth of the mark stack
    pub fn depth(&self) -> usize {
        self.marks.len()
    }

    /// Check if we're inside a macro expansion
    pub fn in_macro_expansion(&self) -> bool {
        self.marks
            .iter()
            .any(|m| m.kind == MarkKind::MacroExpansion)
    }

    /// Check if we're inside a quote
    pub fn in_quote(&self) -> bool {
        self.marks.iter().any(|m| m.kind == MarkKind::Quote)
    }

    /// Get all marks of a specific kind
    pub fn marks_of_kind(&self, kind: MarkKind) -> impl Iterator<Item = &ScopeMark> {
        self.marks.iter().filter(move |m| m.kind == kind)
    }
}

impl Default for MarkStack {
    fn default() -> Self {
        Self::new()
    }
}

/// Information about a macro expansion context
#[derive(Debug, Clone)]
pub struct ExpansionInfo {
    /// Name of the macro being expanded
    pub macro_name: Text,
    /// Scope ID of the macro definition
    pub def_scope: ScopeId,
    /// Scope ID of the macro use site
    pub use_scope: ScopeId,
    /// The mark for this expansion
    pub mark: ScopeMark,
}

impl ExpansionInfo {
    /// Create new expansion info
    pub fn new(macro_name: Text, def_scope: ScopeId, use_scope: ScopeId, mark: ScopeMark) -> Self {
        Self {
            macro_name,
            def_scope,
            use_scope,
            mark,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mark_stack() {
        let mut stack = MarkStack::new();
        assert_eq!(stack.depth(), 0);

        let mark1 = stack.push(MarkKind::MacroExpansion);
        assert_eq!(stack.depth(), 1);
        assert!(stack.in_macro_expansion());

        let mark2 = stack.push(MarkKind::Quote);
        assert_eq!(stack.depth(), 2);
        assert!(stack.in_quote());

        assert_eq!(stack.pop(), Some(mark2));
        assert_eq!(stack.pop(), Some(mark1));
        assert_eq!(stack.depth(), 0);
    }

    #[test]
    fn test_mark_properties() {
        let def_mark = ScopeMark::macro_def(1, 0);
        assert!(def_mark.adds_scope());
        assert!(!def_mark.removes_scope());

        let unquote_mark = ScopeMark::unquote(2, 0);
        assert!(!unquote_mark.adds_scope());
        assert!(unquote_mark.removes_scope());
    }
}
