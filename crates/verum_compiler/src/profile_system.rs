//! Language Profile System
//!
//! Three profiles for progressive complexity:
//! - **Application**: Safe, productive, full checks. VBC-interpretable.
//! - **Systems**: Performance-critical, optional unsafe. NOT VBC-interpretable.
//! - **Research**: Experimental features, dependent types. VBC-interpretable.
//!
//! # VBC Interpretability
//!
//! The **Application** and **Research** profiles produce code that can be executed
//! by the VBC interpreter (Tier 0). This enables rapid development iteration.
//!
//! The **Systems** profile produces code that is **NOT interpretable** by VBC.
//! VBC serves only as an intermediate representation for systems profile code.
//! Systems code must be compiled to native code via AOT (Tier 1) before execution.
//! This profile is intended for:
//! - Embedded systems
//! - OS kernels
//! - Device drivers
//! - Low-level system programming
//!
//! # No-libc Linking
//!
//! All profiles use Verum's self-contained runtime without libc dependency:
//! - **Linux**: Direct syscalls (stable ABI)
//! - **macOS**: libSystem.B.dylib only
//! - **Windows**: ntdll.dll + kernel32.dll only
//!
//! Language Profile System (Compilation Pipeline Phase):
//! Three profiles control progressive complexity in the compilation pipeline.
//! Application profile enables all safety checks and is VBC-interpretable for
//! rapid development. Systems profile enables raw pointers, inline assembly,
//! and no-libc linking — code is NOT VBC-interpretable and requires AOT.
//! Research profile enables dependent types, formal proofs, and linear types
//! while remaining VBC-interpretable. Each profile gates feature availability:
//! Application forbids unsafe blocks; Systems allows @unsafe regions with
//! manual safety proofs; Research adds experimental type system features.
//! V-LLSI architecture ensures VBC bytecode is the universal IR, with
//! interpretation available for Application/Research and LLVM AOT for all.

use verum_common::Set;

use crate::phases::ExecutionTier;

/// Language profile
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Profile {
    /// Application profile: Safe and productive.
    /// VBC-interpretable for rapid development.
    Application,

    /// Systems profile: Performance-critical.
    /// NOT VBC-interpretable - AOT compilation required.
    /// Used for embedded, OS kernels, drivers.
    Systems,

    /// Research profile: Experimental features.
    /// VBC-interpretable for experimentation.
    Research,
}

impl Profile {
    /// Get the default profile
    pub fn default() -> Self {
        Profile::Application
    }

    /// Get profile name
    pub fn name(&self) -> &str {
        match self {
            Profile::Application => "Application",
            Profile::Systems => "Systems",
            Profile::Research => "Research",
        }
    }

    /// Get profile description
    pub fn description(&self) -> &str {
        match self {
            Profile::Application => "Safe, productive development with full safety checks",
            Profile::Systems => "Performance-critical code with optional unsafe (AOT only)",
            Profile::Research => "Experimental features and formal verification",
        }
    }

    /// Check if this profile allows VBC interpretation.
    ///
    /// # VBC Interpretability Rules
    ///
    /// - **Application**: VBC-interpretable (Tier 0 execution supported)
    /// - **Systems**: NOT interpretable (VBC is intermediate IR only, AOT required)
    /// - **Research**: VBC-interpretable (Tier 0 execution supported)
    ///
    /// Systems profile code cannot be interpreted because:
    /// 1. It may use raw pointers and unsafe operations not supported by interpreter
    /// 2. It may require direct hardware access (memory-mapped I/O, interrupts)
    /// 3. It is intended for embedded/OS kernel development where interpretation
    ///    is not meaningful
    /// 4. VBC serves only as a portable IR for cross-compilation, not execution
    pub fn is_vbc_interpretable(&self) -> bool {
        match self {
            Profile::Application => true,
            Profile::Systems => false, // VBC is intermediate IR only
            Profile::Research => true,
        }
    }

    /// Check if this profile requires AOT compilation for execution.
    ///
    /// Systems profile MUST use AOT compilation - VBC is only an intermediate
    /// representation for portable cross-compilation.
    pub fn requires_aot(&self) -> bool {
        match self {
            Profile::Application => false,
            Profile::Systems => true, // Always requires AOT
            Profile::Research => false,
        }
    }

    /// Check if this profile allows embedded/bare-metal targets.
    ///
    /// Only Systems profile is appropriate for embedded development.
    pub fn allows_embedded(&self) -> bool {
        match self {
            Profile::Application => false,
            Profile::Systems => true,
            Profile::Research => false,
        }
    }

    /// Get the default execution tier for this profile.
    ///
    /// - Application/Research: Tier 0 (Interpreter) for fast iteration
    /// - Systems: Tier 1 (AOT) required - no interpreter support
    pub fn default_execution_tier(&self) -> ExecutionTier {
        match self {
            Profile::Application => ExecutionTier::Interpreter,
            Profile::Systems => ExecutionTier::Aot,
            Profile::Research => ExecutionTier::Interpreter,
        }
    }

    /// Get enabled features for this profile
    pub fn enabled_features(&self) -> Set<Feature> {
        let mut features = Set::new();

        // Base features (all profiles)
        features.insert(Feature::BasicTypes);
        features.insert(Feature::Functions);
        features.insert(Feature::Generics);
        features.insert(Feature::Traits);
        features.insert(Feature::Async);

        match self {
            Profile::Application => {
                features.insert(Feature::RefinementTypes);
                features.insert(Feature::ContextSystem);
                features.insert(Feature::Cbgr);
            }
            Profile::Systems => {
                features.insert(Feature::RefinementTypes);
                features.insert(Feature::ContextSystem);
                features.insert(Feature::Cbgr);
                features.insert(Feature::UnsafeCode);
                features.insert(Feature::InlineAssembly);
                features.insert(Feature::RawPointers);
            }
            Profile::Research => {
                features.insert(Feature::RefinementTypes);
                features.insert(Feature::ContextSystem);
                features.insert(Feature::Cbgr);
                features.insert(Feature::DependentTypes);
                features.insert(Feature::FormalProofs);
                features.insert(Feature::LinearTypes);
                // Note: Verum does NOT support algebraic effects by design.
                // Use ContextSystem for dependency injection instead.
            }
        }

        features
    }

    /// Check if a feature is enabled
    pub fn is_feature_enabled(&self, feature: Feature) -> bool {
        self.enabled_features().contains(&feature)
    }
}

/// Language features
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feature {
    // Base features
    BasicTypes,
    Functions,
    Generics,
    Traits,
    Async,

    // Safety features
    RefinementTypes,
    ContextSystem,
    Cbgr,

    // Systems features
    UnsafeCode,
    InlineAssembly,
    RawPointers,

    // Research features
    DependentTypes,
    FormalProofs,
    LinearTypes,
    // Note: No EffectSystem - Verum uses ContextSystem, not algebraic effects
}

impl Feature {
    /// Get feature name
    pub fn name(&self) -> &str {
        match self {
            Feature::BasicTypes => "basic_types",
            Feature::Functions => "functions",
            Feature::Generics => "generics",
            Feature::Traits => "traits",
            Feature::Async => "async",
            Feature::RefinementTypes => "refinement_types",
            Feature::ContextSystem => "context_system",
            Feature::Cbgr => "cbgr",
            Feature::UnsafeCode => "unsafe_code",
            Feature::InlineAssembly => "inline_assembly",
            Feature::RawPointers => "raw_pointers",
            Feature::DependentTypes => "dependent_types",
            Feature::FormalProofs => "formal_proofs",
            Feature::LinearTypes => "linear_types",
        }
    }
}

/// Profile manager
pub struct ProfileManager {
    current_profile: Profile,
}

impl ProfileManager {
    pub fn new(profile: Profile) -> Self {
        Self {
            current_profile: profile,
        }
    }

    pub fn profile(&self) -> Profile {
        self.current_profile
    }

    pub fn set_profile(&mut self, profile: Profile) {
        self.current_profile = profile;
    }

    pub fn is_feature_enabled(&self, feature: Feature) -> bool {
        self.current_profile.is_feature_enabled(feature)
    }
}

impl Default for ProfileManager {
    fn default() -> Self {
        Self::new(Profile::Application)
    }
}

// =============================================================================
// No-Libc Linking Configuration (V-LLSI Architecture)
// =============================================================================

// Re-export linking types from verum_codegen - single source of truth
pub use verum_codegen::link::{NoLibcConfig, Platform};

/// Backward-compatible alias for NoLibcConfig
pub type LinkingConfig = NoLibcConfig;

/// Extension trait for backward-compatible LinkingConfig methods.
pub trait LinkingConfigExt {
    /// Alias for backward compatibility
    fn linux_no_libc() -> NoLibcConfig {
        NoLibcConfig::linux()
    }

    /// Alias for backward compatibility
    fn macos_no_libc() -> NoLibcConfig {
        NoLibcConfig::macos()
    }

    /// Alias for backward compatibility
    fn windows_no_libc() -> NoLibcConfig {
        NoLibcConfig::windows()
    }

    /// Alias for backward compatibility
    fn freebsd_no_libc() -> NoLibcConfig {
        NoLibcConfig::freebsd()
    }

    /// Check if this is a no-libc configuration.
    fn is_no_libc(&self) -> bool;
}

impl LinkingConfigExt for NoLibcConfig {
    fn is_no_libc(&self) -> bool {
        // All our configurations are no-libc by design
        true
    }
}

/// Extension trait for Platform with VBC interpretation support check.
pub trait PlatformExt {
    /// Check if platform supports VBC interpretation.
    ///
    /// Embedded platforms cannot support VBC interpretation because:
    /// - No OS to provide syscalls
    /// - Limited memory for interpreter overhead
    /// - Real-time constraints
    fn supports_vbc_interpretation(&self) -> bool;
}

impl PlatformExt for Platform {
    fn supports_vbc_interpretation(&self) -> bool {
        match self {
            Platform::Linux | Platform::MacOS | Platform::Windows | Platform::FreeBSD => true,
            Platform::WasmWasi | Platform::WasmEmbedded => true, // WASM supports VBC interpretation
            Platform::Embedded => false,
        }
    }
}
