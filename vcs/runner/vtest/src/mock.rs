//! Context mock registry for `@mock: <ContextType>` test directive (#62).
//!
//! When a spec file carries `@mock: Database` (or any other context type name),
//! the test runner uses this registry to look up the mock value that should be
//! injected in place of the real context.
//!
//! # Design
//!
//! The registry is a thin `HashMap<String, MockEntry>` keyed by the unqualified
//! context type name (e.g. `"Database"`, `"Logger"`).  Each entry stores a
//! serialised value (currently a JSON string) that the VBC interpreter can
//! deserialise back into the expected context type at the call site.
//!
//! The runner builds a `MockRegistry` for each test file, populates it with
//! entries from `TestDirectives::mocks`, and passes it through to the executor.
//!
//! # Example workflow
//!
//! ```text
//! // In my_test.vr:
//! // @mock: Database
//! // @mock: Logger
//!
//! // In the test setup code:
//! let mut registry = MockRegistry::new();
//! registry.register("Database", MockEntry::value("{\"pool_size\": 1}"));
//! registry.register("Logger", MockEntry::noop());
//! ```

use std::collections::HashMap;

/// A single mock entry in the registry.
#[derive(Debug, Clone)]
pub struct MockEntry {
    /// Serialised (JSON) mock value passed to the VBC runtime.
    pub value: Option<String>,
    /// If true, any access to this context panics with a clear message.
    pub panic_on_access: bool,
}

impl MockEntry {
    /// A mock that returns a concrete serialised value.
    pub fn value(json: impl Into<String>) -> Self {
        Self { value: Some(json.into()), panic_on_access: false }
    }

    /// A no-op mock that provides an empty/default value.
    pub fn noop() -> Self {
        Self { value: Some("{}".to_string()), panic_on_access: false }
    }

    /// A mock that panics when the context is accessed (useful for ensuring
    /// a code path does not touch a given dependency).
    pub fn forbidden() -> Self {
        Self { value: None, panic_on_access: true }
    }
}

/// Registry of mocks to inject for a single test run.
#[derive(Debug, Default)]
pub struct MockRegistry {
    entries: HashMap<String, MockEntry>,
}

impl MockRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self { entries: HashMap::new() }
    }

    /// Register a mock for the given context type name.
    ///
    /// Overwrites any previously registered mock for the same type.
    pub fn register(&mut self, context_type: impl Into<String>, entry: MockEntry) {
        self.entries.insert(context_type.into(), entry);
    }

    /// Look up the mock entry for a context type.
    ///
    /// Returns `None` if no mock was registered for that type.
    pub fn get(&self, context_type: &str) -> Option<&MockEntry> {
        self.entries.get(context_type)
    }

    /// Returns `true` if at least one mock is registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total number of registered mocks.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns the set of registered context type names.
    pub fn registered_types(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_registry_is_empty() {
        let r = MockRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn register_and_get() {
        let mut r = MockRegistry::new();
        r.register("Database", MockEntry::value("{\"url\": \"sqlite::memory:\"}"));
        let entry = r.get("Database").expect("Database must be registered");
        assert!(!entry.panic_on_access);
        assert_eq!(entry.value.as_deref(), Some("{\"url\": \"sqlite::memory:\"}"));
    }

    #[test]
    fn noop_entry_is_not_panic() {
        let e = MockEntry::noop();
        assert!(!e.panic_on_access);
        assert_eq!(e.value.as_deref(), Some("{}"));
    }

    #[test]
    fn forbidden_entry_has_panic_on_access() {
        let e = MockEntry::forbidden();
        assert!(e.panic_on_access);
        assert!(e.value.is_none());
    }

    #[test]
    fn get_unknown_returns_none() {
        let r = MockRegistry::new();
        assert!(r.get("Logger").is_none());
    }

    #[test]
    fn overwrite_existing_mock() {
        let mut r = MockRegistry::new();
        r.register("Logger", MockEntry::noop());
        r.register("Logger", MockEntry::forbidden());
        let entry = r.get("Logger").unwrap();
        assert!(entry.panic_on_access);
    }

    #[test]
    fn len_counts_distinct_types() {
        let mut r = MockRegistry::new();
        r.register("A", MockEntry::noop());
        r.register("B", MockEntry::noop());
        r.register("A", MockEntry::forbidden()); // overwrite
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn registered_types_iter() {
        let mut r = MockRegistry::new();
        r.register("Database", MockEntry::noop());
        r.register("Cache", MockEntry::noop());
        let mut types: Vec<&str> = r.registered_types().collect();
        types.sort();
        assert_eq!(types, ["Cache", "Database"]);
    }
}
