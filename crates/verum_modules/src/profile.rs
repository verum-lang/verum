//! Language profile system for module-level control.
//!

//! Implements the profile system from Section 13 of the specification:
//! - Module-level profile declarations (@profile attribute)
//! - Profile inheritance from parent modules
//! - Selective feature enabling (@feature attribute)
//! - Profile compatibility validation
//! - Profile-aware module resolution
//!

//! # Overview
//!

//! Verum supports three language profiles:
//! - **Application**: Safe, productive, async-first (default)
//! - **Systems**: Unsafe allowed, manual memory management
//! - **Research**: Formal verification, dependent types, proofs
//!

//! # Profile Hierarchy
//!

//! ```text
//! Research (most permissive)
//!  ↓
//! Systems (intermediate)
//!  ↓
//! Application (most restrictive, default)
//! ```
//!

//! Research profile can access everything.
//! Systems profile can access Systems and Application modules.
//! Application profile can only access Application modules.
//!

//! Profile hierarchy (most permissive to most restrictive):
//! Research > Systems > Application (default)

use crate::ModuleInfo;
use crate::error::{ModuleError, ModuleResult};
use crate::path::ModulePath;
use std::collections::HashSet;
use verum_ast::Span;
use verum_common::{List, Map, Maybe, Text};

/// Language profile - determines what features are available in a module.
///

/// Determines what features are available in a module. Declared with
/// @profile(application|systems|research). Default is Application.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    Default,
)]
pub enum LanguageProfile {
    /// Application profile: Safe, productive, async-first (default).
    ///

    /// Features:
    /// - Full async/await support
    /// - Context system (using/provide)
    /// - Safe references only (&T, &checked T)
    /// - Automatic memory management
    #[default]
    Application,

    /// Systems profile: Maximum control for low-level programming.
    ///

    /// Features (in addition to Application):
    /// - Unsafe blocks and raw pointers
    /// - Manual memory management
    /// - Inline assembly
    /// - Custom allocators
    /// - FFI without wrappers
    Systems,

    /// Research profile: Formal verification and dependent types.
    ///

    /// Features (in addition to Systems):
    /// - Dependent types (Pi/Sigma)
    /// - Formal proofs (@verify)
    /// - Refinement type inference
    /// - SMT solver integration
    /// - Contract verification
    Research,
}

/// Per-variant projection for [`LanguageProfile`].
///
/// `permissiveness_rank` encodes the profile hierarchy as a dense
/// monotone scale: Application=0 (most restrictive, fewest features
/// available), Systems=1, Research=2 (least restrictive, all features
/// available). Both directional rules — "can access target" and
/// "can be child of parent" — collapse to single rank comparisons:
///
///   * `self.can_access(target)`        = `self.rank >= target.rank`
///   * `self.can_be_child_of(parent)`   = `self.rank <= parent.rank`
///
/// This makes profile transitivity / antisymmetry / well-foundedness
/// structurally guaranteed instead of riding two parallel 9-arm
/// match tables.
///
/// `unavailable_features` returns a static slice of human-readable
/// labels (cached as `&'static [&'static str]`) instead of allocating
/// a fresh `List<Text>` on every call. `Application` listed first;
/// `Research` is always empty.
#[derive(Debug, Clone, Copy)]
pub struct LanguageProfileMeta {
    pub name: &'static str,
    pub permissiveness_rank: u8,
    pub unavailable_features_static: &'static [&'static str],
}

impl LanguageProfile {
    pub const ALL: &'static [Self] = &[Self::Application, Self::Systems, Self::Research];

    pub const fn meta(self) -> LanguageProfileMeta {
        match self {
            Self::Application => LanguageProfileMeta {
                name: "application",
                permissiveness_rank: 0,
                unavailable_features_static: &[
                    "Unsafe pointer operations",
                    "Manual memory management",
                    "Inline assembly",
                    "Raw FFI bindings",
                ],
            },
            Self::Systems => LanguageProfileMeta {
                name: "systems",
                permissiveness_rank: 1,
                unavailable_features_static: &[
                    "Dependent types (Pi/Sigma)",
                    "Formal proof verification",
                    "SMT-backed contracts",
                ],
            },
            Self::Research => LanguageProfileMeta {
                name: "research",
                permissiveness_rank: 2,
                unavailable_features_static: &[],
            },
        }
    }

    /// Permissiveness rank: Application=0 (most restrictive),
    /// Research=2 (least restrictive). The rank is dense and
    /// strictly monotone in declaration order.
    #[inline]
    pub const fn permissiveness_rank(&self) -> u8 {
        self.meta().permissiveness_rank
    }

    /// Check if this profile can access modules with the target
    /// profile. Profile hierarchy: Research ≥ Systems ≥ Application.
    /// A profile may access targets at its own permissiveness level
    /// or below (`self.rank >= target.rank`).
    #[inline]
    pub const fn can_access(&self, target: LanguageProfile) -> bool {
        self.permissiveness_rank() >= target.permissiveness_rank()
    }

    /// Check if a child module can have this profile given the
    /// parent profile. Child modules can be the same or MORE
    /// restrictive than the parent — never less restrictive
    /// (`self.rank <= parent.rank`).
    #[inline]
    pub const fn can_be_child_of(&self, parent: LanguageProfile) -> bool {
        self.permissiveness_rank() <= parent.permissiveness_rank()
    }

    /// Parse profile from string (case-insensitive).
    pub fn from_str(s: &str) -> Option<Self> {
        let lowered = s.to_lowercase();
        for v in Self::ALL {
            if v.meta().name == lowered.as_str() {
                return Some(*v);
            }
        }
        None
    }

    /// Get the display name of this profile.
    #[inline]
    pub const fn name(&self) -> &'static str {
        self.meta().name
    }

    /// Convenience synonym for `name()` matching the meta() series
    /// idiom across the codebase. Both methods return identical
    /// strings.
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        self.meta().name
    }

    /// Static label slice of features unavailable from this profile
    /// (zero-alloc — closes a perf defect: the legacy
    /// `unavailable_features` allocated a fresh `List<Text>` of 0/3/4
    /// entries on every call).
    #[inline]
    pub const fn unavailable_features_static(&self) -> &'static [&'static str] {
        self.meta().unavailable_features_static
    }

    /// Owning version preserved for source compatibility with the
    /// legacy signature: `_target` is intentionally unused (the
    /// answer depends only on `self` — a feature unavailable from
    /// `Application` is unavailable to every target it can access).
    pub fn unavailable_features(&self, _target: LanguageProfile) -> List<Text> {
        let mut features = List::new();
        for label in self.unavailable_features_static() {
            features.push(Text::from(*label));
        }
        features
    }
}

impl std::fmt::Display for LanguageProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Optional features that can be enabled within a profile.
///

/// Features are independent of profiles and can be selectively enabled
/// using `@feature(enable: [...])` attribute.
///

/// Features are independent of profiles and can be selectively enabled
/// using `@feature(enable: [...])`. Features must be compatible with the
/// base profile and are additive (don't remove profile capabilities).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ModuleFeature {
    /// Allow unsafe blocks in this module
    Unsafe,
    /// Allow inline assembly
    InlineAsm,
    /// Allow custom allocators
    CustomAllocator,
    /// Enable dependent types
    DependentTypes,
    /// Enable formal verification
    FormalVerification,
    /// Enable SMT solver integration
    SmtSolver,
    /// Enable GPU compute
    GpuCompute,
    /// Enable SIMD intrinsics
    Simd,
    /// Enable FFI without wrappers
    RawFfi,
}

/// Per-variant projection for [`ModuleFeature`].
///
/// `name` is the canonical snake_case form returned by `name()` /
/// `as_str()`. `aliases` carries the legacy CamelCase / shortened
/// parse aliases (e.g. `"InlineAsm"`, `"smt"`, `"gpu"`).
/// `minimum_profile` is the least permissive profile under which this
/// feature is available — every more-permissive profile inherits it
/// automatically. `is_compatible_with(profile)` collapses to
/// `profile.permissiveness_rank() >= self.minimum_profile().permissiveness_rank()`,
/// so adding a new feature only requires picking its `minimum_profile`
/// once.
#[derive(Debug, Clone, Copy)]
pub struct ModuleFeatureMeta {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub minimum_profile: LanguageProfile,
}

impl ModuleFeature {
    pub const ALL: &'static [Self] = &[
        Self::Unsafe,
        Self::InlineAsm,
        Self::CustomAllocator,
        Self::DependentTypes,
        Self::FormalVerification,
        Self::SmtSolver,
        Self::GpuCompute,
        Self::Simd,
        Self::RawFfi,
    ];

    pub const fn meta(self) -> ModuleFeatureMeta {
        match self {
            Self::Unsafe => ModuleFeatureMeta {
                name: "unsafe",
                aliases: &[],
                minimum_profile: LanguageProfile::Systems,
            },
            Self::InlineAsm => ModuleFeatureMeta {
                name: "inline_asm",
                aliases: &["inlineasm"],
                minimum_profile: LanguageProfile::Systems,
            },
            Self::CustomAllocator => ModuleFeatureMeta {
                name: "custom_allocator",
                aliases: &["customallocator"],
                minimum_profile: LanguageProfile::Systems,
            },
            Self::DependentTypes => ModuleFeatureMeta {
                name: "dependent_types",
                aliases: &["dependenttypes"],
                minimum_profile: LanguageProfile::Research,
            },
            Self::FormalVerification => ModuleFeatureMeta {
                name: "formal_verification",
                aliases: &["formalverification"],
                minimum_profile: LanguageProfile::Research,
            },
            Self::SmtSolver => ModuleFeatureMeta {
                name: "smt_solver",
                aliases: &["smtsolver", "smt"],
                minimum_profile: LanguageProfile::Research,
            },
            Self::GpuCompute => ModuleFeatureMeta {
                name: "gpu_compute",
                aliases: &["gpucompute", "gpu"],
                minimum_profile: LanguageProfile::Application,
            },
            Self::Simd => ModuleFeatureMeta {
                name: "simd",
                aliases: &[],
                minimum_profile: LanguageProfile::Application,
            },
            Self::RawFfi => ModuleFeatureMeta {
                name: "raw_ffi",
                aliases: &["rawffi"],
                minimum_profile: LanguageProfile::Systems,
            },
        }
    }

    /// Parse feature from string (case-insensitive — accepts the
    /// canonical snake_case name plus any legacy alias listed in
    /// `meta().aliases`).
    pub fn from_str(s: &str) -> Option<Self> {
        let lowered = s.to_lowercase();
        for v in Self::ALL {
            let m = v.meta();
            if m.name == lowered.as_str() {
                return Some(*v);
            }
            for alias in m.aliases {
                if *alias == lowered.as_str() {
                    return Some(*v);
                }
            }
        }
        None
    }

    /// Get the canonical snake_case name of this feature.
    #[inline]
    pub const fn name(&self) -> &'static str {
        self.meta().name
    }

    /// Convenience synonym for `name()` matching the meta() series
    /// idiom.
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        self.meta().name
    }

    /// Get the minimum required profile for this feature.
    #[inline]
    pub const fn minimum_profile(&self) -> LanguageProfile {
        self.meta().minimum_profile
    }

    /// Check if this feature is compatible with the given profile.
    /// True when `profile` is at least as permissive as
    /// `self.minimum_profile()` — i.e. their permissiveness ranks
    /// are ordered correctly. Single rank comparison instead of two
    /// parallel matches!.
    #[inline]
    pub const fn is_compatible_with(&self, profile: LanguageProfile) -> bool {
        profile.permissiveness_rank() >= self.minimum_profile().permissiveness_rank()
    }
}

impl std::fmt::Display for ModuleFeature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Profile configuration for a module.
///

/// Stores the declared profile and any enabled features.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModuleProfile {
    /// The declared language profile
    pub profile: LanguageProfile,
    /// Explicitly enabled features
    pub enabled_features: HashSet<ModuleFeature>,
    /// Whether this profile was explicitly declared or inherited
    pub explicit: bool,
    /// Span of the @profile attribute (if explicit)
    pub span: Option<Span>,
}

impl ModuleProfile {
    /// Create a new module profile with explicit declaration.
    pub fn new(profile: LanguageProfile) -> Self {
        Self {
            profile,
            enabled_features: HashSet::new(),
            explicit: true,
            span: None,
        }
    }

    /// Create an inherited profile from parent.
    pub fn inherited(parent_profile: LanguageProfile) -> Self {
        Self {
            profile: parent_profile,
            enabled_features: HashSet::new(),
            explicit: false,
            span: None,
        }
    }

    /// Create default Application profile.
    pub fn default_application() -> Self {
        Self {
            profile: LanguageProfile::Application,
            enabled_features: HashSet::new(),
            explicit: false,
            span: None,
        }
    }

    /// Enable a feature in this profile.
    pub fn enable_feature(&mut self, feature: ModuleFeature) -> ModuleResult<()> {
        if !feature.is_compatible_with(self.profile) {
            return Err(ModuleError::Other {
                message: Text::from(format!(
                    "Feature '{}' requires {} profile, but module has {} profile",
                    feature,
                    feature.minimum_profile(),
                    self.profile
                )),
                span: self.span,
            });
        }
        self.enabled_features.insert(feature);
        Ok(())
    }

    /// Check if a feature is enabled.
    pub fn has_feature(&self, feature: ModuleFeature) -> bool {
        // Feature is available if:
        // 1. Explicitly enabled via @feature
        // 2. OR profile inherently supports it (e.g., Systems supports Unsafe)
        self.enabled_features.contains(&feature) || self.profile_supports_feature(feature)
    }

    /// Check if the profile inherently supports a feature.
    fn profile_supports_feature(&self, feature: ModuleFeature) -> bool {
        match self.profile {
            LanguageProfile::Research => true, // Research supports all features
            LanguageProfile::Systems => matches!(
                feature,
                ModuleFeature::Unsafe
                    | ModuleFeature::InlineAsm
                    | ModuleFeature::CustomAllocator
                    | ModuleFeature::RawFfi
                    | ModuleFeature::GpuCompute
                    | ModuleFeature::Simd
            ),
            LanguageProfile::Application => {
                matches!(feature, ModuleFeature::GpuCompute | ModuleFeature::Simd)
            }
        }
    }

    /// Set the span for error reporting.
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
}

impl Default for ModuleProfile {
    fn default() -> Self {
        Self::default_application()
    }
}

/// Profile checker - validates profile compatibility across modules.
///

/// Implements profile validation from Section 13.5-13.6:
/// - Validates module profile declarations
/// - Checks profile inheritance rules
/// - Validates import compatibility
/// - Generates detailed error messages
///

/// Validates profile compatibility across modules using the profile
/// resolution algorithm: (1) check module declaration, (2) check
/// inheritance chain, (3) check feature gates, (4) validate imports,
/// (5) error if incompatible with detailed resolution suggestions.
#[derive(Debug)]
pub struct ProfileChecker {
    /// Current compilation profile (from command line or verum.toml)
    compilation_profile: LanguageProfile,
    /// Module profiles cache
    module_profiles: Map<ModulePath, ModuleProfile>,
}

impl ProfileChecker {
    /// Create a new profile checker with the given compilation profile.
    pub fn new(compilation_profile: LanguageProfile) -> Self {
        Self {
            compilation_profile,
            module_profiles: Map::new(),
        }
    }

    /// Register a module's profile.
    pub fn register_profile(&mut self, path: ModulePath, profile: ModuleProfile) {
        self.module_profiles.insert(path, profile);
    }

    /// Get a module's profile.
    pub fn get_profile(&self, path: &ModulePath) -> Maybe<&ModuleProfile> {
        match self.module_profiles.get(path) {
            Some(p) => Maybe::Some(p),
            None => Maybe::None,
        }
    }

    /// Extract profile from module's attributes.
    ///

    /// Parses @profile() and @feature() attributes from the module AST.
    /// Attributes are stored in each Item's attributes field, not as separate items.
    pub fn extract_profile(
        &self,
        module: &ModuleInfo,
        parent_profile: Option<LanguageProfile>,
    ) -> ModuleResult<ModuleProfile> {
        let mut profile = parent_profile
            .map(ModuleProfile::inherited)
            .unwrap_or_else(ModuleProfile::default_application);

        // Look for @profile and @feature attributes in module's items
        for item in &module.ast.items {
            for attr in &item.attributes {
                // Check for @profile attribute
                if attr.name.as_str() == "profile" {
                    if let Maybe::Some(ref args) = attr.args
                        && let Some(first_arg) = args.first()
                        && let Some(profile_name) = self.extract_expr_string(first_arg)
                        && let Some(lang_profile) = LanguageProfile::from_str(&profile_name)
                    {
                        // Validate profile inheritance
                        if let Some(parent) = parent_profile
                            && !lang_profile.can_be_child_of(parent)
                        {
                            return Err(ModuleError::ProfileIncompatible {
                                module_path: module.path.clone(),
                                required_profile: Text::from(parent.name()),
                                current_profile: Text::from(lang_profile.name()),
                                span: Some(item.span),
                            });
                        }
                        profile.profile = lang_profile;
                        profile.explicit = true;
                        profile.span = Some(attr.span);
                    }
                }
                // Check for @feature attribute
                else if attr.name.as_str() == "feature" {
                    self.parse_feature_attr(attr, &mut profile)?;
                }
            }
        }

        // Also check module-level attributes (on the Module itself)
        for attr in &module.ast.attributes {
            if attr.name.as_str() == "profile" {
                if let Maybe::Some(ref args) = attr.args
                    && let Some(first_arg) = args.first()
                    && let Some(profile_name) = self.extract_expr_string(first_arg)
                    && let Some(lang_profile) = LanguageProfile::from_str(&profile_name)
                {
                    if let Some(parent) = parent_profile
                        && !lang_profile.can_be_child_of(parent)
                    {
                        return Err(ModuleError::ProfileIncompatible {
                            module_path: module.path.clone(),
                            required_profile: Text::from(parent.name()),
                            current_profile: Text::from(lang_profile.name()),
                            span: Some(attr.span),
                        });
                    }
                    profile.profile = lang_profile;
                    profile.explicit = true;
                    profile.span = Some(attr.span);
                }
            } else if attr.name.as_str() == "feature" {
                self.parse_feature_attr(attr, &mut profile)?;
            }
        }

        Ok(profile)
    }

    /// Parse @feature(enable: [...]) attribute.
    fn parse_feature_attr(
        &self,
        attr: &verum_ast::Attribute,
        profile: &mut ModuleProfile,
    ) -> ModuleResult<()> {
        if let Maybe::Some(ref args) = attr.args {
            for arg in args {
                // Look for array arguments with feature names
                if let Some(features) = self.extract_expr_string_list(arg) {
                    for feature_name in features {
                        if let Some(feature) = ModuleFeature::from_str(&feature_name) {
                            profile.enable_feature(feature)?;
                        } else {
                            return Err(ModuleError::Other {
                                message: Text::from(format!("Unknown feature: '{}'", feature_name)),
                                span: Some(attr.span),
                            });
                        }
                    }
                } else if let Some(feature_name) = self.extract_expr_string(arg) {
                    // Single feature name
                    if let Some(feature) = ModuleFeature::from_str(&feature_name) {
                        profile.enable_feature(feature)?;
                    } else {
                        return Err(ModuleError::Other {
                            message: Text::from(format!("Unknown feature: '{}'", feature_name)),
                            span: Some(attr.span),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Extract string value from an expression.
    fn extract_expr_string(&self, expr: &verum_ast::expr::Expr) -> Option<String> {
        use verum_ast::LiteralKind;
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Text(s) => Some(s.as_str().to_string()),
                _ => None,
            },
            ExprKind::Path(path) => {
                // Handle identifier-style arguments like @profile(application)
                if path.segments.len() == 1
                    && let verum_ast::PathSegment::Name(ident) = &path.segments[0]
                {
                    return Some(ident.name.to_string());
                }
                None
            }
            _ => None,
        }
    }

    /// Extract list of strings from an array expression.
    fn extract_expr_string_list(&self, expr: &verum_ast::expr::Expr) -> Option<List<String>> {
        use verum_ast::ArrayExpr;
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            ExprKind::Array(array_expr) => match array_expr {
                ArrayExpr::List(elements) => {
                    let strings: List<String> = elements
                        .iter()
                        .filter_map(|e| self.extract_expr_string(e))
                        .collect();
                    if strings.is_empty() {
                        None
                    } else {
                        Some(strings)
                    }
                }
                ArrayExpr::Repeat { .. } => None, // Repeat arrays don't make sense for feature lists
            },
            ExprKind::Tuple(elements) => {
                let strings: List<String> = elements
                    .iter()
                    .filter_map(|e| self.extract_expr_string(e))
                    .collect();
                if strings.is_empty() {
                    None
                } else {
                    Some(strings)
                }
            }
            _ => None,
        }
    }

    /// Check if importing from target module is allowed.
    ///

    /// Validates that the current module's profile can access the target module.
    ///

    /// Profile compatibility follows the hierarchy:
    /// Research can access everything, Systems can access Systems+Application,
    /// Application can only access Application.
    pub fn check_import(
        &self,
        from_module: &ModulePath,
        target_module: &ModulePath,
        span: Option<Span>,
    ) -> ModuleResult<()> {
        let from_profile = self.get_effective_profile(from_module);
        let target_profile = self.get_effective_profile(target_module);

        if !from_profile.can_access(target_profile) {
            let _unavailable = from_profile.unavailable_features(target_profile);

            return Err(ModuleError::ProfileIncompatible {
                module_path: target_module.clone(),
                required_profile: Text::from(target_profile.name()),
                current_profile: Text::from(from_profile.name()),
                span,
            });
        }

        Ok(())
    }

    /// Get the effective profile for a module.
    ///

    /// Returns the module's declared profile, inherited profile, or compilation default.
    pub fn get_effective_profile(&self, path: &ModulePath) -> LanguageProfile {
        if let Maybe::Some(profile) = self.get_profile(path) {
            return profile.profile;
        }

        // Try to inherit from parent
        if let Maybe::Some(parent) = path.parent() {
            return self.get_effective_profile(&parent);
        }

        // Fall back to compilation profile
        self.compilation_profile
    }

    /// Check if an unsafe block is allowed in the module.
    pub fn is_unsafe_allowed(&self, module_path: &ModulePath) -> bool {
        if let Maybe::Some(profile) = self.get_profile(module_path) {
            return profile.has_feature(ModuleFeature::Unsafe);
        }

        // Check if compilation profile allows unsafe
        matches!(
            self.compilation_profile,
            LanguageProfile::Systems | LanguageProfile::Research
        )
    }

    /// Validate all registered module profiles.
    ///

    /// Checks:
    /// 1. Profile inheritance is valid (child not less restrictive than parent)
    /// 2. All enabled features are compatible with profile
    /// 3. All imports are compatible with profile
    pub fn validate_all(&self) -> ModuleResult<()> {
        for (path, profile) in self.module_profiles.iter() {
            // Check parent compatibility
            if let Maybe::Some(parent_path) = path.parent() {
                let parent_profile = self.get_effective_profile(&parent_path);
                if !profile.profile.can_be_child_of(parent_profile) {
                    return Err(ModuleError::ProfileIncompatible {
                        module_path: path.clone(),
                        required_profile: Text::from(format!(
                            "{} or more restrictive",
                            parent_profile
                        )),
                        current_profile: Text::from(profile.profile.name()),
                        span: profile.span,
                    });
                }
            }

            // Validate features are compatible
            for feature in &profile.enabled_features {
                if !feature.is_compatible_with(profile.profile) {
                    return Err(ModuleError::Other {
                        message: Text::from(format!(
                            "Feature '{}' in module '{}' requires {} profile, but has {} profile",
                            feature,
                            path,
                            feature.minimum_profile(),
                            profile.profile
                        )),
                        span: profile.span,
                    });
                }
            }
        }

        Ok(())
    }

    /// Get the compilation profile.
    pub fn compilation_profile(&self) -> LanguageProfile {
        self.compilation_profile
    }

    /// Generate detailed error message for profile incompatibility.
    ///

    /// Generates detailed error including: required profile, current profile,
    /// unavailable features list, and resolution suggestions (change verum.toml,
    /// move module, or use compatible public APIs).
    pub fn format_profile_error(
        &self,
        from_module: &ModulePath,
        target_module: &ModulePath,
    ) -> Text {
        let from_profile = self.get_effective_profile(from_module);
        let target_profile = self.get_effective_profile(target_module);
        let unavailable = from_profile.unavailable_features(target_profile);

        let mut msg = format!(
            "ERROR: Module incompatible with current profile\n\n\
             Module '{}' requires one of: [{}]\n\
             Current compilation profile: {}\n",
            target_module, target_profile, from_profile
        );

        if !unavailable.is_empty() {
            msg.push_str("\nThe following features are unavailable:\n");
            for feature in unavailable {
                msg.push_str(&format!("  - {}\n", feature));
            }
        }

        msg.push_str(&format!(
            "\nHelp: Either:\n\
             1. Change verum.toml profile = \"{}\"\n\
             2. Move this module to a {}-only submodule\n\
             3. Use public APIs from {}-compatible modules\n",
            target_profile, target_profile, from_profile
        ));

        Text::from(msg)
    }
}

impl Default for ProfileChecker {
    fn default() -> Self {
        Self::new(LanguageProfile::Application)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_hierarchy() {
        // Research can access everything
        assert!(LanguageProfile::Research.can_access(LanguageProfile::Research));
        assert!(LanguageProfile::Research.can_access(LanguageProfile::Systems));
        assert!(LanguageProfile::Research.can_access(LanguageProfile::Application));

        // Systems can access Systems and Application
        assert!(!LanguageProfile::Systems.can_access(LanguageProfile::Research));
        assert!(LanguageProfile::Systems.can_access(LanguageProfile::Systems));
        assert!(LanguageProfile::Systems.can_access(LanguageProfile::Application));

        // Application can only access Application
        assert!(!LanguageProfile::Application.can_access(LanguageProfile::Research));
        assert!(!LanguageProfile::Application.can_access(LanguageProfile::Systems));
        assert!(LanguageProfile::Application.can_access(LanguageProfile::Application));
    }

    #[test]
    fn test_profile_inheritance() {
        // Research parent allows any child
        assert!(LanguageProfile::Application.can_be_child_of(LanguageProfile::Research));
        assert!(LanguageProfile::Systems.can_be_child_of(LanguageProfile::Research));
        assert!(LanguageProfile::Research.can_be_child_of(LanguageProfile::Research));

        // Systems parent allows Systems or Application
        assert!(LanguageProfile::Application.can_be_child_of(LanguageProfile::Systems));
        assert!(LanguageProfile::Systems.can_be_child_of(LanguageProfile::Systems));
        assert!(!LanguageProfile::Research.can_be_child_of(LanguageProfile::Systems));

        // Application parent only allows Application
        assert!(LanguageProfile::Application.can_be_child_of(LanguageProfile::Application));
        assert!(!LanguageProfile::Systems.can_be_child_of(LanguageProfile::Application));
        assert!(!LanguageProfile::Research.can_be_child_of(LanguageProfile::Application));
    }

    #[test]
    fn test_feature_compatibility() {
        // Unsafe requires Systems
        assert!(!ModuleFeature::Unsafe.is_compatible_with(LanguageProfile::Application));
        assert!(ModuleFeature::Unsafe.is_compatible_with(LanguageProfile::Systems));
        assert!(ModuleFeature::Unsafe.is_compatible_with(LanguageProfile::Research));

        // DependentTypes requires Research
        assert!(!ModuleFeature::DependentTypes.is_compatible_with(LanguageProfile::Application));
        assert!(!ModuleFeature::DependentTypes.is_compatible_with(LanguageProfile::Systems));
        assert!(ModuleFeature::DependentTypes.is_compatible_with(LanguageProfile::Research));

        // SIMD works everywhere
        assert!(ModuleFeature::Simd.is_compatible_with(LanguageProfile::Application));
        assert!(ModuleFeature::Simd.is_compatible_with(LanguageProfile::Systems));
        assert!(ModuleFeature::Simd.is_compatible_with(LanguageProfile::Research));
    }

    #[test]
    fn test_module_profile_features() {
        let mut profile = ModuleProfile::new(LanguageProfile::Systems);

        // Systems profile inherently supports Unsafe
        assert!(profile.has_feature(ModuleFeature::Unsafe));
        assert!(profile.has_feature(ModuleFeature::Simd));

        // Systems profile does NOT support DependentTypes
        assert!(!profile.has_feature(ModuleFeature::DependentTypes));

        // Cannot enable incompatible feature
        assert!(
            profile
                .enable_feature(ModuleFeature::DependentTypes)
                .is_err()
        );
    }

    #[test]
    fn test_profile_checker_import_validation() {
        let mut checker = ProfileChecker::new(LanguageProfile::Application);

        let app_module = ModulePath::from_str("app.main");
        let sys_module = ModulePath::from_str("sys.memory");

        checker.register_profile(
            app_module.clone(),
            ModuleProfile::new(LanguageProfile::Application),
        );
        checker.register_profile(
            sys_module.clone(),
            ModuleProfile::new(LanguageProfile::Systems),
        );

        // App cannot import from Systems
        assert!(
            checker
                .check_import(&app_module, &sys_module, None)
                .is_err()
        );

        // Systems can import from App (hypothetically if we change the compilation profile)
        let mut sys_checker = ProfileChecker::new(LanguageProfile::Systems);
        sys_checker.register_profile(
            app_module.clone(),
            ModuleProfile::new(LanguageProfile::Application),
        );
        sys_checker.register_profile(
            sys_module.clone(),
            ModuleProfile::new(LanguageProfile::Systems),
        );

        assert!(
            sys_checker
                .check_import(&sys_module, &app_module, None)
                .is_ok()
        );
    }

    #[test]
    fn test_profile_from_str() {
        assert_eq!(
            LanguageProfile::from_str("application"),
            Some(LanguageProfile::Application)
        );
        assert_eq!(
            LanguageProfile::from_str("APPLICATION"),
            Some(LanguageProfile::Application)
        );
        assert_eq!(
            LanguageProfile::from_str("systems"),
            Some(LanguageProfile::Systems)
        );
        assert_eq!(
            LanguageProfile::from_str("research"),
            Some(LanguageProfile::Research)
        );
        assert_eq!(LanguageProfile::from_str("invalid"), None);
    }

    // -------------------------------------------------------------------
    // meta() consolidation drift pins. Each pin closes a failure mode
    // exposed by the rank-based collapse of can_access /
    // can_be_child_of / is_compatible_with into single comparisons.
    // -------------------------------------------------------------------

    #[test]
    fn meta_pin_language_profile_round_trip_unique_and_dense_rank() {
        // Round-trip via the canonical name + ALL slice.
        for v in LanguageProfile::ALL {
            let s = v.name();
            assert_eq!(
                LanguageProfile::from_str(s),
                Some(*v),
                "LanguageProfile::{:?}: name '{}' must round-trip",
                v,
                s
            );
            assert_eq!(v.as_str(), v.name(), "as_str ↔ name agreement");
        }
        // Names are unique across variants.
        let names: Vec<&str> =
            LanguageProfile::ALL.iter().map(|v| v.name()).collect();
        let mut dedup = names.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(dedup.len(), names.len(), "duplicate canonical name");
        // Permissiveness ranks are dense 0..=2 in declaration order.
        for (i, v) in LanguageProfile::ALL.iter().enumerate() {
            assert_eq!(
                v.permissiveness_rank() as usize,
                i,
                "LanguageProfile::{:?}: rank drift at slot {}",
                v,
                i
            );
        }
        // Strict monotonicity.
        for w in LanguageProfile::ALL.windows(2) {
            assert!(
                w[0].permissiveness_rank() < w[1].permissiveness_rank(),
                "rank monotonicity violated: {:?} -> {:?}",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    fn meta_pin_language_profile_can_access_table_full() {
        // Reference table — exhaustive; pinned exactly to the legacy
        // 9-arm match. Rows = self, cols = target.
        //                  Application  Systems  Research
        //   Application       true       false    false
        //   Systems           true       true     false
        //   Research          true       true     true
        let table: [[bool; 3]; 3] = [
            [true, false, false],
            [true, true, false],
            [true, true, true],
        ];
        for (i, a) in LanguageProfile::ALL.iter().enumerate() {
            for (j, b) in LanguageProfile::ALL.iter().enumerate() {
                assert_eq!(
                    a.can_access(*b),
                    table[i][j],
                    "can_access drift: {:?} -> {:?}",
                    a,
                    b
                );
            }
        }
        // Reflexivity: every profile can access itself.
        for v in LanguageProfile::ALL {
            assert!(v.can_access(*v));
        }
        // can_be_child_of is the dual: can_be_child_of(parent) iff
        // self.rank <= parent.rank, which is parent.can_access(self).
        for a in LanguageProfile::ALL {
            for b in LanguageProfile::ALL {
                assert_eq!(
                    a.can_be_child_of(*b),
                    b.can_access(*a),
                    "duality: a.can_be_child_of(b) == b.can_access(a) — \
                     a={:?}, b={:?}",
                    a,
                    b
                );
            }
        }
    }

    #[test]
    fn meta_pin_module_feature_round_trip_unique_and_min_profile_partition() {
        // Round-trip + uniqueness + alias coverage.
        for v in ModuleFeature::ALL {
            let s = v.name();
            assert_eq!(
                ModuleFeature::from_str(s),
                Some(*v),
                "ModuleFeature::{:?}: name '{}' round-trip",
                v,
                s
            );
            assert_eq!(v.as_str(), v.name());
            for alias in v.meta().aliases {
                assert_eq!(
                    ModuleFeature::from_str(alias),
                    Some(*v),
                    "ModuleFeature::{:?}: alias '{}' must parse",
                    v,
                    alias
                );
            }
        }
        // Case-insensitivity preserved.
        assert_eq!(
            ModuleFeature::from_str("UNSAFE"),
            Some(ModuleFeature::Unsafe)
        );
        assert_eq!(
            ModuleFeature::from_str("Smt"),
            Some(ModuleFeature::SmtSolver)
        );
        // is_compatible_with classification — every variant agrees
        // with the rank comparison and with the legacy partition.
        for f in ModuleFeature::ALL {
            for p in LanguageProfile::ALL {
                let expected = p.permissiveness_rank()
                    >= f.minimum_profile().permissiveness_rank();
                assert_eq!(
                    f.is_compatible_with(*p),
                    expected,
                    "is_compatible_with drift: {:?} on {:?}",
                    f,
                    p
                );
            }
        }
        // Bucket counts pin: exactly 4 require Systems, exactly 3
        // require Research, exactly 2 work in Application.
        let app_features = ModuleFeature::ALL
            .iter()
            .filter(|f| f.minimum_profile() == LanguageProfile::Application)
            .count();
        let sys_features = ModuleFeature::ALL
            .iter()
            .filter(|f| f.minimum_profile() == LanguageProfile::Systems)
            .count();
        let res_features = ModuleFeature::ALL
            .iter()
            .filter(|f| f.minimum_profile() == LanguageProfile::Research)
            .count();
        assert_eq!(app_features, 2, "Application-min: GpuCompute, Simd");
        assert_eq!(
            sys_features, 4,
            "Systems-min: Unsafe, InlineAsm, CustomAllocator, RawFfi"
        );
        assert_eq!(
            res_features, 3,
            "Research-min: DependentTypes, FormalVerification, SmtSolver"
        );
        assert_eq!(app_features + sys_features + res_features, 9);
    }

    #[test]
    fn meta_pin_unavailable_features_zero_alloc_static_slice() {
        // Static-slice access (zero-alloc fast path).
        assert_eq!(
            LanguageProfile::Application.unavailable_features_static().len(),
            4
        );
        assert_eq!(
            LanguageProfile::Systems.unavailable_features_static().len(),
            3
        );
        assert!(
            LanguageProfile::Research
                .unavailable_features_static()
                .is_empty()
        );
        // Owning version preserves the legacy contract: the labels
        // produced by the static slice match those returned by the
        // owning method (target argument is intentionally inert —
        // the answer depends only on `self`).
        for from in LanguageProfile::ALL {
            for to in LanguageProfile::ALL {
                let owned = from.unavailable_features(*to);
                let static_slice = from.unavailable_features_static();
                assert_eq!(
                    owned.len(),
                    static_slice.len(),
                    "unavailable_features count mismatch from={:?} to={:?}",
                    from,
                    to
                );
                for (i, label) in static_slice.iter().enumerate() {
                    assert_eq!(
                        owned.get(i).map(|t| t.as_str()),
                        Some(*label),
                        "unavailable_features label mismatch at {}",
                        i
                    );
                }
            }
        }
    }

    #[test]
    fn test_feature_from_str() {
        assert_eq!(
            ModuleFeature::from_str("unsafe"),
            Some(ModuleFeature::Unsafe)
        );
        assert_eq!(
            ModuleFeature::from_str("inline_asm"),
            Some(ModuleFeature::InlineAsm)
        );
        assert_eq!(
            ModuleFeature::from_str("inlineasm"),
            Some(ModuleFeature::InlineAsm)
        );
        assert_eq!(
            ModuleFeature::from_str("smt"),
            Some(ModuleFeature::SmtSolver)
        );
        assert_eq!(
            ModuleFeature::from_str("gpu"),
            Some(ModuleFeature::GpuCompute)
        );
        assert_eq!(ModuleFeature::from_str("invalid"), None);
    }

    #[test]
    fn test_profile_checker_validate_all() {
        let mut checker = ProfileChecker::new(LanguageProfile::Systems);

        let parent = ModulePath::from_str("parent");
        let child = ModulePath::from_str("parent.child");

        // Valid: Systems parent with Application child
        checker.register_profile(parent.clone(), ModuleProfile::new(LanguageProfile::Systems));
        checker.register_profile(
            child.clone(),
            ModuleProfile::new(LanguageProfile::Application),
        );

        assert!(checker.validate_all().is_ok());
    }

    #[test]
    fn test_profile_checker_invalid_inheritance() {
        let mut checker = ProfileChecker::new(LanguageProfile::Application);

        let parent = ModulePath::from_str("parent");
        let child = ModulePath::from_str("parent.child");

        // Invalid: Application parent with Systems child
        checker.register_profile(
            parent.clone(),
            ModuleProfile::new(LanguageProfile::Application),
        );
        checker.register_profile(child.clone(), ModuleProfile::new(LanguageProfile::Systems));

        assert!(checker.validate_all().is_err());
    }
}
