//! Context Groups - Reusable context sets
//!
//! Context group expansion: resolving context group names to their constituent contexts recursively — Context Requirements
//!
//! Context groups allow defining reusable sets of contexts that are commonly
//! used together. Groups are defined with the `using` keyword:
//!
//! ```verum
//! using WebContext = [Database, Logger, Auth, Metrics]
//! ```
//!
//! Functions can then use the group instead of listing all contexts:
//!
//! ```verum
//! fn handle_request() using WebContext { ... }
//! ```

use serde::{Deserialize, Serialize};
use std::fmt;
use verum_common::{List, Map, Maybe, Set, Text};

use super::requirement::{ContextRef, ContextRequirement};

/// Context group - a named set of contexts for reuse
///
/// Context group expansion: resolving context group names to their constituent contexts recursively — Context Requirements
///
/// Context groups provide a way to define reusable sets of contexts.
/// This is particularly useful for cross-cutting concerns that are
/// commonly used together.
///
/// # Properties
///
/// - **name**: The group name (e.g., "WebContext", "Observability")
/// - **contexts**: List of contexts in this group
/// - **doc_comment**: Optional documentation
///
/// # Examples
///
/// ```no_run
/// use verum_types::di::group::ContextGroup;
/// # use verum_types::di::requirement::ContextRef;
/// # let logger_ref = ContextRef::new("Logger".into(), std::any::TypeId::of::<()>());
/// # let db_ref = ContextRef::new("Database".into(), std::any::TypeId::of::<()>());
/// # let auth_ref = ContextRef::new("Auth".into(), std::any::TypeId::of::<()>());
/// # let metrics_ref = ContextRef::new("Metrics".into(), std::any::TypeId::of::<()>());
///
/// let web_context = ContextGroup::new(
///     "WebContext".into(),
///     vec![logger_ref, db_ref, auth_ref, metrics_ref]
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextGroup {
    /// Group name (e.g., "WebContext", "Observability")
    pub name: Text,

    /// Contexts included in this group
    pub contexts: List<ContextRef>,

    /// Documentation comment for this group
    pub doc_comment: Maybe<Text>,
}

/// Registry of context groups for a module or program
///
/// Stores all defined context groups and provides lookup functionality.
#[derive(Debug, Clone, Default)]
pub struct ContextGroupRegistry {
    /// Map from group name to group definition
    groups: Map<Text, ContextGroup>,
}

impl ContextGroup {
    /// Create a new context group
    ///
    /// # Arguments
    ///
    /// * `name` - The group name
    /// * `contexts` - Iterator of context references
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use verum_types::di::group::ContextGroup;
    /// # use verum_types::di::requirement::ContextRef;
    /// # let logger_ref = ContextRef::new("Logger".into(), std::any::TypeId::of::<()>());
    /// # let db_ref = ContextRef::new("Database".into(), std::any::TypeId::of::<()>());
    ///
    /// let group = ContextGroup::new(
    ///     "WebContext".into(),
    ///     vec![logger_ref, db_ref]
    /// );
    /// ```
    pub fn new(name: Text, contexts: impl IntoIterator<Item = ContextRef>) -> Self {
        ContextGroup {
            name,
            contexts: contexts.into_iter().collect(),
            doc_comment: Maybe::None,
        }
    }

    /// Create an empty context group
    ///
    /// # Arguments
    ///
    /// * `name` - The group name
    pub fn empty(name: Text) -> Self {
        ContextGroup {
            name,
            contexts: List::new(),
            doc_comment: Maybe::None,
        }
    }

    /// Add a context to this group
    ///
    /// # Arguments
    ///
    /// * `context` - The context reference to add
    pub fn add_context(&mut self, context: ContextRef) {
        self.contexts.push(context);
    }

    /// Set the documentation comment
    pub fn set_doc_comment(&mut self, doc: Text) {
        self.doc_comment = Maybe::Some(doc);
    }

    /// Get the number of contexts in this group
    pub fn len(&self) -> usize {
        self.contexts.len()
    }

    /// Check if this group is empty
    pub fn is_empty(&self) -> bool {
        self.contexts.is_empty()
    }

    /// Check if this group contains a specific context
    ///
    /// # Arguments
    ///
    /// * `name` - The context name to check
    pub fn contains(&self, name: &str) -> bool {
        self.contexts.iter().any(|c| c.name.as_str() == name)
    }

    /// Expand this group into a context requirement
    ///
    /// Converts the group into a ContextRequirement containing all contexts.
    ///
    /// # Returns
    ///
    /// A ContextRequirement with all contexts from this group
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use verum_types::di::group::ContextGroup;
    /// # use verum_types::di::requirement::ContextRef;
    /// # let contexts = vec![];
    ///
    /// let group = ContextGroup::new("WebContext".into(), contexts);
    /// let requirement = group.expand();
    /// // requirement now contains all contexts from WebContext
    /// ```
    pub fn expand(&self) -> ContextRequirement {
        ContextRequirement::from_contexts(self.contexts.iter().cloned())
    }

    /// Get all context names in this group
    pub fn context_names(&self) -> List<&Text> {
        self.contexts.iter().map(|c| &c.name).collect()
    }

    /// Validate this context group
    ///
    /// Checks:
    /// - At least one context in the group
    /// - No duplicate contexts
    ///
    /// # Returns
    ///
    /// `Ok(())` if valid, `Err(GroupError)` otherwise
    pub fn validate(&self) -> Result<(), GroupError> {
        // Must have at least one context
        if self.contexts.is_empty() {
            return Err(GroupError::EmptyGroup(self.name.clone()));
        }

        // Check for duplicates
        let mut seen = Set::new();
        for ctx in &self.contexts {
            if seen.contains(&ctx.name) {
                return Err(GroupError::DuplicateContext {
                    group: self.name.clone(),
                    context: ctx.name.clone(),
                });
            }
            seen.insert(ctx.name.clone());
        }

        Ok(())
    }

    /// Merge this group with another
    ///
    /// Creates a new group containing contexts from both groups.
    /// Duplicates are removed.
    ///
    /// # Arguments
    ///
    /// * `other` - The other group to merge
    /// * `new_name` - Name for the merged group
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use verum_types::di::group::ContextGroup;
    /// # use verum_types::di::requirement::ContextRef;
    /// # let logger = ContextRef::new("Logger".into(), std::any::TypeId::of::<()>());
    /// # let db = ContextRef::new("Database".into(), std::any::TypeId::of::<()>());
    /// # let auth = ContextRef::new("Auth".into(), std::any::TypeId::of::<()>());
    ///
    /// let web = ContextGroup::new("WebContext".into(), vec![logger, db]);
    /// let admin = ContextGroup::new("AdminContext".into(), vec![auth]);
    /// let combined = web.merge(&admin, "FullContext".into());
    /// // combined has logger, db, and auth
    /// ```
    pub fn merge(&self, other: &Self, new_name: Text) -> Self {
        let mut contexts = self.contexts.clone();

        // Add contexts from other, avoiding duplicates
        for ctx in &other.contexts {
            if !contexts.iter().any(|c| c.name == ctx.name) {
                contexts.push(ctx.clone());
            }
        }

        ContextGroup {
            name: new_name,
            contexts,
            doc_comment: Maybe::None,
        }
    }
}

impl ContextGroupRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        ContextGroupRegistry { groups: Map::new() }
    }

    /// Register a context group
    ///
    /// # Arguments
    ///
    /// * `group` - The group to register
    ///
    /// # Returns
    ///
    /// `Ok(())` if successful, `Err(GroupError)` if group name already exists
    pub fn register(&mut self, group: ContextGroup) -> Result<(), GroupError> {
        if self.groups.contains_key(&group.name) {
            return Err(GroupError::AlreadyDefined(group.name.clone()));
        }

        self.groups.insert(group.name.clone(), group);
        Ok(())
    }

    /// Get a context group by name
    ///
    /// # Arguments
    ///
    /// * `name` - The group name
    ///
    /// # Returns
    ///
    /// `Some(&ContextGroup)` if found, `None` otherwise
    pub fn get(&self, name: &str) -> Maybe<&ContextGroup> {
        self.groups
            .get(&name.into()).and_then(Maybe::Some)
    }

    /// Check if a group is registered
    ///
    /// # Arguments
    ///
    /// * `name` - The group name to check
    pub fn has_group(&self, name: &str) -> bool {
        self.groups.contains_key(&name.into())
    }

    /// Expand a group into a context requirement
    ///
    /// # Arguments
    ///
    /// * `name` - The group name to expand
    ///
    /// # Returns
    ///
    /// `Ok(ContextRequirement)` if found, `Err(GroupError)` otherwise
    pub fn expand(&self, name: &str) -> Result<ContextRequirement, GroupError> {
        match self.get(name) {
            Option::Some(group) => Ok(group.expand()),
            Option::None => Err(GroupError::NotFound(name.into())),
        }
    }

    /// Get all registered group names
    pub fn group_names(&self) -> List<&Text> {
        self.groups.keys().collect()
    }

    /// Get the number of registered groups
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// Clear all registered groups
    pub fn clear(&mut self) {
        self.groups.clear();
    }
}

/// Errors that can occur with context groups
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GroupError {
    /// Context group not found
    #[error("context group '{0}' not found")]
    NotFound(Text),

    /// Context group already defined
    #[error("context group '{0}' already defined")]
    AlreadyDefined(Text),

    /// Context group is empty
    #[error("context group '{0}' is empty")]
    EmptyGroup(Text),

    /// Duplicate context in group
    #[error("duplicate context '{context}' in group '{group}'")]
    DuplicateContext { group: Text, context: Text },
}

impl fmt::Display for ContextGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "using {} = [", self.name)?;
        for (i, ctx) in self.contexts.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", ctx.name)?;
        }
        write!(f, "]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::TypeId;

    fn make_context_ref(name: &str) -> ContextRef {
        ContextRef::new(name.into(), TypeId::of::<()>())
    }

    #[test]
    fn test_create_group() {
        let logger = make_context_ref("Logger");
        let database = make_context_ref("Database");

        let group = ContextGroup::new("WebContext".into(), vec![logger, database]);

        assert_eq!(group.name, "WebContext");
        assert_eq!(group.len(), 2);
        assert!(group.contains("Logger"));
        assert!(group.contains("Database"));
    }

    #[test]
    fn test_empty_group() {
        let group = ContextGroup::empty("EmptyGroup".into());

        assert!(group.is_empty());
        assert_eq!(group.len(), 0);
    }

    #[test]
    fn test_add_context() {
        let mut group = ContextGroup::empty("TestGroup".into());

        group.add_context(make_context_ref("Logger"));
        assert_eq!(group.len(), 1);

        group.add_context(make_context_ref("Database"));
        assert_eq!(group.len(), 2);
    }

    #[test]
    fn test_expand_group() {
        let logger = make_context_ref("Logger");
        let database = make_context_ref("Database");

        let group = ContextGroup::new("WebContext".into(), vec![logger, database]);
        let requirement = group.expand();

        assert_eq!(requirement.len(), 2);
        assert!(requirement.requires("Logger"));
        assert!(requirement.requires("Database"));
    }

    #[test]
    fn test_validate_empty() {
        let group = ContextGroup::empty("Empty".into());

        assert!(matches!(group.validate(), Err(GroupError::EmptyGroup(_))));
    }

    #[test]
    fn test_validate_duplicates() {
        let logger1 = make_context_ref("Logger");
        let logger2 = make_context_ref("Logger");

        let group = ContextGroup::new("Duplicate".into(), vec![logger1, logger2]);

        assert!(matches!(
            group.validate(),
            Err(GroupError::DuplicateContext { .. })
        ));
    }

    #[test]
    fn test_validate_success() {
        let logger = make_context_ref("Logger");
        let database = make_context_ref("Database");

        let group = ContextGroup::new("Valid".into(), vec![logger, database]);

        assert!(group.validate().is_ok());
    }

    #[test]
    fn test_merge_groups() {
        let logger = make_context_ref("Logger");
        let database = make_context_ref("Database");
        let auth = make_context_ref("Auth");

        let group1 = ContextGroup::new("Group1".into(), vec![logger, database]);
        let group2 = ContextGroup::new("Group2".into(), vec![auth]);

        let merged = group1.merge(&group2, "Merged".into());

        assert_eq!(merged.len(), 3);
        assert!(merged.contains("Logger"));
        assert!(merged.contains("Database"));
        assert!(merged.contains("Auth"));
    }

    #[test]
    fn test_merge_removes_duplicates() {
        let logger1 = make_context_ref("Logger");
        let logger2 = make_context_ref("Logger");
        let database = make_context_ref("Database");

        let group1 = ContextGroup::new("Group1".into(), vec![logger1, database]);
        let group2 = ContextGroup::new("Group2".into(), vec![logger2]);

        let merged = group1.merge(&group2, "Merged".into());

        // Should only have 2 contexts (Logger and Database), not 3
        assert_eq!(merged.len(), 2);
        assert!(merged.contains("Logger"));
        assert!(merged.contains("Database"));
    }

    #[test]
    fn test_registry_register() {
        let mut registry = ContextGroupRegistry::new();

        let logger = make_context_ref("Logger");
        let group = ContextGroup::new("WebContext".into(), vec![logger]);

        assert!(registry.register(group).is_ok());
        assert_eq!(registry.len(), 1);
        assert!(registry.has_group("WebContext"));
    }

    #[test]
    fn test_registry_duplicate() {
        let mut registry = ContextGroupRegistry::new();

        let logger = make_context_ref("Logger");
        let group1 = ContextGroup::new("WebContext".into(), vec![logger.clone()]);
        let group2 = ContextGroup::new("WebContext".into(), vec![logger]);

        assert!(registry.register(group1).is_ok());
        assert!(matches!(
            registry.register(group2),
            Err(GroupError::AlreadyDefined(_))
        ));
    }

    #[test]
    fn test_registry_get() {
        let mut registry = ContextGroupRegistry::new();

        let logger = make_context_ref("Logger");
        let group = ContextGroup::new("WebContext".into(), vec![logger]);

        registry.register(group).unwrap();

        let retrieved = registry.get("WebContext");
        assert!(matches!(retrieved, Maybe::Some(_)));

        let missing = registry.get("MissingContext");
        assert!(matches!(missing, Maybe::None));
    }

    #[test]
    fn test_registry_expand() {
        let mut registry = ContextGroupRegistry::new();

        let logger = make_context_ref("Logger");
        let database = make_context_ref("Database");
        let group = ContextGroup::new("WebContext".into(), vec![logger, database]);

        registry.register(group).unwrap();

        let requirement = registry.expand("WebContext").unwrap();
        assert_eq!(requirement.len(), 2);
        assert!(requirement.requires("Logger"));
        assert!(requirement.requires("Database"));
    }

    #[test]
    fn test_registry_expand_not_found() {
        let registry = ContextGroupRegistry::new();

        let result = registry.expand("NonExistent");
        assert!(matches!(result, Err(GroupError::NotFound(_))));
    }

    #[test]
    fn test_context_names() {
        let logger = make_context_ref("Logger");
        let database = make_context_ref("Database");
        let group = ContextGroup::new("WebContext".into(), vec![logger, database]);

        let names = group.context_names();
        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|n| n.as_str() == "Logger"));
        assert!(names.iter().any(|n| n.as_str() == "Database"));
    }
}
