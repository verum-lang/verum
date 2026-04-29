//! Link-Time Optimization (LTO) support
//!
//! Provides ThinLTO and Full LTO with incremental caching.

// On MSVC, the linker processes static libraries in single-pass order.
// Force it to resolve LTO symbols from LLVMLTO.lib.
use verum_llvm_sys::lto::*;
use std::ffi::{CStr, c_char};
use std::path::{Path, PathBuf};

use crate::error::{LlvmError, LlvmResult};
use crate::support::to_c_str;

/// Mirror of LTOObjectBuffer with accessible fields
#[repr(C)]
struct ObjectBuffer {
    buffer: *const c_char,
    size: usize,
}

/// LTO mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
pub enum LtoMode {
    /// No LTO
    None,
    /// ThinLTO (fast, parallel, cacheable)
    #[default]
    Thin,
    /// Full LTO (slower, better optimization)
    Full,
}


/// ThinLTO cache configuration
#[derive(Debug, Clone)]
pub struct ThinLtoCache {
    /// Cache directory
    pub dir: PathBuf,
    /// Pruning interval in seconds (default: 1 day)
    pub pruning_interval: u32,
    /// Cache entry expiration in seconds (default: 1 week)
    pub expiration: u32,
    /// Maximum cache size in bytes (0 = unlimited)
    pub max_size_bytes: u64,
    /// Maximum cache size as percentage of available space (0-100)
    pub max_size_percentage: u32,
}

impl ThinLtoCache {
    /// Create new cache with default settings
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().to_path_buf(),
            pruning_interval: 86400,  // 1 day
            expiration: 604800,       // 1 week
            max_size_bytes: 0,        // unlimited
            max_size_percentage: 75,  // 75% of available space
        }
    }

    /// Set pruning interval
    pub fn with_pruning_interval(mut self, seconds: u32) -> Self {
        self.pruning_interval = seconds;
        self
    }

    /// Set expiration
    pub fn with_expiration(mut self, seconds: u32) -> Self {
        self.expiration = seconds;
        self
    }

    /// Set maximum size in bytes
    pub fn with_max_size_bytes(mut self, bytes: u64) -> Self {
        self.max_size_bytes = bytes;
        self
    }

    /// Set maximum size as percentage of available space
    pub fn with_max_size_percentage(mut self, percentage: u32) -> Self {
        self.max_size_percentage = percentage.min(100);
        self
    }
}

impl Default for ThinLtoCache {
    fn default() -> Self {
        Self::new(std::env::temp_dir().join("verum-lto-cache"))
    }
}

/// LTO configuration
#[derive(Debug, Clone)]
pub struct LtoConfig {
    /// LTO mode
    pub mode: LtoMode,
    /// CPU for code generation
    pub cpu: String,
    /// Target features
    pub features: String,
    /// Optimization level (0-3)
    pub opt_level: u32,
    /// Generate position-independent code
    pub pic: bool,
    /// Generate debug info
    pub debug_info: bool,
    /// ThinLTO cache (only used in Thin mode)
    pub cache: Option<ThinLtoCache>,
    /// Internalize symbols
    pub internalize: bool,
    /// Run whole-program optimizations
    pub whole_program: bool,
}

impl LtoConfig {
    /// Create default configuration
    pub fn new(mode: LtoMode) -> Self {
        Self {
            mode,
            cpu: "generic".to_string(),
            features: String::new(),
            opt_level: 2,
            pic: true,
            debug_info: false,
            cache: None,
            internalize: true,
            whole_program: true,
        }
    }

    /// Create ThinLTO configuration with cache
    pub fn thin_with_cache(cache_dir: impl AsRef<Path>) -> Self {
        Self {
            mode: LtoMode::Thin,
            cache: Some(ThinLtoCache::new(cache_dir)),
            ..Self::default()
        }
    }

    /// Set CPU
    pub fn with_cpu(mut self, cpu: &str) -> Self {
        self.cpu = cpu.to_string();
        self
    }

    /// Set features
    pub fn with_features(mut self, features: &str) -> Self {
        self.features = features.to_string();
        self
    }

    /// Set optimization level
    pub fn with_opt_level(mut self, level: u32) -> Self {
        self.opt_level = level.min(3);
        self
    }

    /// Set PIC
    pub fn with_pic(mut self, enable: bool) -> Self {
        self.pic = enable;
        self
    }

    /// Set debug info
    pub fn with_debug_info(mut self, enable: bool) -> Self {
        self.debug_info = enable;
        self
    }

    /// Set cache
    pub fn with_cache(mut self, cache: ThinLtoCache) -> Self {
        self.cache = Some(cache);
        self
    }
}

impl Default for LtoConfig {
    fn default() -> Self {
        Self::new(LtoMode::Thin)
    }
}

/// ThinLTO code generator
pub struct ThinLtoCodegen {
    codegen: thinlto_code_gen_t,
}

impl std::fmt::Debug for ThinLtoCodegen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThinLtoCodegen").finish_non_exhaustive()
    }
}

impl ThinLtoCodegen {
    /// Create new ThinLTO code generator
    pub fn new() -> Self {
        let codegen = unsafe { thinlto_create_codegen() };
        Self { codegen }
    }

    /// Add a module (bitcode)
    pub fn add_module(&self, name: &str, data: &[u8]) {
        let name_c = to_c_str(name);
        unsafe {
            thinlto_codegen_add_module(
                self.codegen,
                name_c.as_ptr(),
                data.as_ptr() as *const c_char,
                data.len() as std::ffi::c_int,
            );
        }
    }

    /// Set CPU
    pub fn set_cpu(&self, cpu: &str) {
        let cpu_c = to_c_str(cpu);
        unsafe {
            thinlto_codegen_set_cpu(self.codegen, cpu_c.as_ptr());
        }
    }

    /// Set cache directory
    pub fn set_cache_dir(&self, dir: &Path) {
        if let Some(dir_str) = dir.to_str() {
            let dir_c = to_c_str(dir_str);
            unsafe {
                thinlto_codegen_set_cache_dir(self.codegen, dir_c.as_ptr());
            }
        }
    }

    /// Set cache pruning interval
    pub fn set_cache_pruning_interval(&self, seconds: u32) {
        unsafe {
            thinlto_codegen_set_cache_pruning_interval(self.codegen, seconds as std::ffi::c_int);
        }
    }

    /// Set cache entry expiration
    pub fn set_cache_entry_expiration(&self, seconds: u32) {
        unsafe {
            thinlto_codegen_set_cache_entry_expiration(self.codegen, seconds);
        }
    }

    /// Set maximum cache size relative to available space
    pub fn set_cache_size_percentage(&self, percentage: u32) {
        unsafe {
            thinlto_codegen_set_final_cache_size_relative_to_available_space(
                self.codegen,
                percentage,
            );
        }
    }

    /// Apply configuration
    pub fn apply_config(&self, config: &LtoConfig) {
        self.set_cpu(&config.cpu);

        if let Some(ref cache) = config.cache {
            self.set_cache_dir(&cache.dir);
            self.set_cache_pruning_interval(cache.pruning_interval);
            self.set_cache_entry_expiration(cache.expiration);
            self.set_cache_size_percentage(cache.max_size_percentage);
        }

        // Note: PIC model setting for ThinLTO is not directly supported
        // The PIC model is determined by the input bitcode modules
        let _ = config.pic; // Acknowledge the config field
    }

    /// Process all modules
    pub fn process(&self) {
        unsafe {
            thinlto_codegen_process(self.codegen);
        }
    }

    /// Get number of output objects
    pub fn num_objects(&self) -> u32 {
        let count = unsafe { thinlto_module_get_num_objects(self.codegen) };
        count as u32
    }

    /// Get object at index
    pub fn get_object(&self, index: u32) -> Option<Vec<u8>> {
        if index >= self.num_objects() {
            return None;
        }

        unsafe {
            let obj = thinlto_module_get_object(self.codegen, index);
            // Transmute to our mirror struct to access the fields
            let obj_buf: ObjectBuffer = std::mem::transmute(obj);

            if obj_buf.buffer.is_null() || obj_buf.size == 0 {
                return None;
            }
            Some(std::slice::from_raw_parts(
                obj_buf.buffer as *const u8,
                obj_buf.size,
            ).to_vec())
        }
    }

    /// Get all objects
    pub fn get_all_objects(&self) -> Vec<Vec<u8>> {
        let mut objects = Vec::new();
        for i in 0..self.num_objects() {
            if let Some(obj) = self.get_object(i) {
                objects.push(obj);
            }
        }
        objects
    }
}

impl Default for ThinLtoCodegen {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ThinLtoCodegen {
    fn drop(&mut self) {
        if !self.codegen.is_null() {
            unsafe {
                thinlto_codegen_dispose(self.codegen);
            }
        }
    }
}

// Safety: ThinLtoCodegen owns its codegen handle
unsafe impl Send for ThinLtoCodegen {}

/// Full LTO code generator
pub struct FullLtoCodegen {
    codegen: lto_code_gen_t,
}

impl std::fmt::Debug for FullLtoCodegen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FullLtoCodegen").finish_non_exhaustive()
    }
}

impl FullLtoCodegen {
    /// Create new full LTO code generator
    pub fn new() -> Self {
        let codegen = unsafe { lto_codegen_create() };
        Self { codegen }
    }

    /// Add a module (bitcode)
    pub fn add_module(&self, data: &[u8]) -> LlvmResult<()> {
        // Create LTO module from memory
        let module = unsafe {
            lto_module_create_from_memory(
                data.as_ptr() as *const std::ffi::c_void,
                data.len(),
            )
        };

        if module.is_null() {
            return Err(LlvmError::LtoError("Failed to create LTO module".to_string()));
        }

        let result = unsafe { lto_codegen_add_module(self.codegen, module) };

        if result != 0 {
            return Err(LlvmError::LtoError("Failed to add module to LTO".to_string()));
        }

        Ok(())
    }

    /// Set CPU
    pub fn set_cpu(&self, cpu: &str) {
        let cpu_c = to_c_str(cpu);
        unsafe {
            lto_codegen_set_cpu(self.codegen, cpu_c.as_ptr());
        }
    }

    /// Set debug model
    pub fn set_debug_model(&self, enable: bool) {
        let model = if enable {
            lto_debug_model::LTO_DEBUG_MODEL_DWARF
        } else {
            lto_debug_model::LTO_DEBUG_MODEL_NONE
        };
        unsafe {
            lto_codegen_set_debug_model(self.codegen, model);
        }
    }

    /// Set PIC model
    pub fn set_pic_model(&self, pic: bool) {
        let model = if pic {
            lto_codegen_model::LTO_CODEGEN_PIC_MODEL_DYNAMIC
        } else {
            lto_codegen_model::LTO_CODEGEN_PIC_MODEL_STATIC
        };
        unsafe {
            lto_codegen_set_pic_model(self.codegen, model);
        }
    }

    /// Set whether the linker should internalize non-preserved
    /// symbols.
    ///
    /// Internalization is the LTO optimization that converts
    /// external linkage to internal linkage for symbols that
    /// aren't part of the public API. This unlocks aggressive
    /// inlining and dead-code elimination on functions that
    /// would otherwise be considered linker-visible.
    ///
    /// Wraps `lto_codegen_set_should_internalize`.
    pub fn set_internalize(&self, internalize: bool) {
        unsafe {
            lto_codegen_set_should_internalize(
                self.codegen,
                if internalize { 1 } else { 0 },
            );
        }
    }

    /// Apply configuration
    pub fn apply_config(&self, config: &LtoConfig) {
        self.set_cpu(&config.cpu);
        self.set_debug_model(config.debug_info);
        self.set_pic_model(config.pic);
        // Wire `internalize` — the C API exists for FullLTO and
        // is the canonical knob for "make all non-preserved
        // symbols internal so the inliner can eat them". Pre-fix
        // the field landed on LtoConfig but no code path consulted
        // it, so the LLVM default (typically true for whole-program
        // LTO) was always used regardless of the manifest.
        self.set_internalize(config.internalize);
    }

    /// Add must-preserve symbol
    pub fn preserve_symbol(&self, symbol: &str) {
        let symbol_c = to_c_str(symbol);
        unsafe {
            lto_codegen_add_must_preserve_symbol(self.codegen, symbol_c.as_ptr());
        }
    }

    /// Optimize modules
    pub fn optimize(&self) -> LlvmResult<()> {
        let result = unsafe { lto_codegen_optimize(self.codegen) };
        if result != 0 {
            let error = unsafe {
                let ptr = lto_get_error_message();
                if ptr.is_null() {
                    "Unknown LTO error".to_string()
                } else {
                    CStr::from_ptr(ptr).to_string_lossy().into_owned()
                }
            };
            return Err(LlvmError::LtoError(error));
        }
        Ok(())
    }

    /// Compile to object
    pub fn compile(&self) -> LlvmResult<Vec<u8>> {
        let mut size: usize = 0;
        let buffer = unsafe { lto_codegen_compile(self.codegen, &mut size) };

        if buffer.is_null() || size == 0 {
            let error = unsafe {
                let ptr = lto_get_error_message();
                if ptr.is_null() {
                    "LTO compilation failed".to_string()
                } else {
                    CStr::from_ptr(ptr).to_string_lossy().into_owned()
                }
            };
            return Err(LlvmError::LtoError(error));
        }

        let data = unsafe { std::slice::from_raw_parts(buffer as *const u8, size) };
        Ok(data.to_vec())
    }

    /// Write merged modules (for debugging)
    pub fn write_merged_modules(&self, path: &str) -> LlvmResult<()> {
        let path_c = to_c_str(path);
        let result = unsafe { lto_codegen_write_merged_modules(self.codegen, path_c.as_ptr()) };
        if result != 0 {
            return Err(LlvmError::LtoError("Failed to write merged modules".to_string()));
        }
        Ok(())
    }
}

impl Default for FullLtoCodegen {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for FullLtoCodegen {
    fn drop(&mut self) {
        if !self.codegen.is_null() {
            unsafe {
                lto_codegen_dispose(self.codegen);
            }
        }
    }
}

// Safety: FullLtoCodegen owns its codegen handle
unsafe impl Send for FullLtoCodegen {}

/// High-level LTO compilation function
///
/// Compiles multiple bitcode modules using LTO.
pub fn lto_compile(modules: &[&[u8]], config: &LtoConfig) -> LlvmResult<Vec<Vec<u8>>> {
    match config.mode {
        LtoMode::None => {
            // Just return the modules as-is (caller should compile individually)
            Ok(modules.iter().map(|m| m.to_vec()).collect())
        }
        LtoMode::Thin => {
            let codegen = ThinLtoCodegen::new();
            codegen.apply_config(config);

            for (i, module) in modules.iter().enumerate() {
                codegen.add_module(&format!("module{}", i), module);
            }

            codegen.process();
            Ok(codegen.get_all_objects())
        }
        LtoMode::Full => {
            let codegen = FullLtoCodegen::new();
            codegen.apply_config(config);

            for module in modules {
                codegen.add_module(module)?;
            }

            codegen.optimize()?;
            let result = codegen.compile()?;
            Ok(vec![result])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lto_config() {
        let config = LtoConfig::thin_with_cache("/tmp/test-cache")
            .with_cpu("znver4")
            .with_opt_level(3);

        assert_eq!(config.mode, LtoMode::Thin);
        assert_eq!(config.cpu, "znver4");
        assert_eq!(config.opt_level, 3);
        assert!(config.cache.is_some());
    }

    #[test]
    fn test_thin_lto_cache_config() {
        let cache = ThinLtoCache::new("/tmp/cache")
            .with_pruning_interval(3600)
            .with_max_size_percentage(50);

        assert_eq!(cache.pruning_interval, 3600);
        assert_eq!(cache.max_size_percentage, 50);
    }

    #[test]
    fn lto_config_internalize_default_is_true() {
        // Pin: the documented default keeps internalization on
        // — full whole-program LTO with internal-linkage
        // promotion is the canonical aggressive-optimization
        // setup. Embedders that need to preserve external
        // linkage opt out via `internalize: false`.
        let cfg = LtoConfig::new(LtoMode::Full);
        assert!(
            cfg.internalize,
            "default internalize must stay true for full whole-program LTO",
        );
    }

    #[test]
    fn lto_config_internalize_round_trips() {
        // Pin: the field is mutable on the public surface so
        // embedders can flip it without going through builders.
        // Round-trip the value to confirm assignment works.
        let mut cfg = LtoConfig::new(LtoMode::Full);
        assert!(cfg.internalize);
        cfg.internalize = false;
        assert!(!cfg.internalize);
    }
}
