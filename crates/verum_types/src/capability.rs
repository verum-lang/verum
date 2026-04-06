//! Capability tracking for context attenuation
//!
//! This module implements compile-time tracking of context capabilities
//! Context system: capability-based dependency injection with "context" declarations, "using" requirements, "provide" injection, ~5-30ns runtime overhead via task-local storage — Section 10.
//!
//! # Overview
//!
//! The capability system tracks which capabilities are available for each
//! context at compile time, enabling:
//!
//! - Static verification that operations match available capabilities
//! - Capability intersection during attenuation
//! - Clear error messages for capability violations
//!
//! # Integration with Type System
//!
//! Context types track their associated capabilities:
//!
//! ```text
//! Context<T> with Capabilities[ReadOnly, Query]
//! ```
//!
//! When attenuating, the type system verifies that:
//! 1. New capabilities are a subset of existing capabilities
//! 2. Operations only use available capabilities
//! 3. Capability requirements propagate through function calls

use verum_ast::expr::{Capability, CapabilitySet};
use verum_common::{List, Maybe, Set, Text};

/// Type-level capability tracking for contexts
///
/// This associates a capability set with a context type, enabling
/// compile-time verification of capability requirements.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextCapabilities {
    /// The context name (e.g., "Database", "FileSystem")
    pub context_name: Text,

    /// Set of available capabilities
    pub capabilities: TypeCapabilitySet,

    /// Whether this is an attenuated context
    pub is_attenuated: bool,

    /// Original capabilities (if attenuated)
    pub original_capabilities: Maybe<TypeCapabilitySet>,
}

impl ContextCapabilities {
    /// Create a new context with full capabilities
    pub fn full(context_name: Text) -> Self {
        Self {
            context_name,
            capabilities: TypeCapabilitySet::all(),
            is_attenuated: false,
            original_capabilities: Maybe::None,
        }
    }

    /// Create a new context with specific capabilities
    pub fn with_capabilities(context_name: Text, capabilities: TypeCapabilitySet) -> Self {
        Self {
            context_name,
            capabilities,
            is_attenuated: false,
            original_capabilities: Maybe::None,
        }
    }

    /// Attenuate this context with a new capability set
    ///
    /// Returns a new ContextCapabilities with the intersection of capabilities.
    pub fn attenuate(&self, new_capabilities: TypeCapabilitySet) -> Self {
        let intersected = self.capabilities.intersect(&new_capabilities);
        Self {
            context_name: self.context_name.clone(),
            capabilities: intersected,
            is_attenuated: true,
            original_capabilities: if self.is_attenuated {
                self.original_capabilities.clone()
            } else {
                Maybe::Some(self.capabilities.clone())
            },
        }
    }

    /// Check if a capability is available
    pub fn has_capability(&self, cap: &TypeCapability) -> bool {
        self.capabilities.contains(cap)
    }

    /// Check if this context can satisfy required capabilities
    pub fn satisfies(&self, required: &TypeCapabilitySet) -> bool {
        required.is_subset_of(&self.capabilities)
    }

    /// Get missing capabilities compared to requirements
    pub fn missing_capabilities(&self, required: &TypeCapabilitySet) -> List<TypeCapability> {
        required.difference(&self.capabilities)
    }
}

/// Type-level capability representation
///
/// Similar to AST Capability but used during type checking.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeCapability {
    /// Read-only access
    ReadOnly,
    /// Write-only access
    WriteOnly,
    /// Full read-write access
    ReadWrite,
    /// Administrative privileges
    Admin,
    /// Transaction management
    Transaction,
    /// Network access
    Network,
    /// File system access
    FileSystem,
    /// Database query
    Query,
    /// Database mutation
    Execute,
    /// Logging
    Logging,
    /// Metrics/telemetry
    Metrics,
    /// Configuration
    Config,
    /// Cache access
    Cache,
    /// Authentication
    Auth,
    /// Custom named capability
    Custom(Text),
}

impl TypeCapability {
    /// Convert from AST capability
    pub fn from_ast(cap: &Capability) -> Self {
        match cap {
            Capability::ReadOnly => TypeCapability::ReadOnly,
            Capability::WriteOnly => TypeCapability::WriteOnly,
            Capability::ReadWrite => TypeCapability::ReadWrite,
            Capability::Admin => TypeCapability::Admin,
            Capability::Transaction => TypeCapability::Transaction,
            Capability::Network => TypeCapability::Network,
            Capability::FileSystem => TypeCapability::FileSystem,
            Capability::Query => TypeCapability::Query,
            Capability::Execute => TypeCapability::Execute,
            Capability::Logging => TypeCapability::Logging,
            Capability::Metrics => TypeCapability::Metrics,
            Capability::Config => TypeCapability::Config,
            Capability::Cache => TypeCapability::Cache,
            Capability::Auth => TypeCapability::Auth,
            Capability::Custom(name) => TypeCapability::Custom(name.clone()),
        }
    }

    /// Get display name
    pub fn name(&self) -> &str {
        match self {
            TypeCapability::ReadOnly => "ReadOnly",
            TypeCapability::WriteOnly => "WriteOnly",
            TypeCapability::ReadWrite => "ReadWrite",
            TypeCapability::Admin => "Admin",
            TypeCapability::Transaction => "Transaction",
            TypeCapability::Network => "Network",
            TypeCapability::FileSystem => "FileSystem",
            TypeCapability::Query => "Query",
            TypeCapability::Execute => "Execute",
            TypeCapability::Logging => "Logging",
            TypeCapability::Metrics => "Metrics",
            TypeCapability::Config => "Config",
            TypeCapability::Cache => "Cache",
            TypeCapability::Auth => "Auth",
            TypeCapability::Custom(name) => name.as_str(),
        }
    }
}

/// Set of type-level capabilities
///
/// Used during type checking to track available capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeCapabilitySet {
    capabilities: Set<TypeCapability>,
}

impl TypeCapabilitySet {
    /// Create an empty capability set
    pub fn empty() -> Self {
        Self {
            capabilities: Set::new(),
        }
    }

    /// Create a capability set with all capabilities
    pub fn all() -> Self {
        let mut set = Set::new();
        set.insert(TypeCapability::ReadOnly);
        set.insert(TypeCapability::WriteOnly);
        set.insert(TypeCapability::ReadWrite);
        set.insert(TypeCapability::Admin);
        set.insert(TypeCapability::Transaction);
        set.insert(TypeCapability::Network);
        set.insert(TypeCapability::FileSystem);
        set.insert(TypeCapability::Query);
        set.insert(TypeCapability::Execute);
        set.insert(TypeCapability::Logging);
        set.insert(TypeCapability::Metrics);
        set.insert(TypeCapability::Config);
        set.insert(TypeCapability::Cache);
        set.insert(TypeCapability::Auth);
        Self { capabilities: set }
    }

    /// Create from a list of capabilities
    pub fn from_list(caps: List<TypeCapability>) -> Self {
        let mut set = Set::new();
        for cap in caps {
            set.insert(cap);
        }
        Self { capabilities: set }
    }

    /// Create from AST CapabilitySet
    pub fn from_ast(ast_caps: &CapabilitySet) -> Self {
        let caps: List<TypeCapability> = ast_caps
            .capabilities
            .iter()
            .map(TypeCapability::from_ast)
            .collect();
        Self::from_list(caps)
    }

    /// Add a capability
    pub fn insert(&mut self, cap: TypeCapability) {
        self.capabilities.insert(cap);
    }

    /// Check if a capability is present
    pub fn contains(&self, cap: &TypeCapability) -> bool {
        self.capabilities.contains(cap)
    }

    /// Check if this set is a subset of another
    pub fn is_subset_of(&self, other: &TypeCapabilitySet) -> bool {
        self.capabilities
            .iter()
            .all(|cap| other.capabilities.contains(cap))
    }

    /// Get capabilities in this set but not in another (difference)
    pub fn difference(&self, other: &TypeCapabilitySet) -> List<TypeCapability> {
        self.capabilities
            .iter()
            .filter(|cap| !other.capabilities.contains(cap))
            .cloned()
            .collect()
    }

    /// Intersect with another capability set
    pub fn intersect(&self, other: &TypeCapabilitySet) -> Self {
        let mut set = Set::new();
        for cap in self.capabilities.iter() {
            if other.capabilities.contains(cap) {
                set.insert(cap.clone());
            }
        }
        Self { capabilities: set }
    }

    /// Union with another capability set
    pub fn union(&self, other: &TypeCapabilitySet) -> Self {
        let mut set = self.capabilities.clone();
        for cap in other.capabilities.iter() {
            set.insert(cap.clone());
        }
        Self { capabilities: set }
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }

    /// Get the number of capabilities
    pub fn len(&self) -> usize {
        self.capabilities.len()
    }

    /// Get all capabilities as a list
    pub fn to_list(&self) -> List<TypeCapability> {
        self.capabilities.iter().cloned().collect()
    }

    /// Get capability names for display
    pub fn names(&self) -> List<Text> {
        self.capabilities
            .iter()
            .map(|c| Text::from(c.name()))
            .collect()
    }
}

impl Default for TypeCapabilitySet {
    fn default() -> Self {
        Self::all()
    }
}

/// Method capability mapper
///
/// Maps context methods to their required capabilities based on:
/// 1. Explicit capability annotations (future feature)
/// 2. Method name heuristics
/// 3. Context declaration sub-context membership
pub struct MethodCapabilityMapper {
    /// Custom method-to-capability mappings
    custom_mappings: std::collections::HashMap<(Text, Text), TypeCapabilitySet>,
}

impl MethodCapabilityMapper {
    /// Create a new method capability mapper
    pub fn new() -> Self {
        Self {
            custom_mappings: std::collections::HashMap::new(),
        }
    }

    /// Register a custom capability mapping for a method
    ///
    /// # Arguments
    ///
    /// * `context_name` - The context name (e.g., "Database")
    /// * `method_name` - The method name (e.g., "execute")
    /// * `capabilities` - The required capabilities
    pub fn register_method(
        &mut self,
        context_name: Text,
        method_name: Text,
        capabilities: TypeCapabilitySet,
    ) {
        self.custom_mappings
            .insert((context_name, method_name), capabilities);
    }

    /// Extract capability requirements for a context method
    ///
    /// This implements the method-level capability extraction algorithm:
    /// 1. Check for custom mappings first
    /// 2. Use sub-context information if available
    /// 3. Fall back to method name heuristics
    ///
    /// # Arguments
    ///
    /// * `context_name` - The context name
    /// * `method_name` - The method name
    /// * `context_decl` - Optional context declaration for sub-context lookup
    ///
    /// # Returns
    ///
    /// A TypeCapabilitySet containing the required capabilities
    pub fn extract_method_capabilities(
        &self,
        context_name: &Text,
        method_name: &Text,
        context_decl: Option<&verum_ast::decl::ContextDecl>,
    ) -> TypeCapabilitySet {
        // 1. Check custom mappings first
        if let Some(caps) = self
            .custom_mappings
            .get(&(context_name.clone(), method_name.clone()))
        {
            return caps.clone();
        }

        // 2. Check sub-context membership if context declaration is available
        if let Some(decl) = context_decl
            && let Maybe::Some(caps) = self.extract_from_sub_context(decl, method_name)
        {
            return caps;
        }

        // 3. Fall back to method name heuristics
        self.extract_from_method_name(method_name)
    }

    /// Extract capabilities from sub-context membership
    ///
    /// If a method is defined in a sub-context, use the sub-context name
    /// as a hint for the required capability.
    fn extract_from_sub_context(
        &self,
        context_decl: &verum_ast::decl::ContextDecl,
        method_name: &Text,
    ) -> Maybe<TypeCapabilitySet> {
        // Check each sub-context
        for sub_context in &context_decl.sub_contexts {
            // Check if this method is in this sub-context
            for method in &sub_context.methods {
                if method.name.name.as_str() == method_name.as_str() {
                    // Found it - use sub-context name as capability hint
                    let sub_context_name = sub_context.name.name.as_str();
                    return Maybe::Some(self.capability_from_name(sub_context_name));
                }
            }
        }

        Maybe::None
    }

    /// Extract capabilities from method name using heuristics
    fn extract_from_method_name(&self, method_name: &Text) -> TypeCapabilitySet {
        let method_name_lower = method_name.as_str().to_lowercase();
        let mut caps = TypeCapabilitySet::empty();

        // Admin operations - check first as they are more specific
        if method_name_lower.contains("admin")
            || method_name_lower.contains("migrate")
            || method_name_lower.contains("backup")
            || method_name_lower.contains("restore")
        {
            caps.insert(TypeCapability::Admin);
            return caps;
        }

        // Transaction operations
        if method_name_lower.contains("begin")
            || method_name_lower.contains("commit")
            || method_name_lower.contains("rollback")
            || method_name_lower.contains("transaction")
        {
            caps.insert(TypeCapability::Transaction);
            return caps;
        }

        // Write operations
        if method_name_lower.contains("write")
            || method_name_lower.contains("delete")
            || method_name_lower.contains("update")
            || method_name_lower.contains("insert")
            || method_name_lower.contains("create")
            || method_name_lower.contains("drop")
            || method_name_lower.contains("execute")
            || method_name_lower.starts_with("set_")
        {
            caps.insert(TypeCapability::Execute);
            return caps;
        }

        // Read operations
        if method_name_lower.contains("query")
            || method_name_lower.contains("read")
            || method_name_lower.contains("get")
            || method_name_lower.contains("select")
            || method_name_lower.contains("fetch")
            || method_name_lower.contains("find")
            || method_name_lower.contains("search")
            || method_name_lower.contains("list")
        {
            caps.insert(TypeCapability::Query);
            return caps;
        }

        // Logging
        if method_name_lower.contains("log")
            || method_name_lower.contains("trace")
            || method_name_lower.contains("debug")
            || method_name_lower.contains("info")
            || method_name_lower.contains("warn")
            || method_name_lower.contains("error")
        {
            caps.insert(TypeCapability::Logging);
            return caps;
        }

        // Metrics
        if method_name_lower.contains("metric")
            || method_name_lower.contains("count")
            || method_name_lower.contains("gauge")
            || method_name_lower.contains("histogram")
        {
            caps.insert(TypeCapability::Metrics);
            return caps;
        }

        // Network
        if method_name_lower.contains("connect")
            || method_name_lower.contains("send")
            || method_name_lower.contains("receive")
            || method_name_lower.contains("request")
            || method_name_lower.contains("response")
        {
            caps.insert(TypeCapability::Network);
            return caps;
        }

        // FileSystem
        if method_name_lower.contains("file")
            || method_name_lower.contains("directory")
            || method_name_lower.contains("path")
            || method_name_lower.contains("open")
            || method_name_lower.contains("close")
        {
            caps.insert(TypeCapability::FileSystem);
            return caps;
        }

        // Cache
        if method_name_lower.contains("cache")
            || method_name_lower.contains("invalidate")
            || method_name_lower.contains("evict")
        {
            caps.insert(TypeCapability::Cache);
            return caps;
        }

        // Auth
        if method_name_lower.contains("auth")
            || method_name_lower.contains("login")
            || method_name_lower.contains("logout")
            || method_name_lower.contains("verify")
            || method_name_lower.contains("validate")
        {
            caps.insert(TypeCapability::Auth);
            return caps;
        }

        // Config
        if method_name_lower.contains("config")
            || method_name_lower.contains("setting")
            || method_name_lower.contains("preference")
        {
            caps.insert(TypeCapability::Config);
            return caps;
        }

        // Default: ReadOnly for unknown methods
        caps.insert(TypeCapability::ReadOnly);
        caps
    }

    /// Convert a sub-context name to a capability
    fn capability_from_name(&self, name: &str) -> TypeCapabilitySet {
        let mut caps = TypeCapabilitySet::empty();

        match name.to_lowercase().as_str() {
            "read" | "readonly" => caps.insert(TypeCapability::ReadOnly),
            "query" => caps.insert(TypeCapability::Query),
            "write" | "writeonly" => caps.insert(TypeCapability::WriteOnly),
            "execute" => caps.insert(TypeCapability::Execute),
            "readwrite" => {
                caps.insert(TypeCapability::ReadWrite);
            }
            "admin" | "administrator" => caps.insert(TypeCapability::Admin),
            "transaction" | "tx" => caps.insert(TypeCapability::Transaction),
            "network" | "net" => caps.insert(TypeCapability::Network),
            "filesystem" | "fs" => caps.insert(TypeCapability::FileSystem),
            "logging" | "log" => caps.insert(TypeCapability::Logging),
            "metrics" | "telemetry" => caps.insert(TypeCapability::Metrics),
            "config" | "configuration" => caps.insert(TypeCapability::Config),
            "cache" => caps.insert(TypeCapability::Cache),
            "auth" | "authentication" => caps.insert(TypeCapability::Auth),
            _ => {
                // Default to ReadOnly
                caps.insert(TypeCapability::ReadOnly);
            }
        }

        caps
    }
}

impl Default for MethodCapabilityMapper {
    fn default() -> Self {
        Self::new()
    }
}

/// Capability requirement for a function or operation
///
/// Associates an operation with the capabilities it requires.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityRequirement {
    /// The context name this requirement applies to
    pub context_name: Text,

    /// Required capabilities
    pub required_capabilities: TypeCapabilitySet,

    /// Operation description (for error messages)
    pub operation: Text,
}

impl CapabilityRequirement {
    /// Create a new capability requirement
    pub fn new(
        context_name: Text,
        required_capabilities: TypeCapabilitySet,
        operation: Text,
    ) -> Self {
        Self {
            context_name,
            required_capabilities,
            operation,
        }
    }

    /// Check if this requirement is satisfied by available capabilities
    pub fn is_satisfied_by(&self, available: &ContextCapabilities) -> bool {
        // Check context name matches
        if self.context_name != available.context_name {
            return false;
        }

        // Check capabilities
        available.satisfies(&self.required_capabilities)
    }

    /// Get missing capabilities
    pub fn missing_capabilities(&self, available: &ContextCapabilities) -> List<TypeCapability> {
        available.missing_capabilities(&self.required_capabilities)
    }
}

/// Capability checker for type checking
///
/// Verifies that operations have the required capabilities at compile time.
pub struct CapabilityChecker {
    /// Currently available context capabilities
    context_capabilities: List<ContextCapabilities>,
}

impl CapabilityChecker {
    /// Create a new capability checker
    pub fn new() -> Self {
        Self {
            context_capabilities: List::new(),
        }
    }

    /// Register a context with its capabilities
    pub fn register_context(&mut self, caps: ContextCapabilities) {
        // Replace existing context with same name
        self.context_capabilities
            .retain(|c| c.context_name != caps.context_name);
        self.context_capabilities.push(caps);
    }

    /// Get capabilities for a context
    pub fn get_context_capabilities(&self, context_name: &str) -> Maybe<&ContextCapabilities> {
        self.context_capabilities
            .iter()
            .find(|c| c.context_name.as_str() == context_name)
            .map(Maybe::Some)
            .unwrap_or(Maybe::None)
    }

    /// Check if a capability requirement is satisfied
    pub fn check_requirement(
        &self,
        requirement: &CapabilityRequirement,
    ) -> Result<(), CapabilityError> {
        // Find the context
        let context = match self.get_context_capabilities(requirement.context_name.as_str()) {
            Maybe::Some(ctx) => ctx,
            Maybe::None => {
                return Err(CapabilityError::ContextNotFound {
                    context_name: requirement.context_name.clone(),
                    operation: requirement.operation.clone(),
                });
            }
        };

        // Check if requirement is satisfied
        if requirement.is_satisfied_by(context) {
            Ok(())
        } else {
            let missing = requirement.missing_capabilities(context);
            Err(CapabilityError::InsufficientCapabilities {
                context_name: requirement.context_name.clone(),
                operation: requirement.operation.clone(),
                required: requirement.required_capabilities.clone(),
                available: context.capabilities.clone(),
                missing,
            })
        }
    }

    /// Attenuate a context
    pub fn attenuate_context(
        &mut self,
        context_name: &str,
        new_capabilities: TypeCapabilitySet,
    ) -> Result<ContextCapabilities, CapabilityError> {
        // Find existing context
        let context = match self.get_context_capabilities(context_name) {
            Maybe::Some(ctx) => ctx.clone(),
            Maybe::None => {
                return Err(CapabilityError::ContextNotFound {
                    context_name: Text::from(context_name),
                    operation: Text::from("attenuate"),
                });
            }
        };

        // Create attenuated context
        let attenuated = context.attenuate(new_capabilities);

        // Register attenuated context (replaces existing)
        self.register_context(attenuated.clone());

        Ok(attenuated)
    }
}

impl Default for CapabilityChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that can occur during capability checking
#[derive(Debug, Clone, PartialEq)]
pub enum CapabilityError {
    /// Context not found in environment
    ContextNotFound { context_name: Text, operation: Text },

    /// Insufficient capabilities for operation
    InsufficientCapabilities {
        context_name: Text,
        operation: Text,
        required: TypeCapabilitySet,
        available: TypeCapabilitySet,
        missing: List<TypeCapability>,
    },

    /// Invalid capability specification
    InvalidCapability {
        context_name: Text,
        capability: Text,
        reason: Text,
    },
}

impl CapabilityError {
    /// Get a human-readable error message
    pub fn message(&self) -> Text {
        match self {
            CapabilityError::ContextNotFound {
                context_name,
                operation,
            } => Text::from(format!(
                "Context '{}' not found for operation '{}'",
                context_name, operation
            )),

            CapabilityError::InsufficientCapabilities {
                context_name,
                operation,
                missing,
                ..
            } => {
                let missing_names: Vec<String> =
                    missing.iter().map(|c| c.name().to_string()).collect();
                Text::from(format!(
                    "Operation '{}' on context '{}' requires capabilities: {}",
                    operation,
                    context_name,
                    missing_names.join(", ")
                ))
            }

            CapabilityError::InvalidCapability {
                context_name,
                capability,
                reason,
            } => Text::from(format!(
                "Invalid capability '{}' for context '{}': {}",
                capability, context_name, reason
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_set_operations() {
        let mut set1 = TypeCapabilitySet::empty();
        set1.insert(TypeCapability::ReadOnly);
        set1.insert(TypeCapability::Query);

        let mut set2 = TypeCapabilitySet::empty();
        set2.insert(TypeCapability::ReadOnly);
        set2.insert(TypeCapability::Execute);

        // Test intersection
        let intersection = set1.intersect(&set2);
        assert!(intersection.contains(&TypeCapability::ReadOnly));
        assert!(!intersection.contains(&TypeCapability::Query));
        assert!(!intersection.contains(&TypeCapability::Execute));

        // Test union
        let union = set1.union(&set2);
        assert!(union.contains(&TypeCapability::ReadOnly));
        assert!(union.contains(&TypeCapability::Query));
        assert!(union.contains(&TypeCapability::Execute));
    }

    #[test]
    fn test_context_attenuation() {
        let full = ContextCapabilities::full(Text::from("Database"));

        let mut read_only_caps = TypeCapabilitySet::empty();
        read_only_caps.insert(TypeCapability::ReadOnly);
        read_only_caps.insert(TypeCapability::Query);

        let attenuated = full.attenuate(read_only_caps);

        assert!(attenuated.is_attenuated);
        assert!(attenuated.has_capability(&TypeCapability::ReadOnly));
        assert!(attenuated.has_capability(&TypeCapability::Query));
        assert!(!attenuated.has_capability(&TypeCapability::Execute));
    }

    #[test]
    fn test_capability_checking() {
        let mut checker = CapabilityChecker::new();

        let mut caps = TypeCapabilitySet::empty();
        caps.insert(TypeCapability::ReadOnly);
        caps.insert(TypeCapability::Query);

        let context = ContextCapabilities::with_capabilities(Text::from("Database"), caps);
        checker.register_context(context);

        // Should succeed
        let mut required = TypeCapabilitySet::empty();
        required.insert(TypeCapability::ReadOnly);

        let req = CapabilityRequirement::new(Text::from("Database"), required, Text::from("query"));
        assert!(checker.check_requirement(&req).is_ok());

        // Should fail
        let mut required_write = TypeCapabilitySet::empty();
        required_write.insert(TypeCapability::Execute);

        let req_write = CapabilityRequirement::new(
            Text::from("Database"),
            required_write,
            Text::from("delete"),
        );
        assert!(checker.check_requirement(&req_write).is_err());
    }

    // ===== Method Capability Mapper Tests =====

    #[test]
    fn test_method_capability_mapper_heuristics_write_operations() {
        let mapper = MethodCapabilityMapper::new();

        // Test write operations
        let caps = mapper.extract_from_method_name(&Text::from("execute"));
        assert!(caps.contains(&TypeCapability::Execute));

        let caps = mapper.extract_from_method_name(&Text::from("delete_user"));
        assert!(caps.contains(&TypeCapability::Execute));

        let caps = mapper.extract_from_method_name(&Text::from("update_record"));
        assert!(caps.contains(&TypeCapability::Execute));

        let caps = mapper.extract_from_method_name(&Text::from("insert_data"));
        assert!(caps.contains(&TypeCapability::Execute));

        let caps = mapper.extract_from_method_name(&Text::from("create_table"));
        assert!(caps.contains(&TypeCapability::Execute));
    }

    #[test]
    fn test_method_capability_mapper_heuristics_read_operations() {
        let mapper = MethodCapabilityMapper::new();

        // Test read operations
        let caps = mapper.extract_from_method_name(&Text::from("query"));
        assert!(caps.contains(&TypeCapability::Query));

        let caps = mapper.extract_from_method_name(&Text::from("read_data"));
        assert!(caps.contains(&TypeCapability::Query));

        let caps = mapper.extract_from_method_name(&Text::from("get_user"));
        assert!(caps.contains(&TypeCapability::Query));

        let caps = mapper.extract_from_method_name(&Text::from("select_all"));
        assert!(caps.contains(&TypeCapability::Query));

        let caps = mapper.extract_from_method_name(&Text::from("fetch_records"));
        assert!(caps.contains(&TypeCapability::Query));

        let caps = mapper.extract_from_method_name(&Text::from("find_by_id"));
        assert!(caps.contains(&TypeCapability::Query));
    }

    #[test]
    fn test_method_capability_mapper_heuristics_transaction_operations() {
        let mapper = MethodCapabilityMapper::new();

        let caps = mapper.extract_from_method_name(&Text::from("begin_transaction"));
        assert!(caps.contains(&TypeCapability::Transaction));

        let caps = mapper.extract_from_method_name(&Text::from("commit"));
        assert!(caps.contains(&TypeCapability::Transaction));

        let caps = mapper.extract_from_method_name(&Text::from("rollback"));
        assert!(caps.contains(&TypeCapability::Transaction));
    }

    #[test]
    fn test_method_capability_mapper_heuristics_admin_operations() {
        let mapper = MethodCapabilityMapper::new();

        let caps = mapper.extract_from_method_name(&Text::from("admin_delete_all"));
        assert!(caps.contains(&TypeCapability::Admin));

        let caps = mapper.extract_from_method_name(&Text::from("migrate_schema"));
        assert!(caps.contains(&TypeCapability::Admin));

        let caps = mapper.extract_from_method_name(&Text::from("backup_database"));
        assert!(caps.contains(&TypeCapability::Admin));
    }

    #[test]
    fn test_method_capability_mapper_heuristics_logging() {
        let mapper = MethodCapabilityMapper::new();

        let caps = mapper.extract_from_method_name(&Text::from("log_message"));
        assert!(caps.contains(&TypeCapability::Logging));

        let caps = mapper.extract_from_method_name(&Text::from("debug"));
        assert!(caps.contains(&TypeCapability::Logging));

        let caps = mapper.extract_from_method_name(&Text::from("info"));
        assert!(caps.contains(&TypeCapability::Logging));

        let caps = mapper.extract_from_method_name(&Text::from("error"));
        assert!(caps.contains(&TypeCapability::Logging));
    }

    #[test]
    fn test_method_capability_mapper_heuristics_network() {
        let mapper = MethodCapabilityMapper::new();

        let caps = mapper.extract_from_method_name(&Text::from("connect"));
        assert!(caps.contains(&TypeCapability::Network));

        let caps = mapper.extract_from_method_name(&Text::from("send_request"));
        assert!(caps.contains(&TypeCapability::Network));

        let caps = mapper.extract_from_method_name(&Text::from("receive_data"));
        assert!(caps.contains(&TypeCapability::Network));
    }

    #[test]
    fn test_method_capability_mapper_heuristics_default() {
        let mapper = MethodCapabilityMapper::new();

        // Unknown method should default to ReadOnly
        let caps = mapper.extract_from_method_name(&Text::from("unknown_method"));
        assert!(caps.contains(&TypeCapability::ReadOnly));
    }

    #[test]
    fn test_method_capability_mapper_custom_mappings() {
        let mut mapper = MethodCapabilityMapper::new();

        // Register a custom mapping
        let mut custom_caps = TypeCapabilitySet::empty();
        custom_caps.insert(TypeCapability::Execute);
        custom_caps.insert(TypeCapability::Admin);

        mapper.register_method(
            Text::from("Database"),
            Text::from("special_operation"),
            custom_caps.clone(),
        );

        // Extract should use custom mapping
        let extracted = mapper.extract_method_capabilities(
            &Text::from("Database"),
            &Text::from("special_operation"),
            None,
        );

        assert!(extracted.contains(&TypeCapability::Execute));
        assert!(extracted.contains(&TypeCapability::Admin));
        assert_eq!(extracted.len(), 2);
    }

    #[test]
    fn test_method_capability_mapper_sub_context_extraction() {
        use verum_ast::decl::{ContextDecl, FunctionDecl, Visibility};
        use verum_ast::span::Span;
        use verum_ast::ty::Ident;
        use verum_common::Maybe;

        let mapper = MethodCapabilityMapper::new();

        // Create a context declaration with sub-contexts
        let mut main_context = ContextDecl {
            visibility: Visibility::Private,
            is_async: false,
            name: Ident::new("Database".to_string(), Span::default()),
            generics: vec![].into(),
            methods: vec![].into(),
            sub_contexts: vec![].into(),
            associated_types: List::new(),
            associated_consts: List::new(),
            span: Span::default(),
        };

        // Create a Query sub-context
        let mut query_subcontext = ContextDecl {
            visibility: Visibility::Private,
            is_async: false,
            name: Ident::new("Query".to_string(), Span::default()),
            generics: vec![].into(),
            methods: vec![].into(),
            sub_contexts: vec![].into(),
            associated_types: List::new(),
            associated_consts: List::new(),
            span: Span::default(),
        };

        // Add a method to the Query sub-context
        let query_method = FunctionDecl {
            visibility: Visibility::Private,
            is_async: false,
            is_meta: false,
            stage_level: 0,
            is_pure: false,
            is_generator: false,
            is_cofix: false,
            is_unsafe: false,
            is_transparent: false,
            is_variadic: false,
            extern_abi: verum_common::Maybe::None,
            name: Ident::new("select_all".to_string(), Span::default()),
            generics: verum_common::List::new(),
            params: verum_common::List::new(),
            throws_clause: verum_common::Maybe::None,
            return_type: verum_common::Maybe::None,
            std_attr: verum_common::Maybe::None,
            contexts: verum_common::List::new(),
            generic_where_clause: verum_common::Maybe::None,
            meta_where_clause: verum_common::Maybe::None,
            requires: verum_common::List::new(),
            ensures: verum_common::List::new(),
            attributes: verum_common::List::new(),
            body: verum_common::Maybe::None,
            span: Span::default(),
        };

        query_subcontext.methods.push(query_method);
        main_context.sub_contexts.push(query_subcontext);

        // Extract capabilities - should use sub-context name "Query"
        let caps = mapper.extract_method_capabilities(
            &Text::from("Database"),
            &Text::from("select_all"),
            Some(&main_context),
        );

        // Should get Query capability from sub-context name
        assert!(caps.contains(&TypeCapability::Query));
    }

    #[test]
    fn test_capability_from_name() {
        let mapper = MethodCapabilityMapper::new();

        let caps = mapper.capability_from_name("Query");
        assert!(caps.contains(&TypeCapability::Query));

        let caps = mapper.capability_from_name("Execute");
        assert!(caps.contains(&TypeCapability::Execute));

        let caps = mapper.capability_from_name("Admin");
        assert!(caps.contains(&TypeCapability::Admin));

        let caps = mapper.capability_from_name("ReadOnly");
        assert!(caps.contains(&TypeCapability::ReadOnly));

        let caps = mapper.capability_from_name("Network");
        assert!(caps.contains(&TypeCapability::Network));

        let caps = mapper.capability_from_name("UnknownCapability");
        assert!(caps.contains(&TypeCapability::ReadOnly)); // Default
    }

    #[test]
    fn test_capability_subtyping_via_type_system() {
        use crate::subtype::Subtyping;
        use crate::ty::Type;
        use verum_ast::ty::{Ident, Path, PathSegment};
        use verum_ast::span::Span;

        let subtyping = Subtyping::new();

        // Create base type: Database
        let db_path = Path {
            segments: vec![PathSegment::Name(Ident::new("Database".to_string(), Span::default()))].into(),
            span: Span::default(),
        };
        let db_type = Type::Named { path: db_path, args: List::new() };

        // T with [Read, Write, Admin] should be subtype of T with [Read]
        let mut full_caps = TypeCapabilitySet::empty();
        full_caps.insert(TypeCapability::ReadOnly);
        full_caps.insert(TypeCapability::WriteOnly);
        full_caps.insert(TypeCapability::Admin);

        let mut read_only_caps = TypeCapabilitySet::empty();
        read_only_caps.insert(TypeCapability::ReadOnly);

        let full_type = Type::CapabilityRestricted {
            base: Box::new(db_type.clone()),
            capabilities: full_caps.clone(),
        };

        let read_type = Type::CapabilityRestricted {
            base: Box::new(db_type.clone()),
            capabilities: read_only_caps.clone(),
        };

        // More capabilities => subtype (monotonic attenuation is safe)
        assert!(subtyping.is_subtype(&full_type, &read_type),
            "T with [Read, Write, Admin] should be subtype of T with [Read]");

        // Fewer capabilities => NOT subtype (can't gain capabilities)
        assert!(!subtyping.is_subtype(&read_type, &full_type),
            "T with [Read] should NOT be subtype of T with [Read, Write, Admin]");

        // T with [Caps] should be subtype of T (forgetful upcast)
        assert!(subtyping.is_subtype(&full_type, &db_type),
            "T with [Read, Write, Admin] should be subtype of T");

        // Same capabilities => subtype (reflexive)
        let read_type2 = Type::CapabilityRestricted {
            base: Box::new(db_type.clone()),
            capabilities: read_only_caps.clone(),
        };
        assert!(subtyping.is_subtype(&read_type, &read_type2),
            "T with [Read] should be subtype of T with [Read]");
    }
}
