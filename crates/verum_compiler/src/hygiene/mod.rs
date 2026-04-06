//! Hygiene System for Macro Expansion
//!
//! Implements proper sets-of-scopes hygiene (Scheme/Racket model) for
//! hygienic macro expansion.
//!
//! ## Module Structure
//!
//! - `scope` - Scope, ScopeSet, ScopeId, HygienicIdent
//! - `marks` - ScopeMark for expansion phases
//! - `gensym` - Unique identifier generation
//! - `resolver` - Identifier resolution with scope awareness
//! - `violations` - Hygiene violation types and collection
//! - `expander` - Quote expansion with hygiene enforcement
//! - `checker` - Post-expansion hygiene verification
//!
//! ## Design
//!
//! The hygiene system ensures that:
//! 1. Identifiers introduced by macros don't capture user bindings
//! 2. User references to bindings aren't captured by macro-introduced bindings
//! 3. Macros can intentionally introduce hygiene-breaking bindings when needed
//!
//! This is achieved through the sets-of-scopes model where each identifier
//! carries a set of scopes, and resolution finds the binding whose scopes
//! are a subset of the identifier's scopes (preferring the most specific).
//!
//! ## Pipeline
//!
//! The quote expansion pipeline consists of:
//! 1. **Parse Quote** - Parse quote { ... } into QuoteAST
//! 2. **Mark Phase** - Assign fresh marks to introduced bindings
//! 3. **Splice Phase** - Substitute $var and $[for...] expressions
//! 4. **Hygiene Check** - Verify no accidental capture occurs
//! 5. **Emit Code** - Generate TokenStream with hygiene metadata
//!
//! Hygienic macro system: ensures macro expansion preserves lexical scoping
//! through syntax context marks, preventing accidental name capture.
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

pub mod checker;
pub mod expander;
pub mod gensym;
pub mod marks;
pub mod resolver;
pub mod scope;
pub mod syntax_context;
pub mod violations;

use std::sync::atomic::{AtomicU64, Ordering};

use verum_ast::Span;
use verum_common::{List, Map, Text};

pub use checker::{CheckResult, CheckStats, CheckerConfig, HygieneChecker, HygieneVisitor};
pub use expander::{
    ConstValue, ExpansionConfig, LiftedValue, QuoteExpander, SpliceResult, StageBinding,
    StageContext, Token, TokenKind, TokenStream,
};
pub use gensym::{GensymGenerator, SuffixHygieneGenerator};
pub use marks::{ExpansionInfo, MarkKind, MarkStack, ScopeMark};
pub use resolver::{Resolution, ResolvedBinding, ScopeResolver};
pub use scope::{
    BindingInfo, BindingKind, HygienicIdent, Scope, ScopeId, ScopeKind, ScopeSet,
};
pub use syntax_context::{
    Mark, MarkSet, SyntaxContext, SyntaxContextId, SyntaxContextRegistry, Transparency,
};
pub use violations::{HygieneViolation, HygieneViolations};

/// Hygiene context for managing scopes during macro expansion
#[derive(Debug)]
pub struct HygieneContext {
    /// Counter for generating unique scope IDs
    scope_counter: AtomicU64,
    /// Gensym generator for unique identifiers
    gensym: GensymGenerator,
    /// Suffix-based generator for legacy compatibility
    suffix_gen: SuffixHygieneGenerator,
    /// Scope resolver
    resolver: ScopeResolver,
    /// Mark stack for tracking macro expansions
    marks: MarkStack,
    /// Current scope stack
    scope_stack: List<ScopeId>,
    /// All registered scopes
    scopes: Map<ScopeId, Scope>,
}

impl HygieneContext {
    /// Create a new hygiene context
    pub fn new() -> Self {
        Self {
            scope_counter: AtomicU64::new(0),
            gensym: GensymGenerator::new(),
            suffix_gen: SuffixHygieneGenerator::new(),
            resolver: ScopeResolver::new(),
            marks: MarkStack::new(),
            scope_stack: List::new(),
            scopes: Map::new(),
        }
    }

    // ========================================================================
    // Scope Management
    // ========================================================================

    /// Generate a fresh scope ID
    pub fn fresh_scope_id(&self) -> ScopeId {
        ScopeId::new(self.scope_counter.fetch_add(1, Ordering::SeqCst))
    }

    /// Enter a new scope
    ///
    /// Creates a new scope of the given kind and pushes it onto the stack.
    pub fn enter_scope(&mut self, kind: ScopeKind) -> ScopeId {
        let scope_id = self.fresh_scope_id();
        let parent = self.scope_stack.last().copied();
        let phase = self.marks.current_phase();

        let scope = Scope::new(scope_id, parent, kind, phase);
        self.scopes.insert(scope_id, scope.clone());
        self.resolver.register_scope(scope);
        self.scope_stack.push(scope_id);

        scope_id
    }

    /// Exit the current scope
    pub fn exit_scope(&mut self) -> Option<ScopeId> {
        self.scope_stack.pop()
    }

    /// Get the current scope ID
    pub fn current_scope(&self) -> Option<ScopeId> {
        self.scope_stack.last().copied()
    }

    /// Get the current scope set (all scopes on the stack)
    pub fn current_scopes(&self) -> ScopeSet {
        ScopeSet::from_iter(self.scope_stack.iter().copied())
    }

    /// Get a scope by ID
    pub fn get_scope(&self, id: &ScopeId) -> Option<&Scope> {
        self.scopes.get(id)
    }

    /// Get a mutable scope by ID
    pub fn get_scope_mut(&mut self, id: &ScopeId) -> Option<&mut Scope> {
        self.scopes.get_mut(id)
    }

    // ========================================================================
    // Macro Expansion
    // ========================================================================

    /// Enter a macro expansion
    ///
    /// Pushes a macro expansion mark and scope.
    pub fn enter_macro_expansion(&mut self, macro_name: &str) -> ExpansionInfo {
        let mark = self.marks.push(MarkKind::MacroExpansion);
        let def_scope = self.current_scope().unwrap_or(ScopeId::new(0));
        let use_scope = self.enter_scope(ScopeKind::MacroUse);

        ExpansionInfo::new(Text::from(macro_name), def_scope, use_scope, mark)
    }

    /// Exit a macro expansion
    pub fn exit_macro_expansion(&mut self) {
        self.exit_scope();
        self.marks.pop();
    }

    /// Enter a quote expression
    pub fn enter_quote(&mut self) -> ScopeId {
        self.marks.push(MarkKind::Quote);
        self.enter_scope(ScopeKind::Quote)
    }

    /// Exit a quote expression
    pub fn exit_quote(&mut self) {
        self.exit_scope();
        self.marks.pop();
    }

    /// Enter an unquote expression
    pub fn enter_unquote(&mut self) -> ScopeId {
        self.marks.push(MarkKind::Unquote);
        self.enter_scope(ScopeKind::Unquote)
    }

    /// Exit an unquote expression
    pub fn exit_unquote(&mut self) {
        self.exit_scope();
        self.marks.pop();
    }

    /// Check if we're currently inside a macro expansion
    pub fn in_macro_expansion(&self) -> bool {
        self.marks.in_macro_expansion()
    }

    /// Check if we're currently inside a quote
    pub fn in_quote(&self) -> bool {
        self.marks.in_quote()
    }

    // ========================================================================
    // Identifier Generation
    // ========================================================================

    /// Generate a unique identifier (gensym)
    ///
    /// The generated identifier is guaranteed to not conflict with any
    /// user-written code.
    pub fn gensym(&self, base: &str) -> Text {
        self.gensym.gensym(base)
    }

    /// Generate a hygienic identifier with current scopes
    pub fn gensym_hygienic(&self, base: &str, span: Span) -> HygienicIdent {
        self.gensym.gensym_hygienic(base, self.current_scopes(), span)
    }

    /// Generate a temporary variable name
    pub fn temp_var(&self) -> Text {
        self.gensym.temp_var()
    }

    /// Generate a temporary label name
    pub fn temp_label(&self) -> Text {
        self.gensym.temp_label()
    }

    /// Generate a hygienic identifier using suffix format (legacy)
    ///
    /// This creates a unique identifier based on the given name that
    /// won't conflict with user code, using the simple `name__id` format.
    pub fn generate(&self, base_name: &str) -> Text {
        self.suffix_gen.generate(base_name)
    }

    /// Check if an identifier was generated by the hygiene system
    pub fn is_hygienic(name: &str) -> bool {
        GensymGenerator::is_gensym(name) || SuffixHygieneGenerator::is_hygienic(name)
    }

    /// Extract the base name from a hygienic identifier
    pub fn base_name(name: &str) -> Text {
        if let Some(base) = GensymGenerator::base_name(name) {
            base
        } else {
            SuffixHygieneGenerator::base_name(name)
        }
    }

    // ========================================================================
    // Binding Management
    // ========================================================================

    /// Add a binding to the current scope
    pub fn add_binding(&mut self, name: Text, info: BindingInfo) {
        if let Some(scope_id) = self.current_scope() {
            if let Some(scope) = self.scopes.get_mut(&scope_id) {
                scope.add_binding(name, info);
            }
        }
    }

    /// Create a binding info for a variable
    pub fn variable_binding(&self, name: &str, is_mutable: bool) -> BindingInfo {
        BindingInfo {
            original_name: Text::from(name),
            hygienic_name: self.gensym(name),
            scope_id: self.current_scope().unwrap_or(ScopeId::new(0)),
            is_mutable,
            kind: BindingKind::Variable,
        }
    }

    /// Create a binding info for a function parameter
    pub fn parameter_binding(&self, name: &str) -> BindingInfo {
        BindingInfo {
            original_name: Text::from(name),
            hygienic_name: self.gensym(name),
            scope_id: self.current_scope().unwrap_or(ScopeId::new(0)),
            is_mutable: false,
            kind: BindingKind::Parameter,
        }
    }

    // ========================================================================
    // Resolution
    // ========================================================================

    /// Resolve an identifier
    pub fn resolve(&mut self, ident: &HygienicIdent) -> Resolution {
        self.resolver.resolve(ident)
    }

    /// Create a hygienic identifier for resolution
    pub fn make_ident(&self, name: &str, span: Span) -> HygienicIdent {
        HygienicIdent::new(Text::from(name), self.current_scopes(), span)
    }
}

impl Default for HygieneContext {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for HygieneContext {
    fn clone(&self) -> Self {
        Self {
            scope_counter: AtomicU64::new(self.scope_counter.load(Ordering::Relaxed)),
            gensym: self.gensym.clone(),
            suffix_gen: self.suffix_gen.clone(),
            resolver: ScopeResolver::new(), // Don't clone resolver cache
            marks: self.marks.clone(),
            scope_stack: self.scope_stack.clone(),
            scopes: self.scopes.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hygiene_context_creation() {
        let ctx = HygieneContext::new();
        assert!(ctx.current_scope().is_none());
        assert!(!ctx.in_macro_expansion());
    }

    #[test]
    fn test_scope_management() {
        let mut ctx = HygieneContext::new();

        let scope1 = ctx.enter_scope(ScopeKind::Module);
        assert_eq!(ctx.current_scope(), Some(scope1));

        let scope2 = ctx.enter_scope(ScopeKind::Function);
        assert_eq!(ctx.current_scope(), Some(scope2));

        ctx.exit_scope();
        assert_eq!(ctx.current_scope(), Some(scope1));

        ctx.exit_scope();
        assert!(ctx.current_scope().is_none());
    }

    #[test]
    fn test_macro_expansion() {
        let mut ctx = HygieneContext::new();

        let info = ctx.enter_macro_expansion("test_macro");
        assert!(ctx.in_macro_expansion());
        assert_eq!(info.macro_name.as_str(), "test_macro");

        ctx.exit_macro_expansion();
        assert!(!ctx.in_macro_expansion());
    }

    #[test]
    fn test_gensym() {
        let ctx = HygieneContext::new();

        let id1 = ctx.gensym("x");
        let id2 = ctx.gensym("x");

        assert_ne!(id1, id2);
        assert!(HygieneContext::is_hygienic(id1.as_str()));
    }

    #[test]
    fn test_legacy_generate() {
        let ctx = HygieneContext::new();

        let id1 = ctx.generate("foo");
        let id2 = ctx.generate("foo");

        assert_ne!(id1, id2);
        assert!(HygieneContext::is_hygienic(id1.as_str()));
        assert_eq!(HygieneContext::base_name(id1.as_str()).as_str(), "foo");
    }

    #[test]
    fn test_current_scopes() {
        let mut ctx = HygieneContext::new();

        ctx.enter_scope(ScopeKind::Module);
        ctx.enter_scope(ScopeKind::Function);

        let scopes = ctx.current_scopes();
        assert_eq!(scopes.len(), 2);
    }
}
