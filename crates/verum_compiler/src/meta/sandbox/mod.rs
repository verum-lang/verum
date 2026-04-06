//! Meta Sandbox Module
//!
//! This module implements the security sandbox that ensures meta functions
//! cannot perform I/O operations during compile-time execution.
//!
//! Enforcement occurs at THREE levels:
//! 1. Parser level: Rejects I/O calls syntactically during Pass 1 meta block parsing
//! 2. Interpreter level: Restricted syscall environment during Pass 2 meta execution,
//!    blocks file/network/process operations even if they pass parsing
//! 3. Type checker level: Purity analysis verifies no I/O function calls escape detection,
//!    tracks taint from external inputs through dataflow
//!
//! CRITICAL: I/O is forbidden in ALL meta contexts (meta fn, meta async fn, @derive,
//! procedural macros, meta blocks). This ensures deterministic builds, security (no
//! network access during compilation), and reproducibility (builds work offline).
//! meta async fn IS allowed for parallel pure computation, but NOT for I/O.
//!
//! ## Module Structure
//!
//! - `errors`: Error types and operation enum
//! - `allowlist`: Function allowlists/blocklists organized by category
//! - `resource_limits`: Execution limits and RAII guards
//! - `validation`: Expression validation for sandbox compliance
//! - `execution`: Sandboxed executor for meta expressions
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

pub mod allowlist;
pub mod errors;
pub mod execution;
pub mod resource_limits;
pub mod validation;

// Re-exports for convenience
pub use allowlist::AllowlistRegistry;
pub use errors::{Operation, SandboxError};
pub use execution::SandboxedExecutor;
pub use resource_limits::{MemoryGuard, RecursionGuard, ResourceLimiter, ResourceLimits};
pub use validation::ExpressionValidator;

use std::time::Instant;

use verum_ast::decl::FunctionDecl;
use verum_ast::expr::Expr;
use verum_common::Text;

use crate::meta::context::{ConstValue, MetaContext};

/// Sandbox for meta function execution with comprehensive I/O restrictions
///
/// The sandbox enforces that meta functions can only perform pure computations
/// and cannot execute I/O operations like file access, network calls, or
/// process spawning.
///
/// **Defense in Depth**: Multiple enforcement layers ensure no forbidden
/// operations can occur, even if one layer fails.
#[derive(Debug, Clone)]
pub struct MetaSandbox {
    /// Executor for running expressions
    executor: SandboxedExecutor,
}

impl MetaSandbox {
    /// Create a new meta sandbox with default settings
    pub fn new() -> Self {
        Self {
            executor: SandboxedExecutor::new(),
        }
    }

    /// Create a sandbox with custom resource limits
    pub fn with_limits(limits: ResourceLimits) -> Self {
        let limiter = ResourceLimiter::with_limits(limits);
        let executor = SandboxedExecutor::with_components(
            limiter,
            ExpressionValidator::new(),
            AllowlistRegistry::new(),
        );
        Self { executor }
    }

    // ========================================================================
    // Execution
    // ========================================================================

    /// Execute an expression in the sandbox
    ///
    /// This is the main entry point for executing meta functions at compile-time.
    pub fn execute(
        &self,
        ctx: &MetaContext,
        expr: &Expr,
    ) -> Result<ConstValue, SandboxError> {
        self.executor.execute(ctx, expr)
    }

    /// Execute a meta function with appropriate asset loading permissions
    pub fn execute_meta_function<F, R>(&self, func: &FunctionDecl, executor: F) -> R
    where
        F: FnOnce() -> R,
    {
        if ExpressionValidator::has_build_assets_context(func) {
            self.executor.limiter().with_asset_loading(executor)
        } else {
            executor()
        }
    }

    // ========================================================================
    // Validation
    // ========================================================================

    /// Validate a function call for sandbox compliance
    pub fn validate_function_call(&self, call: &Expr) -> Result<(), SandboxError> {
        self.executor.validator().validate_function_call(call)
    }

    /// Validate an expression for sandbox compliance (recursive)
    pub fn validate_expr(&self, expr: &Expr) -> Result<(), SandboxError> {
        self.executor.validator().validate_expr(expr)
    }

    /// Validate an async meta function for I/O operations
    pub fn validate_async_meta_fn(&self, func: &FunctionDecl) -> Result<(), SandboxError> {
        self.executor.validator().validate_async_meta_fn(func)
    }

    // ========================================================================
    // Asset Loading Control
    // ========================================================================

    /// Enable asset loading for the current execution context
    pub fn enable_asset_loading(&self) {
        self.executor.limiter().enable_asset_loading();
    }

    /// Disable asset loading for the current execution context
    pub fn disable_asset_loading(&self) {
        self.executor.limiter().disable_asset_loading();
    }

    /// Check if asset loading is currently allowed
    pub fn is_asset_loading_allowed(&self) -> bool {
        self.executor.limiter().is_asset_loading_allowed()
    }

    /// Execute a function with asset loading enabled
    pub fn with_asset_loading<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        self.executor.limiter().with_asset_loading(f)
    }

    // ========================================================================
    // Resource Limit Checks
    // ========================================================================

    /// Check if iteration limit has been exceeded
    pub fn check_iteration_limit(&self) -> Result<(), SandboxError> {
        self.executor.limiter().check_iteration_limit()
    }

    /// Check if recursion limit has been exceeded
    pub fn check_recursion_limit(&self) -> Result<(), SandboxError> {
        self.executor.limiter().check_recursion_limit()
    }

    /// Check if memory limit has been exceeded
    pub fn check_memory_limit(&self, bytes: usize) -> Result<(), SandboxError> {
        self.executor.limiter().check_memory_limit(bytes)
    }

    /// Check if execution timeout has been exceeded
    pub fn check_timeout(&self, start: Instant) -> Result<(), SandboxError> {
        self.executor.limiter().check_timeout(start)
    }

    /// Reset execution state counters
    pub fn reset_execution_state(&self) {
        self.executor.limiter().reset_execution_state();
    }

    // ========================================================================
    // State Accessors
    // ========================================================================

    /// Get current iteration count
    pub fn current_iterations(&self) -> usize {
        self.executor.limiter().current_iterations()
    }

    /// Get current recursion depth
    pub fn current_recursion_depth(&self) -> usize {
        self.executor.limiter().current_recursion_depth()
    }

    /// Get current memory usage
    pub fn current_memory_usage(&self) -> usize {
        self.executor.limiter().current_memory_usage()
    }

    // ========================================================================
    // Function Category Checks
    // ========================================================================

    /// Check if a function is a filesystem operation
    pub fn is_filesystem_function(&self, name: &Text) -> bool {
        self.executor.allowlist().is_filesystem_function(name)
    }

    /// Check if a function is a network operation
    pub fn is_network_function(&self, name: &Text) -> bool {
        self.executor.allowlist().is_network_function(name)
    }

    /// Check if a function is a process operation
    pub fn is_process_function(&self, name: &Text) -> bool {
        self.executor.allowlist().is_process_function(name)
    }

    /// Check if a function is a time operation
    pub fn is_time_function(&self, name: &Text) -> bool {
        self.executor.allowlist().is_time_function(name)
    }

    /// Check if a function is an environment operation
    pub fn is_env_function(&self, name: &Text) -> bool {
        self.executor.allowlist().is_env_function(name)
    }

    /// Check if a function is a random operation
    pub fn is_random_function(&self, name: &Text) -> bool {
        self.executor.allowlist().is_random_function(name)
    }

    /// Check if a function is an unsafe operation
    pub fn is_unsafe_function(&self, name: &Text) -> bool {
        self.executor.allowlist().is_unsafe_function(name)
    }

    /// Check if a function is an FFI operation
    pub fn is_ffi_function(&self, name: &Text) -> bool {
        self.executor.allowlist().is_ffi_function(name)
    }

    /// Check if a function is an asset loading operation
    pub fn is_asset_loading_function(&self, name: &Text) -> bool {
        self.executor.allowlist().is_asset_loading_function(name)
    }

    /// Check if a function name is a forbidden I/O operation
    pub fn is_forbidden_io_function(&self, name: &Text) -> bool {
        self.executor.allowlist().is_forbidden_io_function(name)
    }

    /// Check if an operation is allowed
    pub fn is_operation_allowed(&self, op: Operation) -> bool {
        self.executor.allowlist().is_operation_allowed(op)
    }

    /// Check if a function declaration uses the BuildAssets context
    pub fn has_build_assets_context(func: &FunctionDecl) -> bool {
        ExpressionValidator::has_build_assets_context(func)
    }

    // ========================================================================
    // Builtin Function Execution
    // ========================================================================

    /// Execute a builtin function with the given arguments
    pub fn execute_builtin_function(
        &self,
        name: &Text,
        args: &[ConstValue],
        _ctx: &MetaContext,
    ) -> Result<ConstValue, SandboxError> {
        // Delegate to the simple implementations
        match name.as_str() {
            "typeof" | "type_of" => {
                if args.len() != 1 {
                    return Err(SandboxError::UnsafeOperation {
                        operation: Text::from("typeof"),
                        reason: Text::from("typeof expects exactly 1 argument"),
                    });
                }
                let type_name = match &args[0] {
                    ConstValue::Unit => "Unit",
                    ConstValue::Bool(_) => "Bool",
                    ConstValue::Int(_) => "Int",
                    ConstValue::UInt(_) => "UInt",
                    ConstValue::Float(_) => "Float",
                    ConstValue::Char(_) => "Char",
                    ConstValue::Text(_) => "Text",
                    ConstValue::Array(_) => "List",
                    ConstValue::Tuple(_) => "Tuple",
                    ConstValue::Map(_) => "Map",
                    ConstValue::Set(_) => "Set",
                    ConstValue::Expr(_) => "Expr",
                    ConstValue::Type(_) => "Type",
                    ConstValue::Pattern(_) => "Pattern",
                    ConstValue::Item(_) => "Item",
                    ConstValue::Items(_) => "Items",
                    ConstValue::Bytes(_) => "Bytes",
                    ConstValue::Maybe(_) => "Maybe",
                };
                Ok(ConstValue::Text(Text::from(type_name)))
            }
            "len" => {
                if args.len() != 1 {
                    return Err(SandboxError::UnsafeOperation {
                        operation: Text::from("len"),
                        reason: Text::from("len expects exactly 1 argument"),
                    });
                }
                let len = match &args[0] {
                    ConstValue::Array(arr) => arr.len() as i128,
                    ConstValue::Text(t) => t.len() as i128,
                    ConstValue::Tuple(t) => t.len() as i128,
                    _ => {
                        return Err(SandboxError::UnsafeOperation {
                            operation: Text::from("len"),
                            reason: Text::from("len requires a collection or text"),
                        });
                    }
                };
                Ok(ConstValue::Int(len))
            }
            "size_of" => {
                if args.is_empty() {
                    return Err(SandboxError::UnsafeOperation {
                        operation: Text::from("size_of"),
                        reason: Text::from("size_of expects exactly 1 argument"),
                    });
                }
                let size = compute_size_of(&args[0])?;
                Ok(ConstValue::Int(size.into()))
            }
            "align_of" => {
                if args.is_empty() {
                    return Err(SandboxError::UnsafeOperation {
                        operation: Text::from("align_of"),
                        reason: Text::from("align_of expects exactly 1 argument"),
                    });
                }
                let align = compute_align_of(&args[0])?;
                Ok(ConstValue::Int(align.into()))
            }
            _ => Err(SandboxError::UnsafeOperation {
                operation: name.clone(),
                reason: Text::from("Unknown builtin function"),
            }),
        }
    }
}

impl Default for MetaSandbox {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Size and Alignment Computation (moved from original sandbox.rs)
// ============================================================================

/// Compute the size in bytes for a type or value
fn compute_size_of(arg: &ConstValue) -> Result<i64, SandboxError> {
    match arg {
        ConstValue::Text(type_name) => {
            match type_name.as_str() {
                // Signed integers
                "Int8" | "i8" => Ok(1),
                "Int16" | "i16" => Ok(2),
                "Int32" | "i32" => Ok(4),
                "Int64" | "i64" => Ok(8),
                "Int128" | "i128" => Ok(16),
                "Int" => Ok(8),
                "ISize" | "isize" => Ok(8),

                // Unsigned integers
                "UInt8" | "u8" | "Byte" => Ok(1),
                "UInt16" | "u16" => Ok(2),
                "UInt32" | "u32" => Ok(4),
                "UInt64" | "u64" | "UInt" => Ok(8),
                "UInt128" | "u128" => Ok(16),
                "USize" | "usize" => Ok(8),

                // Floating point
                "Float32" | "f32" => Ok(4),
                "Float64" | "Float" | "f64" => Ok(8),

                // Boolean and character
                "Bool" | "bool" => Ok(1),
                "Char" | "char" => Ok(4),

                // Unit type
                "Unit" | "()" => Ok(0),

                // Pointer types
                "ptr" | "Ptr" | "&" | "*const" | "*mut" => Ok(8),

                // Collection types
                "List" | "Array" => Ok(24),
                "Text" | "String" => Ok(24),
                "Map" | "HashMap" => Ok(8),
                "Set" | "HashSet" => Ok(8),

                // Optional/Result types
                "Maybe" | "Option" => Ok(16),
                "Result" => Ok(16),

                // Smart pointers
                "Heap" | "Box" => Ok(8),
                "Shared" | "Rc" | "Arc" => Ok(16),

                // Function pointers
                "fn" | "Fn" => Ok(8),

                _ => Err(SandboxError::UnsafeOperation {
                    operation: Text::from("size_of"),
                    reason: Text::from(format!("Unknown type: {}", type_name.as_str())),
                }),
            }
        }
        ConstValue::Unit => Ok(0),
        ConstValue::Bool(_) => Ok(1),
        ConstValue::Int(_) => Ok(8),
        ConstValue::UInt(_) => Ok(8),
        ConstValue::Float(_) => Ok(8),
        ConstValue::Array(_) => Ok(24),
        ConstValue::Tuple(elements) => {
            let mut total = 0i64;
            for elem in elements.iter() {
                total += compute_size_of(elem)?;
            }
            Ok(total)
        }
        ConstValue::Map(_) => Ok(48), // BTreeMap overhead
        ConstValue::Set(_) => Ok(24), // BTreeSet overhead
        ConstValue::Expr(_) => Ok(8),
        ConstValue::Type(_) => Ok(8),
        ConstValue::Pattern(_) => Ok(8),
        ConstValue::Item(_) => Ok(8),
        ConstValue::Items(_) => Ok(8),
        ConstValue::Bytes(b) => Ok(b.len() as i64),
        ConstValue::Char(_) => Ok(4),
        ConstValue::Maybe(_) => Ok(16),
    }
}

/// Compute the alignment in bytes for a type or value
fn compute_align_of(arg: &ConstValue) -> Result<i64, SandboxError> {
    match arg {
        ConstValue::Text(type_name) => {
            match type_name.as_str() {
                // 1-byte alignment
                "Int8" | "i8" | "UInt8" | "u8" | "Byte" => Ok(1),
                "Bool" | "bool" => Ok(1),
                "Unit" | "()" => Ok(1),

                // 2-byte alignment
                "Int16" | "i16" | "UInt16" | "u16" => Ok(2),

                // 4-byte alignment
                "Int32" | "i32" | "UInt32" | "u32" => Ok(4),
                "Float32" | "f32" => Ok(4),
                "Char" | "char" => Ok(4),

                // 8-byte alignment
                "Int64" | "i64" | "UInt64" | "u64" => Ok(8),
                "Int" | "UInt" => Ok(8),
                "ISize" | "isize" | "USize" | "usize" => Ok(8),
                "Float64" | "Float" | "f64" => Ok(8),
                "ptr" | "Ptr" | "&" | "*const" | "*mut" | "fn" | "Fn" => Ok(8),
                "List" | "Array" | "Text" | "String" => Ok(8),
                "Map" | "HashMap" | "Set" | "HashSet" => Ok(8),
                "Maybe" | "Option" | "Result" => Ok(8),
                "Heap" | "Box" | "Shared" | "Rc" | "Arc" => Ok(8),

                // 16-byte alignment
                "Int128" | "i128" | "UInt128" | "u128" => Ok(16),

                _ => Err(SandboxError::UnsafeOperation {
                    operation: Text::from("align_of"),
                    reason: Text::from(format!("Unknown type: {}", type_name.as_str())),
                }),
            }
        }
        ConstValue::Unit => Ok(1),
        ConstValue::Bool(_) => Ok(1),
        ConstValue::Int(_) => Ok(8),
        ConstValue::UInt(_) => Ok(8),
        ConstValue::Float(_) => Ok(8),
        ConstValue::Array(_) => Ok(8),
        ConstValue::Tuple(elements) => {
            let mut max_align = 1i64;
            for elem in elements.iter() {
                let align = compute_align_of(elem)?;
                if align > max_align {
                    max_align = align;
                }
            }
            Ok(max_align)
        }
        ConstValue::Bytes(_) => Ok(1),
        ConstValue::Char(_) => Ok(4),
        ConstValue::Maybe(_) => Ok(8),
        ConstValue::Map(_) => Ok(8),
        ConstValue::Set(_) => Ok(8),
        ConstValue::Expr(_) => Ok(8),
        ConstValue::Type(_) => Ok(8),
        ConstValue::Pattern(_) => Ok(8),
        ConstValue::Item(_) => Ok(8),
        ConstValue::Items(_) => Ok(8),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_creation() {
        let sandbox = MetaSandbox::new();
        assert_eq!(sandbox.current_iterations(), 0);
        assert_eq!(sandbox.current_recursion_depth(), 0);
    }

    #[test]
    fn test_forbidden_operations() {
        let sandbox = MetaSandbox::new();

        // File system operations
        assert!(sandbox.is_filesystem_function(&Text::from("std.fs.read")));
        assert!(sandbox.is_filesystem_function(&Text::from("std.fs.write")));

        // Network operations
        assert!(sandbox.is_network_function(&Text::from("std.net.http_get")));
        assert!(sandbox.is_network_function(&Text::from("std.net.tcp_connect")));

        // Process operations
        assert!(sandbox.is_process_function(&Text::from("std.process.spawn")));
        assert!(sandbox.is_process_function(&Text::from("std.process.exec")));

        // Time operations
        assert!(sandbox.is_time_function(&Text::from("std.time.now")));

        // Environment operations
        assert!(sandbox.is_env_function(&Text::from("std.env.var")));

        // Random operations
        assert!(sandbox.is_random_function(&Text::from("std.random.gen")));

        // Unsafe operations
        assert!(sandbox.is_unsafe_function(&Text::from("std.mem.transmute")));

        // FFI operations
        assert!(sandbox.is_ffi_function(&Text::from("std.ffi.call")));
    }

    #[test]
    fn test_size_of() {
        assert_eq!(compute_size_of(&ConstValue::Text(Text::from("Int8"))).unwrap(), 1);
        assert_eq!(compute_size_of(&ConstValue::Text(Text::from("Int32"))).unwrap(), 4);
        assert_eq!(compute_size_of(&ConstValue::Text(Text::from("Int64"))).unwrap(), 8);
        assert_eq!(compute_size_of(&ConstValue::Text(Text::from("Bool"))).unwrap(), 1);
    }

    #[test]
    fn test_align_of() {
        assert_eq!(compute_align_of(&ConstValue::Text(Text::from("Int8"))).unwrap(), 1);
        assert_eq!(compute_align_of(&ConstValue::Text(Text::from("Int32"))).unwrap(), 4);
        assert_eq!(compute_align_of(&ConstValue::Text(Text::from("Int64"))).unwrap(), 8);
    }
}
