//! Security Sub-Context
//!
//! Manages enabled contexts, resource limits, and security-related configuration
//! for meta function execution.
//!
//! ## Responsibility
//!
//! - Enabled contexts (MetaTypes, MetaRuntime, CompileDiag, BuildAssets)
//! - Resource limits (iteration, recursion, memory, timeout)
//! - Sandboxing configuration
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

use verum_common::{List, Text};

use crate::meta::builtins::{EnabledContexts, RequiredContext};

/// Resource limits for meta function execution
#[derive(Debug, Clone, Copy)]
pub struct ResourceLimits {
    /// Maximum iterations for loops
    pub iteration_limit: u64,

    /// Maximum recursion depth
    pub recursion_limit: u64,

    /// Maximum memory usage in bytes
    pub memory_limit: u64,

    /// Timeout in milliseconds
    pub timeout_ms: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            iteration_limit: 1_000_000,
            recursion_limit: 1_000,
            memory_limit: 100 * 1024 * 1024, // 100 MB
            timeout_ms: 30_000,              // 30 seconds
        }
    }
}

impl ResourceLimits {
    /// Create new resource limits with custom values
    pub fn new(iteration_limit: u64, recursion_limit: u64, memory_limit: u64, timeout_ms: u64) -> Self {
        Self {
            iteration_limit,
            recursion_limit,
            memory_limit,
            timeout_ms,
        }
    }

    /// Create restrictive limits for untrusted code
    pub fn restrictive() -> Self {
        Self {
            iteration_limit: 10_000,
            recursion_limit: 100,
            memory_limit: 10 * 1024 * 1024, // 10 MB
            timeout_ms: 5_000,               // 5 seconds
        }
    }

    /// Create permissive limits for trusted code
    pub fn permissive() -> Self {
        Self {
            iteration_limit: 100_000_000,
            recursion_limit: 10_000,
            memory_limit: 1024 * 1024 * 1024, // 1 GB
            timeout_ms: 300_000,              // 5 minutes
        }
    }
}

/// Security context for meta function execution
///
/// Controls which contexts are enabled and enforces resource limits
/// during compile-time evaluation.
#[derive(Debug, Clone)]
pub struct SecurityContext {
    /// Enabled contexts for builtin function access
    enabled_contexts: EnabledContexts,

    /// Resource limits
    limits: ResourceLimits,

    /// User-defined contexts (for extensibility)
    user_contexts: List<Text>,

    /// Whether sandbox mode is enabled
    sandboxed: bool,

    /// Whether to allow dynamic code execution
    allow_dynamic_code: bool,

    /// Trusted source paths (for BuildAssets)
    trusted_paths: List<Text>,
}

impl Default for SecurityContext {
    fn default() -> Self {
        Self::new()
    }
}

impl SecurityContext {
    /// Create a new security context with minimal permissions
    pub fn new() -> Self {
        Self {
            enabled_contexts: EnabledContexts::new(),
            limits: ResourceLimits::default(),
            user_contexts: List::new(),
            sandboxed: true,
            allow_dynamic_code: false,
            trusted_paths: List::new(),
        }
    }

    /// Create a security context with all contexts enabled
    pub fn all_contexts() -> Self {
        Self {
            enabled_contexts: EnabledContexts::all(),
            limits: ResourceLimits::default(),
            user_contexts: List::new(),
            sandboxed: true,
            allow_dynamic_code: false,
            trusted_paths: List::new(),
        }
    }

    /// Create a security context from a using clause
    pub fn from_using_clause(names: &[Text]) -> Self {
        Self {
            enabled_contexts: EnabledContexts::from_using_clause(names),
            limits: ResourceLimits::default(),
            user_contexts: List::new(),
            sandboxed: true,
            allow_dynamic_code: false,
            trusted_paths: List::new(),
        }
    }

    // ======== Context Operations ========

    /// Enable a context
    pub fn enable_context(&mut self, context: RequiredContext) {
        self.enabled_contexts.enable(context);
    }

    /// Enable multiple contexts
    pub fn enable_contexts(&mut self, contexts: &[RequiredContext]) {
        for context in contexts {
            self.enabled_contexts.enable(*context);
        }
    }

    /// Check if a context is enabled
    pub fn is_context_enabled(&self, context: RequiredContext) -> bool {
        self.enabled_contexts.is_enabled(context)
    }

    /// Get enabled contexts reference
    pub fn enabled_contexts(&self) -> &EnabledContexts {
        &self.enabled_contexts
    }

    /// Get mutable enabled contexts
    pub fn enabled_contexts_mut(&mut self) -> &mut EnabledContexts {
        &mut self.enabled_contexts
    }

    /// Set enabled contexts
    pub fn set_enabled_contexts(&mut self, contexts: EnabledContexts) {
        self.enabled_contexts = contexts;
    }

    // ======== User Contexts ========

    /// Add a user-defined context
    pub fn add_user_context(&mut self, name: Text) {
        if !self.user_contexts.contains(&name) {
            self.user_contexts.push(name);
        }
    }

    /// Check if a user context is enabled
    pub fn has_user_context(&self, name: &Text) -> bool {
        self.user_contexts.contains(name)
    }

    /// Get user contexts
    pub fn user_contexts(&self) -> &List<Text> {
        &self.user_contexts
    }

    // ======== Resource Limits ========

    /// Get resource limits
    #[inline]
    pub fn limits(&self) -> &ResourceLimits {
        &self.limits
    }

    /// Set resource limits.
    ///
    /// Call `MetaContext::apply_security_context(sec)` (or the
    /// `MetaContext::from_security_context(sec)` builder) to actually
    /// gate evaluator/sandbox execution by these limits. Until that
    /// linkage was added the four fields below were silently inert.
    #[inline]
    pub fn set_limits(&mut self, limits: ResourceLimits) {
        self.limits = limits;
    }

    /// Get iteration limit
    #[inline]
    pub fn iteration_limit(&self) -> u64 {
        self.limits.iteration_limit
    }

    /// Set iteration limit
    #[inline]
    pub fn set_iteration_limit(&mut self, limit: u64) {
        self.limits.iteration_limit = limit;
    }

    /// Get recursion limit
    #[inline]
    pub fn recursion_limit(&self) -> u64 {
        self.limits.recursion_limit
    }

    /// Set recursion limit
    #[inline]
    pub fn set_recursion_limit(&mut self, limit: u64) {
        self.limits.recursion_limit = limit;
    }

    /// Get memory limit
    #[inline]
    pub fn memory_limit(&self) -> u64 {
        self.limits.memory_limit
    }

    /// Set memory limit
    #[inline]
    pub fn set_memory_limit(&mut self, limit: u64) {
        self.limits.memory_limit = limit;
    }

    /// Get timeout
    #[inline]
    pub fn timeout_ms(&self) -> u64 {
        self.limits.timeout_ms
    }

    /// Set timeout
    #[inline]
    pub fn set_timeout_ms(&mut self, timeout: u64) {
        self.limits.timeout_ms = timeout;
    }

    // ======== Sandbox Settings ========

    /// Check if sandboxed
    #[inline]
    pub fn is_sandboxed(&self) -> bool {
        self.sandboxed
    }

    /// Enable/disable sandbox mode
    #[inline]
    pub fn set_sandboxed(&mut self, sandboxed: bool) {
        self.sandboxed = sandboxed;
    }

    /// Check if dynamic code execution is allowed
    #[inline]
    pub fn allows_dynamic_code(&self) -> bool {
        self.allow_dynamic_code
    }

    /// Enable/disable dynamic code execution
    #[inline]
    pub fn set_allow_dynamic_code(&mut self, allow: bool) {
        self.allow_dynamic_code = allow;
    }

    // ======== Trusted Paths ========

    /// Add a trusted path
    pub fn add_trusted_path(&mut self, path: Text) {
        if !self.trusted_paths.contains(&path) {
            self.trusted_paths.push(path);
        }
    }

    /// Check if a path is trusted
    pub fn is_path_trusted(&self, path: &Text) -> bool {
        // Check exact match or prefix match
        self.trusted_paths.iter().any(|trusted| {
            path == trusted || path.starts_with(trusted.as_str())
        })
    }

    /// Get trusted paths
    pub fn trusted_paths(&self) -> &List<Text> {
        &self.trusted_paths
    }

    /// Clear trusted paths
    pub fn clear_trusted_paths(&mut self) {
        self.trusted_paths.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_enabling() {
        let mut security = SecurityContext::new();
        assert!(!security.is_context_enabled(RequiredContext::MetaTypes));

        security.enable_context(RequiredContext::MetaTypes);
        assert!(security.is_context_enabled(RequiredContext::MetaTypes));

        // Tier 0 (None) is always enabled
        assert!(security.is_context_enabled(RequiredContext::None));
    }

    #[test]
    fn test_resource_limits() {
        let mut security = SecurityContext::new();
        assert_eq!(security.iteration_limit(), 1_000_000);

        security.set_iteration_limit(100);
        assert_eq!(security.iteration_limit(), 100);

        let restrictive = ResourceLimits::restrictive();
        security.set_limits(restrictive);
        assert_eq!(security.iteration_limit(), 10_000);
    }

    #[test]
    fn test_trusted_paths() {
        let mut security = SecurityContext::new();
        assert!(!security.is_path_trusted(&Text::from("/home/user/project")));

        security.add_trusted_path(Text::from("/home/user"));
        assert!(security.is_path_trusted(&Text::from("/home/user/project")));
        assert!(security.is_path_trusted(&Text::from("/home/user")));
        assert!(!security.is_path_trusted(&Text::from("/home/other")));
    }

    #[test]
    fn test_from_using_clause() {
        let security = SecurityContext::from_using_clause(&[
            Text::from("MetaTypes"),
            Text::from("CompileDiag"),
        ]);
        assert!(security.is_context_enabled(RequiredContext::MetaTypes));
        assert!(security.is_context_enabled(RequiredContext::CompileDiag));
        assert!(!security.is_context_enabled(RequiredContext::MetaRuntime));
    }
}
