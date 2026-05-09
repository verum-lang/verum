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

// =========================================================================
// Profile metadata — single source of truth for the 3 variants.
//
// Pre-refactor Profile carried seven parallel match-arm accessors —
// `name`, `description`, `is_vbc_interpretable`, `requires_aot`,
// `allows_embedded`, `default_execution_tier`, `enabled_features`
// — each independently spelling out per-variant data.  Cross-cutting
// invariants (e.g. `is_vbc_interpretable ⊕ requires_aot`,
// `allows_embedded ⇒ requires_aot`) lived implicitly across the
// independent matches and could drift on rename.
//
// Latent performance defect: `enabled_features()` constructed a
// fresh `Set<Feature>` on every call by pushing 8-11 entries, and
// `is_feature_enabled(f)` in turn called `enabled_features()` —
// every feature lookup allocated.  Hot consumers (e.g. the
// `phases::ffi_boundary` audit pass) hit this on every FFI
// declaration.  After consolidation, `enabled_features` returns a
// const `&'static [Feature]` slice and `is_feature_enabled`
// becomes a slice scan — zero allocation.
//
// Same drift-collapse pattern as the verum_vbc sub-opcode meta()
// series + verum_compiler LintMeta + verum_ast BinOpMeta.
// =========================================================================

/// Co-located metadata for one `Profile` variant.
#[derive(Debug, Clone, Copy)]
pub struct ProfileMeta {
    /// Display name (`"Application"`, `"Systems"`, `"Research"`).
    pub name: &'static str,
    /// One-line description for `--help` output.
    pub description: &'static str,
    /// Whether code in this profile can run on the Tier-0
    /// VBC interpreter.  Mutually exclusive with `requires_aot`.
    pub is_vbc_interpretable: bool,
    /// Whether code in this profile must be AOT-compiled.
    /// Mirror of `!is_vbc_interpretable` (pinned by drift test).
    pub requires_aot: bool,
    /// Whether this profile allows embedded/bare-metal targets.
    /// Implies `requires_aot` (pinned by drift test).
    pub allows_embedded: bool,
    /// Default execution tier for this profile.  Consistent
    /// with `is_vbc_interpretable` / `requires_aot` (pinned).
    pub default_execution_tier: ExecutionTier,
    /// Static slice of features enabled by this profile.
    /// Pre-computed at compile time; zero-allocation membership
    /// check via slice scan.
    pub enabled_features: &'static [Feature],
}

// --- Per-profile feature sets, defined once as static slices ---
//
// Co-locating these constants here (vs inlining at each match arm)
// makes them grep-able and lets the meta() table stay narrow.  Each
// slice is `&'static [Feature]` so `is_feature_enabled` does a
// const-time linear scan with no allocation.

/// Application profile: base + safety triad.
const APPLICATION_FEATURES: &[Feature] = &[
    Feature::BasicTypes, Feature::Functions, Feature::Generics, Feature::Traits, Feature::Async,
    Feature::RefinementTypes, Feature::ContextSystem, Feature::Cbgr,
];

/// Systems profile: base + safety triad + systems triad.
const SYSTEMS_FEATURES: &[Feature] = &[
    Feature::BasicTypes, Feature::Functions, Feature::Generics, Feature::Traits, Feature::Async,
    Feature::RefinementTypes, Feature::ContextSystem, Feature::Cbgr,
    Feature::UnsafeCode, Feature::InlineAssembly, Feature::RawPointers,
];

/// Research profile: base + safety triad + research triad.
/// Note: Verum does NOT support algebraic effects by design — the
/// ContextSystem is the dependency-injection mechanism instead.
const RESEARCH_FEATURES: &[Feature] = &[
    Feature::BasicTypes, Feature::Functions, Feature::Generics, Feature::Traits, Feature::Async,
    Feature::RefinementTypes, Feature::ContextSystem, Feature::Cbgr,
    Feature::DependentTypes, Feature::FormalProofs, Feature::LinearTypes,
];

impl Profile {
    /// All variants in stable order — canonical iteration source
    /// for drift-pin tests.
    pub const ALL: &'static [Self] = &[
        Self::Application,
        Self::Systems,
        Self::Research,
    ];

    /// Returns co-located metadata for this profile.  Single
    /// source of truth for `name` / `description` / interpretability
    /// flags / default tier / enabled feature set.
    pub const fn meta(self) -> ProfileMeta {
        match self {
            Self::Application => ProfileMeta {
                name: "Application",
                description: "Safe, productive development with full safety checks",
                is_vbc_interpretable: true,
                requires_aot: false,
                allows_embedded: false,
                default_execution_tier: ExecutionTier::Interpreter,
                enabled_features: APPLICATION_FEATURES,
            },
            Self::Systems => ProfileMeta {
                // Systems profile cannot be interpreted: it may
                // use raw pointers / unsafe ops not supported by
                // the interpreter and is intended for embedded /
                // OS-kernel development where interpretation is
                // not meaningful.  VBC serves only as a portable
                // IR for cross-compilation in this profile.
                name: "Systems",
                description: "Performance-critical code with optional unsafe (AOT only)",
                is_vbc_interpretable: false,
                requires_aot: true,
                allows_embedded: true,
                default_execution_tier: ExecutionTier::Aot,
                enabled_features: SYSTEMS_FEATURES,
            },
            Self::Research => ProfileMeta {
                name: "Research",
                description: "Experimental features and formal verification",
                is_vbc_interpretable: true,
                requires_aot: false,
                allows_embedded: false,
                default_execution_tier: ExecutionTier::Interpreter,
                enabled_features: RESEARCH_FEATURES,
            },
        }
    }

    /// Get the default profile
    pub fn default() -> Self {
        Profile::Application
    }

    /// Get profile name
    #[inline]
    pub const fn name(&self) -> &'static str {
        self.meta().name
    }

    /// Get profile description
    #[inline]
    pub const fn description(&self) -> &'static str {
        self.meta().description
    }

    /// Check if this profile allows VBC interpretation.
    ///
    /// VBC Interpretability Rules:
    /// - **Application**: VBC-interpretable (Tier 0 execution supported)
    /// - **Systems**: NOT interpretable (VBC is intermediate IR only, AOT required)
    /// - **Research**: VBC-interpretable (Tier 0 execution supported)
    #[inline]
    pub const fn is_vbc_interpretable(&self) -> bool {
        self.meta().is_vbc_interpretable
    }

    /// Check if this profile requires AOT compilation for execution.
    ///
    /// Systems profile MUST use AOT compilation - VBC is only an intermediate
    /// representation for portable cross-compilation.
    #[inline]
    pub const fn requires_aot(&self) -> bool {
        self.meta().requires_aot
    }

    /// Check if this profile allows embedded/bare-metal targets.
    ///
    /// Only Systems profile is appropriate for embedded development.
    #[inline]
    pub const fn allows_embedded(&self) -> bool {
        self.meta().allows_embedded
    }

    /// Get the default execution tier for this profile.
    ///
    /// - Application/Research: Tier 0 (Interpreter) for fast iteration
    /// - Systems: Tier 1 (AOT) required - no interpreter support
    #[inline]
    pub const fn default_execution_tier(&self) -> ExecutionTier {
        self.meta().default_execution_tier
    }

    /// Get enabled features for this profile.
    ///
    /// Returns a `Set<Feature>` for back-compat; the underlying
    /// data is a `&'static [Feature]` slice (see
    /// `enabled_features_slice` for the zero-allocation accessor).
    pub fn enabled_features(&self) -> Set<Feature> {
        self.meta().enabled_features.iter().copied().collect()
    }

    /// Get enabled features as a static slice — zero-allocation
    /// accessor for hot-path consumers.  Preserves the canonical
    /// declaration order from the per-profile feature constants.
    #[inline]
    pub const fn enabled_features_slice(&self) -> &'static [Feature] {
        self.meta().enabled_features
    }

    /// Check if a feature is enabled.
    ///
    /// Pre-refactor this method allocated a `Set<Feature>` per
    /// call by invoking `enabled_features()`.  Now it scans the
    /// static feature slice — zero allocation, O(n=14) worst
    /// case but n is bounded and the slice is L1-cache resident.
    #[inline]
    pub fn is_feature_enabled(&self, feature: Feature) -> bool {
        self.meta().enabled_features.contains(&feature)
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

// =========================================================================
// Profile meta() drift-pin tests
//
// These tests pin the cross-cutting invariants between the seven
// per-profile fields that the legacy implementation maintained
// implicitly across independent match statements.
// =========================================================================

#[cfg(test)]
mod profile_meta_drift_pins {
    use super::*;

    #[test]
    fn profile_count_pinned_at_three() {
        assert_eq!(Profile::ALL.len(), 3,
            "Profile variant count drift: expected 3 (Application / Systems / Research)");
    }

    /// `is_vbc_interpretable` ⊕ `requires_aot` — every profile is
    /// either interpretable or AOT-required, never both, never
    /// neither.
    #[test]
    fn profile_interpretable_xor_aot_required() {
        for &p in Profile::ALL {
            assert_ne!(p.is_vbc_interpretable(), p.requires_aot(),
                "{:?}: is_vbc_interpretable={} but requires_aot={} — must be mutually exclusive",
                p, p.is_vbc_interpretable(), p.requires_aot());
        }
    }

    /// `allows_embedded` ⇒ `requires_aot` — embedded targets
    /// cannot use the VBC interpreter.
    #[test]
    fn profile_embedded_implies_aot() {
        for &p in Profile::ALL {
            if p.allows_embedded() {
                assert!(p.requires_aot(),
                    "{:?}: allows_embedded but does not require_aot — embedded targets cannot interpret",
                    p);
            }
        }
    }

    /// `default_execution_tier` is consistent with
    /// `is_vbc_interpretable` / `requires_aot`.
    #[test]
    fn profile_default_tier_consistent_with_interpretability() {
        for &p in Profile::ALL {
            match p.default_execution_tier() {
                ExecutionTier::Interpreter => assert!(p.is_vbc_interpretable(),
                    "{:?}: default tier is Interpreter but is_vbc_interpretable=false", p),
                ExecutionTier::Aot => assert!(p.requires_aot(),
                    "{:?}: default tier is Aot but requires_aot=false", p),
                _ => {}  // Check / other tiers — no specific invariant
            }
        }
    }

    /// Every profile carries the five base features
    /// (BasicTypes, Functions, Generics, Traits, Async).
    #[test]
    fn profile_base_features_present_in_every_profile() {
        let base = [
            Feature::BasicTypes,
            Feature::Functions,
            Feature::Generics,
            Feature::Traits,
            Feature::Async,
        ];
        for &p in Profile::ALL {
            for &f in &base {
                assert!(p.is_feature_enabled(f),
                    "{:?}: missing base feature {:?}", p, f);
            }
        }
    }

    /// Every profile carries the safety triad (RefinementTypes,
    /// ContextSystem, Cbgr).  These are foundational Verum
    /// features — every profile uses them.
    #[test]
    fn profile_safety_triad_present_in_every_profile() {
        let safety = [
            Feature::RefinementTypes,
            Feature::ContextSystem,
            Feature::Cbgr,
        ];
        for &p in Profile::ALL {
            for &f in &safety {
                assert!(p.is_feature_enabled(f),
                    "{:?}: missing safety-triad feature {:?}", p, f);
            }
        }
    }

    /// Systems-specific features are exclusive to Systems profile.
    #[test]
    fn profile_systems_features_exclusive() {
        let systems_only = [
            Feature::UnsafeCode,
            Feature::InlineAssembly,
            Feature::RawPointers,
        ];
        for &f in &systems_only {
            assert!(Profile::Systems.is_feature_enabled(f),
                "Systems should have {:?}", f);
            assert!(!Profile::Application.is_feature_enabled(f),
                "Application should NOT have {:?}", f);
            assert!(!Profile::Research.is_feature_enabled(f),
                "Research should NOT have {:?}", f);
        }
    }

    /// Research-specific features are exclusive to Research profile.
    #[test]
    fn profile_research_features_exclusive() {
        let research_only = [
            Feature::DependentTypes,
            Feature::FormalProofs,
            Feature::LinearTypes,
        ];
        for &f in &research_only {
            assert!(Profile::Research.is_feature_enabled(f),
                "Research should have {:?}", f);
            assert!(!Profile::Application.is_feature_enabled(f),
                "Application should NOT have {:?}", f);
            assert!(!Profile::Systems.is_feature_enabled(f),
                "Systems should NOT have {:?}", f);
        }
    }

    /// `enabled_features_slice` returns the same set as
    /// `enabled_features` (the slow Set<Feature>-allocating
    /// back-compat accessor).
    #[test]
    fn profile_enabled_features_slice_matches_set() {
        for &p in Profile::ALL {
            let slice_set: Set<Feature> = p.enabled_features_slice().iter().copied().collect();
            let alloc_set = p.enabled_features();
            assert_eq!(slice_set, alloc_set,
                "{:?}: slice and Set accessors disagree", p);
        }
    }

    /// Every profile's name is unique and non-empty.
    #[test]
    fn profile_names_unique_and_non_empty() {
        let mut seen: Vec<&'static str> = Vec::with_capacity(Profile::ALL.len());
        for &p in Profile::ALL {
            let n = p.name();
            assert!(!n.is_empty(), "{:?}: empty name", p);
            assert!(!seen.contains(&n), "duplicate name {:?}", n);
            seen.push(n);
        }
    }

    /// Pin canonical feature counts per profile.  Bumping these
    /// asserts is the explicit signal that a feature was added /
    /// removed from a profile.
    #[test]
    fn profile_feature_counts_pinned() {
        assert_eq!(Profile::Application.enabled_features_slice().len(), 8,
            "Application: 5 base + 3 safety");
        assert_eq!(Profile::Systems.enabled_features_slice().len(), 11,
            "Systems: 5 base + 3 safety + 3 systems");
        assert_eq!(Profile::Research.enabled_features_slice().len(), 11,
            "Research: 5 base + 3 safety + 3 research");
    }
}
