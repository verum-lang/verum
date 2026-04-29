//! Industrial-grade JIT Engine for MLIR-based code execution.
//!
//! Provides comprehensive JIT compilation and execution with:
//!
//! - Type-safe function calling via generic traits
//! - Packed calling convention support (MLIR standard)
//! - External symbol registration with symbol resolver integration
//! - Compilation caching and statistics
//! - Callback mechanism for JIT → Rust interop
//! - Multi-threaded support
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │                           JIT Execution Pipeline                             │
//! └─────────────────────────────────────────────────────────────────────────────┘
//!
//!   MLIR Module (lowered to LLVM dialect)
//!         │
//!         ▼
//! ┌─────────────────┐
//! │   JitCompiler   │  Optimization + Code generation
//! │                 │  (melior ExecutionEngine)
//! └────────┬────────┘
//!          │
//!          ▼
//! ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
//! │  Symbol Linker  │───▶│   JIT Cache     │───▶│  Code Memory    │
//! │ (SymbolResolver)│    │  (compiled fn)  │    │  (executable)   │
//! └─────────────────┘    └─────────────────┘    └─────────────────┘
//!                                │
//!                                ▼
//!                        ┌─────────────────┐
//!                        │  TypedCaller    │  Type-safe invocation
//!                        │  (JitCallable)  │
//!                        └─────────────────┘
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use crate::mlir::jit::{JitEngine, JitConfig};
//!
//! // Create JIT with optimization level 2
//! let config = JitConfig::new().optimization_level(2);
//! let engine = JitEngine::compile(&module, config)?;
//!
//! // Type-safe function call
//! let result: i64 = engine.call("add", (1i64, 2i64))?;
//! assert_eq!(result, 3);
//!
//! // Or use packed calling convention
//! let mut args = [1i64 as *mut (), 2i64 as *mut ()];
//! let mut result = 0i64;
//! unsafe { engine.invoke_packed("add", &mut args, &mut result)?; }
//! ```

use crate::mlir::error::{MlirError, Result};
use crate::mlir::jit::symbol_resolver::SymbolResolver;
use dashmap::DashMap;
use verum_mlir::ir::Module;
use verum_mlir::ExecutionEngine;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::any::TypeId;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use verum_common::Text;

// ============================================================================
// JIT Configuration
// ============================================================================

/// Configuration for JIT compilation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JitConfig {
    /// Optimization level (0-3).
    pub optimization_level: usize,

    /// Enable object file caching.
    pub enable_object_cache: bool,

    /// Shared library paths for symbol resolution.
    pub shared_library_paths: Vec<String>,

    /// Enable verbose output.
    pub verbose: bool,

    /// Enable debug info.
    pub enable_debug_info: bool,

    /// Maximum number of cached compilations.
    pub max_cache_size: usize,

    /// Enable multi-threading in LLVM.
    pub enable_multithreading: bool,

    /// Object dump directory (for debugging).
    pub object_dump_dir: Option<String>,
}

impl JitConfig {
    /// Create new default configuration.
    pub fn new() -> Self {
        Self {
            optimization_level: 2,
            enable_object_cache: true,
            shared_library_paths: Vec::new(),
            verbose: false,
            enable_debug_info: false,
            max_cache_size: 1024,
            enable_multithreading: true,
            object_dump_dir: None,
        }
    }

    /// Builder: set optimization level.
    pub fn optimization_level(mut self, level: usize) -> Self {
        self.optimization_level = level.min(3);
        self
    }

    /// Builder: enable object caching.
    pub fn enable_object_cache(mut self, enable: bool) -> Self {
        self.enable_object_cache = enable;
        self
    }

    /// Builder: add shared library path.
    pub fn add_shared_library(mut self, path: impl Into<String>) -> Self {
        self.shared_library_paths.push(path.into());
        self
    }

    /// Builder: set verbose mode.
    pub fn verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Builder: enable debug info.
    pub fn enable_debug_info(mut self, enable: bool) -> Self {
        self.enable_debug_info = enable;
        self
    }

    /// Builder: set max cache size.
    pub fn max_cache_size(mut self, size: usize) -> Self {
        self.max_cache_size = size;
        self
    }

    /// Builder: set object dump directory.
    pub fn object_dump_dir(mut self, dir: impl Into<String>) -> Self {
        self.object_dump_dir = Some(dir.into());
        self
    }

    /// Create a development configuration (debug, verbose).
    pub fn development() -> Self {
        Self::new()
            .optimization_level(0)
            .verbose(true)
            .enable_debug_info(true)
    }

    /// Create a production configuration (optimized, no debug).
    pub fn production() -> Self {
        Self::new()
            .optimization_level(3)
            .verbose(false)
            .enable_debug_info(false)
    }

    // Legacy builder methods for backwards compatibility
    /// Set optimization level (legacy).
    pub fn with_optimization_level(self, level: usize) -> Self {
        self.optimization_level(level)
    }

    /// Add shared library (legacy).
    pub fn with_shared_library(mut self, path: impl Into<String>) -> Self {
        self.shared_library_paths.push(path.into());
        self
    }

    /// Enable object cache (legacy).
    pub fn with_object_cache(self, enable: bool) -> Self {
        self.enable_object_cache(enable)
    }

    /// Set verbose mode (legacy).
    pub fn with_verbose(self, verbose: bool) -> Self {
        self.verbose(verbose)
    }
}

impl Default for JitConfig {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// JIT Statistics
// ============================================================================

/// Comprehensive JIT statistics.
#[derive(Debug, Default)]
pub struct JitStats {
    /// Number of successful compilations.
    pub compilations: AtomicU64,

    /// Number of function calls.
    pub invocations: AtomicU64,

    /// Number of cache hits.
    pub cache_hits: AtomicU64,

    /// Number of cache misses.
    pub cache_misses: AtomicU64,

    /// Number of symbol resolutions.
    pub symbol_resolutions: AtomicU64,

    /// Number of callback invocations.
    pub callback_invocations: AtomicU64,

    /// Total compilation time (microseconds).
    pub total_compilation_time_us: AtomicU64,

    /// Total invocation time (microseconds).
    pub total_invocation_time_us: AtomicU64,

    /// Number of errors.
    pub errors: AtomicU64,

    // Legacy fields for backwards compatibility
    /// Number of functions compiled (legacy).
    pub functions_compiled: AtomicU64,

    /// Number of function calls (legacy).
    pub function_calls: AtomicU64,
}

impl JitStats {
    /// Create new statistics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get average compilation time in microseconds.
    pub fn avg_compilation_time_us(&self) -> f64 {
        let compilations = self.compilations.load(Ordering::Relaxed);
        if compilations == 0 {
            0.0
        } else {
            self.total_compilation_time_us.load(Ordering::Relaxed) as f64 / compilations as f64
        }
    }

    /// Get average invocation time in microseconds.
    pub fn avg_invocation_time_us(&self) -> f64 {
        let invocations = self.invocations.load(Ordering::Relaxed);
        if invocations == 0 {
            0.0
        } else {
            self.total_invocation_time_us.load(Ordering::Relaxed) as f64 / invocations as f64
        }
    }

    /// Get cache hit rate.
    pub fn cache_hit_rate(&self) -> f64 {
        let total = self.cache_hits.load(Ordering::Relaxed) + self.cache_misses.load(Ordering::Relaxed);
        if total == 0 {
            0.0
        } else {
            self.cache_hits.load(Ordering::Relaxed) as f64 / total as f64
        }
    }

    /// Generate summary report.
    pub fn summary(&self) -> JitStatsSummary {
        JitStatsSummary {
            compilations: self.compilations.load(Ordering::Relaxed),
            invocations: self.invocations.load(Ordering::Relaxed),
            cache_hit_rate: self.cache_hit_rate(),
            avg_compilation_time_us: self.avg_compilation_time_us(),
            avg_invocation_time_us: self.avg_invocation_time_us(),
            errors: self.errors.load(Ordering::Relaxed),
        }
    }
}

/// Summary of JIT statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JitStatsSummary {
    pub compilations: u64,
    pub invocations: u64,
    pub cache_hit_rate: f64,
    pub avg_compilation_time_us: f64,
    pub avg_invocation_time_us: f64,
    pub errors: u64,
}

// ============================================================================
// Type-Safe Calling Traits
// ============================================================================

/// Trait for types that can be passed to JIT functions.
///
/// # Safety
///
/// Implementors must ensure the type is FFI-safe and can be safely
/// passed across the JIT boundary.
pub unsafe trait JitArg {
    /// Get pointer to the value for passing to JIT.
    fn as_ptr(&self) -> *const ();

    /// Get mutable pointer for receiving results.
    fn as_mut_ptr(&mut self) -> *mut ();
}

// Implement JitArg for primitive types
macro_rules! impl_jit_arg {
    ($($ty:ty),*) => {
        $(
            unsafe impl JitArg for $ty {
                fn as_ptr(&self) -> *const () {
                    self as *const $ty as *const ()
                }

                fn as_mut_ptr(&mut self) -> *mut () {
                    self as *mut $ty as *mut ()
                }
            }
        )*
    };
}

impl_jit_arg!(i8, i16, i32, i64, u8, u16, u32, u64, f32, f64, bool, usize, isize);

// Implement for pointers
unsafe impl<T> JitArg for *const T {
    fn as_ptr(&self) -> *const () {
        self as *const *const T as *const ()
    }

    fn as_mut_ptr(&mut self) -> *mut () {
        self as *mut *const T as *mut ()
    }
}

unsafe impl<T> JitArg for *mut T {
    fn as_ptr(&self) -> *const () {
        self as *const *mut T as *const ()
    }

    fn as_mut_ptr(&mut self) -> *mut () {
        self as *mut *mut T as *mut ()
    }
}

/// Trait for tuple argument packing.
pub trait JitArgs {
    /// Pack arguments into a slice of pointers.
    fn pack(&self) -> Vec<*mut ()>;

    /// Number of arguments.
    fn len(&self) -> usize;

    /// Check if empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// Implement JitArgs for tuples
impl JitArgs for () {
    fn pack(&self) -> Vec<*mut ()> {
        vec![]
    }

    fn len(&self) -> usize {
        0
    }
}

macro_rules! impl_jit_args_tuple {
    ($(($($T:ident),+)),*) => {
        $(
            impl<$($T: JitArg + Copy),+> JitArgs for ($($T,)+) {
                #[allow(non_snake_case)]
                fn pack(&self) -> Vec<*mut ()> {
                    let ($($T,)+) = self;
                    vec![$(JitArg::as_ptr($T) as *mut ()),+]
                }

                fn len(&self) -> usize {
                    impl_jit_args_tuple!(@count $($T),+)
                }
            }
        )*
    };
    (@count $($T:ident),+) => {
        <[()]>::len(&[$(impl_jit_args_tuple!(@void $T)),+])
    };
    (@void $T:ident) => { () };
}

impl_jit_args_tuple!(
    (A),
    (A, B),
    (A, B, C),
    (A, B, C, D),
    (A, B, C, D, E),
    (A, B, C, D, E, F),
    (A, B, C, D, E, F, G),
    (A, B, C, D, E, F, G, H)
);

/// Trait for JIT return types.
pub trait JitReturn: Sized + Default {
    /// Initialize storage for the return value.
    fn init() -> Self {
        Self::default()
    }
}

impl JitReturn for () {}

macro_rules! impl_jit_return {
    ($($ty:ty),*) => {
        $(
            impl JitReturn for $ty {}
        )*
    };
}

impl_jit_return!(i8, i16, i32, i64, u8, u16, u32, u64, f32, f64, bool, usize, isize);

impl<T> JitReturn for *const T {
    fn init() -> Self {
        std::ptr::null()
    }
}

impl<T> JitReturn for *mut T {
    fn init() -> Self {
        std::ptr::null_mut()
    }
}

// ============================================================================
// Callback System
// ============================================================================

/// Type for callback functions that can be called from JIT code.
pub type JitCallback = Box<dyn Fn(&[*mut ()]) -> *mut () + Send + Sync>;

/// Callback registry for JIT → Rust interop.
pub struct CallbackRegistry {
    callbacks: DashMap<Text, JitCallback>,
    stats: Arc<JitStats>,
}

impl CallbackRegistry {
    /// Create new callback registry.
    pub fn new(stats: Arc<JitStats>) -> Self {
        Self {
            callbacks: DashMap::new(),
            stats,
        }
    }

    /// Register a callback.
    pub fn register<F>(&self, name: impl Into<Text>, callback: F)
    where
        F: Fn(&[*mut ()]) -> *mut () + Send + Sync + 'static,
    {
        self.callbacks.insert(name.into(), Box::new(callback));
    }

    /// Invoke a callback.
    pub fn invoke(&self, name: &str, args: &[*mut ()]) -> Result<*mut ()> {
        self.stats.callback_invocations.fetch_add(1, Ordering::Relaxed);

        self.callbacks
            .get(&Text::from(name))
            .map(|cb| cb(args))
            .ok_or_else(|| MlirError::JitCallbackError {
                message: Text::from(format!("Callback not found: {}", name)),
            })
    }

    /// Check if callback exists.
    pub fn contains(&self, name: &str) -> bool {
        self.callbacks.contains_key(&Text::from(name))
    }

    /// Get all registered callback names.
    pub fn names(&self) -> Vec<Text> {
        self.callbacks.iter().map(|e| e.key().clone()).collect()
    }
}

// ============================================================================
// Compiled Function Handle
// ============================================================================

/// Handle to a compiled JIT function.
pub struct CompiledFunction {
    /// Function name.
    pub name: Text,

    /// Function address.
    pub address: *mut (),

    /// Parameter type IDs (for validation).
    pub param_types: Vec<TypeId>,

    /// Return type ID.
    pub return_type: TypeId,

    /// Compilation timestamp.
    pub compiled_at: instant::Instant,

    /// Number of invocations.
    pub invocations: AtomicU64,
}

// SAFETY: CompiledFunction can be sent/shared across threads because:
// - `address` is a raw pointer to JIT-compiled machine code that is immutable once compiled
// - `invocations` is an AtomicU64, inherently thread-safe
// - All other fields are immutable after construction (name, signature, compiled_at)
// - The pointed-to code lives in JIT memory that outlives all CompiledFunction references
unsafe impl Send for CompiledFunction {}
unsafe impl Sync for CompiledFunction {}

// ============================================================================
// JIT Engine
// ============================================================================

/// Industrial-grade JIT engine.
///
/// Provides:
/// - Type-safe function calling
/// - External symbol resolution
/// - Callback mechanism
/// - Compilation caching
/// - Comprehensive statistics
pub struct JitEngine {
    /// MLIR execution engine.
    engine: ExecutionEngine,

    /// Configuration.
    config: JitConfig,

    /// Symbol resolver for external symbols.
    symbol_resolver: Arc<SymbolResolver>,

    /// Callback registry.
    callbacks: CallbackRegistry,

    /// Compiled function cache.
    function_cache: DashMap<Text, Arc<CompiledFunction>>,

    /// Statistics.
    stats: Arc<JitStats>,

    /// Registered external symbols.
    registered_symbols: DashMap<Text, *mut ()>,
}

// SAFETY: JitEngine can be sent/shared across threads because:
// - `engine` (ExecutionEngine) manages its own thread safety for JIT code
// - `registered_symbols` (DashMap) is a concurrent map; the *mut () values point to
//   static library symbols or JIT code that outlives the engine
// - `function_cache` and `callbacks` use thread-safe containers (DashMap, Arc)
// - `stats` is wrapped in Arc with atomic counters
// - All raw pointers are to immutable, long-lived code/symbol addresses
unsafe impl Send for JitEngine {}
unsafe impl Sync for JitEngine {}

impl JitEngine {
    /// Create a new JIT engine from an MLIR module (legacy constructor).
    pub fn new(module: &Module<'_>, config: JitConfig) -> Result<Self> {
        Self::compile(module, config)
    }

    /// Compile a module and create a JIT engine.
    pub fn compile(module: &Module<'_>, config: JitConfig) -> Result<Self> {
        let start = instant::Instant::now();

        // Prepare shared library paths
        let lib_paths: Vec<&str> = config.shared_library_paths.iter().map(|s| s.as_str()).collect();

        // Create execution engine
        let engine = ExecutionEngine::new(
            module,
            config.optimization_level,
            &lib_paths,
            config.enable_object_cache,
        );

        let stats = Arc::new(JitStats::new());
        let symbol_resolver = Arc::new(SymbolResolver::new().with_verbose(config.verbose));

        let jit_engine = Self {
            engine,
            config,
            symbol_resolver,
            callbacks: CallbackRegistry::new(stats.clone()),
            function_cache: DashMap::new(),
            stats: stats.clone(),
            registered_symbols: DashMap::new(),
        };

        // Record compilation time
        let elapsed = start.elapsed();
        stats.total_compilation_time_us.fetch_add(elapsed.as_micros() as u64, Ordering::Relaxed);
        stats.compilations.fetch_add(1, Ordering::Relaxed);

        if jit_engine.config.verbose {
            tracing::info!("JIT engine created in {:?}", elapsed);
        }

        Ok(jit_engine)
    }

    /// Get the symbol resolver.
    pub fn symbol_resolver(&self) -> &SymbolResolver {
        &self.symbol_resolver
    }

    /// Get the callback registry.
    pub fn callbacks(&self) -> &CallbackRegistry {
        &self.callbacks
    }

    /// Get statistics.
    pub fn stats(&self) -> &JitStats {
        &self.stats
    }

    /// Register an external symbol.
    ///
    /// # Safety
    ///
    /// The pointer must be valid for the lifetime of the engine.
    pub unsafe fn register_symbol_unsafe(&self, name: impl Into<Text>, ptr: *mut ()) {
        let name = name.into();
        self.registered_symbols.insert(name.clone(), ptr);
        // SAFETY: Caller guarantees pointer validity
        unsafe { self.engine.register_symbol(name.as_str(), ptr) };
    }

    /// Register a symbol (legacy method).
    pub fn register_symbol(&self, name: &str, ptr: *mut ()) -> Result<()> {
        // SAFETY: Caller guarantees pointer validity
        unsafe {
            self.engine.register_symbol(name, ptr);
        }
        self.registered_symbols.insert(Text::from(name), ptr);

        if self.config.verbose {
            tracing::debug!("Registered JIT symbol: {}", name);
        }

        Ok(())
    }

    /// Register stdlib symbols for resolution (legacy method).
    pub fn register_stdlib(&self) -> Result<()> {
        let symbols = self.symbol_resolver.stdlib_symbols();
        for (name, ptr) in symbols {
            self.register_symbol(name.as_str(), ptr)?;
        }
        Ok(())
    }

    /// Register all symbols from a symbol resolver.
    pub fn register_symbols_from_resolver(&self) -> Result<usize> {
        let symbols = self.symbol_resolver.stdlib_symbols();
        let count = symbols.len();

        for (name, ptr) in symbols {
            // SAFETY: Symbols come from loaded libraries
            unsafe {
                self.engine.register_symbol(name.as_str(), ptr);
            }
            self.registered_symbols.insert(name, ptr);
        }

        self.stats.symbol_resolutions.fetch_add(count as u64, Ordering::Relaxed);
        Ok(count)
    }

    /// Load and register verum_std symbols.
    pub fn load_verum_std(&self) -> Result<usize> {
        self.symbol_resolver.load_verum_std()?;
        let count = self.symbol_resolver.preload_stdlib_symbols()?;
        self.register_symbols_from_resolver()
    }

    /// Look up a symbol address.
    pub fn lookup(&self, name: &str) -> Option<*mut ()> {
        let ptr = self.engine.lookup(name);
        if ptr.is_null() {
            // Try registered symbols
            self.registered_symbols.get(&Text::from(name)).map(|r| *r)
        } else {
            Some(ptr)
        }
    }

    /// Type-safe function call.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let result: i64 = engine.call("add", (1i64, 2i64))?;
    /// ```
    pub fn call<Args, Ret>(&self, name: &str, args: Args) -> Result<Ret>
    where
        Args: JitArgs,
        Ret: JitReturn + JitArg,
    {
        let start = instant::Instant::now();

        // Pack arguments
        let mut packed_args = args.pack();

        // Initialize return value
        let mut result = Ret::init();

        // Insert result pointer at the front (MLIR packed convention)
        packed_args.insert(0, result.as_mut_ptr());

        // Call via packed convention
        unsafe {
            self.engine
                .invoke_packed(name, &mut packed_args)
                .map_err(|e| MlirError::JitInvocationError {
                    function: Text::from(name),
                    message: Text::from(format!("{:?}", e)),
                })?;
        }

        // Record statistics
        let elapsed = start.elapsed();
        self.stats.invocations.fetch_add(1, Ordering::Relaxed);
        self.stats.function_calls.fetch_add(1, Ordering::Relaxed);
        self.stats.total_invocation_time_us.fetch_add(elapsed.as_micros() as u64, Ordering::Relaxed);

        Ok(result)
    }

    /// Call a void function.
    pub fn call_void_safe<Args>(&self, name: &str, args: Args) -> Result<()>
    where
        Args: JitArgs,
    {
        let start = instant::Instant::now();

        let mut packed_args = args.pack();

        unsafe {
            self.engine
                .invoke_packed(name, &mut packed_args)
                .map_err(|e| MlirError::JitInvocationError {
                    function: Text::from(name),
                    message: Text::from(format!("{:?}", e)),
                })?;
        }

        let elapsed = start.elapsed();
        self.stats.invocations.fetch_add(1, Ordering::Relaxed);
        self.stats.function_calls.fetch_add(1, Ordering::Relaxed);
        self.stats.total_invocation_time_us.fetch_add(elapsed.as_micros() as u64, Ordering::Relaxed);

        Ok(())
    }

    /// Call a function with the given arguments (legacy).
    ///
    /// # Safety
    ///
    /// The argument types must match the function signature.
    pub unsafe fn call_raw(&self, name: &str, args: &mut [*mut ()]) -> Result<()> {
        self.stats.function_calls.fetch_add(1, Ordering::Relaxed);
        self.stats.invocations.fetch_add(1, Ordering::Relaxed);

        // SAFETY: Caller guarantees argument types match function signature
        unsafe {
            self.engine
                .invoke_packed(name, args)
                .map_err(|e| MlirError::JitInvocationError {
                    function: Text::from(name),
                    message: Text::from(format!("{:?}", e)),
                })
        }
    }

    /// Low-level invoke with packed arguments.
    ///
    /// # Safety
    ///
    /// Arguments must match the function signature exactly.
    pub unsafe fn invoke_packed(&self, name: &str, args: &mut [*mut ()]) -> Result<()> {
        // SAFETY: Caller guarantees arguments match function signature exactly
        unsafe {
            self.engine
                .invoke_packed(name, args)
                .map_err(|e| MlirError::JitInvocationError {
                    function: Text::from(name),
                    message: Text::from(format!("{:?}", e)),
                })
        }
    }

    /// Call function returning i64.
    pub fn call_i64(&self, name: &str, args: &[i64]) -> Result<i64> {
        let start = instant::Instant::now();
        let mut result: i64 = 0;

        let mut packed: Vec<*mut ()> = Vec::with_capacity(args.len() + 1);
        packed.push(&mut result as *mut i64 as *mut ());
        for arg in args {
            packed.push(arg as *const i64 as *mut ());
        }

        unsafe {
            self.engine
                .invoke_packed(name, &mut packed)
                .map_err(|e| MlirError::JitInvocationError {
                    function: Text::from(name),
                    message: Text::from(format!("{:?}", e)),
                })?;
        }

        let elapsed = start.elapsed();
        self.stats.invocations.fetch_add(1, Ordering::Relaxed);
        self.stats.function_calls.fetch_add(1, Ordering::Relaxed);
        self.stats.total_invocation_time_us.fetch_add(elapsed.as_micros() as u64, Ordering::Relaxed);

        Ok(result)
    }

    /// Call function returning i32.
    pub fn call_i32(&self, name: &str, args: &[i32]) -> Result<i32> {
        let start = instant::Instant::now();
        let mut result: i32 = 0;

        let mut packed: Vec<*mut ()> = Vec::with_capacity(args.len() + 1);
        packed.push(&mut result as *mut i32 as *mut ());
        for arg in args {
            packed.push(arg as *const i32 as *mut ());
        }

        unsafe {
            self.engine
                .invoke_packed(name, &mut packed)
                .map_err(|e| MlirError::JitInvocationError {
                    function: Text::from(name),
                    message: Text::from(format!("{:?}", e)),
                })?;
        }

        let elapsed = start.elapsed();
        self.stats.invocations.fetch_add(1, Ordering::Relaxed);
        self.stats.function_calls.fetch_add(1, Ordering::Relaxed);
        self.stats.total_invocation_time_us.fetch_add(elapsed.as_micros() as u64, Ordering::Relaxed);

        Ok(result)
    }

    /// Call function returning f64.
    pub fn call_f64(&self, name: &str, args: &[f64]) -> Result<f64> {
        let start = instant::Instant::now();
        let mut result: f64 = 0.0;

        let mut packed: Vec<*mut ()> = Vec::with_capacity(args.len() + 1);
        packed.push(&mut result as *mut f64 as *mut ());
        for arg in args {
            packed.push(arg as *const f64 as *mut ());
        }

        unsafe {
            self.engine
                .invoke_packed(name, &mut packed)
                .map_err(|e| MlirError::JitInvocationError {
                    function: Text::from(name),
                    message: Text::from(format!("{:?}", e)),
                })?;
        }

        let elapsed = start.elapsed();
        self.stats.invocations.fetch_add(1, Ordering::Relaxed);
        self.stats.function_calls.fetch_add(1, Ordering::Relaxed);
        self.stats.total_invocation_time_us.fetch_add(elapsed.as_micros() as u64, Ordering::Relaxed);

        Ok(result)
    }

    /// Call function with no arguments and void return (legacy).
    ///
    /// # Safety
    ///
    /// The function must have the signature `() -> ()`.
    pub unsafe fn call_void(&self, name: &str) -> Result<()> {
        // SAFETY: Caller guarantees function has () -> () signature
        unsafe { self.call_raw(name, &mut []) }
    }

    /// Call function returning pointer.
    pub fn call_ptr(&self, name: &str, args: &[*mut ()]) -> Result<*mut ()> {
        let start = instant::Instant::now();
        let mut result: *mut () = std::ptr::null_mut();

        let mut packed: Vec<*mut ()> = Vec::with_capacity(args.len() + 1);
        packed.push(&mut result as *mut *mut () as *mut ());
        packed.extend_from_slice(args);

        unsafe {
            self.engine
                .invoke_packed(name, &mut packed)
                .map_err(|e| MlirError::JitInvocationError {
                    function: Text::from(name),
                    message: Text::from(format!("{:?}", e)),
                })?;
        }

        let elapsed = start.elapsed();
        self.stats.invocations.fetch_add(1, Ordering::Relaxed);
        self.stats.function_calls.fetch_add(1, Ordering::Relaxed);
        self.stats.total_invocation_time_us.fetch_add(elapsed.as_micros() as u64, Ordering::Relaxed);

        Ok(result)
    }

    /// Dump the compiled module to an object file.
    pub fn dump_to_object_file(&self, path: impl AsRef<Path>) {
        self.engine.dump_to_object_file(path.as_ref().to_str().unwrap_or("output.o"));
    }

    /// Get function list (from registered symbols).
    pub fn functions(&self) -> Vec<Text> {
        self.registered_symbols.iter().map(|e| e.key().clone()).collect()
    }

    /// Check if a function is available.
    pub fn has_function(&self, name: &str) -> bool {
        self.lookup(name).is_some()
    }

    /// Get compiled function info.
    pub fn get_function(&self, name: &str) -> Option<Arc<CompiledFunction>> {
        // Check cache first
        if let Some(func) = self.function_cache.get(&Text::from(name)) {
            self.stats.cache_hits.fetch_add(1, Ordering::Relaxed);
            return Some(func.clone());
        }

        self.stats.cache_misses.fetch_add(1, Ordering::Relaxed);

        // Try to find the function
        if let Some(addr) = self.lookup(name) {
            let func = Arc::new(CompiledFunction {
                name: Text::from(name),
                address: addr,
                param_types: vec![],
                return_type: TypeId::of::<()>(),
                compiled_at: instant::Instant::now(),
                invocations: AtomicU64::new(0),
            });

            // Honour `JitConfig.max_cache_size`: when the cache
            // would exceed the configured cap, evict the oldest
            // entry by `compiled_at` to make room. The bound is a
            // soft cap (DashMap's per-shard locking means the
            // size check + insert is not strictly atomic), but
            // long-running sessions stay well below the
            // documented ceiling instead of growing unboundedly.
            // Without this gate the field was inert.
            if self.function_cache.len() >= self.config.max_cache_size
                && self.config.max_cache_size > 0
            {
                if let Some(oldest_key) = self
                    .function_cache
                    .iter()
                    .min_by_key(|entry| entry.value().compiled_at)
                    .map(|entry| entry.key().clone())
                {
                    self.function_cache.remove(&oldest_key);
                }
            }
            self.function_cache.insert(Text::from(name), func.clone());
            Some(func)
        } else {
            None
        }
    }

    /// Clear function cache.
    pub fn clear_cache(&self) {
        self.function_cache.clear();
    }

    /// Get configuration.
    pub fn config(&self) -> &JitConfig {
        &self.config
    }
}

// ============================================================================
// JIT Compiler (High-Level Interface)
// ============================================================================

/// High-level JIT compiler interface.
///
/// Provides a builder pattern for configuring and creating JIT engines.
pub struct JitCompiler {
    config: JitConfig,
    symbol_resolver: Option<Arc<SymbolResolver>>,
    preload_stdlib: bool,
}

impl JitCompiler {
    /// Create new JIT compiler.
    pub fn new() -> Self {
        Self {
            config: JitConfig::new(),
            symbol_resolver: None,
            preload_stdlib: true,
        }
    }

    /// Set configuration.
    pub fn config(mut self, config: JitConfig) -> Self {
        self.config = config;
        self
    }

    /// Set optimization level.
    pub fn optimization_level(mut self, level: usize) -> Self {
        self.config.optimization_level = level.min(3);
        self
    }

    /// Enable verbose output.
    pub fn verbose(mut self, verbose: bool) -> Self {
        self.config.verbose = verbose;
        self
    }

    /// Set custom symbol resolver.
    pub fn symbol_resolver(mut self, resolver: Arc<SymbolResolver>) -> Self {
        self.symbol_resolver = Some(resolver);
        self
    }

    /// Enable/disable stdlib preloading.
    pub fn preload_stdlib(mut self, preload: bool) -> Self {
        self.preload_stdlib = preload;
        self
    }

    /// Add shared library path.
    pub fn add_shared_library(mut self, path: impl Into<String>) -> Self {
        self.config.shared_library_paths.push(path.into());
        self
    }

    /// Compile a module.
    pub fn compile(self, module: &Module<'_>) -> Result<JitEngine> {
        let mut engine = JitEngine::compile(module, self.config)?;

        // If custom resolver provided, use it
        if let Some(resolver) = self.symbol_resolver {
            engine.symbol_resolver = resolver;
        }

        // Preload stdlib if requested
        if self.preload_stdlib {
            // Try to load but don't fail if not found
            let _ = engine.load_verum_std();
        }

        Ok(engine)
    }
}

impl Default for JitCompiler {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jit_config_default() {
        let config = JitConfig::new();
        assert_eq!(config.optimization_level, 2);
        assert!(config.enable_object_cache);
        assert!(!config.verbose);
    }

    #[test]
    fn test_jit_config_builder() {
        let config = JitConfig::new()
            .optimization_level(3)
            .verbose(true)
            .add_shared_library("test.so");

        assert_eq!(config.optimization_level, 3);
        assert!(config.verbose);
        assert_eq!(config.shared_library_paths.len(), 1);
    }

    #[test]
    fn test_jit_config_legacy_builder() {
        let config = JitConfig::new()
            .with_optimization_level(3)
            .with_object_cache(false)
            .with_verbose(true);

        assert_eq!(config.optimization_level, 3);
        assert!(!config.enable_object_cache);
        assert!(config.verbose);
    }

    #[test]
    fn test_jit_config_presets() {
        let dev = JitConfig::development();
        assert_eq!(dev.optimization_level, 0);
        assert!(dev.verbose);

        let prod = JitConfig::production();
        assert_eq!(prod.optimization_level, 3);
        assert!(!prod.verbose);
    }

    #[test]
    fn test_jit_stats() {
        let stats = JitStats::new();
        stats.compilations.fetch_add(5, Ordering::Relaxed);
        stats.total_compilation_time_us.fetch_add(1000, Ordering::Relaxed);

        assert_eq!(stats.avg_compilation_time_us(), 200.0);
    }

    #[test]
    fn test_jit_args_tuple() {
        let args = (1i64, 2i64);
        let packed = args.pack();
        assert_eq!(packed.len(), 2);
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn test_jit_args_empty() {
        let args = ();
        let packed = args.pack();
        assert_eq!(packed.len(), 0);
        assert!(args.is_empty());
    }

    #[test]
    fn test_callback_registry() {
        let stats = Arc::new(JitStats::new());
        let registry = CallbackRegistry::new(stats.clone());

        registry.register("test", |_args| std::ptr::null_mut());

        assert!(registry.contains("test"));
        assert!(!registry.contains("nonexistent"));

        let names = registry.names();
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn test_jit_compiler_builder() {
        let compiler = JitCompiler::new()
            .optimization_level(2)
            .verbose(true)
            .preload_stdlib(false);

        assert_eq!(compiler.config.optimization_level, 2);
        assert!(compiler.config.verbose);
        assert!(!compiler.preload_stdlib);
    }
}
