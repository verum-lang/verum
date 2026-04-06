//! Scope Definitions for Hygiene System
//!
//! Implements sets-of-scopes hygiene (Scheme/Racket model).
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

use verum_common::{Map, Text};

/// Unique identifier for a scope
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScopeId(pub u64);

impl ScopeId {
    /// Create a new scope ID
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Get the raw ID value
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

/// Kind of scope (determines scope coloring rules)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScopeKind {
    /// Module-level scope
    Module,
    /// Function body scope
    Function,
    /// Let binding scope
    LetBinding,
    /// Macro definition scope
    MacroDef,
    /// Macro use (expansion) scope
    MacroUse,
    /// Quote expression scope
    Quote,
    /// Unquote (splice) scope
    Unquote,
    /// Block expression scope
    Block,
    /// Loop body scope
    Loop,
    /// Match arm scope
    MatchArm,
    /// For loop scope
    ForLoop,
    /// If condition scope
    IfCondition,
}

impl ScopeKind {
    /// Check if this scope kind introduces bindings
    pub fn introduces_bindings(&self) -> bool {
        matches!(
            self,
            ScopeKind::Function
                | ScopeKind::LetBinding
                | ScopeKind::MacroDef
                | ScopeKind::MatchArm
                | ScopeKind::ForLoop
        )
    }

    /// Check if this scope is macro-related
    pub fn is_macro_related(&self) -> bool {
        matches!(
            self,
            ScopeKind::MacroDef | ScopeKind::MacroUse | ScopeKind::Quote | ScopeKind::Unquote
        )
    }
}

/// A scope in the hygiene system
#[derive(Debug, Clone)]
pub struct Scope {
    /// Unique identifier
    pub id: ScopeId,
    /// Parent scope (if any)
    pub parent: Option<ScopeId>,
    /// Kind of scope
    pub kind: ScopeKind,
    /// Expansion phase when this scope was created
    pub phase: u32,
    /// Bindings introduced in this scope
    bindings: Map<Text, BindingInfo>,
}

impl Scope {
    /// Create a new scope
    pub fn new(id: ScopeId, parent: Option<ScopeId>, kind: ScopeKind, phase: u32) -> Self {
        Self {
            id,
            parent,
            kind,
            phase,
            bindings: Map::new(),
        }
    }

    /// Add a binding to this scope
    pub fn add_binding(&mut self, name: Text, info: BindingInfo) {
        self.bindings.insert(name, info);
    }

    /// Look up a binding in this scope
    pub fn get_binding(&self, name: &Text) -> Option<&BindingInfo> {
        self.bindings.get(name)
    }

    /// Check if this scope has a binding
    pub fn has_binding(&self, name: &Text) -> bool {
        self.bindings.contains_key(name)
    }

    /// Get all bindings in this scope
    pub fn bindings(&self) -> impl Iterator<Item = (&Text, &BindingInfo)> {
        self.bindings.iter()
    }
}

/// Information about a binding
#[derive(Debug, Clone)]
pub struct BindingInfo {
    /// The original name of the binding
    pub original_name: Text,
    /// The hygienic name (may differ from original)
    pub hygienic_name: Text,
    /// The scope where this binding was introduced
    pub scope_id: ScopeId,
    /// Whether this binding is mutable
    pub is_mutable: bool,
    /// Kind of binding
    pub kind: BindingKind,
}

/// Kind of binding
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingKind {
    /// Variable binding (let)
    Variable,
    /// Function parameter
    Parameter,
    /// Function definition
    Function,
    /// Type binding
    Type,
    /// Macro binding
    Macro,
    /// Pattern binding in match
    Pattern,
    /// Loop label
    Label,
}

/// A set of scopes (for sets-of-scopes hygiene)
///
/// An identifier's meaning is determined by the set of scopes it carries.
/// Resolution finds the binding whose scopes are a subset of the identifier's
/// scopes, preferring the most specific (largest subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSet {
    /// The set of scope IDs
    scopes: BTreeSet<ScopeId>,
}

impl ScopeSet {
    /// Create an empty scope set
    pub fn new() -> Self {
        Self {
            scopes: BTreeSet::new(),
        }
    }

    /// Create a scope set with a single scope
    pub fn singleton(scope: ScopeId) -> Self {
        let mut scopes = BTreeSet::new();
        scopes.insert(scope);
        Self { scopes }
    }

    /// Create a scope set from an iterator
    pub fn from_iter(iter: impl IntoIterator<Item = ScopeId>) -> Self {
        Self {
            scopes: iter.into_iter().collect(),
        }
    }

    /// Add a scope to this set
    pub fn add(&mut self, scope: ScopeId) {
        self.scopes.insert(scope);
    }

    /// Remove a scope from this set
    pub fn remove(&mut self, scope: &ScopeId) {
        self.scopes.remove(scope);
    }

    /// Check if this set contains a scope
    pub fn contains(&self, scope: &ScopeId) -> bool {
        self.scopes.contains(scope)
    }

    /// Check if this set is a subset of another
    pub fn is_subset_of(&self, other: &ScopeSet) -> bool {
        self.scopes.is_subset(&other.scopes)
    }

    /// Check if this set is a superset of another
    pub fn is_superset_of(&self, other: &ScopeSet) -> bool {
        self.scopes.is_superset(&other.scopes)
    }

    /// Get the intersection with another set
    pub fn intersection(&self, other: &ScopeSet) -> ScopeSet {
        Self {
            scopes: self.scopes.intersection(&other.scopes).copied().collect(),
        }
    }

    /// Get the union with another set
    pub fn union(&self, other: &ScopeSet) -> ScopeSet {
        Self {
            scopes: self.scopes.union(&other.scopes).copied().collect(),
        }
    }

    /// Get the difference from another set
    pub fn difference(&self, other: &ScopeSet) -> ScopeSet {
        Self {
            scopes: self.scopes.difference(&other.scopes).copied().collect(),
        }
    }

    /// Check if this set is empty
    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }

    /// Get the number of scopes in this set
    pub fn len(&self) -> usize {
        self.scopes.len()
    }

    /// Iterate over the scopes
    pub fn iter(&self) -> impl Iterator<Item = &ScopeId> {
        self.scopes.iter()
    }
}

impl Default for ScopeSet {
    fn default() -> Self {
        Self::new()
    }
}

impl Hash for ScopeSet {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // BTreeSet maintains sorted order, so hashing elements in iteration
        // order produces consistent hashes for equal sets
        self.scopes.len().hash(state);
        for scope_id in &self.scopes {
            scope_id.hash(state);
        }
    }
}

impl FromIterator<ScopeId> for ScopeSet {
    fn from_iter<T: IntoIterator<Item = ScopeId>>(iter: T) -> Self {
        Self {
            scopes: iter.into_iter().collect(),
        }
    }
}

/// A hygienic identifier with scope information
#[derive(Debug, Clone)]
pub struct HygienicIdent {
    /// The name of the identifier
    pub name: Text,
    /// The set of scopes this identifier carries
    pub scopes: ScopeSet,
    /// Source span (for diagnostics)
    pub span: verum_ast::Span,
}

impl HygienicIdent {
    /// Create a new hygienic identifier
    pub fn new(name: Text, scopes: ScopeSet, span: verum_ast::Span) -> Self {
        Self { name, scopes, span }
    }

    /// Create an identifier with no scopes (unhygienic)
    pub fn unhygienic(name: Text, span: verum_ast::Span) -> Self {
        Self {
            name,
            scopes: ScopeSet::new(),
            span,
        }
    }

    /// Add a scope to this identifier
    pub fn with_scope(mut self, scope: ScopeId) -> Self {
        self.scopes.add(scope);
        self
    }

    /// Remove a scope from this identifier
    pub fn without_scope(mut self, scope: &ScopeId) -> Self {
        self.scopes.remove(scope);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_set_operations() {
        let mut set1 = ScopeSet::new();
        set1.add(ScopeId::new(1));
        set1.add(ScopeId::new(2));

        let mut set2 = ScopeSet::new();
        set2.add(ScopeId::new(2));
        set2.add(ScopeId::new(3));

        let intersection = set1.intersection(&set2);
        assert_eq!(intersection.len(), 1);
        assert!(intersection.contains(&ScopeId::new(2)));

        let union = set1.union(&set2);
        assert_eq!(union.len(), 3);
    }

    #[test]
    fn test_scope_set_subset() {
        let mut small = ScopeSet::new();
        small.add(ScopeId::new(1));

        let mut large = ScopeSet::new();
        large.add(ScopeId::new(1));
        large.add(ScopeId::new(2));

        assert!(small.is_subset_of(&large));
        assert!(!large.is_subset_of(&small));
    }

    #[test]
    fn test_scope_kinds() {
        assert!(ScopeKind::Function.introduces_bindings());
        assert!(!ScopeKind::Block.introduces_bindings());
        assert!(ScopeKind::MacroDef.is_macro_related());
    }
}
