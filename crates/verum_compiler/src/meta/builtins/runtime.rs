//! Runtime/Target Information Intrinsics (Tier 1 - Requires MetaRuntime)
//!
//! Provides compile-time information about the target platform and build.
//! All functions in this module require the `MetaRuntime` context since they
//! access build configuration and environment information.
//!
//! ## Target Information
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `target_os()` | `() -> Text` | Get target OS (linux, macos, windows) |
//! | `target_arch()` | `() -> Text` | Get target architecture (x86_64, aarch64) |
//! | `target_triple()` | `() -> Text` | Get full target triple |
//! | `target_pointer_width()` | `() -> Int` | Get pointer width in bits |
//! | `target_endian()` | `() -> Text` | Get endianness ("little" or "big") |
//! | `target_has_feature(feat)` | `(Text) -> Bool` | Check target feature |
//!
//! ## Build Information
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `crate_name()` / `cog_name()` | `() -> Text` | Get current crate name |
//! | `module_path()` | `() -> Text` | Get current module path |
//! | `crate_version()` / `cog_version()` | `() -> Text` | Get crate version |
//! | `is_debug()` | `() -> Bool` | Check if debug build |
//! | `is_release()` | `() -> Bool` | Check if release build |
//! | `opt_level()` | `() -> Int` | Get optimization level (0-3) |
//! | `compiler_version()` | `() -> Text` | Get compiler version |
//!
//! ## Feature Flags
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `has_feature(name)` | `(Text) -> Bool` | Check if feature enabled |
//! | `enabled_features()` | `() -> List<Text>` | List all enabled features |
//!
//! ## Environment
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `env(name)` | `(Text) -> Maybe<Text>` | Get environment variable |
//! | `is_ci()` | `() -> Bool` | Check if running in CI |
//!
//! ## Context Requirements
//!
//! **Tier 1**: All functions require `using [MetaRuntime]` context.
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

use verum_common::{List, Maybe, Text};

use super::context_requirements::{BuiltinInfo, BuiltinRegistry};
use super::{ConstValue, MetaContext, MetaError};

/// Register runtime builtins with context requirements
///
/// All runtime functions require MetaRuntime context since they access
/// build configuration and environment information.
pub fn register_builtins(map: &mut BuiltinRegistry) {
    // ========================================================================
    // Target Information (Tier 1 - MetaRuntime)
    // ========================================================================

    map.insert(
        Text::from("target_os"),
        BuiltinInfo::meta_runtime(
            meta_target_os,
            "Get target operating system",
            "() -> Text",
        ),
    );
    map.insert(
        Text::from("target_arch"),
        BuiltinInfo::meta_runtime(
            meta_target_arch,
            "Get target architecture",
            "() -> Text",
        ),
    );
    map.insert(
        Text::from("target_triple"),
        BuiltinInfo::meta_runtime(
            meta_target_triple,
            "Get full target triple",
            "() -> Text",
        ),
    );
    map.insert(
        Text::from("target_pointer_width"),
        BuiltinInfo::meta_runtime(
            meta_target_pointer_width,
            "Get pointer width in bits",
            "() -> Int",
        ),
    );
    map.insert(
        Text::from("target_endian"),
        BuiltinInfo::meta_runtime(
            meta_target_endian,
            "Get target endianness",
            "() -> Text",
        ),
    );
    map.insert(
        Text::from("target_has_feature"),
        BuiltinInfo::meta_runtime(
            meta_target_has_feature,
            "Check if target has specific feature",
            "(Text) -> Bool",
        ),
    );

    // ========================================================================
    // Build Information (Tier 1 - MetaRuntime)
    // ========================================================================

    map.insert(
        Text::from("crate_name"),
        BuiltinInfo::meta_runtime(
            meta_crate_name,
            "Get current crate/cog name",
            "() -> Text",
        ),
    );
    map.insert(
        Text::from("cog_name"),
        BuiltinInfo::meta_runtime(
            meta_crate_name,
            "Get current cog name (alias for crate_name)",
            "() -> Text",
        ),
    );
    map.insert(
        Text::from("module_path"),
        BuiltinInfo::meta_runtime(
            meta_module_path,
            "Get current module path",
            "() -> Text",
        ),
    );
    map.insert(
        Text::from("crate_version"),
        BuiltinInfo::meta_runtime(
            meta_crate_version,
            "Get current crate version",
            "() -> Text",
        ),
    );
    map.insert(
        Text::from("cog_version"),
        BuiltinInfo::meta_runtime(
            meta_crate_version,
            "Get current cog version (alias for crate_version)",
            "() -> Text",
        ),
    );
    map.insert(
        Text::from("is_debug"),
        BuiltinInfo::meta_runtime(
            meta_is_debug,
            "Check if this is a debug build",
            "() -> Bool",
        ),
    );
    map.insert(
        Text::from("is_release"),
        BuiltinInfo::meta_runtime(
            meta_is_release,
            "Check if this is a release build",
            "() -> Bool",
        ),
    );
    map.insert(
        Text::from("opt_level"),
        BuiltinInfo::meta_runtime(
            meta_opt_level,
            "Get optimization level (0-3)",
            "() -> Int",
        ),
    );
    map.insert(
        Text::from("compiler_version"),
        BuiltinInfo::meta_runtime(
            meta_compiler_version,
            "Get Verum compiler version",
            "() -> Text",
        ),
    );

    // ========================================================================
    // Feature Flags (Tier 1 - MetaRuntime)
    // ========================================================================

    map.insert(
        Text::from("has_feature"),
        BuiltinInfo::meta_runtime(
            meta_has_feature,
            "Check if a feature flag is enabled",
            "(Text) -> Bool",
        ),
    );
    map.insert(
        Text::from("enabled_features"),
        BuiltinInfo::meta_runtime(
            meta_enabled_features,
            "Get list of all enabled features",
            "() -> List<Text>",
        ),
    );

    // ========================================================================
    // Environment (Tier 1 - MetaRuntime)
    // ========================================================================

    map.insert(
        Text::from("env"),
        BuiltinInfo::meta_runtime(
            meta_env,
            "Get environment variable value",
            "(Text) -> Maybe<Text>",
        ),
    );
    map.insert(
        Text::from("is_ci"),
        BuiltinInfo::meta_runtime(
            meta_is_ci,
            "Check if running in CI environment",
            "() -> Bool",
        ),
    );

    // ========================================================================
    // Runtime Configuration (Tier 1 - MetaRuntime)
    // ========================================================================

    map.insert(
        Text::from("runtime_config"),
        BuiltinInfo::meta_runtime(
            meta_runtime_config,
            "Get runtime configuration name",
            "() -> Text",
        ),
    );
    map.insert(
        Text::from("recursion_limit"),
        BuiltinInfo::meta_runtime(
            meta_recursion_limit,
            "Get meta evaluation recursion limit",
            "() -> Int",
        ),
    );
    map.insert(
        Text::from("iteration_limit"),
        BuiltinInfo::meta_runtime(
            meta_iteration_limit,
            "Get meta evaluation iteration limit",
            "() -> Int",
        ),
    );
    map.insert(
        Text::from("memory_limit"),
        BuiltinInfo::meta_runtime(
            meta_memory_limit,
            "Get meta evaluation memory limit",
            "() -> Int",
        ),
    );
    map.insert(
        Text::from("timeout_ms"),
        BuiltinInfo::meta_runtime(
            meta_timeout_ms,
            "Get meta evaluation timeout in milliseconds",
            "() -> Int",
        ),
    );
    map.insert(
        Text::from("config_get"),
        BuiltinInfo::meta_runtime(
            meta_config_get,
            "Get configuration value by key",
            "(Text) -> Maybe<Text>",
        ),
    );
    map.insert(
        Text::from("config_get_int"),
        BuiltinInfo::meta_runtime(
            meta_config_get_int,
            "Get configuration value as integer",
            "(Text) -> Maybe<Int>",
        ),
    );
    map.insert(
        Text::from("config_get_bool"),
        BuiltinInfo::meta_runtime(
            meta_config_get_bool,
            "Get configuration value as boolean",
            "(Text) -> Maybe<Bool>",
        ),
    );
    map.insert(
        Text::from("config_get_array"),
        BuiltinInfo::meta_runtime(
            meta_config_get_array,
            "Get configuration array by key",
            "(Text) -> Maybe<List<Text>>",
        ),
    );
}

// ============================================================================
// Target Information (compile-time constants)
// ============================================================================

/// Get target operating system
fn meta_target_os(_ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    // Use compile-time cfg to determine OS
    #[cfg(target_os = "linux")]
    let os = "linux";
    #[cfg(target_os = "macos")]
    let os = "macos";
    #[cfg(target_os = "windows")]
    let os = "windows";
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    let os = "unknown";
    Ok(ConstValue::Text(Text::from(os)))
}

/// Get target architecture
fn meta_target_arch(_ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    #[cfg(target_arch = "x86_64")]
    let arch = "x86_64";
    #[cfg(target_arch = "aarch64")]
    let arch = "aarch64";
    #[cfg(target_arch = "x86")]
    let arch = "x86";
    #[cfg(target_arch = "arm")]
    let arch = "arm";
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "x86", target_arch = "arm")))]
    let arch = "unknown";
    Ok(ConstValue::Text(Text::from(arch)))
}

/// Get full target triple
fn meta_target_triple(_ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    // Construct a triple from available info
    let triple = format!(
        "{}-{}-{}",
        std::env::consts::ARCH,
        "unknown", // vendor
        std::env::consts::OS
    );
    Ok(ConstValue::Text(Text::from(triple)))
}

/// Get pointer width in bits
fn meta_target_pointer_width(_ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Int((std::mem::size_of::<usize>() * 8) as i128))
}

/// Get target endianness
fn meta_target_endian(_ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    #[cfg(target_endian = "little")]
    let endian = "little";
    #[cfg(target_endian = "big")]
    let endian = "big";
    Ok(ConstValue::Text(Text::from(endian)))
}

/// Check if target has a specific feature
fn meta_target_has_feature(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(feature) => {
            // Mirrors meta_target_os / meta_target_arch: read the
            // compile-time `target_feature` cfg of the compiler host.
            // `cfg!(target_feature = ...)` requires a literal so the
            // user's runtime string is matched against a known set.
            // Unknown/unsupported features fall through to `false` —
            // safer than over-claiming a feature the host doesn't
            // actually have. Cross-compilation queries should consult
            // ProjectInfoData.target_triple via a separate
            // build-target-aware path (not yet plumbed).
            Ok(ConstValue::Bool(detect_target_feature(feature.as_str())))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Resolve a target feature name against the compiler host's
/// compile-time `target_feature` cfgs. The returned bool is
/// honest for the listed features and conservatively `false`
/// for everything else (unknown name → unknown answer → false).
///
/// Coverage mirrors what stdlib detection macros expose for the
/// two architectures we ship for:
///   - x86_64 / x86: SSE family, AVX family, FMA, BMI, LZCNT,
///     POPCNT, AES, PCLMULQDQ, RDRAND/RDSEED
///   - aarch64 / arm: NEON, FP, ASIMD, CRC, AES, SHA2/SHA3,
///     LSE, RCPC, DOTPROD, VFP2/3/4
fn detect_target_feature(feature: &str) -> bool {
    match feature {
        // x86 / x86_64 — SSE family
        "sse" => cfg!(target_feature = "sse"),
        "sse2" => cfg!(target_feature = "sse2"),
        "sse3" => cfg!(target_feature = "sse3"),
        "ssse3" => cfg!(target_feature = "ssse3"),
        "sse4.1" => cfg!(target_feature = "sse4.1"),
        "sse4.2" => cfg!(target_feature = "sse4.2"),
        // x86 / x86_64 — AVX family
        "avx" => cfg!(target_feature = "avx"),
        "avx2" => cfg!(target_feature = "avx2"),
        "avx512f" => cfg!(target_feature = "avx512f"),
        "avx512bw" => cfg!(target_feature = "avx512bw"),
        "avx512cd" => cfg!(target_feature = "avx512cd"),
        "avx512dq" => cfg!(target_feature = "avx512dq"),
        "avx512vl" => cfg!(target_feature = "avx512vl"),
        // x86 / x86_64 — bit ops + crypto
        "fma" => cfg!(target_feature = "fma"),
        "bmi1" => cfg!(target_feature = "bmi1"),
        "bmi2" => cfg!(target_feature = "bmi2"),
        "lzcnt" => cfg!(target_feature = "lzcnt"),
        "popcnt" => cfg!(target_feature = "popcnt"),
        "aes" => cfg!(target_feature = "aes"),
        "pclmulqdq" => cfg!(target_feature = "pclmulqdq"),
        "rdrand" => cfg!(target_feature = "rdrand"),
        "rdseed" => cfg!(target_feature = "rdseed"),
        // aarch64
        "neon" => cfg!(target_feature = "neon"),
        "fp" => cfg!(target_feature = "fp"),
        "asimd" => cfg!(target_feature = "asimd"),
        "crc" => cfg!(target_feature = "crc"),
        "sha2" => cfg!(target_feature = "sha2"),
        "sha3" => cfg!(target_feature = "sha3"),
        "lse" => cfg!(target_feature = "lse"),
        "rcpc" => cfg!(target_feature = "rcpc"),
        "dotprod" => cfg!(target_feature = "dotprod"),
        // arm 32-bit
        "vfp2" => cfg!(target_feature = "vfp2"),
        "vfp3" => cfg!(target_feature = "vfp3"),
        "vfp4" => cfg!(target_feature = "vfp4"),
        _ => false,
    }
}

// ============================================================================
// Build Information
// ============================================================================

/// Get current crate/cog name
fn meta_crate_name(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    match &ctx.runtime_info.crate_name {
        Some(name) => Ok(ConstValue::Text(name.clone())),
        None => Ok(ConstValue::Text(Text::from("unknown"))),
    }
}

/// Get current module path
fn meta_module_path(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    match &ctx.runtime_info.module_path {
        Some(path) => Ok(ConstValue::Text(path.clone())),
        None => Ok(ConstValue::Text(Text::from(""))),
    }
}

/// Get crate/cog version
fn meta_crate_version(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    match ctx.runtime_info.crate_version {
        Some((major, minor, patch)) => {
            Ok(ConstValue::Text(Text::from(format!("{}.{}.{}", major, minor, patch))))
        }
        None => Ok(ConstValue::Text(Text::from("0.0.0"))),
    }
}

/// Check if debug build
fn meta_is_debug(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Bool(ctx.runtime_info.is_debug))
}

/// Check if release build
fn meta_is_release(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Bool(!ctx.runtime_info.is_debug))
}

/// Get optimization level
fn meta_opt_level(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Int(ctx.runtime_info.opt_level as i128))
}

/// Get compiler version
fn meta_compiler_version(_ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Text(Text::from(env!("CARGO_PKG_VERSION"))))
}

// ============================================================================
// Feature Flags
// ============================================================================

/// Check if a feature is enabled
fn meta_has_feature(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(feature) => {
            let has = ctx.runtime_info.enabled_features.iter().any(|f| f == feature);
            Ok(ConstValue::Bool(has))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Get list of enabled features
fn meta_enabled_features(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    let features: Vec<ConstValue> = ctx
        .runtime_info
        .enabled_features
        .iter()
        .map(|f| ConstValue::Text(f.clone()))
        .collect();
    Ok(ConstValue::Array(List::from(features)))
}

// ============================================================================
// Environment
// ============================================================================

/// Get environment variable
fn meta_env(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(name) => {
            match std::env::var(name.as_str()) {
                Ok(value) => Ok(ConstValue::Maybe(Maybe::Some(verum_common::Heap::new(
                    ConstValue::Text(Text::from(value)),
                )))),
                Err(_) => Ok(ConstValue::Maybe(Maybe::None)),
            }
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Check if running in CI environment
fn meta_is_ci(_ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    let is_ci = std::env::var("CI").is_ok()
        || std::env::var("GITHUB_ACTIONS").is_ok()
        || std::env::var("GITLAB_CI").is_ok()
        || std::env::var("JENKINS_URL").is_ok();
    Ok(ConstValue::Bool(is_ci))
}

// ============================================================================
// Runtime Configuration
// ============================================================================

/// Get runtime configuration name
fn meta_runtime_config(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    match &ctx.runtime_info.runtime_config {
        Some(config) => Ok(ConstValue::Text(config.clone())),
        None => Ok(ConstValue::Text(Text::from("full"))),
    }
}

/// Get recursion limit
fn meta_recursion_limit(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Int(ctx.runtime_info.recursion_limit as i128))
}

/// Get iteration limit
fn meta_iteration_limit(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Int(ctx.runtime_info.iteration_limit as i128))
}

/// Get memory limit
fn meta_memory_limit(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Int(ctx.runtime_info.memory_limit as i128))
}

/// Get timeout in milliseconds
fn meta_timeout_ms(ctx: &mut MetaContext, _args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    Ok(ConstValue::Int(ctx.runtime_info.timeout_ms as i128))
}

/// Get config value by key
fn meta_config_get(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(key) => {
            match ctx.runtime_info.config.get(key) {
                Some(value) => Ok(ConstValue::Maybe(Maybe::Some(verum_common::Heap::new(
                    ConstValue::Text(value.clone()),
                )))),
                None => Ok(ConstValue::Maybe(Maybe::None)),
            }
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Get config value as integer
fn meta_config_get_int(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(key) => {
            match ctx.runtime_info.config.get(key) {
                Some(value) => {
                    match value.parse::<i128>() {
                        Ok(i) => Ok(ConstValue::Maybe(Maybe::Some(verum_common::Heap::new(
                            ConstValue::Int(i),
                        )))),
                        Err(_) => Ok(ConstValue::Maybe(Maybe::None)),
                    }
                }
                None => Ok(ConstValue::Maybe(Maybe::None)),
            }
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Get config value as boolean
fn meta_config_get_bool(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(key) => {
            match ctx.runtime_info.config.get(key) {
                Some(value) => {
                    let b = value.as_str() == "true" || value.as_str() == "1";
                    Ok(ConstValue::Maybe(Maybe::Some(verum_common::Heap::new(
                        ConstValue::Bool(b),
                    ))))
                }
                None => Ok(ConstValue::Maybe(Maybe::None)),
            }
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

/// Get config array by key
fn meta_config_get_array(ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(key) => {
            match ctx.runtime_info.config_arrays.get(key) {
                Some(arr) => {
                    let items: Vec<ConstValue> = arr
                        .iter()
                        .map(|s| ConstValue::Text(s.clone()))
                        .collect();
                    Ok(ConstValue::Maybe(Maybe::Some(verum_common::Heap::new(
                        ConstValue::Array(List::from(items)),
                    ))))
                }
                None => Ok(ConstValue::Maybe(Maybe::None)),
            }
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_target_os() {
        let mut ctx = MetaContext::new();
        let result = meta_target_os(&mut ctx, List::new()).unwrap();
        // Should return one of the known OS values
        if let ConstValue::Text(os) = result {
            assert!(
                os.as_str() == "linux"
                    || os.as_str() == "macos"
                    || os.as_str() == "windows"
                    || os.as_str() == "unknown"
            );
        } else {
            panic!("Expected Text");
        }
    }

    #[test]
    fn test_target_arch() {
        let mut ctx = MetaContext::new();
        let result = meta_target_arch(&mut ctx, List::new()).unwrap();
        if let ConstValue::Text(arch) = result {
            assert!(
                arch.as_str() == "x86_64"
                    || arch.as_str() == "aarch64"
                    || arch.as_str() == "x86"
                    || arch.as_str() == "arm"
                    || arch.as_str() == "unknown"
            );
        } else {
            panic!("Expected Text");
        }
    }

    #[test]
    fn test_pointer_width() {
        let mut ctx = MetaContext::new();
        let result = meta_target_pointer_width(&mut ctx, List::new()).unwrap();
        if let ConstValue::Int(width) = result {
            assert!(width == 32 || width == 64);
        } else {
            panic!("Expected Int");
        }
    }

    #[test]
    fn test_is_debug() {
        let mut ctx = MetaContext::new();
        let result = meta_is_debug(&mut ctx, List::new()).unwrap();
        assert!(matches!(result, ConstValue::Bool(_)));
    }

    #[test]
    fn test_opt_level() {
        let mut ctx = MetaContext::new();
        let result = meta_opt_level(&mut ctx, List::new()).unwrap();
        if let ConstValue::Int(level) = result {
            assert!(level >= 0 && level <= 3);
        } else {
            panic!("Expected Int");
        }
    }

    #[test]
    fn test_has_feature() {
        let mut ctx = MetaContext::new();
        ctx.runtime_info.enabled_features.push(Text::from("test_feature"));

        let args = List::from(vec![ConstValue::Text(Text::from("test_feature"))]);
        let result = meta_has_feature(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));

        let args = List::from(vec![ConstValue::Text(Text::from("nonexistent"))]);
        let result = meta_has_feature(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(false));
    }

    #[test]
    fn test_recursion_limit() {
        let mut ctx = MetaContext::new();
        let result = meta_recursion_limit(&mut ctx, List::new()).unwrap();
        if let ConstValue::Int(limit) = result {
            assert!(limit > 0);
        } else {
            panic!("Expected Int");
        }
    }

    #[test]
    fn test_config_get() {
        let mut ctx = MetaContext::new();
        ctx.runtime_info.config.insert(Text::from("test_key"), Text::from("test_value"));

        let args = List::from(vec![ConstValue::Text(Text::from("test_key"))]);
        let result = meta_config_get(&mut ctx, args).unwrap();

        if let ConstValue::Maybe(Maybe::Some(boxed)) = result {
            if let ConstValue::Text(value) = boxed.as_ref() {
                assert_eq!(value.as_str(), "test_value");
            } else {
                panic!("Expected Text in Some");
            }
        } else {
            panic!("Expected Some");
        }
    }

    #[test]
    fn target_has_feature_unknown_returns_false() {
        // Names that don't appear in the dispatch table fall
        // through to false. Pre-fix this was the only path
        // (every input → false); the test ensures unknown names
        // remain conservatively false post-fix.
        let mut ctx = MetaContext::new();
        let args =
            List::from(vec![ConstValue::Text(Text::from("nonexistent_feature_xyz"))]);
        let result = meta_target_has_feature(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(false));
    }

    #[test]
    fn target_has_feature_baseline_isa_is_detected() {
        // Each Tier-1 architecture has a baseline ISA feature that
        // Rust's default target requires. Pre-fix the function
        // returned false for every input — so a passing assertion
        // here is the strongest single-test pin that wiring is in
        // place: if `cfg!(target_feature = ...)` reaches the
        // dispatch path, at least one of these must light up.
        let mut ctx = MetaContext::new();
        let baseline = if cfg!(target_arch = "x86_64") {
            "sse2"
        } else if cfg!(target_arch = "aarch64") {
            "neon"
        } else {
            // Other archs: skip the positive assertion, the
            // unknown-returns-false pin still locks the dispatch.
            return;
        };
        let args = List::from(vec![ConstValue::Text(Text::from(baseline))]);
        let result = meta_target_has_feature(&mut ctx, args).unwrap();
        assert_eq!(
            result,
            ConstValue::Bool(true),
            "{} must be detected on this Tier-1 arch — pre-fix would \
             return false unconditionally",
            baseline
        );
    }

    #[test]
    fn target_has_feature_arity_and_type_errors() {
        let mut ctx = MetaContext::new();

        // Wrong arity (no args)
        let result = meta_target_has_feature(&mut ctx, List::new());
        assert!(matches!(result, Err(MetaError::ArityMismatch { .. })));

        // Wrong arg type (Int instead of Text)
        let args = List::from(vec![ConstValue::Int(42)]);
        let result = meta_target_has_feature(&mut ctx, args);
        assert!(matches!(result, Err(MetaError::TypeMismatch { .. })));
    }
}
