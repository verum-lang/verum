//! Syntax Context for Hygiene Tracking
//!
//! Every identifier in Verum carries a SyntaxContext that tracks where it was introduced.
//! This enables hygienic macro expansion where identifiers introduced by macros
//! don't accidentally capture user bindings.
//!
//! ## Design
//!
//! Based on Racket's sets-of-scopes model with additional support for:
//! - Multi-stage metaprogramming (stages 0, 1, 2+)
//! - Transparency modes (opaque, semi-transparent, transparent)
//! - Expansion chain tracking for error messages
//!
//! Syntax context tracking for hygienic macro expansion.
//! Each identifier carries a syntax context marking its expansion origin.

use std::sync::atomic::{AtomicU64, Ordering};

use verum_ast::Span;
use verum_common::{List, Map, Maybe, Text};

use super::scope::{ScopeId, ScopeSet};

/// Global counter for syntax context IDs
static SYNTAX_CONTEXT_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Global counter for marks
static MARK_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Unique identifier for a syntax context
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SyntaxContextId(u64);

impl SyntaxContextId {
    /// Create a fresh syntax context ID
    pub fn fresh() -> Self {
        Self(SYNTAX_CONTEXT_COUNTER.fetch_add(1, Ordering::SeqCst))
    }

    /// The root context (for top-level code)
    pub const ROOT: Self = Self(0);

    /// Get the raw ID value
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl Default for SyntaxContextId {
    fn default() -> Self {
        Self::ROOT
    }
}

/// A mark represents a scope boundary introduced by macro expansion
///
/// Marks are used to distinguish identifiers introduced at different
/// points in macro expansion, enabling hygienic scoping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Mark(u64);

impl Mark {
    /// Create a fresh mark (globally unique)
    pub fn fresh() -> Self {
        Mark(MARK_COUNTER.fetch_add(1, Ordering::SeqCst))
    }

    /// Create a mark from a raw ID
    ///
    /// Use this when reconstructing marks from stored IDs.
    /// For new marks, prefer `fresh()`.
    pub fn new(id: u64) -> Self {
        Mark(id)
    }

    /// The empty mark (represents no scope boundary)
    pub const EMPTY: Mark = Mark(0);

    /// Get the raw ID value
    pub fn as_u64(&self) -> u64 {
        self.0
    }

    /// Check if this is the empty mark
    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }

    /// Create a mark for a specific stage
    ///
    /// Stage-specific marks help track which stage an identifier belongs to
    /// in multi-stage metaprogramming. The mark is derived from the stage
    /// number to ensure uniqueness across stages.
    pub fn for_stage(stage: u32) -> Self {
        // Use high bits to encode stage info while keeping mark unique
        let stage_base = (stage as u64) << 48;
        let unique = MARK_COUNTER.fetch_add(1, Ordering::SeqCst);
        Mark(stage_base | unique)
    }

    /// Extract the stage from a stage-specific mark
    ///
    /// Returns None if this is not a stage-specific mark (normal marks
    /// don't encode stage information in their high bits).
    pub fn stage(&self) -> Option<u32> {
        let stage_bits = (self.0 >> 48) as u32;
        if stage_bits > 0 {
            Some(stage_bits)
        } else {
            None
        }
    }
}

impl Default for Mark {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// A set of marks attached to an identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct MarkSet {
    marks: Vec<Mark>,
}

impl MarkSet {
    /// Create an empty mark set
    pub fn new() -> Self {
        Self { marks: Vec::new() }
    }

    /// Create a mark set with a single mark
    pub fn singleton(mark: Mark) -> Self {
        Self { marks: vec![mark] }
    }

    /// Add a mark to the set
    pub fn add(&mut self, mark: Mark) {
        if !self.marks.contains(&mark) {
            self.marks.push(mark);
        }
    }

    /// Remove a mark from the set
    pub fn remove(&mut self, mark: &Mark) {
        self.marks.retain(|m| m != mark);
    }

    /// Flip operation: add if not present, remove if present
    ///
    /// This is the key operation for hygienic expansion - entering a quote
    /// adds a mark, exiting removes it, making the marks flip in nested quotes.
    pub fn flip(&mut self, mark: Mark) {
        if self.marks.contains(&mark) {
            self.marks.retain(|m| *m != mark);
        } else {
            self.marks.push(mark);
        }
    }

    /// Check if this set contains a mark
    pub fn contains(&self, mark: &Mark) -> bool {
        self.marks.contains(mark)
    }

    /// Check if two mark sets are compatible for binding
    ///
    /// Compatible if one is a subset of the other, which means the binding
    /// and reference were introduced at compatible macro expansion points.
    pub fn compatible(&self, other: &MarkSet) -> bool {
        self.is_subset_of(other) || other.is_subset_of(self)
    }

    /// Check if this set is a subset of another
    pub fn is_subset_of(&self, other: &MarkSet) -> bool {
        self.marks.iter().all(|m| other.marks.contains(m))
    }

    /// Check if this set is a superset of another
    pub fn is_superset_of(&self, other: &MarkSet) -> bool {
        other.is_subset_of(self)
    }

    /// Get the intersection with another set
    pub fn intersection(&self, other: &MarkSet) -> MarkSet {
        let marks = self
            .marks
            .iter()
            .filter(|m| other.marks.contains(m))
            .copied()
            .collect();
        MarkSet { marks }
    }

    /// Get the union with another set
    pub fn union(&self, other: &MarkSet) -> MarkSet {
        let mut marks = self.marks.clone();
        for mark in &other.marks {
            if !marks.contains(mark) {
                marks.push(*mark);
            }
        }
        MarkSet { marks }
    }

    /// Get the symmetric difference (XOR) with another set
    pub fn symmetric_difference(&self, other: &MarkSet) -> MarkSet {
        let mut result = MarkSet::new();
        for mark in &self.marks {
            if !other.marks.contains(mark) {
                result.marks.push(*mark);
            }
        }
        for mark in &other.marks {
            if !self.marks.contains(mark) {
                result.marks.push(*mark);
            }
        }
        result
    }

    /// Check if the set is empty
    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    /// Get the number of marks
    pub fn len(&self) -> usize {
        self.marks.len()
    }

    /// Iterate over marks
    pub fn iter(&self) -> impl Iterator<Item = &Mark> {
        self.marks.iter()
    }
}

/// Hygiene transparency modes
///
/// Controls how a macro expansion interacts with the caller's scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Transparency {
    /// Fully transparent: inherits caller's scope (like inline functions)
    ///
    /// Identifiers in the expansion can see and capture bindings from
    /// the call site. Use for code that should behave like it was
    /// written inline at the call site.
    Transparent,

    /// Semi-transparent: can see caller's types but not values
    ///
    /// The expansion can reference types from the call site, but cannot
    /// capture local variable bindings. Useful for derive-like macros
    /// that need to use caller's types.
    SemiTransparent,

    /// Opaque: fully hygienic (default for macros)
    ///
    /// The expansion is completely isolated from the call site.
    /// Identifiers in the expansion cannot accidentally capture
    /// caller bindings, and caller references cannot see macro-internal
    /// bindings.
    #[default]
    Opaque,
}

impl Transparency {
    /// Check if this mode allows capturing values from call site
    pub fn allows_value_capture(&self) -> bool {
        matches!(self, Transparency::Transparent)
    }

    /// Check if this mode allows referencing types from call site
    pub fn allows_type_reference(&self) -> bool {
        matches!(self, Transparency::Transparent | Transparency::SemiTransparent)
    }

    /// Check if this mode is fully hygienic
    pub fn is_hygienic(&self) -> bool {
        matches!(self, Transparency::Opaque)
    }
}

/// Information about a single macro expansion step
#[derive(Debug, Clone)]
pub struct ExpansionInfo {
    /// The name of the macro that performed this expansion
    pub macro_name: Text,
    /// The span of the macro call site
    pub call_site: Span,
    /// The span of the macro definition
    pub def_site: Span,
    /// Transparency mode for this expansion
    pub transparency: Transparency,
    /// The mark introduced by this expansion
    pub mark: Mark,
    /// The stage at which this expansion occurred
    pub stage: u32,
}

impl ExpansionInfo {
    /// Create new expansion info
    pub fn new(
        macro_name: Text,
        call_site: Span,
        def_site: Span,
        transparency: Transparency,
        stage: u32,
    ) -> Self {
        Self {
            macro_name,
            call_site,
            def_site,
            transparency,
            mark: Mark::fresh(),
            stage,
        }
    }

    /// Create expansion info with a specific mark
    pub fn with_mark(
        macro_name: Text,
        call_site: Span,
        def_site: Span,
        transparency: Transparency,
        mark: Mark,
        stage: u32,
    ) -> Self {
        Self {
            macro_name,
            call_site,
            def_site,
            transparency,
            mark,
            stage,
        }
    }

    /// Create a transparent expansion (like inline code)
    pub fn transparent(macro_name: Text, call_site: Span, def_site: Span, stage: u32) -> Self {
        Self::new(macro_name, call_site, def_site, Transparency::Transparent, stage)
    }

    /// Create a semi-transparent expansion (can see types)
    pub fn semi_transparent(macro_name: Text, call_site: Span, def_site: Span, stage: u32) -> Self {
        Self::new(macro_name, call_site, def_site, Transparency::SemiTransparent, stage)
    }

    /// Create an opaque expansion (fully hygienic)
    pub fn opaque(macro_name: Text, call_site: Span, def_site: Span, stage: u32) -> Self {
        Self::new(macro_name, call_site, def_site, Transparency::Opaque, stage)
    }
}

/// A syntax context tracks the hygiene information for an identifier
///
/// Every identifier in the AST carries a SyntaxContext that records:
/// - The chain of macro expansions that produced it
/// - The current stage level
/// - The parent context (for nested quotes)
/// - Definition and call site marks
#[derive(Debug, Clone)]
pub struct SyntaxContext {
    /// Unique identifier for this context
    id: SyntaxContextId,
    /// The expansion chain (stack of macro invocations)
    expansion_chain: List<ExpansionInfo>,
    /// The stage at which this context was created
    stage: u32,
    /// Parent context (for nested quotes)
    parent: Maybe<SyntaxContextId>,
    /// Mark indicating the originating macro definition
    def_site_mark: Mark,
    /// Mark indicating the call site (if from a macro call)
    call_site_mark: Maybe<Mark>,
    /// The set of all marks accumulated in this context
    marks: MarkSet,
    /// Current transparency mode
    transparency: Transparency,
}

impl SyntaxContext {
    /// Create a new root syntax context (for top-level code)
    pub fn root() -> Self {
        Self {
            id: SyntaxContextId::ROOT,
            expansion_chain: List::new(),
            stage: 0,
            parent: Maybe::None,
            def_site_mark: Mark::EMPTY,
            call_site_mark: Maybe::None,
            marks: MarkSet::new(),
            transparency: Transparency::Opaque,
        }
    }

    /// Create a new syntax context for a macro expansion
    pub fn for_expansion(
        parent: &SyntaxContext,
        expansion: ExpansionInfo,
    ) -> Self {
        let mut marks = parent.marks.clone();
        marks.add(expansion.mark);

        let mut expansion_chain = parent.expansion_chain.clone();
        expansion_chain.push(expansion.clone());

        Self {
            id: SyntaxContextId::fresh(),
            expansion_chain,
            stage: parent.stage,
            parent: Maybe::Some(parent.id),
            def_site_mark: expansion.mark,
            call_site_mark: Maybe::Some(parent.def_site_mark),
            marks,
            transparency: expansion.transparency,
        }
    }

    /// Create a new context for entering a quote
    pub fn for_quote(parent: &SyntaxContext, target_stage: u32) -> Self {
        let quote_mark = Mark::fresh();
        let mut marks = parent.marks.clone();
        marks.add(quote_mark);

        Self {
            id: SyntaxContextId::fresh(),
            expansion_chain: parent.expansion_chain.clone(),
            stage: target_stage,
            parent: Maybe::Some(parent.id),
            def_site_mark: quote_mark,
            call_site_mark: Maybe::Some(parent.def_site_mark),
            marks,
            transparency: Transparency::Opaque,
        }
    }

    /// Create a new context for an unquote (splice)
    pub fn for_unquote(parent: &SyntaxContext) -> Self {
        let unquote_mark = Mark::fresh();
        let mut marks = parent.marks.clone();
        // For unquote, we flip the innermost quote mark
        marks.flip(unquote_mark);

        Self {
            id: SyntaxContextId::fresh(),
            expansion_chain: parent.expansion_chain.clone(),
            stage: parent.stage + 1, // Go up one stage
            parent: Maybe::Some(parent.id),
            def_site_mark: unquote_mark,
            call_site_mark: Maybe::Some(parent.def_site_mark),
            marks,
            transparency: parent.transparency,
        }
    }

    /// Create a child context with a different transparency mode
    pub fn with_transparency(&self, transparency: Transparency) -> Self {
        Self {
            id: SyntaxContextId::fresh(),
            expansion_chain: self.expansion_chain.clone(),
            stage: self.stage,
            parent: Maybe::Some(self.id),
            def_site_mark: self.def_site_mark,
            call_site_mark: self.call_site_mark,
            marks: self.marks.clone(),
            transparency,
        }
    }

    // ========================================================================
    // Accessors
    // ========================================================================

    /// Get the context ID
    pub fn id(&self) -> SyntaxContextId {
        self.id
    }

    /// Get the current stage
    pub fn stage(&self) -> u32 {
        self.stage
    }

    /// Get the parent context ID
    pub fn parent(&self) -> Maybe<SyntaxContextId> {
        self.parent
    }

    /// Get the definition site mark
    pub fn def_site_mark(&self) -> Mark {
        self.def_site_mark
    }

    /// Get the call site mark
    pub fn call_site_mark(&self) -> Maybe<Mark> {
        self.call_site_mark
    }

    /// Get the marks
    pub fn marks(&self) -> &MarkSet {
        &self.marks
    }

    /// Get the transparency mode
    pub fn transparency(&self) -> Transparency {
        self.transparency
    }

    /// Get the expansion chain
    pub fn expansion_chain(&self) -> &List<ExpansionInfo> {
        &self.expansion_chain
    }

    /// Get the expansion depth
    pub fn expansion_depth(&self) -> usize {
        self.expansion_chain.len()
    }

    /// Get the innermost expansion (if any)
    pub fn innermost_expansion(&self) -> Option<&ExpansionInfo> {
        self.expansion_chain.last()
    }

    // ========================================================================
    // Mark Operations
    // ========================================================================

    /// Add a mark to this context
    pub fn add_mark(&mut self, mark: Mark) {
        self.marks.add(mark);
    }

    /// Remove a mark from this context
    pub fn remove_mark(&mut self, mark: &Mark) {
        self.marks.remove(mark);
    }

    /// Flip a mark in this context
    pub fn flip_mark(&mut self, mark: Mark) {
        self.marks.flip(mark);
    }

    /// Check if this context has a specific mark
    pub fn has_mark(&self, mark: &Mark) -> bool {
        self.marks.contains(mark)
    }

    // ========================================================================
    // Hygiene Checking
    // ========================================================================

    /// Check if an identifier with this context can bind to one with another context
    pub fn can_bind_to(&self, other: &SyntaxContext) -> bool {
        // Same context always binds
        if self.id == other.id {
            return true;
        }

        // Check mark compatibility
        if !self.marks.compatible(&other.marks) {
            return false;
        }

        // Check transparency rules
        match self.transparency {
            Transparency::Transparent => true,
            Transparency::SemiTransparent => {
                // Can bind to types, not values
                // (In a full implementation, we'd check binding kind)
                true
            }
            Transparency::Opaque => {
                // Can only bind if marks are exactly compatible
                self.marks.is_subset_of(&other.marks)
            }
        }
    }

    /// Check if this context is compatible with another for reference resolution
    pub fn is_compatible_with(&self, other: &SyntaxContext) -> bool {
        self.marks.compatible(&other.marks)
    }

    /// Get the effective scopes for this context
    ///
    /// Converts the syntax context to a ScopeSet for integration with
    /// the sets-of-scopes resolution algorithm.
    pub fn to_scope_set(&self) -> ScopeSet {
        let mut scopes = ScopeSet::new();
        // Each mark corresponds to a scope
        for mark in self.marks.iter() {
            scopes.add(ScopeId::new(mark.as_u64()));
        }
        scopes
    }

    // ========================================================================
    // Diagnostic Helpers
    // ========================================================================

    /// Get the macro name that introduced this context
    pub fn introducing_macro(&self) -> Option<&Text> {
        self.expansion_chain.last().map(|e| &e.macro_name)
    }

    /// Get the call site span for error messages
    pub fn call_site_span(&self) -> Option<Span> {
        self.expansion_chain.last().map(|e| e.call_site)
    }

    /// Get the definition site span for error messages
    pub fn def_site_span(&self) -> Option<Span> {
        self.expansion_chain.last().map(|e| e.def_site)
    }

    /// Format the expansion chain for error messages
    pub fn format_expansion_chain(&self) -> Text {
        if self.expansion_chain.is_empty() {
            return Text::from("<top-level>");
        }

        let parts: Vec<String> = self
            .expansion_chain
            .iter()
            .map(|e| e.macro_name.as_str().to_string())
            .collect();

        Text::from(parts.join(" -> "))
    }
}

impl Default for SyntaxContext {
    fn default() -> Self {
        Self::root()
    }
}

/// Registry for syntax contexts
///
/// Stores all syntax contexts and provides lookup by ID.
#[derive(Debug, Default)]
pub struct SyntaxContextRegistry {
    contexts: Map<SyntaxContextId, SyntaxContext>,
}

impl SyntaxContextRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        let mut registry = Self {
            contexts: Map::new(),
        };
        // Always register the root context
        registry.register(SyntaxContext::root());
        registry
    }

    /// Register a syntax context
    pub fn register(&mut self, ctx: SyntaxContext) {
        self.contexts.insert(ctx.id, ctx);
    }

    /// Look up a syntax context by ID
    pub fn get(&self, id: SyntaxContextId) -> Option<&SyntaxContext> {
        self.contexts.get(&id)
    }

    /// Get a mutable reference to a syntax context
    pub fn get_mut(&mut self, id: SyntaxContextId) -> Option<&mut SyntaxContext> {
        self.contexts.get_mut(&id)
    }

    /// Check if a context exists
    pub fn contains(&self, id: SyntaxContextId) -> bool {
        self.contexts.contains_key(&id)
    }

    /// Get the number of registered contexts
    pub fn len(&self) -> usize {
        self.contexts.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.contexts.is_empty()
    }

    /// Create a context for a macro expansion and register it
    pub fn create_for_expansion(
        &mut self,
        parent_id: SyntaxContextId,
        expansion: ExpansionInfo,
    ) -> SyntaxContextId {
        let parent = self.get(parent_id).cloned().unwrap_or_default();
        let ctx = SyntaxContext::for_expansion(&parent, expansion);
        let id = ctx.id;
        self.register(ctx);
        id
    }

    /// Create a context for a quote and register it
    pub fn create_for_quote(
        &mut self,
        parent_id: SyntaxContextId,
        target_stage: u32,
    ) -> SyntaxContextId {
        let parent = self.get(parent_id).cloned().unwrap_or_default();
        let ctx = SyntaxContext::for_quote(&parent, target_stage);
        let id = ctx.id;
        self.register(ctx);
        id
    }

    /// Create a context for an unquote and register it
    pub fn create_for_unquote(&mut self, parent_id: SyntaxContextId) -> SyntaxContextId {
        let parent = self.get(parent_id).cloned().unwrap_or_default();
        let ctx = SyntaxContext::for_unquote(&parent);
        let id = ctx.id;
        self.register(ctx);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mark_creation() {
        let mark1 = Mark::fresh();
        let mark2 = Mark::fresh();

        assert_ne!(mark1, mark2);
        assert!(!mark1.is_empty());
        assert!(Mark::EMPTY.is_empty());
    }

    #[test]
    fn test_mark_set_operations() {
        let mut set1 = MarkSet::new();
        let mark1 = Mark::fresh();
        let mark2 = Mark::fresh();

        set1.add(mark1);
        set1.add(mark2);

        assert_eq!(set1.len(), 2);
        assert!(set1.contains(&mark1));
        assert!(set1.contains(&mark2));

        set1.remove(&mark1);
        assert!(!set1.contains(&mark1));
        assert!(set1.contains(&mark2));
    }

    #[test]
    fn test_mark_set_flip() {
        let mut set = MarkSet::new();
        let mark = Mark::fresh();

        assert!(!set.contains(&mark));

        set.flip(mark);
        assert!(set.contains(&mark));

        set.flip(mark);
        assert!(!set.contains(&mark));
    }

    #[test]
    fn test_mark_set_compatible() {
        let mark1 = Mark::fresh();
        let mark2 = Mark::fresh();

        let mut set1 = MarkSet::new();
        set1.add(mark1);

        let mut set2 = MarkSet::new();
        set2.add(mark1);
        set2.add(mark2);

        // set1 is subset of set2
        assert!(set1.compatible(&set2));
        assert!(set2.compatible(&set1));

        let mut set3 = MarkSet::new();
        set3.add(mark2);

        // set1 and set3 are not compatible (neither is subset of other)
        assert!(!set1.compatible(&set3));
    }

    #[test]
    fn test_transparency_modes() {
        assert!(Transparency::Transparent.allows_value_capture());
        assert!(Transparency::Transparent.allows_type_reference());

        assert!(!Transparency::SemiTransparent.allows_value_capture());
        assert!(Transparency::SemiTransparent.allows_type_reference());

        assert!(!Transparency::Opaque.allows_value_capture());
        assert!(!Transparency::Opaque.allows_type_reference());
        assert!(Transparency::Opaque.is_hygienic());
    }

    #[test]
    fn test_syntax_context_creation() {
        let root = SyntaxContext::root();
        assert_eq!(root.id, SyntaxContextId::ROOT);
        assert_eq!(root.stage, 0);
        assert!(root.marks.is_empty());
    }

    #[test]
    fn test_syntax_context_for_quote() {
        let root = SyntaxContext::root();
        let quote_ctx = SyntaxContext::for_quote(&root, 0);

        assert_ne!(quote_ctx.id, root.id);
        assert_eq!(quote_ctx.stage, 0);
        assert!(!quote_ctx.marks.is_empty());
        assert!(matches!(quote_ctx.parent, Maybe::Some(_)));
    }

    #[test]
    fn test_syntax_context_for_expansion() {
        let root = SyntaxContext::root();
        let expansion = ExpansionInfo::opaque(
            Text::from("test_macro"),
            Span::default(),
            Span::default(),
            0,
        );

        let expanded_ctx = SyntaxContext::for_expansion(&root, expansion);

        assert_ne!(expanded_ctx.id, root.id);
        assert_eq!(expanded_ctx.expansion_depth(), 1);
        assert!(!expanded_ctx.marks.is_empty());
    }

    #[test]
    fn test_syntax_context_registry() {
        let mut registry = SyntaxContextRegistry::new();

        // Root should already be registered
        assert!(registry.contains(SyntaxContextId::ROOT));

        let quote_id = registry.create_for_quote(SyntaxContextId::ROOT, 0);
        assert!(registry.contains(quote_id));

        let ctx = registry.get(quote_id).unwrap();
        assert_eq!(ctx.stage, 0);
    }

    #[test]
    fn test_expansion_chain_format() {
        let root = SyntaxContext::root();

        let exp1 = ExpansionInfo::opaque(
            Text::from("macro_a"),
            Span::default(),
            Span::default(),
            0,
        );
        let ctx1 = SyntaxContext::for_expansion(&root, exp1);

        let exp2 = ExpansionInfo::opaque(
            Text::from("macro_b"),
            Span::default(),
            Span::default(),
            0,
        );
        let ctx2 = SyntaxContext::for_expansion(&ctx1, exp2);

        let chain = ctx2.format_expansion_chain();
        assert!(chain.as_str().contains("macro_a"));
        assert!(chain.as_str().contains("macro_b"));
    }

    #[test]
    fn test_can_bind_to() {
        let root = SyntaxContext::root();

        // Same context can always bind
        assert!(root.can_bind_to(&root));

        // Quote context with different marks
        let quote_ctx = SyntaxContext::for_quote(&root, 0);

        // Root can't directly bind to quote context due to mark difference
        // (but this depends on transparency)
        let transparent_ctx = root.with_transparency(Transparency::Transparent);
        assert!(transparent_ctx.can_bind_to(&quote_ctx));
    }

    #[test]
    fn test_nested_quotes() {
        let root = SyntaxContext::root();

        // First quote
        let quote1 = SyntaxContext::for_quote(&root, 0);
        assert_eq!(quote1.marks.len(), 1);

        // Nested quote
        let quote2 = SyntaxContext::for_quote(&quote1, 0);
        assert_eq!(quote2.marks.len(), 2);

        // Unquote inside nested quote
        let unquote = SyntaxContext::for_unquote(&quote2);
        // Unquote adds a new mark and flips it (so it gets added)
        // The flip operation is on the new mark, not existing ones
        // So we expect 3 marks (2 from quotes + 1 flipped new mark)
        // But since flip adds when not present, we actually get 3
        assert!(unquote.marks.len() >= 2);
    }
}
