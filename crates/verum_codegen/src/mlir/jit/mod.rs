//! Industrial-grade JIT compilation using MLIR ExecutionEngine.
//!
//! Provides comprehensive just-in-time compilation for Verum code using MLIR's
//! ExecutionEngine, which is built on LLVM's ORC JIT.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │                           Verum JIT System                                   │
//! └─────────────────────────────────────────────────────────────────────────────┘
//!
//! ┌────────────────┐    ┌─────────────────┐    ┌──────────────────┐
//! │   JitCompiler  │───▶│   JitEngine     │───▶│  Execution       │
//! │   (Builder)    │    │  (Core engine)  │    │  (Call/Invoke)   │
//! └────────────────┘    └────────┬────────┘    └──────────────────┘
//!                                │
//!                    ┌───────────┼───────────┬───────────┐
//!                    │           │           │           │
//!              ┌─────▼─────┐ ┌───▼───┐ ┌─────▼─────┐ ┌───▼───┐
//!              │ SymbolRes │ │Cache  │ │  REPL     │ │ Hot   │
//!              │ (FFI)     │ │(Incr) │ │  Session  │ │Reload │
//!              └───────────┘ └───────┘ └───────────┘ └───────┘
//! ```
//!
//! # Phase 4 Features (Full Implementation)
//!
//! - **P4.1 - ExecutionEngine Integration**: Complete JIT engine with MLIR
//! - **P4.2 - Symbol Resolution**: Dynamic library loading, FFI symbol resolution
//! - **P4.3 - Incremental Compilation**: Content-based caching, dependency tracking
//! - **P4.4 - REPL Integration**: Interactive session, expression evaluation
//! - **P4.5 - Hot Code Replacement**: Runtime function replacement, rollback
//!
//! # Example
//!
//! ```rust,ignore
//! use crate::mlir::jit::{JitEngine, JitConfig, JitCompiler};
//!
//! // Using builder pattern
//! let engine = JitCompiler::new()
//!     .optimization_level(2)
//!     .verbose(true)
//!     .compile(&mlir_module)?;
//!
//! // Type-safe function call
//! let result: i64 = engine.call("add", (1i64, 2i64))?;
//!
//! // Or using configuration directly
//! let config = JitConfig::production();
//! let engine = JitEngine::compile(&mlir_module, config)?;
//!
//! // Load stdlib symbols
//! engine.load_verum_std()?;
//!
//! // Call functions
//! let result = engine.call_i64("factorial", &[10])?;
//! ```
//!
//! # REPL Example
//!
//! ```rust,ignore
//! use crate::mlir::jit::{ReplSession, ReplConfig};
//!
//! let session = ReplSession::new(ReplConfig::default())?;
//!
//! session.eval("let x = 42")?;
//! let result = session.eval("x + 1")?;
//! println!("Result: {}", result.value); // 43
//! ```
//!
//! # Hot Reload Example
//!
//! ```rust,ignore
//! use crate::mlir::jit::{HotReloader, HotReloadConfig};
//!
//! let reloader = HotReloader::new(HotReloadConfig::default());
//!
//! // Register function for hot reloading
//! unsafe { reloader.register("process", func_addr, sig_hash)?; }
//!
//! // Later, replace with new version
//! unsafe { reloader.replace("process", new_addr, sig_hash, src_hash)?; }
//!
//! // Rollback if needed
//! reloader.rollback("process")?;
//! ```

mod engine;
mod symbol_resolver;
mod incremental;
mod repl;
mod hot_reload;

// Re-export engine types
pub use engine::{
    // Core types
    JitEngine,
    JitConfig,
    JitStats,
    JitStatsSummary,
    JitCompiler,

    // Type-safe calling traits
    JitArg,
    JitArgs,
    JitReturn,

    // Callback system
    CallbackRegistry,
    JitCallback,

    // Compiled function handle
    CompiledFunction,
};

// Re-export symbol resolver types
pub use symbol_resolver::{
    // Core resolver
    SymbolResolver,
    SymbolResolverStats,

    // Symbol information
    SymbolInfo,
    SymbolMetadata,
    SymbolCategory,
    FfiType,
};

// Re-export incremental compilation types
pub use incremental::{
    // Core cache
    IncrementalCache,
    CacheConfig,
    CacheEntry,
    CacheOptions,

    // Statistics
    CacheStats,
    CacheStatsSummary,

    // Dependency tracking
    DependencyTracker,

    // Hashing
    ContentHash,
    ContentHasher,
};

// Re-export REPL types
pub use repl::{
    // Session management
    ReplSession,
    ReplConfig,
    SessionId,

    // Evaluation
    EvalResult,
    Binding,
    HistoryEntry,

    // Commands
    ReplCommand,

    // Statistics
    SessionStats,
    SessionStatsSummary,
};

// Re-export hot reload types
pub use hot_reload::{
    // Core reloader
    HotReloader,
    HotReloadConfig,

    // Function management
    HotFunction,
    FunctionVersion,

    // Statistics
    HotReloadStats,
    HotReloadStatsSummary,

    // Signature helpers
    SignatureHasher,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jit_module_exports() {
        // Verify all expected types are exported
        let _ = JitConfig::new();
        let _ = JitStats::new();
        let _ = SymbolResolver::new();
    }

    #[test]
    fn test_jit_config_presets() {
        let dev = JitConfig::development();
        assert_eq!(dev.optimization_level, 0);

        let prod = JitConfig::production();
        assert_eq!(prod.optimization_level, 3);
    }

    #[test]
    fn test_cache_config() {
        let config = CacheConfig::default();
        assert!(config.persistent);

        let mem_only = CacheConfig::memory_only();
        assert!(!mem_only.persistent);
    }

    #[test]
    fn test_repl_config() {
        let config = ReplConfig::default();
        assert!(config.incremental);
        assert!(config.auto_complete);
    }

    #[test]
    fn test_hot_reload_config() {
        let config = HotReloadConfig::default();
        assert!(config.validate_signatures);

        let dev = HotReloadConfig::development();
        assert!(dev.verbose);
    }

    #[test]
    fn test_symbol_resolver_basic() {
        let resolver = SymbolResolver::new();

        // Register a dummy symbol
        unsafe {
            resolver.register("test_symbol", 0x1000 as *mut ());
        }

        assert!(resolver.contains("test_symbol"));
    }

    #[test]
    fn test_content_hasher() {
        let mut hasher1 = ContentHasher::new();
        hasher1.update(b"hello");
        let hash1 = hasher1.finalize();

        let mut hasher2 = ContentHasher::new();
        hasher2.update(b"hello");
        let hash2 = hasher2.finalize();

        let mut hasher3 = ContentHasher::new();
        hasher3.update(b"world");
        let hash3 = hasher3.finalize();

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }
}
