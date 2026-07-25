//! Linter Configuration for Meta Linter
//!
//! Contains LinterConfig for customizing linter behavior.
//!
//! Meta linter: static analysis of meta code for unsafe patterns (unbounded
//! recursion, infinite loops, unsafe interpolation without @safe attribute).

use std::collections::HashSet;

/// Configuration for the meta linter
#[derive(Debug, Clone)]
pub struct LinterConfig {
    /// Error on unsafe meta functions (default: warn)
    pub unsafe_as_error: bool,
    /// Require explicit @safe annotation (default: false)
    pub require_explicit_safe: bool,
    /// Check for performance issues (default: true)
    pub check_performance: bool,
    /// Ensure deterministic meta functions (default: true)
    pub check_determinism: bool,
    /// Maximum meta function complexity
    pub max_cyclomatic_complexity: usize,
    /// Forbidden functions in meta code
    pub forbidden_functions: HashSet<String>,
}

impl Default for LinterConfig {
    fn default() -> Self {
        let mut forbidden = HashSet::new();
        // Default forbidden functions in meta context
        forbidden.insert("println!".to_string());
        forbidden.insert("print!".to_string());
        forbidden.insert("panic!".to_string());
        forbidden.insert("todo!".to_string());
        forbidden.insert("unimplemented!".to_string());

        Self {
            unsafe_as_error: false,
            require_explicit_safe: false,
            check_performance: true,
            check_determinism: true,
            max_cyclomatic_complexity: 10,
            forbidden_functions: forbidden,
        }
    }
}

impl LinterConfig {
    /// Create a strict configuration that treats all unsafe patterns as errors
    pub fn strict() -> Self {
        Self {
            unsafe_as_error: true,
            require_explicit_safe: true,
            ..Self::default()
        }
    }

    /// Create a permissive configuration with minimal checks
    pub fn permissive() -> Self {
        Self {
            unsafe_as_error: false,
            require_explicit_safe: false,
            check_performance: false,
            check_determinism: false,
            max_cyclomatic_complexity: 50,
            forbidden_functions: HashSet::new(),
        }
    }

    /// Add a function to the forbidden list
    pub fn forbid_function(&mut self, func_name: &str) {
        self.forbidden_functions.insert(func_name.to_string());
    }

    /// Check if a function is forbidden
    pub fn is_forbidden(&self, func_name: &str) -> bool {
        self.forbidden_functions.contains(func_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = LinterConfig::default();
        assert!(!config.unsafe_as_error);
        assert!(!config.require_explicit_safe);
        assert!(config.check_performance);
        assert!(config.check_determinism);
        assert_eq!(config.max_cyclomatic_complexity, 10);
        assert!(config.is_forbidden("panic!"));
    }

    #[test]
    fn test_strict_config() {
        let config = LinterConfig::strict();
        assert!(config.unsafe_as_error);
        assert!(config.require_explicit_safe);
    }

    #[test]
    fn test_permissive_config() {
        let config = LinterConfig::permissive();
        assert!(!config.unsafe_as_error);
        assert!(!config.check_performance);
        assert!(config.forbidden_functions.is_empty());
    }

    #[test]
    fn test_forbid_function() {
        let mut config = LinterConfig::default();
        config.forbid_function("my_dangerous_func");
        assert!(config.is_forbidden("my_dangerous_func"));
    }
}
