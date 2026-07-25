//! Allowlist Registry for Meta Sandbox
//!
//! Defines allowlists and blocklists for functions in meta context.
//! Organizes 95+ function names by category.
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

use verum_common::{Set, Text};

use super::errors::Operation;

/// Registry of allowed and forbidden functions organized by category
#[derive(Debug, Clone)]
pub struct AllowlistRegistry {
    /// Allowed operations
    pub allowed_operations: Set<Operation>,

    /// Allowed pure functions (explicitly whitelisted)
    pub allowed_functions: Set<Text>,

    /// Allowed modules for meta code
    pub allowed_modules: Set<Text>,

    /// Explicitly forbidden: All file I/O functions
    pub filesystem_functions: Set<Text>,

    /// Explicitly forbidden: All network I/O functions
    pub network_functions: Set<Text>,

    /// Explicitly forbidden: All process-related functions
    pub process_functions: Set<Text>,

    /// Explicitly forbidden: Non-deterministic time functions
    pub time_functions: Set<Text>,

    /// Explicitly forbidden: Environment variable access
    pub env_functions: Set<Text>,

    /// Explicitly forbidden: Random functions without seed
    pub random_functions: Set<Text>,

    /// Explicitly forbidden: Unsafe memory operations
    pub unsafe_functions: Set<Text>,

    /// Explicitly forbidden: FFI calls
    pub ffi_functions: Set<Text>,

    /// Functions that require `using BuildAssets` context
    pub asset_loading_functions: Set<Text>,
}

impl AllowlistRegistry {
    /// Create a new allowlist registry with default values
    pub fn new() -> Self {
        Self {
            allowed_operations: Self::default_allowed_operations(),
            allowed_functions: Self::default_allowed_functions(),
            allowed_modules: Self::default_allowed_modules(),
            filesystem_functions: Self::default_filesystem_functions(),
            network_functions: Self::default_network_functions(),
            process_functions: Self::default_process_functions(),
            time_functions: Self::default_time_functions(),
            env_functions: Self::default_env_functions(),
            random_functions: Self::default_random_functions(),
            unsafe_functions: Self::default_unsafe_functions(),
            ffi_functions: Self::default_ffi_functions(),
            asset_loading_functions: Self::default_asset_loading_functions(),
        }
    }

    /// Default allowed operations
    fn default_allowed_operations() -> Set<Operation> {
        let mut ops = Set::new();
        ops.insert(Operation::Arithmetic);
        ops.insert(Operation::Comparison);
        ops.insert(Operation::Logic);
        ops.insert(Operation::ArrayOps);
        ops.insert(Operation::StringOps);
        ops.insert(Operation::ControlFlow);
        ops.insert(Operation::FunctionCall);
        ops.insert(Operation::TypeOps);
        ops.insert(Operation::ASTOps);
        ops.insert(Operation::PatternMatch);
        ops
    }

    /// Default allowed pure functions (whitelist)
    fn default_allowed_functions() -> Set<Text> {
        let mut funcs = Set::new();

        // Type introspection intrinsics
        funcs.insert(Text::from("type_of"));
        funcs.insert(Text::from("type_name"));
        funcs.insert(Text::from("type_fields"));
        funcs.insert(Text::from("fields_of"));
        funcs.insert(Text::from("variants_of"));
        funcs.insert(Text::from("field_access"));
        funcs.insert(Text::from("protocols_of"));

        // Type predicates
        funcs.insert(Text::from("is_struct"));
        funcs.insert(Text::from("is_enum"));
        funcs.insert(Text::from("is_tuple"));
        funcs.insert(Text::from("is_protocol"));
        funcs.insert(Text::from("implements"));

        // Asset loading functions (require `using BuildAssets` context)
        funcs.insert(Text::from("load_build_asset"));
        funcs.insert(Text::from("include_str"));
        funcs.insert(Text::from("include_bytes"));
        funcs.insert(Text::from("include_file"));
        funcs.insert(Text::from("embed_file"));

        // Compiler diagnostics
        funcs.insert(Text::from("compile_error"));
        funcs.insert(Text::from("compile_warning"));
        funcs.insert(Text::from("compile_time"));
        funcs.insert(Text::from("cfg"));

        funcs
    }

    /// Default allowed modules (whitelist)
    fn default_allowed_modules() -> Set<Text> {
        let mut modules = Set::new();
        modules.insert(Text::from("verum.meta"));
        modules.insert(Text::from("std.meta"));
        modules.insert(Text::from("core"));
        modules
    }

    /// Forbidden filesystem operations
    fn default_filesystem_functions() -> Set<Text> {
        let mut funcs = Set::new();

        // std.fs module functions
        funcs.insert(Text::from("std.fs.read"));
        funcs.insert(Text::from("std.fs.read_to_string"));
        funcs.insert(Text::from("std.fs.read_to_bytes"));
        funcs.insert(Text::from("std.fs.write"));
        funcs.insert(Text::from("std.fs.write_string"));
        funcs.insert(Text::from("std.fs.write_bytes"));
        funcs.insert(Text::from("std.fs.create"));
        funcs.insert(Text::from("std.fs.create_file"));
        funcs.insert(Text::from("std.fs.create_dir"));
        funcs.insert(Text::from("std.fs.delete"));
        funcs.insert(Text::from("std.fs.delete_file"));
        funcs.insert(Text::from("std.fs.delete_dir"));
        funcs.insert(Text::from("std.fs.rename"));
        funcs.insert(Text::from("std.fs.move"));
        funcs.insert(Text::from("std.fs.copy"));
        funcs.insert(Text::from("std.fs.exists"));
        funcs.insert(Text::from("std.fs.is_file"));
        funcs.insert(Text::from("std.fs.is_dir"));
        funcs.insert(Text::from("std.fs.metadata"));
        funcs.insert(Text::from("std.fs.open"));
        funcs.insert(Text::from("std.fs.open_file"));
        funcs.insert(Text::from("std.fs.mkdir"));
        funcs.insert(Text::from("std.fs.rmdir"));
        funcs.insert(Text::from("std.fs.list_dir"));
        funcs.insert(Text::from("std.fs.walk_dir"));
        funcs.insert(Text::from("std.fs.canonicalize"));
        funcs.insert(Text::from("std.fs.symlink"));

        // Short forms
        funcs.insert(Text::from("read_file"));
        funcs.insert(Text::from("write_file"));
        funcs.insert(Text::from("open_file"));
        funcs.insert(Text::from("create_file"));
        funcs.insert(Text::from("delete_file"));

        funcs
    }

    /// Forbidden network operations
    fn default_network_functions() -> Set<Text> {
        let mut funcs = Set::new();

        // std.net module functions
        funcs.insert(Text::from("std.net.tcp_connect"));
        funcs.insert(Text::from("std.net.tcp_listen"));
        funcs.insert(Text::from("std.net.tcp_accept"));
        funcs.insert(Text::from("std.net.udp_bind"));
        funcs.insert(Text::from("std.net.udp_send"));
        funcs.insert(Text::from("std.net.udp_recv"));
        funcs.insert(Text::from("std.net.http_get"));
        funcs.insert(Text::from("std.net.http_post"));
        funcs.insert(Text::from("std.net.http_put"));
        funcs.insert(Text::from("std.net.http_delete"));
        funcs.insert(Text::from("std.net.http_request"));
        funcs.insert(Text::from("std.net.socket"));
        funcs.insert(Text::from("std.net.connect"));
        funcs.insert(Text::from("std.net.listen"));
        funcs.insert(Text::from("std.net.accept"));
        funcs.insert(Text::from("std.net.send"));
        funcs.insert(Text::from("std.net.recv"));
        funcs.insert(Text::from("std.net.dns_resolve"));
        funcs.insert(Text::from("std.net.dns_lookup"));

        // Short forms
        funcs.insert(Text::from("http_get"));
        funcs.insert(Text::from("http_post"));
        funcs.insert(Text::from("tcp_connect"));
        funcs.insert(Text::from("udp_send"));

        funcs
    }

    /// Forbidden process operations
    fn default_process_functions() -> Set<Text> {
        let mut funcs = Set::new();

        // std.process module functions
        funcs.insert(Text::from("std.process.spawn"));
        funcs.insert(Text::from("std.process.exec"));
        funcs.insert(Text::from("std.process.execute"));
        funcs.insert(Text::from("std.process.run"));
        funcs.insert(Text::from("std.process.command"));
        funcs.insert(Text::from("std.process.shell"));
        funcs.insert(Text::from("std.process.system"));
        funcs.insert(Text::from("std.process.exit"));
        funcs.insert(Text::from("std.process.abort"));
        funcs.insert(Text::from("std.process.kill"));
        funcs.insert(Text::from("std.process.wait"));

        // Short forms
        funcs.insert(Text::from("spawn_process"));
        funcs.insert(Text::from("exec"));
        funcs.insert(Text::from("system"));
        funcs.insert(Text::from("shell"));

        funcs
    }

    /// Forbidden time operations (non-deterministic)
    fn default_time_functions() -> Set<Text> {
        let mut funcs = Set::new();

        // Qualified forms
        funcs.insert(Text::from("std.time.now"));
        funcs.insert(Text::from("std.time.current_time"));
        funcs.insert(Text::from("std.time.current_timestamp"));
        funcs.insert(Text::from("std.time.system_time"));
        funcs.insert(Text::from("std.time.instant"));
        funcs.insert(Text::from("std.time.sleep"));
        funcs.insert(Text::from("std.time.delay"));
        funcs.insert(Text::from("std.time.current_time_millis"));

        // Short forms
        funcs.insert(Text::from("now"));
        funcs.insert(Text::from("current_time"));
        funcs.insert(Text::from("current_time_millis"));
        funcs.insert(Text::from("current_timestamp"));
        funcs.insert(Text::from("system_time"));
        funcs.insert(Text::from("instant"));
        funcs.insert(Text::from("sleep"));
        funcs.insert(Text::from("delay"));

        funcs
    }

    /// Forbidden environment operations
    fn default_env_functions() -> Set<Text> {
        let mut funcs = Set::new();

        funcs.insert(Text::from("std.env.var"));
        funcs.insert(Text::from("std.env.get"));
        funcs.insert(Text::from("std.env.set"));
        funcs.insert(Text::from("std.env.set_var"));
        funcs.insert(Text::from("std.env.remove"));
        funcs.insert(Text::from("std.env.remove_var"));
        funcs.insert(Text::from("std.env.vars"));
        funcs.insert(Text::from("std.env.args"));
        funcs.insert(Text::from("std.env.current_dir"));
        funcs.insert(Text::from("std.env.current_exe"));
        funcs.insert(Text::from("std.env.home_dir"));
        funcs.insert(Text::from("std.env.temp_dir"));

        // Short forms
        funcs.insert(Text::from("env"));
        funcs.insert(Text::from("setenv"));

        funcs
    }

    /// Forbidden random operations (non-deterministic without seed)
    fn default_random_functions() -> Set<Text> {
        let mut funcs = Set::new();

        funcs.insert(Text::from("std.random.gen"));
        funcs.insert(Text::from("std.random.rand"));
        funcs.insert(Text::from("std.random.random"));
        funcs.insert(Text::from("std.random.thread_rng"));
        funcs.insert(Text::from("std.random.uuid"));
        funcs.insert(Text::from("std.random.uuid_v4"));

        funcs
    }

    /// Forbidden unsafe memory operations
    fn default_unsafe_functions() -> Set<Text> {
        let mut funcs = Set::new();

        funcs.insert(Text::from("std.mem.transmute"));
        funcs.insert(Text::from("std.mem.transmute_copy"));
        funcs.insert(Text::from("std.ptr.write"));
        funcs.insert(Text::from("std.ptr.write_volatile"));
        funcs.insert(Text::from("std.ptr.read"));
        funcs.insert(Text::from("std.ptr.read_volatile"));
        funcs.insert(Text::from("std.ptr.copy"));
        funcs.insert(Text::from("std.ptr.swap"));
        funcs.insert(Text::from("std.ptr.offset"));
        funcs.insert(Text::from("std.intrinsics"));

        funcs
    }

    /// Forbidden FFI operations
    fn default_ffi_functions() -> Set<Text> {
        let mut funcs = Set::new();

        funcs.insert(Text::from("std.ffi.call"));
        funcs.insert(Text::from("std.ffi.call_c"));
        funcs.insert(Text::from("std.ffi.load_library"));
        funcs.insert(Text::from("std.ffi.dlopen"));
        funcs.insert(Text::from("std.ffi.dlsym"));

        funcs
    }

    /// Asset loading functions (require BuildAssets context)
    fn default_asset_loading_functions() -> Set<Text> {
        let mut funcs = Set::new();

        funcs.insert(Text::from("load_build_asset"));
        funcs.insert(Text::from("include_file"));
        funcs.insert(Text::from("include_bytes"));
        funcs.insert(Text::from("include_str"));

        funcs
    }

    // ========================================================================
    // Function category checkers
    // ========================================================================

    /// Check if a function is a filesystem operation
    pub fn is_filesystem_function(&self, name: &Text) -> bool {
        self.filesystem_functions.contains(name)
            || name.as_str().starts_with("std.fs.")
            || name.as_str().contains("read_file")
            || name.as_str().contains("write_file")
            || name.as_str().contains("open_file")
    }

    /// Check if a function is a network operation
    pub fn is_network_function(&self, name: &Text) -> bool {
        self.network_functions.contains(name)
            || name.as_str().starts_with("std.net.")
            || name.as_str().contains("http_")
            || name.as_str().contains("tcp_")
            || name.as_str().contains("udp_")
    }

    /// Check if a function is a process operation
    pub fn is_process_function(&self, name: &Text) -> bool {
        self.process_functions.contains(name) || name.as_str().starts_with("std.process.")
    }

    /// Check if a function is a time operation
    pub fn is_time_function(&self, name: &Text) -> bool {
        self.time_functions.contains(name)
            || (name.as_str().starts_with("std.time.") && name.as_str() != "std.time.duration")
    }

    /// Check if a function is an environment operation
    pub fn is_env_function(&self, name: &Text) -> bool {
        self.env_functions.contains(name)
            || (name.as_str().starts_with("std.env.") && name.as_str() != "std.env.cfg")
    }

    /// Check if a function is a random operation
    pub fn is_random_function(&self, name: &Text) -> bool {
        self.random_functions.contains(name)
            || (name.as_str().starts_with("std.random.") && !name.as_str().contains("from_seed"))
    }

    /// Check if a function is an unsafe operation
    pub fn is_unsafe_function(&self, name: &Text) -> bool {
        self.unsafe_functions.contains(name)
            || name.as_str().starts_with("std.mem.transmute")
            || name.as_str().starts_with("std.ptr.")
            || name.as_str().starts_with("std.intrinsics.")
    }

    /// Check if a function is an FFI operation
    pub fn is_ffi_function(&self, name: &Text) -> bool {
        self.ffi_functions.contains(name) || name.as_str().starts_with("std.ffi.")
    }

    /// Check if a function is an asset loading operation
    pub fn is_asset_loading_function(&self, name: &Text) -> bool {
        self.asset_loading_functions.contains(name)
            || name.as_str() == "load_build_asset"
            || name.as_str() == "include_str"
            || name.as_str() == "include_bytes"
            || name.as_str() == "include_file"
            || name.as_str() == "embed_file"
    }

    /// Check if a function name is a forbidden I/O operation
    pub fn is_forbidden_io_function(&self, name: &Text) -> bool {
        self.is_filesystem_function(name)
            || self.is_network_function(name)
            || self.is_process_function(name)
            || self.is_time_function(name)
            || self.is_env_function(name)
            || self.is_random_function(name)
            || self.is_unsafe_function(name)
            || self.is_ffi_function(name)
    }

    /// Check if an operation is allowed
    pub fn is_operation_allowed(&self, op: Operation) -> bool {
        self.allowed_operations.contains(&op)
    }
}

impl Default for AllowlistRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_forbidden_operations() {
        let registry = AllowlistRegistry::new();

        // File system operations
        assert!(registry.is_filesystem_function(&Text::from("std.fs.read")));
        assert!(registry.is_filesystem_function(&Text::from("std.fs.write")));

        // Network operations
        assert!(registry.is_network_function(&Text::from("std.net.http_get")));
        assert!(registry.is_network_function(&Text::from("std.net.tcp_connect")));

        // Process operations
        assert!(registry.is_process_function(&Text::from("std.process.spawn")));
        assert!(registry.is_process_function(&Text::from("std.process.exec")));

        // Time operations
        assert!(registry.is_time_function(&Text::from("std.time.now")));

        // Environment operations
        assert!(registry.is_env_function(&Text::from("std.env.var")));

        // Random operations
        assert!(registry.is_random_function(&Text::from("std.random.gen")));

        // Unsafe operations
        assert!(registry.is_unsafe_function(&Text::from("std.mem.transmute")));

        // FFI operations
        assert!(registry.is_ffi_function(&Text::from("std.ffi.call")));
    }

    #[test]
    fn test_allowed_operations() {
        let registry = AllowlistRegistry::new();
        assert!(registry.is_operation_allowed(Operation::Arithmetic));
        assert!(registry.is_operation_allowed(Operation::Comparison));
        assert!(registry.is_operation_allowed(Operation::FunctionCall));
    }

    #[test]
    fn test_short_form_forbidden() {
        let registry = AllowlistRegistry::new();

        // Network short forms
        assert!(registry.is_network_function(&Text::from("http_get")));
        assert!(registry.is_network_function(&Text::from("http_post")));
        assert!(registry.is_network_function(&Text::from("tcp_connect")));

        // Process short forms
        assert!(registry.is_process_function(&Text::from("exec")));
        assert!(registry.is_process_function(&Text::from("shell")));
        assert!(registry.is_process_function(&Text::from("system")));

        // Random short forms (via starts_with check)
        assert!(registry.is_random_function(&Text::from("std.random.gen")));

        // Combined check
        assert!(registry.is_forbidden_io_function(&Text::from("http_get")));
        assert!(registry.is_forbidden_io_function(&Text::from("exec")));
        assert!(registry.is_forbidden_io_function(&Text::from("std.env.var")));
        assert!(registry.is_forbidden_io_function(&Text::from("std.fs.read")));
    }

    #[test]
    fn test_asset_loading_functions() {
        let registry = AllowlistRegistry::new();
        assert!(registry.is_asset_loading_function(&Text::from("include_str")));
        assert!(registry.is_asset_loading_function(&Text::from("include_bytes")));
        assert!(registry.is_asset_loading_function(&Text::from("load_build_asset")));
    }
}
