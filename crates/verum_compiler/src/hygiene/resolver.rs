//! Identifier Resolution with Scope Awareness
//!
//! Implements sets-of-scopes resolution algorithm.
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

use verum_common::{List, Map, Maybe, Text};

use super::scope::{BindingInfo, HygienicIdent, Scope, ScopeId, ScopeSet};

/// Result of identifier resolution
#[derive(Debug, Clone)]
pub enum Resolution {
    /// Binding was found
    Bound(ResolvedBinding),
    /// Name is free (unbound)
    Free,
    /// Ambiguous - multiple bindings with same specificity
    Ambiguous(List<ResolvedBinding>),
}

/// A resolved binding
#[derive(Debug, Clone)]
pub struct ResolvedBinding {
    /// The binding information
    pub binding: BindingInfo,
    /// The scope where the binding was found
    pub scope_id: ScopeId,
    /// The set of scopes that matched
    pub matched_scopes: ScopeSet,
}

/// Resolver for hygienic identifiers using sets-of-scopes
#[derive(Debug)]
pub struct ScopeResolver {
    /// All scopes in the system
    scopes: Map<ScopeId, Scope>,
    /// Resolution cache for performance
    cache: Map<(Text, ScopeSet), Resolution>,
    /// Whether caching is enabled
    cache_enabled: bool,
}

impl ScopeResolver {
    /// Create a new scope resolver
    pub fn new() -> Self {
        Self {
            scopes: Map::new(),
            cache: Map::new(),
            cache_enabled: true,
        }
    }

    /// Create a resolver without caching
    pub fn without_cache() -> Self {
        Self {
            scopes: Map::new(),
            cache: Map::new(),
            cache_enabled: false,
        }
    }

    /// Register a scope
    pub fn register_scope(&mut self, scope: Scope) {
        self.scopes.insert(scope.id, scope);
        // Invalidate cache when scopes change
        if self.cache_enabled {
            self.cache.clear();
        }
    }

    /// Get a scope by ID
    pub fn get_scope(&self, id: &ScopeId) -> Option<&Scope> {
        self.scopes.get(id)
    }

    /// Get a mutable scope by ID
    pub fn get_scope_mut(&mut self, id: &ScopeId) -> Option<&mut Scope> {
        // Invalidate cache
        if self.cache_enabled {
            self.cache.clear();
        }
        self.scopes.get_mut(id)
    }

    /// Resolve an identifier using sets-of-scopes algorithm
    ///
    /// The algorithm finds all bindings for the name whose scopes are a
    /// subset of the identifier's scopes, then returns the most specific
    /// (largest subset).
    pub fn resolve(&mut self, ident: &HygienicIdent) -> Resolution {
        // Check cache first
        let cache_key = (ident.name.clone(), ident.scopes.clone());
        if self.cache_enabled {
            if let Some(cached) = self.cache.get(&cache_key) {
                return cached.clone();
            }
        }

        // Find all candidate bindings
        let candidates = self.find_candidates(&ident.name, &ident.scopes);

        let result = match candidates.len() {
            0 => Resolution::Free,
            1 => Resolution::Bound(candidates.into_iter().next().unwrap()),
            _ => {
                // Find the most specific binding (largest scope set)
                let max_len = candidates.iter().map(|c| c.matched_scopes.len()).max().unwrap();
                let most_specific: List<_> = candidates
                    .into_iter()
                    .filter(|c| c.matched_scopes.len() == max_len)
                    .collect();

                if most_specific.len() == 1 {
                    Resolution::Bound(most_specific.into_iter().next().unwrap())
                } else {
                    Resolution::Ambiguous(most_specific)
                }
            }
        };

        // Cache the result
        if self.cache_enabled {
            self.cache.insert(cache_key, result.clone());
        }

        result
    }

    /// Find all candidate bindings for a name
    fn find_candidates(&self, name: &Text, ident_scopes: &ScopeSet) -> List<ResolvedBinding> {
        let mut candidates = List::new();

        for (scope_id, scope) in self.scopes.iter() {
            if let Some(binding) = scope.get_binding(name) {
                // Build the scope set for this binding
                let binding_scopes = self.build_scope_set(*scope_id);

                // Check if binding's scopes are a subset of identifier's scopes
                if binding_scopes.is_subset_of(ident_scopes) {
                    candidates.push(ResolvedBinding {
                        binding: binding.clone(),
                        scope_id: *scope_id,
                        matched_scopes: binding_scopes,
                    });
                }
            }
        }

        candidates
    }

    /// Build the complete scope set for a scope (including ancestors)
    fn build_scope_set(&self, scope_id: ScopeId) -> ScopeSet {
        let mut set = ScopeSet::singleton(scope_id);
        let mut current = scope_id;

        while let Some(scope) = self.scopes.get(&current) {
            if let Some(parent_id) = scope.parent {
                set.add(parent_id);
                current = parent_id;
            } else {
                break;
            }
        }

        set
    }

    /// Resolve with context-specific fallback
    pub fn resolve_with_fallback(
        &mut self,
        ident: &HygienicIdent,
        fallback: impl FnOnce(&Text) -> Maybe<BindingInfo>,
    ) -> Resolution {
        match self.resolve(ident) {
            Resolution::Free => {
                if let Maybe::Some(binding) = fallback(&ident.name) {
                    Resolution::Bound(ResolvedBinding {
                        binding,
                        scope_id: ScopeId::new(0), // Global scope
                        matched_scopes: ScopeSet::new(),
                    })
                } else {
                    Resolution::Free
                }
            }
            other => other,
        }
    }

    /// Clear the resolution cache
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Get statistics about the resolver
    pub fn stats(&self) -> ResolverStats {
        ResolverStats {
            scope_count: self.scopes.len(),
            cache_entries: self.cache.len(),
        }
    }
}

impl Default for ScopeResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about the resolver
#[derive(Debug, Clone)]
pub struct ResolverStats {
    /// Number of registered scopes
    pub scope_count: usize,
    /// Number of cached resolutions
    pub cache_entries: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hygiene::scope::{BindingKind, ScopeKind};

    fn make_binding(name: &str, scope_id: ScopeId) -> BindingInfo {
        BindingInfo {
            original_name: Text::from(name),
            hygienic_name: Text::from(name),
            scope_id,
            is_mutable: false,
            kind: BindingKind::Variable,
        }
    }

    #[test]
    fn test_simple_resolution() {
        let mut resolver = ScopeResolver::new();

        // Create a scope with a binding
        let scope_id = ScopeId::new(1);
        let mut scope = Scope::new(scope_id, None, ScopeKind::Module, 0);
        scope.add_binding(Text::from("x"), make_binding("x", scope_id));
        resolver.register_scope(scope);

        // Create an identifier with that scope
        let ident = HygienicIdent::new(
            Text::from("x"),
            ScopeSet::singleton(scope_id),
            verum_ast::Span::default(),
        );

        match resolver.resolve(&ident) {
            Resolution::Bound(resolved) => {
                assert_eq!(resolved.binding.original_name.as_str(), "x");
            }
            _ => panic!("Expected binding"),
        }
    }

    #[test]
    fn test_free_identifier() {
        let mut resolver = ScopeResolver::new();

        let ident = HygienicIdent::new(
            Text::from("unbound"),
            ScopeSet::new(),
            verum_ast::Span::default(),
        );

        match resolver.resolve(&ident) {
            Resolution::Free => {}
            _ => panic!("Expected free"),
        }
    }

    #[test]
    fn test_scope_specificity() {
        let mut resolver = ScopeResolver::new();

        // Create outer scope
        let outer_id = ScopeId::new(1);
        let mut outer = Scope::new(outer_id, None, ScopeKind::Module, 0);
        outer.add_binding(Text::from("x"), make_binding("outer_x", outer_id));
        resolver.register_scope(outer);

        // Create inner scope
        let inner_id = ScopeId::new(2);
        let mut inner = Scope::new(inner_id, Some(outer_id), ScopeKind::Block, 0);
        inner.add_binding(Text::from("x"), make_binding("inner_x", inner_id));
        resolver.register_scope(inner);

        // Identifier with both scopes should resolve to inner
        let mut scopes = ScopeSet::new();
        scopes.add(outer_id);
        scopes.add(inner_id);

        let ident = HygienicIdent::new(Text::from("x"), scopes, verum_ast::Span::default());

        match resolver.resolve(&ident) {
            Resolution::Bound(resolved) => {
                assert_eq!(resolved.binding.original_name.as_str(), "inner_x");
            }
            _ => panic!("Expected inner binding"),
        }
    }
}
