//! Conditional Compilation Configuration (`@cfg`) for Verum.
//!
//! This module provides the AST types and evaluation logic for conditional
//! compilation based on target platform, features, and custom conditions.
//!
//! # Syntax
//!
//! ```verum
//! // Simple predicate
//! @cfg(unix)
//! @cfg(windows)
//! @cfg(debug_assertions)
//!
//! // Key-value predicates
//! @cfg(target_os = "linux")
//! @cfg(target_arch = "x86_64")
//! @cfg(feature = "simd")
//!
//! // Combinators
//! @cfg(all(unix, target_arch = "x86_64"))
//! @cfg(any(target_os = "linux", target_os = "macos"))
//! @cfg(not(windows))
//! @cfg(all(unix, not(target_os = "ios")))
//! ```
//!
//! # Target Configuration
//!
//! The following cfg options are predefined:
//!
//! | Option | Values | Description |
//! |--------|--------|-------------|
//! | `target_os` | linux, macos, windows, ios, android, freebsd, etc. | Operating system |
//! | `target_arch` | x86, x86_64, arm, aarch64, wasm32, riscv64, etc. | CPU architecture |
//! | `target_family` | unix, windows, wasm | OS family |
//! | `target_pointer_width` | "32", "64" | Pointer size in bits |
//! | `target_endian` | "little", "big" | Byte order |
//! | `target_env` | gnu, musl, msvc, sgx | C runtime environment |
//! | `target_vendor` | unknown, apple, pc, nvidia | Hardware vendor |
//! | `feature` | user-defined | Cargo-style feature flags |
//!
//! # Simple Predicates
//!
//! These are shorthand for common conditions:
//!
//! | Predicate | Equivalent |
//! |-----------|------------|
//! | `unix` | `target_family = "unix"` |
//! | `windows` | `target_family = "windows"` |
//! | `test` | Running in test mode |
//! | `debug_assertions` | Debug build |
//!
//! # Conditional Compilation
//!
//! Verum supports only the C ABI for FFI (the only stable, universal ABI).
//! Platform-specific code is selected via @cfg predicates that query target_os,
//! target_arch, target_family, target_pointer_width, target_endian, target_env,
//! target_vendor, and user-defined feature flags. Predicates can be combined
//! with all(...), any(...), and not(...) combinators.

use crate::span::{Span, Spanned};
use serde::{Deserialize, Serialize};
use verum_common::{List, Map, Maybe, Text};

// =============================================================================
// CFG PREDICATE AST
// =============================================================================

/// A cfg predicate for conditional compilation.
///
/// This represents the condition inside `@cfg(...)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CfgPredicate {
    pub kind: CfgPredicateKind,
    pub span: Span,
}

impl CfgPredicate {
    pub fn new(kind: CfgPredicateKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Create a simple identifier predicate like `@cfg(unix)`.
    pub fn ident(name: impl Into<Text>, span: Span) -> Self {
        Self::new(CfgPredicateKind::Ident(name.into()), span)
    }

    /// Create a key-value predicate like `@cfg(target_os = "linux")`.
    pub fn key_value(key: impl Into<Text>, value: impl Into<Text>, span: Span) -> Self {
        Self::new(
            CfgPredicateKind::KeyValue {
                key: key.into(),
                value: value.into(),
            },
            span,
        )
    }

    /// Create an `all(...)` combinator.
    pub fn all(predicates: Vec<CfgPredicate>, span: Span) -> Self {
        Self::new(CfgPredicateKind::All(List::from(predicates)), span)
    }

    /// Create an `any(...)` combinator.
    pub fn any(predicates: Vec<CfgPredicate>, span: Span) -> Self {
        Self::new(CfgPredicateKind::Any(List::from(predicates)), span)
    }

    /// Create a `not(...)` combinator.
    pub fn not(predicate: CfgPredicate, span: Span) -> Self {
        Self::new(CfgPredicateKind::Not(Box::new(predicate)), span)
    }

    /// Evaluate this predicate against a target configuration.
    pub fn evaluate(&self, config: &TargetConfig) -> bool {
        match &self.kind {
            CfgPredicateKind::Ident(name) => config.is_set(name),
            CfgPredicateKind::KeyValue { key, value } => config.matches(key, value),
            CfgPredicateKind::All(predicates) => predicates.iter().all(|p| p.evaluate(config)),
            CfgPredicateKind::Any(predicates) => predicates.iter().any(|p| p.evaluate(config)),
            CfgPredicateKind::Not(predicate) => !predicate.evaluate(config),
        }
    }
}

impl Spanned for CfgPredicate {
    fn span(&self) -> Span {
        self.span
    }
}

/// The kind of cfg predicate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CfgPredicateKind {
    /// Simple identifier: `unix`, `windows`, `test`, `debug_assertions`
    Ident(Text),

    /// Key-value pair: `target_os = "linux"`, `feature = "simd"`
    KeyValue { key: Text, value: Text },

    /// All predicates must be true: `all(unix, target_arch = "x86_64")`
    All(List<CfgPredicate>),

    /// Any predicate must be true: `any(target_os = "linux", target_os = "macos")`
    Any(List<CfgPredicate>),

    /// Negation: `not(windows)`
    Not(Box<CfgPredicate>),
}

// =============================================================================
// TARGET CONFIGURATION
// =============================================================================

/// Target platform configuration for conditional compilation.
///
/// This struct holds all the information about the compilation target
/// that cfg predicates can query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetConfig {
    /// Operating system: "linux", "macos", "windows", "ios", "android", etc.
    pub target_os: Text,

    /// CPU architecture: "x86", "x86_64", "arm", "aarch64", "wasm32", etc.
    pub target_arch: Text,

    /// OS family: "unix", "windows", "wasm"
    pub target_family: Text,

    /// Pointer width: "32" or "64"
    pub target_pointer_width: Text,

    /// Byte order: "little" or "big"
    pub target_endian: Text,

    /// C runtime environment: "gnu", "musl", "msvc", "sgx"
    pub target_env: Text,

    /// Hardware vendor: "unknown", "apple", "pc", "nvidia"
    pub target_vendor: Text,

    /// Enabled feature flags
    pub features: List<Text>,

    /// Test mode enabled
    pub test: bool,

    /// Debug assertions enabled
    pub debug_assertions: bool,

    /// Additional custom cfg options
    pub custom: Map<Text, Text>,
}

impl TargetConfig {
    /// Create a new TargetConfig with default values for the current host.
    #[must_use]
    pub fn host() -> Self {
        // Normalize OS names: std::env::consts::OS returns "darwin" on macOS,
        // but Verum uses "macos" to match Rust's target_os naming convention.
        let raw_os = std::env::consts::OS;
        let normalized_os = match raw_os {
            "darwin" => "macos",
            other => other,
        };

        Self {
            target_os: Text::from(normalized_os),
            target_arch: Text::from(std::env::consts::ARCH),
            target_family: if cfg!(unix) {
                Text::from("unix")
            } else if cfg!(windows) {
                Text::from("windows")
            } else if cfg!(target_family = "wasm") {
                Text::from("wasm")
            } else {
                Text::from("unknown")
            },
            target_pointer_width: if cfg!(target_pointer_width = "64") {
                Text::from("64")
            } else {
                Text::from("32")
            },
            target_endian: if cfg!(target_endian = "little") {
                Text::from("little")
            } else {
                Text::from("big")
            },
            target_env: if cfg!(target_env = "gnu") {
                Text::from("gnu")
            } else if cfg!(target_env = "musl") {
                Text::from("musl")
            } else if cfg!(target_env = "msvc") {
                Text::from("msvc")
            } else {
                Text::from("")
            },
            target_vendor: Text::from("unknown"),
            features: List::new(),
            test: cfg!(test),
            debug_assertions: cfg!(debug_assertions),
            custom: Map::new(),
        }
    }

    /// Create a configuration for a specific target triple.
    #[must_use]
    pub fn for_target(target_triple: &str) -> Self {
        let parts: Vec<&str> = target_triple.split('-').collect();

        let (arch, vendor, os, env) = match parts.len() {
            1 => (parts[0], "unknown", "unknown", ""),
            2 => (parts[0], "unknown", parts[1], ""),
            3 => {
                // Distinguish arch-vendor-os (e.g., x86_64-apple-darwin)
                // from arch-os-env (e.g., thumbv7em-none-eabihf)
                if parts[1] == "none" || parts[1] == "unknown" {
                    (parts[0], "unknown", parts[1], parts[2])
                } else {
                    (parts[0], parts[1], parts[2], "")
                }
            }
            4 => (parts[0], parts[1], parts[2], parts[3]),
            _ => ("unknown", "unknown", "unknown", ""),
        };

        let family = match os {
            "linux" | "macos" | "darwin" | "freebsd" | "openbsd" | "netbsd" | "dragonfly"
            | "illumos" | "solaris" | "ios" | "android" => "unix",
            "windows" | "win32" => "windows",
            "wasm" | "wasi" | "wasm32" | "wasm64" => "wasm",
            _ => "unknown",
        };

        let pointer_width = match arch {
            "x86_64" | "aarch64" | "powerpc64" | "powerpc64le" | "s390x" | "riscv64" | "wasm64" => {
                "64"
            }
            "x86" | "arm" | "armv7" | "powerpc" | "riscv32" | "wasm32" | "mips" | "mipsel" => "32",
            _ => "64", // Default to 64-bit
        };

        let endian = match arch {
            "powerpc" | "powerpc64" | "s390x" | "mips" => "big",
            _ => "little",
        };

        // Normalize OS name
        let normalized_os = match os {
            "darwin" => "macos",
            "win32" => "windows",
            "wasi" => "wasi",
            _ => os,
        };

        Self {
            target_os: Text::from(normalized_os),
            target_arch: Text::from(arch),
            target_family: Text::from(family),
            target_pointer_width: Text::from(pointer_width),
            target_endian: Text::from(endian),
            target_env: Text::from(env),
            target_vendor: Text::from(vendor),
            features: List::new(),
            test: false,
            debug_assertions: true,
            custom: Map::new(),
        }
    }

    /// Check if a simple predicate is set.
    pub fn is_set(&self, name: &str) -> bool {
        match name {
            // OS family shortcuts
            "unix" => self.target_family.as_str() == "unix",
            "windows" => self.target_family.as_str() == "windows",
            "wasm" => self.target_family.as_str() == "wasm",

            // Build mode flags
            "test" => self.test,
            "debug_assertions" => self.debug_assertions,

            // OS shortcuts
            "linux" => self.target_os.as_str() == "linux",
            "macos" => self.target_os.as_str() == "macos",
            "ios" => self.target_os.as_str() == "ios",
            "android" => self.target_os.as_str() == "android",
            "freebsd" => self.target_os.as_str() == "freebsd",

            // Architecture shortcuts
            "x86" => self.target_arch.as_str() == "x86",
            "x86_64" => self.target_arch.as_str() == "x86_64",
            "arm" => self.target_arch.as_str() == "arm",
            "aarch64" => self.target_arch.as_str() == "aarch64",
            "wasm32" => self.target_arch.as_str() == "wasm32",
            "wasm64" => self.target_arch.as_str() == "wasm64",
            "riscv32" => self.target_arch.as_str() == "riscv32",
            "riscv64" => self.target_arch.as_str() == "riscv64",

            // Check custom predicates
            _ => self.custom.contains_key(&Text::from(name)),
        }
    }

    /// Check if a key-value predicate matches.
    pub fn matches(&self, key: &str, value: &str) -> bool {
        match key {
            "target_os" => self.target_os.as_str() == value,
            "target_arch" => self.target_arch.as_str() == value,
            "target_family" => self.target_family.as_str() == value,
            "target_pointer_width" => self.target_pointer_width.as_str() == value,
            "target_endian" => self.target_endian.as_str() == value,
            "target_env" => self.target_env.as_str() == value,
            "target_vendor" => self.target_vendor.as_str() == value,
            "feature" => self.features.iter().any(|f| f.as_str() == value),
            _ => self
                .custom
                .get(&Text::from(key))
                .map(|v| v.as_str() == value)
                .unwrap_or(false),
        }
    }

    /// Enable a feature flag.
    pub fn enable_feature(&mut self, feature: impl Into<Text>) {
        let feature = feature.into();
        if !self.features.contains(&feature) {
            self.features.push(feature);
        }
    }

    /// Check if a feature is enabled.
    pub fn has_feature(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f.as_str() == feature)
    }

    /// Set a custom cfg option.
    pub fn set_custom(&mut self, key: impl Into<Text>, value: impl Into<Text>) {
        self.custom.insert(key.into(), value.into());
    }

    /// Enable test mode.
    pub fn with_test(mut self, enabled: bool) -> Self {
        self.test = enabled;
        self
    }

    /// Enable debug assertions.
    pub fn with_debug_assertions(mut self, enabled: bool) -> Self {
        self.debug_assertions = enabled;
        self
    }
}

impl Default for TargetConfig {
    fn default() -> Self {
        Self::host()
    }
}

// =============================================================================
// CFG PREDICATE PARSING
// =============================================================================

/// Parse a cfg predicate from an attribute expression.
///
/// This function converts a generic Expr (from the parser) into a
/// structured CfgPredicate.
pub fn parse_cfg_predicate(expr: &crate::expr::Expr) -> Maybe<CfgPredicate> {
    use crate::expr::{BinOp, ExprKind};
    use crate::literal::LiteralKind;

    let span = expr.span;

    match &expr.kind {
        // Simple identifier: @cfg(unix)
        ExprKind::Path(path) => {
            path.as_ident().map(|ident| CfgPredicate::ident(ident.name.clone(), span))
        }

        // Key-value: @cfg(target_os = "linux")
        ExprKind::Binary { left, op, right } if *op == BinOp::Assign => {
            if let ExprKind::Path(key_path) = &left.kind {
                if let Some(key_ident) = key_path.as_ident() {
                    if let ExprKind::Literal(lit) = &right.kind {
                        if let LiteralKind::Text(string_lit) = &lit.kind {
                            return Maybe::Some(CfgPredicate::key_value(
                                key_ident.name.clone(),
                                Text::from(string_lit.as_str()),
                                span,
                            ));
                        }
                    }
                }
            }
            Maybe::None
        }

        // Function call: all(...), any(...), not(...)
        ExprKind::Call { func, args, .. } => {
            if let ExprKind::Path(func_path) = &func.kind {
                if let Some(func_ident) = func_path.as_ident() {
                    let name = func_ident.name.as_str();
                    match name {
                        "all" => {
                            let predicates: Vec<CfgPredicate> = args
                                .iter()
                                .filter_map(parse_cfg_predicate)
                                .collect();
                            if predicates.len() == args.len() {
                                Maybe::Some(CfgPredicate::all(predicates, span))
                            } else {
                                Maybe::None
                            }
                        }
                        "any" => {
                            let predicates: Vec<CfgPredicate> = args
                                .iter()
                                .filter_map(parse_cfg_predicate)
                                .collect();
                            if predicates.len() == args.len() {
                                Maybe::Some(CfgPredicate::any(predicates, span))
                            } else {
                                Maybe::None
                            }
                        }
                        "not" => {
                            if args.len() == 1 {
                                parse_cfg_predicate(&args[0]).map(|inner| CfgPredicate::not(inner, span))
                            } else {
                                Maybe::None
                            }
                        }
                        // Single identifier passed as function call (shouldn't happen, but handle gracefully)
                        _ => Maybe::Some(CfgPredicate::ident(name, span)),
                    }
                } else {
                    Maybe::None
                }
            } else {
                Maybe::None
            }
        }

        _ => Maybe::None,
    }
}

// =============================================================================
// CFG EVALUATOR
// =============================================================================

/// Evaluator for @cfg attributes on declarations.
///
/// This struct manages the target configuration and provides methods
/// to filter declarations based on cfg predicates.
#[derive(Debug, Clone)]
pub struct CfgEvaluator {
    config: TargetConfig,
}

impl CfgEvaluator {
    /// Create a new evaluator for the current host.
    pub fn new() -> Self {
        Self {
            config: TargetConfig::host(),
        }
    }

    /// Create an evaluator for a specific target.
    pub fn for_target(target_triple: &str) -> Self {
        Self {
            config: TargetConfig::for_target(target_triple),
        }
    }

    /// Create an evaluator with a custom configuration.
    pub fn with_config(config: TargetConfig) -> Self {
        Self { config }
    }

    /// Get the target configuration.
    pub fn config(&self) -> &TargetConfig {
        &self.config
    }

    /// Get mutable access to the target configuration.
    pub fn config_mut(&mut self) -> &mut TargetConfig {
        &mut self.config
    }

    /// Evaluate a cfg predicate.
    pub fn evaluate(&self, predicate: &CfgPredicate) -> bool {
        predicate.evaluate(&self.config)
    }

    /// Check if an item with the given attributes should be included.
    ///
    /// Returns `true` if all @cfg attributes on the item evaluate to true,
    /// or if the item has no @cfg attributes.
    pub fn should_include(&self, attrs: &[crate::attr::Attribute]) -> bool {
        for attr in attrs {
            if attr.name.as_str() == "cfg" {
                if let Maybe::Some(args) = &attr.args {
                    if let Some(first_arg) = args.first() {
                        if let Maybe::Some(predicate) = parse_cfg_predicate(first_arg) {
                            if !self.evaluate(&predicate) {
                                return false;
                            }
                        }
                    }
                }
            }
        }
        true
    }

    /// Filter a list of declarations, keeping only those that pass cfg checks.
    pub fn filter_decls<D: HasAttributes>(&self, decls: Vec<D>) -> Vec<D> {
        decls
            .into_iter()
            .filter(|d| self.should_include(d.attributes()))
            .collect()
    }

    /// Filter items from a module, keeping only those that pass cfg checks.
    ///
    /// This method is specifically designed for processing module items
    /// which are stored in a `List<Item>`.
    pub fn filter_items(&self, items: &List<crate::decl::Item>) -> List<crate::decl::Item> {
        items
            .iter()
            .filter(|item| self.should_include(&item.attributes))
            .cloned()
            .collect()
    }

    /// Check if an individual Item should be included based on its @cfg attributes.
    pub fn should_include_item(&self, item: &crate::decl::Item) -> bool {
        self.should_include(&item.attributes)
    }
}

impl Default for CfgEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for types that have attributes (for cfg filtering).
pub trait HasAttributes {
    fn attributes(&self) -> &[crate::attr::Attribute];
}

impl HasAttributes for crate::decl::Item {
    fn attributes(&self) -> &[crate::attr::Attribute] {
        &self.attributes
    }
}

impl HasAttributes for crate::Module {
    fn attributes(&self) -> &[crate::attr::Attribute] {
        &self.attributes
    }
}

// =============================================================================
// DISPLAY IMPLEMENTATIONS
// =============================================================================

impl std::fmt::Display for CfgPredicate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            CfgPredicateKind::Ident(name) => write!(f, "{}", name),
            CfgPredicateKind::KeyValue { key, value } => write!(f, "{} = \"{}\"", key, value),
            CfgPredicateKind::All(predicates) => {
                write!(f, "all(")?;
                for (i, p) in predicates.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ")")
            }
            CfgPredicateKind::Any(predicates) => {
                write!(f, "any(")?;
                for (i, p) in predicates.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ")")
            }
            CfgPredicateKind::Not(predicate) => write!(f, "not({})", predicate),
        }
    }
}

impl std::fmt::Display for TargetConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}-{}-{}-{}",
            self.target_arch, self.target_vendor, self.target_os, self.target_env
        )
    }
}

// =============================================================================
// COMMON TARGET CONFIGURATIONS
// =============================================================================

impl TargetConfig {
    /// Linux x86_64 (GNU libc)
    pub fn linux_x86_64() -> Self {
        Self::for_target("x86_64-unknown-linux-gnu")
    }

    /// Linux aarch64 (GNU libc)
    pub fn linux_aarch64() -> Self {
        Self::for_target("aarch64-unknown-linux-gnu")
    }

    /// macOS x86_64
    pub fn macos_x86_64() -> Self {
        Self::for_target("x86_64-apple-darwin")
    }

    /// macOS aarch64 (Apple Silicon)
    pub fn macos_aarch64() -> Self {
        Self::for_target("aarch64-apple-darwin")
    }

    /// Windows x86_64 (MSVC)
    pub fn windows_x86_64() -> Self {
        Self::for_target("x86_64-pc-windows-msvc")
    }

    /// Windows x86_64 (GNU/MinGW)
    pub fn windows_x86_64_gnu() -> Self {
        Self::for_target("x86_64-pc-windows-gnu")
    }

    /// WebAssembly (WASI)
    pub fn wasm32_wasi() -> Self {
        Self::for_target("wasm32-wasi")
    }

    /// WebAssembly (unknown target)
    pub fn wasm32_unknown() -> Self {
        Self::for_target("wasm32-unknown-unknown")
    }

    /// iOS aarch64
    pub fn ios_aarch64() -> Self {
        Self::for_target("aarch64-apple-ios")
    }

    /// Android aarch64
    pub fn android_aarch64() -> Self {
        Self::for_target("aarch64-linux-android")
    }

    // =========================================================================
    // EMBEDDED TARGETS (including <32-bit architectures)
    // =========================================================================

    /// ARM Cortex-M0/M0+ (ARMv6-M, 32-bit, no FPU)
    pub fn thumbv6m_none_eabi() -> Self {
        Self::for_target("thumbv6m-none-eabi")
    }

    /// ARM Cortex-M3 (ARMv7-M, 32-bit, no FPU)
    pub fn thumbv7m_none_eabi() -> Self {
        Self::for_target("thumbv7m-none-eabi")
    }

    /// ARM Cortex-M4/M7 with hardware FPU (ARMv7E-M, 32-bit)
    pub fn thumbv7em_none_eabihf() -> Self {
        Self::for_target("thumbv7em-none-eabihf")
    }

    /// ARM Cortex-M33 (ARMv8-M, 32-bit, TrustZone)
    pub fn thumbv8m_main_none_eabihf() -> Self {
        Self::for_target("thumbv8m.main-none-eabihf")
    }

    /// AVR microcontrollers (8-bit, e.g., ATmega328)
    /// Note: AVR uses 16-bit pointers despite being an 8-bit architecture
    pub fn avr_unknown() -> Self {
        Self {
            target_arch: Text::from("avr"),
            target_os: Text::from("none"),
            target_vendor: Text::from("unknown"),
            target_env: Text::from(""),
            target_family: Text::from(""),
            target_endian: Text::from("little"),
            target_pointer_width: Text::from("16"), // AVR uses 16-bit pointers
            features: List::new(),
            test: false,
            debug_assertions: false,
            custom: Map::new(),
        }
    }

    /// MSP430 microcontrollers (16-bit, Texas Instruments)
    pub fn msp430_none_elf() -> Self {
        Self {
            target_arch: Text::from("msp430"),
            target_os: Text::from("none"),
            target_vendor: Text::from("unknown"),
            target_env: Text::from(""),
            target_family: Text::from(""),
            target_endian: Text::from("little"),
            target_pointer_width: Text::from("16"),
            features: List::new(),
            test: false,
            debug_assertions: false,
            custom: Map::new(),
        }
    }

    /// RISC-V 32-bit embedded (no OS, soft-float)
    pub fn riscv32i_unknown_none_elf() -> Self {
        Self::for_target("riscv32i-unknown-none-elf")
    }

    /// RISC-V 32-bit embedded with compressed instructions
    pub fn riscv32imc_unknown_none_elf() -> Self {
        Self::for_target("riscv32imc-unknown-none-elf")
    }

    /// RISC-V 64-bit embedded
    pub fn riscv64gc_unknown_none_elf() -> Self {
        Self::for_target("riscv64gc-unknown-none-elf")
    }

    /// Xtensa ESP32 (32-bit, Espressif)
    pub fn xtensa_esp32_none_elf() -> Self {
        Self {
            target_arch: Text::from("xtensa"),
            target_os: Text::from("none"),
            target_vendor: Text::from("espressif"),
            target_env: Text::from(""),
            target_family: Text::from(""),
            target_endian: Text::from("little"),
            target_pointer_width: Text::from("32"),
            features: List::new(),
            test: false,
            debug_assertions: false,
            custom: Map::new(),
        }
    }

    /// Generic bare-metal target (no OS) - for custom embedded platforms
    ///
    /// # Arguments
    /// * `arch` - CPU architecture name
    /// * `pointer_width` - Pointer width in bits (8, 16, 24, 32, 64)
    pub fn bare_metal(arch: &str, pointer_width: u8) -> Self {
        Self {
            target_arch: Text::from(arch),
            target_os: Text::from("none"),
            target_vendor: Text::from("unknown"),
            target_env: Text::from(""),
            target_family: Text::from(""),
            target_endian: Text::from("little"),
            target_pointer_width: Text::from(pointer_width.to_string()),
            features: List::new(),
            test: false,
            debug_assertions: false,
            custom: Map::new(),
        }
    }
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::{FileId, Span};

    fn dummy_span() -> Span {
        Span::new(0, 0, FileId::new(0))
    }

    #[test]
    fn test_simple_predicate_unix() {
        let config = TargetConfig::linux_x86_64();
        let pred = CfgPredicate::ident("unix", dummy_span());
        assert!(pred.evaluate(&config));
    }

    #[test]
    fn test_simple_predicate_windows_on_linux() {
        let config = TargetConfig::linux_x86_64();
        let pred = CfgPredicate::ident("windows", dummy_span());
        assert!(!pred.evaluate(&config));
    }

    #[test]
    fn test_key_value_target_os() {
        let config = TargetConfig::macos_aarch64();

        let pred_macos = CfgPredicate::key_value("target_os", "macos", dummy_span());
        assert!(pred_macos.evaluate(&config));

        let pred_linux = CfgPredicate::key_value("target_os", "linux", dummy_span());
        assert!(!pred_linux.evaluate(&config));
    }

    #[test]
    fn test_key_value_target_arch() {
        let config = TargetConfig::linux_x86_64();

        let pred = CfgPredicate::key_value("target_arch", "x86_64", dummy_span());
        assert!(pred.evaluate(&config));

        let pred_arm = CfgPredicate::key_value("target_arch", "aarch64", dummy_span());
        assert!(!pred_arm.evaluate(&config));
    }

    #[test]
    fn test_all_combinator() {
        let config = TargetConfig::linux_x86_64();

        // all(unix, target_arch = "x86_64") should be true
        let pred = CfgPredicate::all(
            vec![
                CfgPredicate::ident("unix", dummy_span()),
                CfgPredicate::key_value("target_arch", "x86_64", dummy_span()),
            ],
            dummy_span(),
        );
        assert!(pred.evaluate(&config));

        // all(unix, target_arch = "aarch64") should be false
        let pred_false = CfgPredicate::all(
            vec![
                CfgPredicate::ident("unix", dummy_span()),
                CfgPredicate::key_value("target_arch", "aarch64", dummy_span()),
            ],
            dummy_span(),
        );
        assert!(!pred_false.evaluate(&config));
    }

    #[test]
    fn test_any_combinator() {
        let config = TargetConfig::linux_x86_64();

        // any(target_os = "linux", target_os = "macos") should be true
        let pred = CfgPredicate::any(
            vec![
                CfgPredicate::key_value("target_os", "linux", dummy_span()),
                CfgPredicate::key_value("target_os", "macos", dummy_span()),
            ],
            dummy_span(),
        );
        assert!(pred.evaluate(&config));

        // any(target_os = "windows", target_os = "freebsd") should be false
        let pred_false = CfgPredicate::any(
            vec![
                CfgPredicate::key_value("target_os", "windows", dummy_span()),
                CfgPredicate::key_value("target_os", "freebsd", dummy_span()),
            ],
            dummy_span(),
        );
        assert!(!pred_false.evaluate(&config));
    }

    #[test]
    fn test_not_combinator() {
        let config = TargetConfig::linux_x86_64();

        // not(windows) should be true on Linux
        let pred = CfgPredicate::not(
            CfgPredicate::ident("windows", dummy_span()),
            dummy_span(),
        );
        assert!(pred.evaluate(&config));

        // not(unix) should be false on Linux
        let pred_false = CfgPredicate::not(
            CfgPredicate::ident("unix", dummy_span()),
            dummy_span(),
        );
        assert!(!pred_false.evaluate(&config));
    }

    #[test]
    fn test_nested_combinators() {
        let config = TargetConfig::linux_x86_64();

        // all(not(windows), any(target_arch = "x86_64", target_arch = "aarch64"))
        let pred = CfgPredicate::all(
            vec![
                CfgPredicate::not(
                    CfgPredicate::ident("windows", dummy_span()),
                    dummy_span(),
                ),
                CfgPredicate::any(
                    vec![
                        CfgPredicate::key_value("target_arch", "x86_64", dummy_span()),
                        CfgPredicate::key_value("target_arch", "aarch64", dummy_span()),
                    ],
                    dummy_span(),
                ),
            ],
            dummy_span(),
        );
        assert!(pred.evaluate(&config));
    }

    #[test]
    fn test_feature_flag() {
        let mut config = TargetConfig::host();

        let pred = CfgPredicate::key_value("feature", "simd", dummy_span());
        assert!(!pred.evaluate(&config));

        config.enable_feature("simd");
        assert!(pred.evaluate(&config));
    }

    #[test]
    fn test_debug_assertions() {
        let config = TargetConfig::host().with_debug_assertions(true);

        let pred = CfgPredicate::ident("debug_assertions", dummy_span());
        assert!(pred.evaluate(&config));

        let config_release = TargetConfig::host().with_debug_assertions(false);
        assert!(!pred.evaluate(&config_release));
    }

    #[test]
    fn test_test_mode() {
        let config = TargetConfig::host().with_test(true);

        let pred = CfgPredicate::ident("test", dummy_span());
        assert!(pred.evaluate(&config));

        let config_no_test = TargetConfig::host().with_test(false);
        assert!(!pred.evaluate(&config_no_test));
    }

    #[test]
    fn test_target_config_from_triple() {
        let config = TargetConfig::for_target("aarch64-apple-darwin");

        assert_eq!(config.target_arch.as_str(), "aarch64");
        assert_eq!(config.target_os.as_str(), "macos");
        assert_eq!(config.target_vendor.as_str(), "apple");
        assert_eq!(config.target_family.as_str(), "unix");
        assert_eq!(config.target_pointer_width.as_str(), "64");
        assert_eq!(config.target_endian.as_str(), "little");
    }

    #[test]
    fn test_windows_target() {
        let config = TargetConfig::windows_x86_64();

        assert_eq!(config.target_os.as_str(), "windows");
        assert_eq!(config.target_family.as_str(), "windows");
        assert!(config.is_set("windows"));
        assert!(!config.is_set("unix"));
    }

    #[test]
    fn test_wasm_target() {
        let config = TargetConfig::wasm32_wasi();

        assert_eq!(config.target_arch.as_str(), "wasm32");
        assert_eq!(config.target_family.as_str(), "wasm");
        assert!(config.is_set("wasm"));
        assert_eq!(config.target_pointer_width.as_str(), "32");
    }

    #[test]
    fn test_cfg_evaluator() {
        let config = TargetConfig::linux_x86_64();
        let evaluator = CfgEvaluator::with_config(config);

        let pred_unix = CfgPredicate::ident("unix", dummy_span());
        assert!(evaluator.evaluate(&pred_unix));

        let pred_windows = CfgPredicate::ident("windows", dummy_span());
        assert!(!evaluator.evaluate(&pred_windows));
    }

    #[test]
    fn test_cfg_predicate_display() {
        let pred = CfgPredicate::all(
            vec![
                CfgPredicate::ident("unix", dummy_span()),
                CfgPredicate::key_value("target_arch", "x86_64", dummy_span()),
            ],
            dummy_span(),
        );

        let display = format!("{}", pred);
        assert_eq!(display, "all(unix, target_arch = \"x86_64\")");
    }

    #[test]
    fn test_custom_cfg() {
        let mut config = TargetConfig::host();
        config.set_custom("my_option", "enabled");

        assert!(config.is_set("my_option"));
        assert!(config.matches("my_option", "enabled"));
        assert!(!config.matches("my_option", "disabled"));
    }
}
