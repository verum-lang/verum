//! Attribute definitions for the Verum AST.
//!
//! This module defines special attributes that can be attached to items,
//! including profile and feature attributes for module-level control.
//!
//! # Specification
//!
//! Language profiles control which language features are available (systems, application, etc.).

use crate::span::{Span, Spanned};
use serde::{Deserialize, Serialize};
use verum_common::{List, Maybe, Text};

/// Language profile for module-level feature control.
///
/// Profiles define different trade-off points in the language's safety/control spectrum:
/// - Application: Safe, productive, async-first (default for web/app development)
/// - Systems: Unsafe allowed, manual memory management (for low-level code)
/// - Research: Formal verification enabled (for critical systems)
///
/// # Examples
///
/// ```verum
/// @profile(application)
/// module web_server { }
///
/// @profile(systems)
/// module low_level { }
///
/// @profile(research)
/// module verified_math { }
///
/// // Multiple profiles
/// @profile(systems, research)
/// module runtime { }
/// ```
///
/// # Specification
///
/// Language profiles control which features are available in a module..1
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Profile {
    /// Application profile: safe, productive, async-first
    /// - No unsafe operations by default
    /// - Automatic memory management
    /// - Async/await enabled
    /// - Focus on productivity and safety
    Application,

    /// Systems profile: unsafe allowed, manual memory management
    /// - Unsafe operations permitted
    /// - Manual memory control
    /// - Custom allocators
    /// - Zero-cost abstractions priority
    Systems,

    /// Research profile: formal verification enabled
    /// - Full verification support
    /// - Proof generation
    /// - SMT solver integration
    /// - Refinement types enforced
    Research,
}

impl Profile {
    /// Get the profile from a string name
    pub fn from_str(s: &str) -> Maybe<Self> {
        match s {
            "application" => Maybe::Some(Profile::Application),
            "systems" => Maybe::Some(Profile::Systems),
            "research" => Maybe::Some(Profile::Research),
            _ => Maybe::None,
        }
    }

    /// Get the string name of this profile
    pub fn as_str(&self) -> &'static str {
        match self {
            Profile::Application => "application",
            Profile::Systems => "systems",
            Profile::Research => "research",
        }
    }

    /// Check if this profile is more restrictive than another
    ///
    /// Restriction hierarchy: Application < Systems < Research
    /// Research is most restrictive (requires proofs)
    /// Application is least restrictive (most permissive features)
    pub fn is_more_restrictive_than(&self, other: &Profile) -> bool {
        use Profile::*;
        matches!(
            (self, other),
            (Research, Application) | (Research, Systems) | (Systems, Application)
        )
    }

    /// Check if this profile allows unsafe operations
    pub fn allows_unsafe(&self) -> bool {
        matches!(self, Profile::Systems)
    }

    /// Check if this profile requires verification
    pub fn requires_verification(&self) -> bool {
        matches!(self, Profile::Research)
    }
}

impl std::fmt::Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Profile attribute: @profile(application|systems|research)
///
/// Declares which language profiles a module supports.
/// Multiple profiles can be specified.
///
/// # Examples
///
/// ```verum
/// @profile(application)
/// module safe_code { }
///
/// @profile(systems, research)
/// module runtime { }
/// ```
///
/// # Specification
///
/// Language profiles control which features are available in a module..1
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileAttr {
    /// The profiles this module supports
    pub profiles: List<Profile>,
    pub span: Span,
}

impl ProfileAttr {
    pub fn new(profiles: List<Profile>, span: Span) -> Self {
        Self { profiles, span }
    }

    /// Create a single-profile attribute
    pub fn single(profile: Profile, span: Span) -> Self {
        Self {
            profiles: vec![profile].into(),
            span,
        }
    }

    /// Check if this attribute includes a specific profile
    pub fn contains(&self, profile: Profile) -> bool {
        self.profiles.contains(&profile)
    }

    /// Check if this attribute is compatible with a parent's profile
    ///
    /// Child can be more restrictive, not less restrictive
    pub fn is_compatible_with(&self, parent: &ProfileAttr) -> bool {
        // Child must support at least one profile that parent supports
        self.profiles.iter().any(|child_profile| {
            parent.profiles.iter().any(|parent_profile| {
                child_profile == parent_profile
                    || child_profile.is_more_restrictive_than(parent_profile)
            })
        })
    }
}

impl Spanned for ProfileAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Feature attribute: @feature(enable: ["unsafe", "inline_asm", ...])
///
/// Enables specific language features beyond the base profile.
/// Features are additive and must be compatible with the base profile.
///
/// # Examples
///
/// ```verum
/// @profile(application)
/// @feature(enable: ["unsafe"])
/// module ffi_bindings { }
///
/// @profile(application)
/// @feature(enable: ["unsafe", "inline_asm"])
/// module crypto { }
/// ```
///
/// # Specification
///
/// Language profiles control which features are available in a module..3
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureAttr {
    /// Features to enable
    pub features: List<Text>,
    pub span: Span,
}

impl FeatureAttr {
    pub fn new(features: List<Text>, span: Span) -> Self {
        Self { features, span }
    }

    /// Check if a specific feature is enabled
    pub fn has_feature(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f.as_str() == feature)
    }

    /// Known feature names for validation
    pub fn known_features() -> &'static [&'static str] {
        &[
            "unsafe",
            "inline_asm",
            "custom_allocator",
            "raw_pointers",
            "manual_drop",
            "volatile_access",
        ]
    }

    /// Validate that all features are known
    pub fn validate(&self) -> Result<(), Text> {
        let known = Self::known_features();
        for feature in self.features.iter() {
            if !known.contains(&feature.as_str()) {
                return Err(Text::from(format!(
                    "Unknown feature '{}'. Known features: {}",
                    feature,
                    known.join(", ")
                )));
            }
        }
        Ok(())
    }
}

impl Spanned for FeatureAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// @std attribute for automatic context provisioning.
///
/// The `@std` attribute provides automatic context provisioning for common use cases,
/// particularly useful for scripts, entry points, and simple applications.
///
/// # Syntax
/// - `@std` - Uses ApplicationContext (default)
/// - `@std(ContextGroup)` - Uses specified context group
///
/// # Examples
///
/// ```verum
/// @std
/// fn main() {
///     // ApplicationContext is automatically provided
/// }
///
/// @std(ServerContext)
/// fn run_server() {
///     // ServerContext is automatically provided
/// }
/// ```
///
/// # Specification
///
/// @std attribute for automatic context provisioning using named context groups.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StdAttr {
    /// Optional context group name (defaults to ApplicationContext if None)
    pub context_group: Maybe<Text>,
    pub span: Span,
}

impl StdAttr {
    pub fn new(context_group: Maybe<Text>, span: Span) -> Self {
        Self {
            context_group,
            span,
        }
    }

    /// Create a @std attribute with default context (ApplicationContext)
    pub fn default(span: Span) -> Self {
        Self {
            context_group: Maybe::None,
            span,
        }
    }

    /// Get the context group name, or default to "ApplicationContext"
    pub fn get_context_group(&self) -> &'static str {
        match &self.context_group {
            Maybe::Some(_) => "custom", // Caller should extract the actual value
            Maybe::None => "ApplicationContext",
        }
    }
}

impl Spanned for StdAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Specialization attribute: @specialize or @specialize(negative|rank=N|when(...))
///
/// The `@specialize` attribute enables protocol implementation specialization,
/// allowing more specific implementations to override more general ones.
///
/// # Syntax Forms
///
/// 1. **Basic Specialization:**
/// ```verum
/// @specialize
/// implement<T: Clone> MyProtocol for List<T> { }
/// ```
///
/// 2. **Negative Specialization:**
/// ```verum
/// @specialize(negative)
/// implement<T: !Clone> MyProtocol for List<T> { }
/// ```
///
/// 3. **Specialization with Rank:**
/// ```verum
/// @specialize(rank = 10)
/// implement MyProtocol for Int { }
/// ```
///
/// 4. **Conditional Specialization:**
/// ```verum
/// @specialize(when(T: Clone + Send))
/// implement<T> MyProtocol for Heap<T> { }
/// ```
///
/// # Specification
///
/// @specialize attribute for protocol implementation specialization (v2.0+ planned).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecializeAttr {
    /// Whether this is negative specialization (!Protocol)
    pub negative: bool,

    /// Optional explicit rank for precedence control
    /// Higher rank = higher priority in specialization lattice
    pub rank: Maybe<i32>,

    /// Optional conditional where clause: when(T: Clone)
    /// Negative specialization: applies when a type does NOT implement a protocol.
    pub when_clause: Maybe<crate::ty::WhereClause>,

    pub span: Span,
}

impl SpecializeAttr {
    pub fn new(
        negative: bool,
        rank: Maybe<i32>,
        when_clause: Maybe<crate::ty::WhereClause>,
        span: Span,
    ) -> Self {
        Self {
            negative,
            rank,
            when_clause,
            span,
        }
    }

    /// Create a basic @specialize attribute
    pub fn basic(span: Span) -> Self {
        Self {
            negative: false,
            rank: Maybe::None,
            when_clause: Maybe::None,
            span,
        }
    }

    /// Create a negative specialization attribute
    pub fn negative(span: Span) -> Self {
        Self {
            negative: true,
            rank: Maybe::None,
            when_clause: Maybe::None,
            span,
        }
    }

    /// Create a specialization with explicit rank
    pub fn with_rank(rank: i32, span: Span) -> Self {
        Self {
            negative: false,
            rank: Maybe::Some(rank),
            when_clause: Maybe::None,
            span,
        }
    }

    /// Check if this has a conditional when clause
    pub fn has_when_clause(&self) -> bool {
        matches!(self.when_clause, Maybe::Some(_))
    }

    /// Get the effective rank (explicit or inferred)
    /// Default rank is 0 if not specified
    pub fn effective_rank(&self) -> i32 {
        match self.rank {
            Maybe::Some(r) => r,
            Maybe::None => 0,
        }
    }
}

impl Spanned for SpecializeAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Lock level attribute: @lock_level(level: N)
///
/// Declares the lock ordering level for a mutex type to enable
/// compile-time deadlock detection.
///
/// # Syntax
///
/// ```verum
/// @lock_level(level: 1)
/// type DatabaseLock is AsyncMutex<Connection>
///
/// @lock_level(level: 2)
/// type CacheLock is AsyncMutex<Cache>
/// ```
///
/// # Semantics
///
/// Lock levels form a strict partial order. A lock with level N can only
/// be acquired while holding locks with levels < N. This prevents deadlock
/// by ensuring a global acquisition order.
///
/// # Specification
///
/// Lock ordering attribute for static deadlock prevention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockLevelAttr {
    /// The lock level (must be >= 0)
    /// Higher levels must be acquired after lower levels
    pub level: u32,
    pub span: Span,
}

impl LockLevelAttr {
    pub fn new(level: u32, span: Span) -> Self {
        Self { level, span }
    }
}

impl Spanned for LockLevelAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Deadlock detection attribute: @deadlock_detection(enabled: bool, timeout: Duration)
///
/// Enables runtime deadlock detection for a function or module.
///
/// # Syntax
///
/// ```verum
/// @deadlock_detection(enabled: true, timeout: 5_seconds)
/// fn critical_section() { }
/// ```
///
/// # Specification
///
/// Lock ordering attribute for static deadlock prevention.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeadlockDetectionAttr {
    /// Whether deadlock detection is enabled
    pub enabled: bool,
    /// Timeout in milliseconds for detecting potential deadlocks
    pub timeout_ms: Maybe<u64>,
    pub span: Span,
}

impl DeadlockDetectionAttr {
    pub fn new(enabled: bool, timeout_ms: Maybe<u64>, span: Span) -> Self {
        Self {
            enabled,
            timeout_ms,
            span,
        }
    }

    /// Create with default timeout (5 seconds)
    pub fn enabled_default(span: Span) -> Self {
        Self {
            enabled: true,
            timeout_ms: Maybe::Some(5000),
            span,
        }
    }
}

impl Spanned for DeadlockDetectionAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Generic attribute structure for items.
///
/// Used for general-purpose attributes like @inline, @deprecated, etc.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Attribute {
    pub name: Text,
    pub args: Maybe<List<crate::expr::Expr>>,
    pub span: Span,
}

impl Attribute {
    pub fn new(name: Text, args: Maybe<List<crate::expr::Expr>>, span: Span) -> Self {
        Self { name, args, span }
    }

    /// Create an attribute without arguments
    pub fn simple(name: Text, span: Span) -> Self {
        Self {
            name,
            args: Maybe::None,
            span,
        }
    }

    /// Check if this is a specific attribute by name.
    pub fn is_named(&self, name: &str) -> bool {
        self.name.as_str() == name
    }
}

impl Spanned for Attribute {
    fn span(&self) -> Span {
        self.span
    }
}

// =============================================================================
// OPTIMIZATION ATTRIBUTES
// Optimization hint attributes for fine-grained compiler control.
// =============================================================================

/// Inline mode for @inline attribute
///
/// Controls function inlining: always, never, release-only, or compiler-decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InlineMode {
    /// @inline - suggest inlining (compiler decides)
    Suggest,
    /// @inline(always) - always inline this function
    Always,
    /// @inline(never) - never inline (e.g., for cold error paths)
    Never,
    /// @inline(release) - inline only in release builds
    Release,
}

impl InlineMode {
    /// Parse inline mode from string
    pub fn from_str(s: &str) -> Maybe<Self> {
        match s {
            "always" => Maybe::Some(InlineMode::Always),
            "never" => Maybe::Some(InlineMode::Never),
            "release" => Maybe::Some(InlineMode::Release),
            _ => Maybe::None,
        }
    }

    /// Get string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            InlineMode::Suggest => "suggest",
            InlineMode::Always => "always",
            InlineMode::Never => "never",
            InlineMode::Release => "release",
        }
    }
}

/// Inline attribute: @inline, @inline(always), @inline(never), @inline(release)
///
/// Controls function inlining behavior for optimal performance.
///
/// # Examples
///
/// ```verum
/// @inline(always)
/// fn hot_path(x: i32) -> i32 { x * 2 }
///
/// @inline(never)
/// fn cold_error_handler(err: Error) -> ! { panic!("Fatal: {err}") }
///
/// @inline(release)
/// fn calculate(data: &[f64]) -> f64 { data.iter().sum() / data.len() as f64 }
///
/// @inline
/// fn maybe_inline(x: i32) -> i32 { x + 1 }
/// ```
///
/// Controls function inlining: always, never, release-only, or compiler-decided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineAttr {
    pub mode: InlineMode,
    pub span: Span,
}

impl InlineAttr {
    pub fn new(mode: InlineMode, span: Span) -> Self {
        Self { mode, span }
    }

    pub fn suggest(span: Span) -> Self {
        Self::new(InlineMode::Suggest, span)
    }

    pub fn always(span: Span) -> Self {
        Self::new(InlineMode::Always, span)
    }

    pub fn never(span: Span) -> Self {
        Self::new(InlineMode::Never, span)
    }
}

impl Spanned for InlineAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Cold attribute: @cold
///
/// Marks functions or code paths that are rarely executed,
/// allowing the optimizer to deprioritize them in favor of hot paths.
///
/// # Performance Benefits
///
/// - Improved hot path performance: 2-5% speedup from reduced instruction cache pressure
/// - Better branch prediction: CPU predictors receive static hints
/// - Reduced binary size: 1-3% through less aggressive cold path optimization
/// - Faster compilation: 5-10% faster builds by skipping expensive optimizations
///
/// # Example
///
/// ```verum
/// @cold
/// fn handle_parse_error(contents: &str) -> Error {
///     log_error("Invalid config format")
///     Error.InvalidFormat
/// }
/// ```
///
/// Marks function as cold (rarely called) -- optimizes for size, not speed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColdAttr {
    pub span: Span,
}

impl ColdAttr {
    pub fn new(span: Span) -> Self {
        Self { span }
    }
}

impl Spanned for ColdAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Hot attribute: @hot
///
/// Marks function as frequently called (hot path).
/// Enables aggressive optimization for critical code paths.
///
/// # Example
///
/// ```verum
/// @hot
/// fn render_frame(ctx: &mut RenderContext) {
///     // Critical path - optimize aggressively
/// }
/// ```
///
/// Marks function as hot (frequently called) -- optimizes aggressively for speed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotAttr {
    pub span: Span,
}

impl HotAttr {
    pub fn new(span: Span) -> Self {
        Self { span }
    }
}

impl Spanned for HotAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Optimization level for @optimize attribute
///
/// Loop unrolling hint: full, N times, or disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OptimizationLevel {
    /// @optimize(none) - No optimization (for debugging)
    None,
    /// @optimize(size) - Optimize for size
    Size,
    /// @optimize(speed) - Optimize for speed
    Speed,
    /// @optimize(balanced) - Balance size and speed
    Balanced,
}

impl OptimizationLevel {
    pub fn from_str(s: &str) -> Maybe<Self> {
        match s {
            "none" => Maybe::Some(OptimizationLevel::None),
            "size" => Maybe::Some(OptimizationLevel::Size),
            "speed" => Maybe::Some(OptimizationLevel::Speed),
            "balanced" => Maybe::Some(OptimizationLevel::Balanced),
            _ => Maybe::None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            OptimizationLevel::None => "none",
            OptimizationLevel::Size => "size",
            OptimizationLevel::Speed => "speed",
            OptimizationLevel::Balanced => "balanced",
        }
    }
}

/// Optimize attribute: @optimize(size|speed|none|balanced)
///
/// Override global optimization level for a specific function.
///
/// # Examples
///
/// ```verum
/// @optimize(size)
/// fn rarely_used_large_function() { /* Complex but rarely executed */ }
///
/// @optimize(speed)
/// fn critical_inner_loop() { /* Hot path needing max performance */ }
///
/// @optimize(none)
/// fn debug_this() { /* Keep code exactly as written */ }
/// ```
///
/// Loop unrolling hint: full, N times, or disabled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptimizeAttr {
    pub level: OptimizationLevel,
    pub span: Span,
}

impl OptimizeAttr {
    pub fn new(level: OptimizationLevel, span: Span) -> Self {
        Self { level, span }
    }
}

impl Spanned for OptimizeAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Vectorization mode for @vectorize and @simd attributes
///
/// Auto-vectorization hint for SIMD acceleration of loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VectorizeMode {
    /// @vectorize - enable auto-vectorization (default)
    Auto,
    /// @vectorize(force) - force vectorization (error if impossible)
    Force,
    /// @simd(prefer) - try vectorization, fall back to scalar if needed
    Prefer,
    /// @no_vectorize or @simd(never) - disable vectorization
    Never,
}

impl VectorizeMode {
    pub fn from_str(s: &str) -> Maybe<Self> {
        match s {
            "auto" | "" => Maybe::Some(VectorizeMode::Auto),
            "force" => Maybe::Some(VectorizeMode::Force),
            "prefer" => Maybe::Some(VectorizeMode::Prefer),
            "never" => Maybe::Some(VectorizeMode::Never),
            _ => Maybe::None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            VectorizeMode::Auto => "auto",
            VectorizeMode::Force => "force",
            VectorizeMode::Prefer => "prefer",
            VectorizeMode::Never => "never",
        }
    }
}

/// Vectorize attribute: @vectorize, @vectorize(force), @simd, @no_vectorize
///
/// Control loop auto-vectorization behavior.
///
/// # Examples
///
/// ```verum
/// @vectorize(force)
/// for i in 0..data.len() { sum += data[i] }
///
/// @no_vectorize
/// fn precise_computation(data: &[f64]) -> f64 { strict_sum(data) }
///
/// @simd(prefer)
/// fn maybe_vectorized(a: &[Float]) -> Float { a.iter().sum() }
/// ```
///
/// Auto-vectorization hint for SIMD acceleration of loops.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorizeAttr {
    pub mode: VectorizeMode,
    /// Optional vectorization width hint (e.g., 4, 8, 16)
    pub width: Maybe<u32>,
    pub span: Span,
}

impl VectorizeAttr {
    pub fn new(mode: VectorizeMode, span: Span) -> Self {
        Self {
            mode,
            width: Maybe::None,
            span,
        }
    }

    pub fn with_width(mode: VectorizeMode, width: u32, span: Span) -> Self {
        Self {
            mode,
            width: Maybe::Some(width),
            span,
        }
    }

    pub fn force(span: Span) -> Self {
        Self::new(VectorizeMode::Force, span)
    }

    pub fn never(span: Span) -> Self {
        Self::new(VectorizeMode::Never, span)
    }
}

impl Spanned for VectorizeAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Unroll mode for @unroll attribute
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnrollMode {
    /// @unroll(N) - unroll loop N times
    Count(u32),
    /// @unroll(full) - fully unroll the loop
    Full,
    /// @no_unroll - prevent loop unrolling
    Never,
}

/// Unroll attribute: @unroll(N), @unroll(full), @no_unroll
///
/// Control loop unrolling behavior explicitly.
///
/// # Examples
///
/// ```verum
/// @unroll(4)
/// for i in 0..a.rows { /* Unroll loop 4 times */ }
///
/// @unroll(full)
/// for i in 0..8 { buffer[i] = source[i] }
///
/// @no_unroll
/// for item in large_collection { process(item) }
/// ```
///
/// Loop optimization hint controlling unrolling behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnrollAttr {
    pub mode: UnrollMode,
    pub span: Span,
}

impl UnrollAttr {
    pub fn new(mode: UnrollMode, span: Span) -> Self {
        Self { mode, span }
    }

    pub fn count(n: u32, span: Span) -> Self {
        Self::new(UnrollMode::Count(n), span)
    }

    pub fn full(span: Span) -> Self {
        Self::new(UnrollMode::Full, span)
    }

    pub fn never(span: Span) -> Self {
        Self::new(UnrollMode::Never, span)
    }
}

impl Spanned for UnrollAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Prefetch access mode
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrefetchAccess {
    #[default]
    Read,
    Write,
}

impl PrefetchAccess {
    pub fn from_str(s: &str) -> Maybe<Self> {
        match s {
            "read" => Maybe::Some(PrefetchAccess::Read),
            "write" => Maybe::Some(PrefetchAccess::Write),
            _ => Maybe::None,
        }
    }
}

/// Prefetch attribute: @prefetch(read|write, locality: N)
///
/// Prefetch data into cache for improved memory access performance.
///
/// # Locality Levels (0-3)
///
/// - 0: No temporal locality (stream)
/// - 1: Low temporal locality
/// - 2: Moderate temporal locality
/// - 3: High temporal locality (keep in all cache levels)
///
/// # Example
///
/// ```verum
/// @prefetch(read, locality: 3)
/// fn process_array(data: &[u8]) {
///     for chunk in data.chunks(64) {
///         @prefetch(&chunk[64], write, locality: 2)
///         process_chunk(chunk)
///     }
/// }
/// ```
///
/// Memory optimization hint for cache prefetching or alignment.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrefetchAttr {
    pub access: PrefetchAccess,
    /// Locality level 0-3 (default: 3)
    pub locality: u8,
    pub span: Span,
}

impl PrefetchAttr {
    pub fn new(access: PrefetchAccess, locality: u8, span: Span) -> Self {
        Self {
            access,
            locality: locality.min(3),
            span,
        }
    }

    pub fn read(locality: u8, span: Span) -> Self {
        Self::new(PrefetchAccess::Read, locality, span)
    }

    pub fn write(locality: u8, span: Span) -> Self {
        Self::new(PrefetchAccess::Write, locality, span)
    }
}

impl Spanned for PrefetchAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Align attribute: @align(N)
///
/// Specify memory alignment for types or variables.
///
/// # Example
///
/// ```verum
/// @align(32)
/// type AlignedBuffer is [f32; 1024]
///
/// @align(64)  // Cache line alignment
/// type CacheOptimized is { hot_field: u64, hot_field2: u64 }
/// ```
///
/// Memory optimization hint for cache prefetching or alignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlignAttr {
    /// Alignment in bytes (must be power of 2)
    pub alignment: u32,
    pub span: Span,
}

impl AlignAttr {
    pub fn new(alignment: u32, span: Span) -> Self {
        Self { alignment, span }
    }

    /// Validate alignment is power of 2
    pub fn is_valid(&self) -> bool {
        self.alignment > 0 && self.alignment.is_power_of_two()
    }
}

impl Spanned for AlignAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Likelihood attribute: @likely, @unlikely
///
/// Guide branch prediction and code layout.
///
/// # Examples
///
/// ```verum
/// fn process_request(req: Request) -> Response {
///     @likely if req.is_valid() {
///         handle_valid_request(req)
///     } else {
///         @unlikely { Response.error(400) }
///     }
/// }
/// ```
///
/// Marks function as hot (frequently called) -- optimizes aggressively for speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Likelihood {
    /// @likely - mark as likely to be taken
    Likely,
    /// @unlikely - mark as unlikely to be taken
    Unlikely,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LikelihoodAttr {
    pub likelihood: Likelihood,
    pub span: Span,
}

impl LikelihoodAttr {
    pub fn likely(span: Span) -> Self {
        Self {
            likelihood: Likelihood::Likely,
            span,
        }
    }

    pub fn unlikely(span: Span) -> Self {
        Self {
            likelihood: Likelihood::Unlikely,
            span,
        }
    }
}

impl Spanned for LikelihoodAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Verification mode for @verify attribute
///
/// Performance contract specifying expected latency/throughput guarantees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VerificationMode {
    /// @verify(proof) - SMT solver proof (highest confidence, slowest)
    Proof,
    /// @verify(static) - Dataflow analysis (medium ~100-500ms per function)
    Static,
    /// @verify(runtime) - Runtime assertions (fastest, adds ~5ns overhead)
    Runtime,
    /// @verify(assume) - Trust programmer, no verification (dangerous)
    Assume,
}

impl VerificationMode {
    pub fn from_str(s: &str) -> Maybe<Self> {
        match s {
            "proof" => Maybe::Some(VerificationMode::Proof),
            "static" => Maybe::Some(VerificationMode::Static),
            "runtime" => Maybe::Some(VerificationMode::Runtime),
            "assume" => Maybe::Some(VerificationMode::Assume),
            _ => Maybe::None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            VerificationMode::Proof => "proof",
            VerificationMode::Static => "static",
            VerificationMode::Runtime => "runtime",
            VerificationMode::Assume => "assume",
        }
    }
}

/// Verify attribute: @verify(proof|static|runtime|assume)
///
/// Contract verification hints to control verification behavior.
///
/// # Examples
///
/// ```verum
/// @verify(proof, timeout: 5s)
/// fn divide(a: Float, b: Float{!= 0.0}) -> Float { a / b }
///
/// @verify(runtime)
/// fn safe_divide(a: Float, b: Float) -> Result<Float, DivisionByZero> {
///     if b == 0.0 { Err(DivisionByZero) } else { Ok(a / b) }
/// }
///
/// @verify([proof, static, runtime])  // Chain strategies
/// fn complex_invariant(x: Int{> 0}, y: Int{> 0}) -> Int{> 0} { x + y }
/// ```
///
/// Performance contract specifying expected latency/throughput guarantees.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifyAttr {
    /// Verification modes (can be chained)
    pub modes: List<VerificationMode>,
    /// Optional timeout for proof verification (in milliseconds)
    pub timeout_ms: Maybe<u64>,
    pub span: Span,
}

impl VerifyAttr {
    pub fn new(modes: List<VerificationMode>, span: Span) -> Self {
        Self {
            modes,
            timeout_ms: Maybe::None,
            span,
        }
    }

    pub fn single(mode: VerificationMode, span: Span) -> Self {
        Self {
            modes: vec![mode].into(),
            timeout_ms: Maybe::None,
            span,
        }
    }

    pub fn proof(span: Span) -> Self {
        Self::single(VerificationMode::Proof, span)
    }

    pub fn runtime(span: Span) -> Self {
        Self::single(VerificationMode::Runtime, span)
    }

    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Maybe::Some(timeout_ms);
        self
    }
}

impl Spanned for VerifyAttr {
    fn span(&self) -> Span {
        self.span
    }
}

// =============================================================================
// TERMINATION PROOF ATTRIBUTES
// Gradual verification attributes for incremental safety assurance.
// =============================================================================

/// Well-founded relation types for termination proofs.
///
/// Standard well-founded relations used to prove termination
/// of recursive functions and loops.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum WellFoundedRelation {
    /// Natural numbers with < (default for most cases)
    #[default]
    NaturalLt,
    /// Lexicographic ordering on tuples
    Lexicographic,
    /// Multiset ordering (for Dafny-style termination)
    Multiset,
    /// Structural subterm ordering (for inductive types)
    Structural,
    /// Custom well-founded relation (user-defined)
    Custom(Text),
}

impl WellFoundedRelation {
    pub fn from_str(s: &str) -> Maybe<Self> {
        match s {
            "natural" | "nat" => Maybe::Some(WellFoundedRelation::NaturalLt),
            "lexicographic" | "lex" => Maybe::Some(WellFoundedRelation::Lexicographic),
            "multiset" => Maybe::Some(WellFoundedRelation::Multiset),
            "structural" => Maybe::Some(WellFoundedRelation::Structural),
            _ => Maybe::Some(WellFoundedRelation::Custom(Text::from(s))),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            WellFoundedRelation::NaturalLt => "natural",
            WellFoundedRelation::Lexicographic => "lexicographic",
            WellFoundedRelation::Multiset => "multiset",
            WellFoundedRelation::Structural => "structural",
            WellFoundedRelation::Custom(name) => name.as_str(),
        }
    }
}

/// Measure attribute: @measure(expr)
///
/// Specifies the termination measure for a recursive function.
/// The measure expression must decrease on each recursive call
/// according to a well-founded relation.
///
/// # Examples
///
/// ```verum
/// @measure(n)
/// fn factorial(n: Int{>= 0}) -> Int {
///     if n == 0 { 1 }
///     else { n * factorial(n - 1) }
/// }
///
/// @measure(list.len())
/// fn sum(list: &List<Int>) -> Int {
///     match list {
///         [] => 0,
///         [head, ..tail] => head + sum(tail)
///     }
/// }
///
/// @measure((fuel, depth), relation: lexicographic)
/// fn search(fuel: Int{>= 0}, depth: Int{>= 0}) -> Maybe<Result> { ... }
/// ```
///
/// # Semantics
///
/// 1. The measure expression is evaluated before each recursive call
/// 2. The verifier proves that the measure strictly decreases
/// 3. Multiple measures form a tuple with lexicographic ordering
/// 4. For loops, use `decreases` instead of `@measure`
///
/// Gradual verification attributes for incremental safety assurance..2
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeasureAttr {
    /// The measure expression(s) - multiple form lexicographic ordering
    pub measures: List<crate::expr::Expr>,
    /// The well-founded relation to use (default: natural numbers <)
    pub relation: WellFoundedRelation,
    /// Optional explicit bound (for bounded loops)
    pub bound: Maybe<crate::expr::Expr>,
    /// Source location
    pub span: Span,
}

impl MeasureAttr {
    /// Create a new measure attribute with a single measure
    pub fn new(measure: crate::expr::Expr, span: Span) -> Self {
        Self {
            measures: vec![measure].into(),
            relation: WellFoundedRelation::default(),
            bound: Maybe::None,
            span,
        }
    }

    /// Create with multiple measures (lexicographic ordering)
    pub fn lexicographic(measures: List<crate::expr::Expr>, span: Span) -> Self {
        Self {
            measures,
            relation: WellFoundedRelation::Lexicographic,
            bound: Maybe::None,
            span,
        }
    }

    /// Set the well-founded relation
    pub fn with_relation(mut self, relation: WellFoundedRelation) -> Self {
        self.relation = relation;
        self
    }

    /// Set an explicit bound
    pub fn with_bound(mut self, bound: crate::expr::Expr) -> Self {
        self.bound = Maybe::Some(bound);
        self
    }

    /// Check if this has multiple measures (lexicographic)
    pub fn is_lexicographic(&self) -> bool {
        self.measures.len() > 1
    }
}

impl Spanned for MeasureAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Decreases attribute: @decreases(expr)
///
/// Specifies the decreasing expression for loop termination.
/// This is the loop-level equivalent of `@measure` for functions.
///
/// # Examples
///
/// ```verum
/// fn binary_search(arr: &[Int], target: Int) -> Maybe<Int> {
///     let mut low = 0;
///     let mut high = arr.len();
///
///     @decreases(high - low)
///     while low < high {
///         let mid = low + (high - low) / 2;
///         if arr[mid] == target { return Some(mid) }
///         else if arr[mid] < target { low = mid + 1 }
///         else { high = mid }
///     }
///     None
/// }
///
/// // Multiple decreasing expressions (lexicographic)
/// @decreases(outer, inner)
/// while outer > 0 {
///     while inner > 0 {
///         inner -= 1;
///     }
///     outer -= 1;
///     inner = reset_value;
/// }
/// ```
///
/// # Semantics
///
/// 1. The decreasing expression must be non-negative at loop entry
/// 2. The expression must decrease with each iteration
/// 3. Combined with `invariant`, proves total correctness
///
/// Note: This is a function-level attribute. For inline syntax,
/// use `decreases EXPR` within the loop (already supported by parser).
///
/// Gradual verification attributes for incremental safety assurance..3
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecreasesAttr {
    /// The decreasing expression(s)
    pub exprs: List<crate::expr::Expr>,
    /// Optional lower bound (default: 0)
    pub lower_bound: Maybe<crate::expr::Expr>,
    /// Source location
    pub span: Span,
}

impl DecreasesAttr {
    /// Create a new decreases attribute with a single expression
    pub fn new(expr: crate::expr::Expr, span: Span) -> Self {
        Self {
            exprs: vec![expr].into(),
            lower_bound: Maybe::None,
            span,
        }
    }

    /// Create with multiple expressions (lexicographic)
    pub fn lexicographic(exprs: List<crate::expr::Expr>, span: Span) -> Self {
        Self {
            exprs,
            lower_bound: Maybe::None,
            span,
        }
    }

    /// Set a custom lower bound (default is 0)
    pub fn with_lower_bound(mut self, bound: crate::expr::Expr) -> Self {
        self.lower_bound = Maybe::Some(bound);
        self
    }
}

impl Spanned for DecreasesAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Well-founded attribute: @well_founded(relation_name)
///
/// Declares or references a well-founded relation for termination proofs.
/// Used when the default natural number ordering is insufficient.
///
/// # Examples
///
/// ```verum
/// // Reference a standard well-founded relation
/// @well_founded(lexicographic)
/// @measure((n, m))
/// fn ackermann(n: Int{>= 0}, m: Int{>= 0}) -> Int {
///     match (n, m) {
///         (0, _) => m + 1,
///         (_, 0) => ackermann(n - 1, 1),
///         _ => ackermann(n - 1, ackermann(n, m - 1))
///     }
/// }
///
/// // Define a custom well-founded relation
/// @well_founded(tree_size)
/// @measure(tree.size())
/// fn fold_tree<T, R>(tree: &Tree<T>, f: fn(T) -> R, combine: fn(R, R) -> R) -> R {
///     match tree {
///         Leaf(v) => f(v),
///         Node { left, right, .. } => combine(fold_tree(left, f, combine),
///                                              fold_tree(right, f, combine))
///     }
/// }
/// ```
///
/// Gradual verification attributes for incremental safety assurance..4
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WellFoundedAttr {
    /// The well-founded relation
    pub relation: WellFoundedRelation,
    /// Optional proof that the relation is well-founded
    pub proof: Maybe<Text>,
    /// Source location
    pub span: Span,
}

impl WellFoundedAttr {
    /// Create a new well-founded attribute
    pub fn new(relation: WellFoundedRelation, span: Span) -> Self {
        Self {
            relation,
            proof: Maybe::None,
            span,
        }
    }

    /// Create with a proof reference
    pub fn with_proof(mut self, proof: Text) -> Self {
        self.proof = Maybe::Some(proof);
        self
    }

    /// Create for lexicographic ordering
    pub fn lexicographic(span: Span) -> Self {
        Self::new(WellFoundedRelation::Lexicographic, span)
    }

    /// Create for structural ordering
    pub fn structural(span: Span) -> Self {
        Self::new(WellFoundedRelation::Structural, span)
    }
}

impl Spanned for WellFoundedAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Termination proof attribute: @termination_proof
///
/// Marks a function or lemma as a termination proof for another function.
/// This enables modular termination verification where the proof is
/// separate from the implementation.
///
/// # Examples
///
/// ```verum
/// // Function that needs termination proof
/// @verify(proof)
/// fn collatz(n: Int{> 0}) -> Int {
///     if n == 1 { 1 }
///     else if n % 2 == 0 { collatz(n / 2) }
///     else { collatz(3 * n + 1) }
/// }
///
/// // Separate termination proof (assumes Collatz conjecture)
/// @termination_proof(for: collatz)
/// @assume(collatz_conjecture)  // Explicit assumption
/// lemma collatz_terminates(n: Int{> 0}) {
///     // Proof that collatz terminates for all n > 0
///     // (Currently assumed via Collatz conjecture)
/// }
///
/// // Termination proof with explicit measure
/// @termination_proof(for: factorial, measure: n)
/// lemma factorial_terminates(n: Int{>= 0}) {
///     // Proof: n decreases on each call, bounded below by 0
///     assert n >= 0;
///     assert n - 1 < n when n > 0;
/// }
/// ```
///
/// Gradual verification attributes for incremental safety assurance..5
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminationProofAttr {
    /// The function this proof applies to
    pub target_function: Text,
    /// Optional explicit measure (if different from function's @measure)
    pub measure: Maybe<Text>,
    /// Whether termination is assumed rather than proven
    pub assumed: bool,
    /// Source location
    pub span: Span,
}

impl TerminationProofAttr {
    /// Create a new termination proof attribute
    pub fn new(target_function: Text, span: Span) -> Self {
        Self {
            target_function,
            measure: Maybe::None,
            assumed: false,
            span,
        }
    }

    /// Set the measure expression reference
    pub fn with_measure(mut self, measure: Text) -> Self {
        self.measure = Maybe::Some(measure);
        self
    }

    /// Mark termination as assumed (not proven)
    pub fn assumed(mut self) -> Self {
        self.assumed = true;
        self
    }
}

impl Spanned for TerminationProofAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Target CPU attribute: @target_cpu(name)
///
/// CPU-specific optimization target.
///
/// # Examples
///
/// ```verum
/// @target_cpu("native")
/// fn platform_optimized() { /* Use all CPU features available */ }
///
/// @target_cpu("x86-64-v3")
/// fn modern_cpu_only() { /* Can use AVX, AVX2, BMI, etc. */ }
/// ```
///
/// Parallel execution hint for independent loop iterations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetCpuAttr {
    pub cpu: Text,
    pub span: Span,
}

impl TargetCpuAttr {
    pub fn new(cpu: Text, span: Span) -> Self {
        Self { cpu, span }
    }

    pub fn native(span: Span) -> Self {
        Self::new(Text::from("native"), span)
    }
}

impl Spanned for TargetCpuAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Target feature attribute: @target_feature(features)
///
/// Enable specific target features for a function.
///
/// # Example
///
/// ```verum
/// @target_feature("+avx2,+fma")
/// fn simd_compute(data: &[f32]) -> f32 { /* Uses AVX2 and FMA instructions */ }
/// ```
///
/// No-alias assertion for pointer/reference disambiguation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetFeatureAttr {
    /// Comma-separated feature list (e.g., "+avx2,+fma")
    pub features: Text,
    pub span: Span,
}

impl TargetFeatureAttr {
    pub fn new(features: Text, span: Span) -> Self {
        Self { features, span }
    }
}

impl Spanned for TargetFeatureAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Const evaluation attribute: @const_eval, @const_fold, @const_prop
///
/// Force compile-time evaluation.
///
/// # Examples
///
/// ```verum
/// @const_eval
/// fn lookup_table() -> [u8; 256] {
///     let mut table = [0; 256]
///     for i in 0..256 { table[i] = compute_value(i) }
///     table
/// }
/// ```
///
/// Profile-guided optimization data collection attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConstEvalMode {
    /// @const_eval - force compile-time evaluation
    Eval,
    /// @const_fold - inline const with optimization
    Fold,
    /// @const_prop - enable aggressive const propagation
    Propagate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstEvalAttr {
    pub mode: ConstEvalMode,
    pub span: Span,
}

impl ConstEvalAttr {
    pub fn new(mode: ConstEvalMode, span: Span) -> Self {
        Self { mode, span }
    }

    pub fn eval(span: Span) -> Self {
        Self::new(ConstEvalMode::Eval, span)
    }
}

impl Spanned for ConstEvalAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Profile attribute for PGO: @profile, @frequency, @branch_probability
///
/// Mark function for profiling or provide expected execution frequency.
///
/// # Examples
///
/// ```verum
/// @profile
/// fn important_function() { /* Compiler instruments for PGO */ }
///
/// @frequency(1000)  // Called ~1000 times per second
/// fn periodic_task() { /* Optimize for frequent execution */ }
///
/// @branch_probability(0.95)
/// if let Ok(value) = try_parse(s) { /* Success 95% of the time */ }
/// ```
///
/// Profile annotation for PGO-guided optimization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PgoAttr {
    /// @profile - mark for profiling
    Profile {
        /// Optional profile name
        name: Maybe<Text>,
        span: Span,
    },
    /// @frequency(N) - expected calls per second
    Frequency { calls_per_sec: u64, span: Span },
    /// @branch_probability(P) - branch taken probability (0.0-1.0)
    BranchProbability { probability: f64, span: Span },
}

impl Default for PgoAttr {
    fn default() -> Self {
        PgoAttr::Profile {
            name: Maybe::None,
            span: Span::default(),
        }
    }
}

impl Spanned for PgoAttr {
    fn span(&self) -> Span {
        match self {
            PgoAttr::Profile { span, .. } => *span,
            PgoAttr::Frequency { span, .. } => *span,
            PgoAttr::BranchProbability { span, .. } => *span,
        }
    }
}

/// LTO attribute: @no_lto, @lto(always|thin)
///
/// Control Link-Time Optimization behavior.
///
/// # Examples
///
/// ```verum
/// @no_lto
/// fn plugin_interface() { /* Preserve exact ABI */ }
///
/// @lto(always)
/// fn performance_critical() { /* Need cross-module inlining */ }
///
/// @lto(thin)
/// module large_module { /* Use thin LTO for faster builds */ }
/// ```
///
/// Target CPU attribute for architecture-specific code generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LtoMode {
    /// @no_lto - exclude from LTO
    None,
    /// @lto(always) - force LTO even in debug
    Always,
    /// @lto(thin) - use Thin LTO
    Thin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LtoAttr {
    pub mode: LtoMode,
    pub span: Span,
}

impl LtoAttr {
    pub fn new(mode: LtoMode, span: Span) -> Self {
        Self { mode, span }
    }

    pub fn none(span: Span) -> Self {
        Self::new(LtoMode::None, span)
    }
}

impl Spanned for LtoAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Visibility attribute: @visibility(hidden|default|protected)
///
/// Control symbol visibility for optimization.
///
/// # Examples
///
/// ```verum
/// @visibility(hidden)
/// fn internal_only() { /* Can be optimized more aggressively */ }
///
/// @export("C")
/// @visibility(default)
/// fn public_api() { /* Preserve for external linkage */ }
/// ```
///
/// Target feature attribute enabling specific CPU instructions (SSE, AVX, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SymbolVisibility {
    /// Hidden - not exported, can be optimized aggressively
    Hidden,
    /// Default - normal visibility
    Default,
    /// Protected - visible to subclasses only
    Protected,
}

impl SymbolVisibility {
    pub fn from_str(s: &str) -> Maybe<Self> {
        match s {
            "hidden" => Maybe::Some(SymbolVisibility::Hidden),
            "default" => Maybe::Some(SymbolVisibility::Default),
            "protected" => Maybe::Some(SymbolVisibility::Protected),
            _ => Maybe::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityAttr {
    pub visibility: SymbolVisibility,
    pub span: Span,
}

impl VisibilityAttr {
    pub fn new(visibility: SymbolVisibility, span: Span) -> Self {
        Self { visibility, span }
    }

    pub fn hidden(span: Span) -> Self {
        Self::new(SymbolVisibility::Hidden, span)
    }
}

impl Spanned for VisibilityAttr {
    fn span(&self) -> Span {
        self.span
    }
}

// ============================================================================
// Linker Control Attributes (Phase 6)
// ============================================================================

/// Alias attribute: @alias(target)
///
/// Create a symbol alias that refers to another symbol.
/// The alias has the same address as the target symbol.
///
/// # Examples
///
/// ```verum
/// @export("C")
/// fn original_function() { }
///
/// @alias(original_function)
/// @export("C")
/// fn legacy_name();  // Points to original_function
///
/// @alias("malloc")
/// fn verum_alloc(size: USize) -> *mut U8;
/// ```
///
/// # Use Cases
///
/// - Backwards compatibility: provide old names for renamed functions
/// - ABI compatibility: create C-compatible aliases for Verum functions
/// - Symbol versioning: multiple entry points to same implementation
///
/// Symbol aliasing attribute for providing alternative names to exported symbols.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasAttr {
    /// Target symbol name that this alias points to.
    /// Can be a function/static name or a string literal for external symbols.
    pub target: Text,
    /// Source location.
    pub span: Span,
}

impl AliasAttr {
    pub fn new(target: Text, span: Span) -> Self {
        Self { target, span }
    }
}

impl Spanned for AliasAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Weak attribute: @weak
///
/// Mark a symbol as weak, allowing it to be overridden by a strong definition.
/// If no strong definition exists, the weak symbol is used.
///
/// # Examples
///
/// ```verum
/// // Weak default implementation - can be overridden
/// @weak
/// fn panic_handler(info: &PanicInfo) {
///     print("Default panic handler");
/// }
///
/// // Weak static - can be overridden by linker script or other object
/// @weak
/// static DEFAULT_STACK_SIZE: USize = 1024 * 1024;
/// ```
///
/// # Semantics
///
/// - Weak symbols have lower precedence than strong (normal) symbols
/// - If multiple weak definitions exist, one is chosen arbitrarily
/// - Useful for providing default implementations that can be customized
/// - Commonly used in embedded/bare-metal for interrupt handlers
///
/// Weak symbol attribute: symbol can be overridden by a strong definition at link time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeakAttr {
    /// Source location.
    pub span: Span,
}

impl WeakAttr {
    pub fn new(span: Span) -> Self {
        Self { span }
    }
}

impl Spanned for WeakAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Symbol linkage kind for fine-grained control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum LinkageKind {
    /// External linkage - visible outside the module (default for pub).
    #[default]
    External,
    /// Internal linkage - visible only within the module (.o file).
    Internal,
    /// Private linkage - visible only within the current translation unit.
    Private,
    /// Weak linkage - can be overridden by strong definition.
    Weak,
    /// Linkonce - merged if duplicated, discarded if unused.
    Linkonce,
    /// Linkonce ODR - like linkonce but for inlined code (C++ inline).
    LinkonceOdr,
    /// Common linkage - tentative definition, merged by linker.
    Common,
    /// Available externally - for optimization only, not emitted.
    AvailableExternally,
}

impl LinkageKind {
    pub fn from_str(s: &str) -> Maybe<Self> {
        match s {
            "external" => Maybe::Some(LinkageKind::External),
            "internal" => Maybe::Some(LinkageKind::Internal),
            "private" => Maybe::Some(LinkageKind::Private),
            "weak" => Maybe::Some(LinkageKind::Weak),
            "linkonce" => Maybe::Some(LinkageKind::Linkonce),
            "linkonce_odr" => Maybe::Some(LinkageKind::LinkonceOdr),
            "common" => Maybe::Some(LinkageKind::Common),
            "available_externally" => Maybe::Some(LinkageKind::AvailableExternally),
            _ => Maybe::None,
        }
    }

    /// Returns true if this linkage allows multiple definitions.
    pub fn allows_multiple_definitions(&self) -> bool {
        matches!(self,
            LinkageKind::Weak |
            LinkageKind::Linkonce |
            LinkageKind::LinkonceOdr |
            LinkageKind::Common
        )
    }
}

/// Linkage attribute: @linkage(kind)
///
/// Explicitly control symbol linkage for advanced linking scenarios.
///
/// # Examples
///
/// ```verum
/// @linkage(internal)
/// fn module_private_helper() { }
///
/// @linkage(linkonce_odr)
/// fn generic_instantiation<T>(x: T) -> T { x }
///
/// @linkage(common)
/// static mut UNINITIALIZED_GLOBAL: I32;
/// ```
///
/// Linkage control attribute for specifying symbol linkage (internal, external, weak).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkageAttr {
    pub kind: LinkageKind,
    pub span: Span,
}

impl LinkageAttr {
    pub fn new(kind: LinkageKind, span: Span) -> Self {
        Self { kind, span }
    }

    pub fn external(span: Span) -> Self {
        Self::new(LinkageKind::External, span)
    }

    pub fn internal(span: Span) -> Self {
        Self::new(LinkageKind::Internal, span)
    }

    pub fn weak(span: Span) -> Self {
        Self::new(LinkageKind::Weak, span)
    }
}

impl Spanned for LinkageAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Init priority attribute: @init_priority(N)
///
/// Control initialization order for static constructors.
/// Lower numbers run first. Valid range is 101-65535.
/// (0-100 are reserved for system use.)
///
/// # Examples
///
/// ```verum
/// @init_priority(200)
/// static EARLY_INIT: Lazy<Database> = Lazy::new(|| init_database());
///
/// @init_priority(500)
/// static LATE_INIT: Lazy<Cache> = Lazy::new(|| init_cache());
/// ```
///
/// # Implementation
///
/// Maps to `.init_array` section with priority on ELF platforms.
/// On other platforms, uses constructor attribute with priority.
///
/// Init section attribute: function runs during program initialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitPriorityAttr {
    /// Priority value (101-65535, lower runs first).
    pub priority: u32,
    /// Source location.
    pub span: Span,
}

impl InitPriorityAttr {
    pub fn new(priority: u32, span: Span) -> Self {
        Self { priority, span }
    }

    /// Check if priority is in valid range.
    pub fn is_valid_priority(&self) -> bool {
        self.priority >= 101 && self.priority <= 65535
    }
}

impl Spanned for InitPriorityAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Section attribute: @section(name)
///
/// Place a function or static in a specific linker section.
///
/// # Examples
///
/// ```verum
/// @section(".text.hot")
/// fn hot_path() { }
///
/// @section(".rodata.config")
/// static CONFIG: Config = Config::default();
///
/// @section(".bss.large")
/// static mut BUFFER: [U8; 1024 * 1024] = [0; 1024 * 1024];
/// ```
///
/// Section control attribute: places symbol in a specific linker section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionAttr {
    /// Section name (e.g., ".text.hot", ".rodata.config").
    pub name: Text,
    /// Source location.
    pub span: Span,
}

impl SectionAttr {
    pub fn new(name: Text, span: Span) -> Self {
        Self { name, span }
    }
}

impl Spanned for SectionAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Export attribute: @export(abi)
///
/// Export a symbol with the specified ABI for FFI.
///
/// # Examples
///
/// ```verum
/// @export("C")
/// fn verum_init() { }
///
/// @export("C", name = "my_custom_name")
/// fn internal_name() { }
///
/// @export("stdcall")
/// fn win32_callback(hwnd: *const (), msg: U32) -> I32 { 0 }
/// ```
///
/// Export visibility control for shared library symbol tables.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportAttr {
    /// ABI name (e.g., "C", "stdcall", "fastcall").
    pub abi: Text,
    /// Optional exported symbol name (defaults to function name).
    pub export_name: Maybe<Text>,
    /// Source location.
    pub span: Span,
}

impl ExportAttr {
    pub fn new(abi: Text, span: Span) -> Self {
        Self {
            abi,
            export_name: Maybe::None,
            span,
        }
    }

    pub fn with_name(abi: Text, name: Text, span: Span) -> Self {
        Self {
            abi,
            export_name: Maybe::Some(name),
            span,
        }
    }

    pub fn c_abi(span: Span) -> Self {
        Self::new(Text::from("C"), span)
    }
}

impl Spanned for ExportAttr {
    fn span(&self) -> Span {
        self.span
    }
}

// ============================================================================
// Additional Linker Control Attributes
// ============================================================================

/// Naked attribute: @naked
///
/// Create a function with no prologue or epilogue. Only inline assembly
/// is allowed in the function body.
///
/// # Examples
///
/// ```verum
/// @naked
/// @no_mangle
/// fn _start() -> ! {
///     @asm(
///         "mov rdi, rsp",
///         "call main",
///         options(noreturn),
///     );
/// }
///
/// @naked
/// fn context_switch(old: *mut Context, new: *const Context) {
///     @asm(
///         "mov [rdi], rsp",
///         "mov rsp, [rsi]",
///         "ret",
///     );
/// }
/// ```
///
/// # Safety
///
/// Naked functions are inherently unsafe and require manual stack management.
/// The compiler will not generate any code except for inline assembly.
///
/// Naked function attribute: no prologue/epilogue, body must be inline assembly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NakedAttr {
    /// Source location.
    pub span: Span,
}

impl NakedAttr {
    pub fn new(span: Span) -> Self {
        Self { span }
    }
}

impl Spanned for NakedAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Link name attribute: @link_name(name)
///
/// Override the symbol name used by the linker.
///
/// # Examples
///
/// ```verum
/// @link_name("memcpy")
/// @extern("C")
/// fn verum_memcpy(dst: *mut Byte, src: *const Byte, n: USize) -> *mut Byte;
///
/// @link_name("__verum_runtime_init")
/// fn runtime_init() { }
/// ```
///
/// Used symbol attribute: prevents linker from stripping this symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkNameAttr {
    /// The symbol name to use for linking.
    pub name: Text,
    /// Source location.
    pub span: Span,
}

impl LinkNameAttr {
    pub fn new(name: Text, span: Span) -> Self {
        Self { name, span }
    }
}

impl Spanned for LinkNameAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// No-return attribute: @noreturn
///
/// Mark a function as never returning (diverging function).
///
/// # Examples
///
/// ```verum
/// @noreturn
/// fn panic(msg: Text) -> ! {
///     print_error(msg);
///     abort();
/// }
///
/// @noreturn
/// @export("C")
/// fn abort_handler() -> ! {
///     loop { @asm("hlt"); }
/// }
/// ```
///
/// Naked function: compiler generates no prologue/epilogue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoReturnAttr {
    /// Source location.
    pub span: Span,
}

impl NoReturnAttr {
    pub fn new(span: Span) -> Self {
        Self { span }
    }
}

impl Spanned for NoReturnAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// No-mangle attribute: @no_mangle
///
/// Prevent name mangling for a symbol.
///
/// # Examples
///
/// ```verum
/// @no_mangle
/// @export("C")
/// fn verum_malloc(size: USize) -> *mut Byte { }
/// ```
///
/// Used attribute: prevents linker dead-stripping of this symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoMangleAttr {
    /// Source location.
    pub span: Span,
}

impl NoMangleAttr {
    pub fn new(span: Span) -> Self {
        Self { span }
    }
}

impl Spanned for NoMangleAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// LLVM-only attribute: @llvm_only or @llvm_only(reason = "...")
///
/// Mark a function as requiring LLVM AOT compilation.
/// Functions with this attribute cannot execute in the VBC interpreter.
///
/// Use cases:
/// - Functions containing inline assembly (`asm { }`)
/// - Functions accessing privileged CPU registers (ring 0)
/// - Functions using platform-specific hardware features
///
/// # Examples
///
/// ```verum
/// @llvm_only(reason = "inline assembly")
/// pub unsafe fn rdmsr(msr: UInt32) -> UInt64;
///
/// @llvm_only(reason = "privileged CPU mode")
/// pub unsafe fn write_cr3(value: UInt64);
///
/// @llvm_only  // short form
/// pub unsafe fn inb(port: UInt16) -> UInt8;
/// ```
///
/// # Compiler Behavior
///
/// In VBC interpreter mode, calling an @llvm_only function produces:
/// "Cannot execute @llvm_only intrinsic '{name}' in interpreter mode. Use --tier aot."
///
/// In AOT compilation mode, the attribute is informational only.
///
/// Extended attribute position declarations for all syntactic elements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlvmOnlyAttr {
    /// Optional reason why this function is LLVM-only
    pub reason: Maybe<Text>,
    /// Source location.
    pub span: Span,
}

impl LlvmOnlyAttr {
    /// Create an @llvm_only attribute without a reason.
    pub fn new(span: Span) -> Self {
        Self {
            reason: Maybe::None,
            span,
        }
    }

    /// Create an @llvm_only attribute with a reason.
    pub fn with_reason(reason: Text, span: Span) -> Self {
        Self {
            reason: Maybe::Some(reason),
            span,
        }
    }
}

impl Spanned for LlvmOnlyAttr {
    fn span(&self) -> Span {
        self.span
    }
}

// ============================================================================
// Formal Verification Attributes
// ============================================================================

/// Ghost attribute: @ghost
///
/// Mark a field or variable as ghost state for formal verification.
/// Ghost state exists only at verification time and is erased at runtime.
///
/// # Examples
///
/// ```verum
/// type VerifiedStack<T> is {
///     data: List<T>,
///     @ghost abstract_state: Seq<T>,  // Only for proofs
///     @ghost count_history: List<USize>,
/// };
///
/// implement<T> VerifiedStack<T> {
///     @ensures(self.abstract_state == old(self.abstract_state).append(item))
///     fn push(&mut self, item: T) {
///         self.data.push(item);
///     }
/// }
/// ```
///
/// MMIO/register hardware access attribute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhostAttr {
    /// Source location.
    pub span: Span,
}

impl GhostAttr {
    pub fn new(span: Span) -> Self {
        Self { span }
    }
}

impl Spanned for GhostAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Requires attribute: @requires(condition)
///
/// Specify a precondition for a function. The condition must hold
/// when the function is called.
///
/// # Examples
///
/// ```verum
/// @requires(index < array.len())
/// fn get_unchecked<T>(array: &[T], index: USize) -> &T {
///     // Safe because precondition guarantees bounds
///     &array[index]
/// }
///
/// @requires(x > 0)
/// @requires(y != 0)
/// fn safe_divide(x: Int, y: Int) -> Int {
///     x / y
/// }
/// ```
///
/// MMIO/register hardware access attribute.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequiresAttr {
    /// The precondition expression.
    pub condition: Box<crate::expr::Expr>,
    /// Source location.
    pub span: Span,
}

impl RequiresAttr {
    pub fn new(condition: crate::expr::Expr, span: Span) -> Self {
        Self {
            condition: Box::new(condition),
            span,
        }
    }
}

impl Spanned for RequiresAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Ensures attribute: @ensures(condition)
///
/// Specify a postcondition for a function. The condition must hold
/// when the function returns.
///
/// # Examples
///
/// ```verum
/// @ensures(result >= 0)
/// fn abs(x: Int) -> Int {
///     if x < 0 { -x } else { x }
/// }
///
/// @ensures(result.len() == old(self.len()) + 1)
/// fn push<T>(&mut self, item: T) {
///     // ...
/// }
/// ```
///
/// # Special Variables
///
/// - `result`: The return value of the function
/// - `old(expr)`: The value of expr at function entry
///
/// MMIO/register hardware access attribute.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnsuresAttr {
    /// The postcondition expression.
    pub condition: Box<crate::expr::Expr>,
    /// Source location.
    pub span: Span,
}

impl EnsuresAttr {
    pub fn new(condition: crate::expr::Expr, span: Span) -> Self {
        Self {
            condition: Box::new(condition),
            span,
        }
    }
}

impl Spanned for EnsuresAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Invariant attribute: @invariant(condition)
///
/// Specify an invariant for loops or types.
///
/// # Examples
///
/// ```verum
/// fn binary_search<T: Ord>(arr: &[T], target: &T) -> Maybe<USize> {
///     let mut low = 0;
///     let mut high = arr.len();
///
///     @invariant(low <= high)
///     @invariant(high <= arr.len())
///     while low < high {
///         let mid = low + (high - low) / 2;
///         match arr[mid].cmp(target) {
///             Less => low = mid + 1,
///             Greater => high = mid,
///             Equal => return Some(mid),
///         }
///     }
///     None
/// }
/// ```
///
/// DMA buffer attribute for hardware direct memory access.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InvariantAttr {
    /// The invariant expression.
    pub condition: Box<crate::expr::Expr>,
    /// Source location.
    pub span: Span,
}

impl InvariantAttr {
    pub fn new(condition: crate::expr::Expr, span: Span) -> Self {
        Self {
            condition: Box::new(condition),
            span,
        }
    }
}

impl Spanned for InvariantAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Used attribute: @used
///
/// Prevent dead code elimination for a static value.
///
/// # Example
///
/// ```verum
/// @used
/// static KEEP_THIS: [u8; 1024] = [0; 1024]
/// ```
///
/// Optimization barrier preventing reordering across this point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsedAttr {
    pub span: Span,
}

impl UsedAttr {
    pub fn new(span: Span) -> Self {
        Self { span }
    }
}

impl Spanned for UsedAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Optimization barrier: @optimize_barrier
///
/// Prevent optimization across this point.
///
/// # Example
///
/// ```verum
/// fn timing_attack_resistant(secret: &[u8], input: &[u8]) -> bool {
///     let mut result = true
///     @optimize_barrier
///     for i in 0..secret.len() {
///         result &= constant_time_eq(secret[i], input[i])
///     }
///     @optimize_barrier
///     result
/// }
/// ```
///
/// Optimization barrier preventing reordering across this point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptimizeBarrierAttr {
    pub span: Span,
}

impl OptimizeBarrierAttr {
    pub fn new(span: Span) -> Self {
        Self { span }
    }
}

impl Spanned for OptimizeBarrierAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Black box: @black_box
///
/// Prevent optimization of value - useful for benchmarks.
///
/// # Example
///
/// ```verum
/// fn benchmark<T>(compute: fn() -> T) {
///     let start = time.now()
///     let result = @black_box(compute())
///     let elapsed = time.now() - start
///     println!("Time: {elapsed}, Result: {result}")
/// }
/// ```
///
/// Optimization barrier preventing reordering across this point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlackBoxAttr {
    pub span: Span,
}

impl BlackBoxAttr {
    pub fn new(span: Span) -> Self {
        Self { span }
    }
}

impl Spanned for BlackBoxAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Performance contract: @constant_time, @max_time, @max_memory
///
/// Declare performance guarantees.
///
/// # Examples
///
/// ```verum
/// @constant_time
/// fn crypto_operation(key: &[u8], data: &[u8]) {
///     /* Compiler ensures no timing variations */
/// }
///
/// @max_time(100_us)
/// fn realtime_response() { /* Compiler warns if might exceed */ }
///
/// @max_memory(1_KB)
/// fn embedded_function() { /* Compiler enforces memory limit */ }
/// ```
///
/// IVDEP (ignore vector dependencies) hint for loop vectorization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PerformanceContract {
    /// @constant_time - guarantee constant time execution
    ConstantTime { span: Span },
    /// @max_time(duration) - maximum execution time in microseconds
    MaxTime { microseconds: u64, span: Span },
    /// @max_memory(bytes) - maximum memory usage
    MaxMemory { bytes: u64, span: Span },
}

impl Spanned for PerformanceContract {
    fn span(&self) -> Span {
        match self {
            PerformanceContract::ConstantTime { span } => *span,
            PerformanceContract::MaxTime { span, .. } => *span,
            PerformanceContract::MaxMemory { span, .. } => *span,
        }
    }
}

/// Memory access pattern hint: @access_pattern
///
/// Hint the expected memory access pattern for optimization.
///
/// # Examples
///
/// ```verum
/// @access_pattern(sequential)
/// fn scan_array(data: &[i32]) -> i32 { data.iter().sum() }
///
/// @access_pattern(random)
/// fn hash_lookup(table: &Map<K, V>, keys: &[K]) { /* ... */ }
///
/// @access_pattern(streaming)
/// fn process_large_file(file: &File) {
///     for chunk in file.chunks(4096) {
///         @non_temporal_store(process_chunk(chunk))
///     }
/// }
/// ```
///
/// Black box optimization barrier preventing constant folding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AccessPattern {
    Sequential,
    Random,
    Streaming,
}

impl AccessPattern {
    pub fn from_str(s: &str) -> Maybe<Self> {
        match s {
            "sequential" => Maybe::Some(AccessPattern::Sequential),
            "random" => Maybe::Some(AccessPattern::Random),
            "streaming" => Maybe::Some(AccessPattern::Streaming),
            _ => Maybe::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessPatternAttr {
    pub pattern: AccessPattern,
    pub span: Span,
}

impl AccessPatternAttr {
    pub fn new(pattern: AccessPattern, span: Span) -> Self {
        Self { pattern, span }
    }
}

impl Spanned for AccessPatternAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Repr attribute: @repr(packed|C|cache_optimal)
///
/// Control type memory layout.
///
/// # Examples
///
/// ```verum
/// @repr(packed)
/// type CompactStruct is { a: u8, b: u32, c: u16 }
///
/// @repr(C)
/// type CCompatible is { x: i32, y: i32 }
///
/// @repr(cache_optimal)
/// type CacheFriendly is { hot_field1: u64, hot_field2: u64 }
/// ```
///
/// Memory optimization hint for cache prefetching or alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Repr {
    /// @repr(packed) - no padding
    Packed,
    /// @repr(C) - C-compatible layout
    C,
    /// @repr(cache_optimal) - optimize for cache
    CacheOptimal,
    /// @repr(transparent) - same layout as single field
    Transparent,
    /// @repr(simd) - SIMD vector type layout
    /// Read access to memory-mapped hardware register.
    /// Ensures alignment and layout suitable for SIMD operations.
    /// Example: @repr(simd) type Vec4f is [Float32; 4];
    Simd,
    /// @repr(simd_mask) - SIMD mask type layout
    /// Read-write access to memory-mapped hardware register.
    /// Boolean mask type for predicated SIMD operations.
    /// Example: @repr(simd_mask) type Mask4 is [Bool; 4];
    SimdMask,
}

impl Repr {
    pub fn from_str(s: &str) -> Maybe<Self> {
        match s {
            "packed" => Maybe::Some(Repr::Packed),
            "C" | "c" => Maybe::Some(Repr::C),
            "cache_optimal" => Maybe::Some(Repr::CacheOptimal),
            "transparent" => Maybe::Some(Repr::Transparent),
            "simd" => Maybe::Some(Repr::Simd),
            "simd_mask" => Maybe::Some(Repr::SimdMask),
            _ => Maybe::None,
        }
    }

    /// Check if this repr is for SIMD vector types
    pub fn is_simd(&self) -> bool {
        matches!(self, Repr::Simd | Repr::SimdMask)
    }

    /// Check if this repr is for SIMD mask types
    pub fn is_simd_mask(&self) -> bool {
        matches!(self, Repr::SimdMask)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReprAttr {
    pub repr: Repr,
    pub span: Span,
}

impl ReprAttr {
    pub fn new(repr: Repr, span: Span) -> Self {
        Self { repr, span }
    }
}

impl Spanned for ReprAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Differentiable attribute: @differentiable(wrt = "params")
///
/// Enable automatic differentiation for tensor functions.
///
/// # Example
///
/// ```verum
/// @differentiable(wrt = "weights, bias")
/// fn dense_layer<N: meta usize, M: meta usize>(
///     input: &Tensor<f32, [N]>,
///     weights: &Tensor<f32, [N, M]>,
///     bias: &Tensor<f32, [M]>
/// ) -> Tensor<f32, [M]>
///     where N > 0 && M > 0
/// {
///     matmul(input, weights) + bias
/// }
/// ```
///
/// Memory representation control: C layout, packed, aligned, transparent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DifferentiableAttr {
    /// Parameters to differentiate with respect to (comma-separated)
    pub wrt: List<Text>,
    /// Differentiation mode: "forward" or "reverse" (default: "reverse")
    pub mode: Text,
    /// Optional custom VJP function name
    pub custom_vjp: Maybe<Text>,
    pub span: Span,
}

impl DifferentiableAttr {
    pub fn new(wrt: List<Text>, span: Span) -> Self {
        Self {
            wrt,
            mode: Text::from("reverse"),
            custom_vjp: Maybe::None,
            span,
        }
    }

    pub fn with_mode(mut self, mode: Text) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_custom_vjp(mut self, vjp: Text) -> Self {
        self.custom_vjp = Maybe::Some(vjp);
        self
    }
}

impl Spanned for DifferentiableAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Assume attribute: @assume(condition)
///
/// Provide optimization hints to the compiler.
///
/// # Examples
///
/// ```verum
/// @assume(data.len() % 8 == 0)  // Help optimizer vectorize
/// fn simd_compute(data: &[f32]) -> f32 { /* ... */ }
///
/// @assume_no_alias  // Assert no pointer aliasing
/// fn fast_copy(dst: &mut [u8], src: &[u8]) { /* ... */ }
///
/// @assume_aligned(16)  // Assert alignment
/// fn aligned_access(ptr: *const f32) { /* ... */ }
/// ```
///
/// Alignment attribute for SIMD-friendly data layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AssumeAttr {
    /// @assume(expr) - assume expression is true
    Condition {
        condition: crate::expr::Expr,
        span: Span,
    },
    /// @assume_no_alias - assume no pointer aliasing
    NoAlias { span: Span },
    /// @assume_no_overflow - assume arithmetic won't overflow
    NoOverflow { span: Span },
    /// @assume_aligned(N) - assume alignment
    Aligned { alignment: u32, span: Span },
}

impl Default for AssumeAttr {
    fn default() -> Self {
        AssumeAttr::NoAlias {
            span: Span::default(),
        }
    }
}

impl Spanned for AssumeAttr {
    fn span(&self) -> Span {
        match self {
            AssumeAttr::Condition { span, .. } => *span,
            AssumeAttr::NoAlias { span } => *span,
            AssumeAttr::NoOverflow { span } => *span,
            AssumeAttr::Aligned { span, .. } => *span,
        }
    }
}

/// CPU dispatch attribute: @cpu_dispatch
///
/// Multi-versioning for different CPUs.
///
/// # Example
///
/// ```verum
/// @cpu_dispatch
/// fn compute(data: &[f64]) -> f64 {
///     @target_cpu("x86-64-v4")
///     default "avx512" { compute_avx512(data) }
///
///     @target_cpu("x86-64-v3")
///     default "avx2" { compute_avx2(data) }
///
///     @target_cpu("generic")
///     default "generic" { compute_scalar(data) }
/// }
/// ```
///
/// Parallel execution hint for independent loop iterations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuDispatchAttr {
    pub span: Span,
}

impl CpuDispatchAttr {
    pub fn new(span: Span) -> Self {
        Self { span }
    }
}

impl Spanned for CpuDispatchAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Multi-version attribute: @multiversion(...)
///
/// Generates multiple function versions optimized for different CPU feature sets.
/// The runtime automatically dispatches to the best available version.
///
/// # Syntax
///
/// ```verum
/// @multiversion(
///     avx512 = "avx512f,avx512vl",
///     avx2 = "avx2,fma",
///     sse4 = "sse4.2",
///     default = "baseline"
/// )
/// fn dot_product(a: &Vec<Float32, 8>, b: &Vec<Float32, 8>) -> Float32 {
///     // Implementation works for all targets
///     a.dot(b)
/// }
/// ```
///
/// # Behavior
///
/// - Generates separate function versions for each specified target
/// - Runtime dispatcher selects best version based on detected CPU features
/// - Dispatch overhead is typically < 1ns (indirect call through resolved pointer)
/// - Compatible with SIMD Vec<T, N> operations for automatic vectorization
///
/// # Targets
///
/// Common target feature sets:
/// - `avx512f,avx512vl` - AVX-512 Foundation + Vector Length
/// - `avx2,fma` - AVX2 with fused multiply-add
/// - `sse4.2` - SSE 4.2 baseline for x86_64
/// - `neon` - ARM NEON SIMD
/// - `sve` - ARM Scalable Vector Extension
///
/// # Example with explicit implementations
///
/// ```verum
/// @multiversion(
///     avx512 = "avx512f",
///     avx2 = "avx2",
///     default = "sse4.2"
/// )
/// fn sum_array(data: &List<Float64>) -> Float64 {
///     let mut acc = 0.0;
///     for x in data {
///         acc += x;
///     }
///     acc
/// }
/// ```
///
/// Multi-version dispatch: generate multiple function versions for different CPU features.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiversionAttr {
    /// Named variants mapping name to feature requirements
    /// e.g., "avx512" -> "avx512f,avx512vl"
    pub variants: List<MultiversionVariant>,
    pub span: Span,
}

/// A single variant in @multiversion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiversionVariant {
    /// Variant name (e.g., "avx512", "default")
    pub name: Text,
    /// Required CPU features (e.g., "avx512f,avx512vl")
    pub features: Text,
    pub span: Span,
}

impl MultiversionAttr {
    pub fn new(variants: List<MultiversionVariant>, span: Span) -> Self {
        Self { variants, span }
    }

    /// Get a variant by name.
    pub fn get_variant(&self, name: &str) -> Maybe<&MultiversionVariant> {
        self.variants.iter().find(|v| v.name.as_str() == name)
    }

    /// Get the default variant if present.
    pub fn default_variant(&self) -> Maybe<&MultiversionVariant> {
        self.get_variant("default")
    }

    /// Check if a specific variant exists.
    pub fn has_variant(&self, name: &str) -> bool {
        self.variants.iter().any(|v| v.name.as_str() == name)
    }

    /// Get all variant names.
    pub fn variant_names(&self) -> List<&Text> {
        self.variants.iter().map(|v| &v.name).collect()
    }
}

impl MultiversionVariant {
    pub fn new(name: impl Into<Text>, features: impl Into<Text>, span: Span) -> Self {
        Self {
            name: name.into(),
            features: features.into(),
            span,
        }
    }

    /// Parse the feature list into individual features.
    pub fn feature_list(&self) -> List<&str> {
        self.features.as_str().split(',').map(|s| s.trim()).collect()
    }
}

impl Spanned for MultiversionAttr {
    fn span(&self) -> Span {
        self.span
    }
}

impl Spanned for MultiversionVariant {
    fn span(&self) -> Span {
        self.span
    }
}

/// Parallel attribute for loop parallelization: @parallel
///
/// Mark a loop for parallel execution.
///
/// # Example
///
/// ```verum
/// @reduce(+)
/// fn parallel_sum(data: &[i32]) -> i32 {
///     @parallel
///     @simd
///     for x in data { accumulate(x) }
/// }
/// ```
///
/// Cache prefetch hint for data locality optimization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelAttr {
    pub span: Span,
}

impl ParallelAttr {
    pub fn new(span: Span) -> Self {
        Self { span }
    }
}

impl Spanned for ParallelAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Reduce attribute: @reduce(op)
///
/// Specify reduction operation for parallel loops.
///
/// Cache prefetch hint for data locality optimization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReductionOp {
    Add,      // +
    Multiply, // *
    Min,      // min
    Max,      // max
    BitAnd,   // &
    BitOr,    // |
    BitXor,   // ^
    LogicAnd, // &&
    LogicOr,  // ||
}

impl ReductionOp {
    pub fn from_str(s: &str) -> Maybe<Self> {
        match s {
            "+" | "add" => Maybe::Some(ReductionOp::Add),
            "*" | "mul" | "multiply" => Maybe::Some(ReductionOp::Multiply),
            "min" => Maybe::Some(ReductionOp::Min),
            "max" => Maybe::Some(ReductionOp::Max),
            "&" | "bitand" => Maybe::Some(ReductionOp::BitAnd),
            "|" | "bitor" => Maybe::Some(ReductionOp::BitOr),
            "^" | "bitxor" => Maybe::Some(ReductionOp::BitXor),
            "&&" | "and" => Maybe::Some(ReductionOp::LogicAnd),
            "||" | "or" => Maybe::Some(ReductionOp::LogicOr),
            _ => Maybe::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReduceAttr {
    pub op: ReductionOp,
    pub span: Span,
}

impl ReduceAttr {
    pub fn new(op: ReductionOp, span: Span) -> Self {
        Self { op, span }
    }
}

impl Spanned for ReduceAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// No-alias attribute: @no_alias
///
/// Assert that pointers in a loop don't alias.
///
/// # Example
///
/// ```verum
/// @no_alias  // Assert no pointer aliasing
/// for k in 0..a.cols { result[i][j] += a[i][k] * b[k][j] }
/// ```
///
/// Loop optimization hint controlling unrolling behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoAliasAttr {
    pub span: Span,
}

impl NoAliasAttr {
    pub fn new(span: Span) -> Self {
        Self { span }
    }
}

impl Spanned for NoAliasAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Independent vector dependency assertion: @ivdep
///
/// Ignore vector dependencies in loop.
///
/// # Example
///
/// ```verum
/// @simd
/// @ivdep  // Ignore vector dependencies
/// for i in 0..src.rows {
///     dst[j][i] = src[i][j]
/// }
/// ```
///
/// Cache prefetch hint for data locality optimization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IvdepAttr {
    pub span: Span,
}

impl IvdepAttr {
    pub fn new(span: Span) -> Self {
        Self { span }
    }
}

impl Spanned for IvdepAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Unified optimization attribute enum for codegen
///
/// This enum represents all parsed optimization attributes in a unified form
/// that can be efficiently processed during code generation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OptimizationAttr {
    Inline(InlineAttr),
    Cold(ColdAttr),
    Hot(HotAttr),
    Optimize(OptimizeAttr),
    Vectorize(VectorizeAttr),
    Unroll(UnrollAttr),
    Prefetch(PrefetchAttr),
    Align(AlignAttr),
    Likelihood(LikelihoodAttr),
    Verify(VerifyAttr),
    TargetCpu(TargetCpuAttr),
    TargetFeature(TargetFeatureAttr),
    ConstEval(ConstEvalAttr),
    Pgo(PgoAttr),
    Lto(LtoAttr),
    Visibility(VisibilityAttr),
    Used(UsedAttr),
    OptimizeBarrier(OptimizeBarrierAttr),
    BlackBox(BlackBoxAttr),
    PerformanceContract(PerformanceContract),
    AccessPattern(AccessPatternAttr),
    Repr(ReprAttr),
    Differentiable(DifferentiableAttr),
    Assume(AssumeAttr),
    CpuDispatch(CpuDispatchAttr),
    Parallel(ParallelAttr),
    Reduce(ReduceAttr),
    NoAlias(NoAliasAttr),
    Ivdep(IvdepAttr),
}

impl Spanned for OptimizationAttr {
    fn span(&self) -> Span {
        match self {
            OptimizationAttr::Inline(a) => a.span(),
            OptimizationAttr::Cold(a) => a.span(),
            OptimizationAttr::Hot(a) => a.span(),
            OptimizationAttr::Optimize(a) => a.span(),
            OptimizationAttr::Vectorize(a) => a.span(),
            OptimizationAttr::Unroll(a) => a.span(),
            OptimizationAttr::Prefetch(a) => a.span(),
            OptimizationAttr::Align(a) => a.span(),
            OptimizationAttr::Likelihood(a) => a.span(),
            OptimizationAttr::Verify(a) => a.span(),
            OptimizationAttr::TargetCpu(a) => a.span(),
            OptimizationAttr::TargetFeature(a) => a.span(),
            OptimizationAttr::ConstEval(a) => a.span(),
            OptimizationAttr::Pgo(a) => a.span(),
            OptimizationAttr::Lto(a) => a.span(),
            OptimizationAttr::Visibility(a) => a.span(),
            OptimizationAttr::Used(a) => a.span(),
            OptimizationAttr::OptimizeBarrier(a) => a.span(),
            OptimizationAttr::BlackBox(a) => a.span(),
            OptimizationAttr::PerformanceContract(a) => a.span(),
            OptimizationAttr::AccessPattern(a) => a.span(),
            OptimizationAttr::Repr(a) => a.span(),
            OptimizationAttr::Differentiable(a) => a.span(),
            OptimizationAttr::Assume(a) => a.span(),
            OptimizationAttr::CpuDispatch(a) => a.span(),
            OptimizationAttr::Parallel(a) => a.span(),
            OptimizationAttr::Reduce(a) => a.span(),
            OptimizationAttr::NoAlias(a) => a.span(),
            OptimizationAttr::Ivdep(a) => a.span(),
        }
    }
}

// =============================================================================
// META-SYSTEM ATTRIBUTES
// Meta-system attributes for compile-time code generation.
// =============================================================================

/// Tagged literal attribute: @tagged_literal("tag")
///
/// Marks a meta function as a handler for tagged literals.
///
/// # Examples
///
/// ```verum
/// @tagged_literal("json")
/// meta fn json_literal(source: &str) -> JsonValue { ... }
///
/// // Usage:
/// let data = json#"{ \"key\": \"value\" }";
/// ```
///
/// Meta-system attributes for compile-time code generation. Section 10
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaggedLiteralAttr {
    /// The literal tag (e.g., "json", "img", "sql")
    pub tag: Text,
    /// Source location
    pub span: Span,
}

impl TaggedLiteralAttr {
    pub fn new(tag: Text, span: Span) -> Self {
        Self { tag, span }
    }
}

impl Spanned for TaggedLiteralAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Collection of optimization attributes for a function/type/loop
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OptimizationHints {
    pub inline: Maybe<InlineAttr>,
    pub cold: Maybe<ColdAttr>,
    pub hot: Maybe<HotAttr>,
    pub optimize: Maybe<OptimizeAttr>,
    pub vectorize: Maybe<VectorizeAttr>,
    pub unroll: Maybe<UnrollAttr>,
    pub prefetch: List<PrefetchAttr>,
    pub align: Maybe<AlignAttr>,
    pub likelihood: Maybe<LikelihoodAttr>,
    pub verify: Maybe<VerifyAttr>,
    pub target_cpu: Maybe<TargetCpuAttr>,
    pub target_feature: Maybe<TargetFeatureAttr>,
    pub const_eval: Maybe<ConstEvalAttr>,
    pub pgo: List<PgoAttr>,
    pub lto: Maybe<LtoAttr>,
    pub visibility: Maybe<VisibilityAttr>,
    pub used: bool,
    pub optimize_barrier: bool,
    pub black_box: bool,
    pub performance_contract: Maybe<PerformanceContract>,
    pub access_pattern: Maybe<AccessPatternAttr>,
    pub repr: Maybe<ReprAttr>,
    pub differentiable: Maybe<DifferentiableAttr>,
    pub assume: List<AssumeAttr>,
    pub cpu_dispatch: bool,
    pub parallel: bool,
    pub reduce: Maybe<ReduceAttr>,
    pub no_alias: bool,
    pub ivdep: bool,
}

impl OptimizationHints {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an optimization attribute to this collection
    pub fn add(&mut self, attr: OptimizationAttr) {
        match attr {
            OptimizationAttr::Inline(a) => self.inline = Maybe::Some(a),
            OptimizationAttr::Cold(a) => self.cold = Maybe::Some(a),
            OptimizationAttr::Hot(a) => self.hot = Maybe::Some(a),
            OptimizationAttr::Optimize(a) => self.optimize = Maybe::Some(a),
            OptimizationAttr::Vectorize(a) => self.vectorize = Maybe::Some(a),
            OptimizationAttr::Unroll(a) => self.unroll = Maybe::Some(a),
            OptimizationAttr::Prefetch(a) => self.prefetch.push(a),
            OptimizationAttr::Align(a) => self.align = Maybe::Some(a),
            OptimizationAttr::Likelihood(a) => self.likelihood = Maybe::Some(a),
            OptimizationAttr::Verify(a) => self.verify = Maybe::Some(a),
            OptimizationAttr::TargetCpu(a) => self.target_cpu = Maybe::Some(a),
            OptimizationAttr::TargetFeature(a) => self.target_feature = Maybe::Some(a),
            OptimizationAttr::ConstEval(a) => self.const_eval = Maybe::Some(a),
            OptimizationAttr::Pgo(a) => self.pgo.push(a),
            OptimizationAttr::Lto(a) => self.lto = Maybe::Some(a),
            OptimizationAttr::Visibility(a) => self.visibility = Maybe::Some(a),
            OptimizationAttr::Used(_) => self.used = true,
            OptimizationAttr::OptimizeBarrier(_) => self.optimize_barrier = true,
            OptimizationAttr::BlackBox(_) => self.black_box = true,
            OptimizationAttr::PerformanceContract(a) => self.performance_contract = Maybe::Some(a),
            OptimizationAttr::AccessPattern(a) => self.access_pattern = Maybe::Some(a),
            OptimizationAttr::Repr(a) => self.repr = Maybe::Some(a),
            OptimizationAttr::Differentiable(a) => self.differentiable = Maybe::Some(a),
            OptimizationAttr::Assume(a) => self.assume.push(a),
            OptimizationAttr::CpuDispatch(_) => self.cpu_dispatch = true,
            OptimizationAttr::Parallel(_) => self.parallel = true,
            OptimizationAttr::Reduce(a) => self.reduce = Maybe::Some(a),
            OptimizationAttr::NoAlias(_) => self.no_alias = true,
            OptimizationAttr::Ivdep(_) => self.ivdep = true,
        }
    }

    /// Check if function should be cold (deprioritized)
    pub fn is_cold(&self) -> bool {
        matches!(self.cold, Maybe::Some(_))
    }

    /// Check if function should be hot (prioritized)
    pub fn is_hot(&self) -> bool {
        matches!(self.hot, Maybe::Some(_))
    }

    /// Get inline mode (if any)
    pub fn get_inline_mode(&self) -> Maybe<InlineMode> {
        self.inline.as_ref().map(|attr| attr.mode)
    }

    /// Check if there are any optimization hints
    pub fn has_hints(&self) -> bool {
        matches!(self.inline, Maybe::Some(_))
            || matches!(self.cold, Maybe::Some(_))
            || matches!(self.hot, Maybe::Some(_))
            || matches!(self.optimize, Maybe::Some(_))
            || matches!(self.vectorize, Maybe::Some(_))
            || matches!(self.unroll, Maybe::Some(_))
            || !self.prefetch.is_empty()
            || matches!(self.align, Maybe::Some(_))
            || matches!(self.likelihood, Maybe::Some(_))
            || matches!(self.verify, Maybe::Some(_))
            || matches!(self.target_cpu, Maybe::Some(_))
            || matches!(self.target_feature, Maybe::Some(_))
            || matches!(self.const_eval, Maybe::Some(_))
            || !self.pgo.is_empty()
            || matches!(self.lto, Maybe::Some(_))
            || matches!(self.visibility, Maybe::Some(_))
            || self.used
            || self.optimize_barrier
            || self.black_box
            || matches!(self.performance_contract, Maybe::Some(_))
            || matches!(self.access_pattern, Maybe::Some(_))
            || matches!(self.repr, Maybe::Some(_))
            || matches!(self.differentiable, Maybe::Some(_))
            || !self.assume.is_empty()
            || self.cpu_dispatch
            || self.parallel
            || matches!(self.reduce, Maybe::Some(_))
            || self.no_alias
            || self.ivdep
    }
}

// =============================================================================
// TERMINATION VERIFICATION HINTS
// Gradual verification attributes for incremental safety assurance.
// =============================================================================

/// Collection of termination verification attributes for a function
///
/// This struct collects all termination-related attributes that can be
/// attached to a function for verification purposes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TerminationHints {
    /// The measure function for termination (@measure)
    pub measure: Maybe<MeasureAttr>,
    /// Decreasing expressions for loops (@decreases)
    pub decreases: List<DecreasesAttr>,
    /// Well-founded relation (@well_founded)
    pub well_founded: Maybe<WellFoundedAttr>,
    /// Termination proof reference (@termination_proof)
    pub termination_proof: Maybe<TerminationProofAttr>,
}

impl TerminationHints {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if any termination hints are present
    pub fn has_hints(&self) -> bool {
        matches!(self.measure, Maybe::Some(_))
            || !self.decreases.is_empty()
            || matches!(self.well_founded, Maybe::Some(_))
            || matches!(self.termination_proof, Maybe::Some(_))
    }

    /// Check if termination is fully specified (has measure + well-founded relation)
    pub fn is_termination_specified(&self) -> bool {
        matches!(self.measure, Maybe::Some(_))
    }

    /// Get the primary measure expression (if any)
    pub fn get_measure(&self) -> Maybe<&MeasureAttr> {
        self.measure.as_ref()
    }

    /// Add a measure attribute
    pub fn set_measure(&mut self, attr: MeasureAttr) {
        self.measure = Maybe::Some(attr);
    }

    /// Add a decreases attribute
    pub fn add_decreases(&mut self, attr: DecreasesAttr) {
        self.decreases.push(attr);
    }

    /// Set the well-founded relation
    pub fn set_well_founded(&mut self, attr: WellFoundedAttr) {
        self.well_founded = Maybe::Some(attr);
    }

    /// Set the termination proof reference
    pub fn set_termination_proof(&mut self, attr: TerminationProofAttr) {
        self.termination_proof = Maybe::Some(attr);
    }
}

/// Unified termination attribute enum for the parser/collector
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TerminationAttr {
    Measure(MeasureAttr),
    Decreases(DecreasesAttr),
    WellFounded(WellFoundedAttr),
    TerminationProof(TerminationProofAttr),
}

impl Spanned for TerminationAttr {
    fn span(&self) -> Span {
        match self {
            TerminationAttr::Measure(a) => a.span(),
            TerminationAttr::Decreases(a) => a.span(),
            TerminationAttr::WellFounded(a) => a.span(),
            TerminationAttr::TerminationProof(a) => a.span(),
        }
    }
}

// =============================================================================
// CONTEXT SYSTEM ATTRIBUTES
// Advanced context patterns: transforms, negative contexts, context polymorphism.
// =============================================================================

/// Transform attribute: @transform
///
/// Marks a function as a custom context transform that can be used in
/// `using [Context.transform_name()]` syntax. The function must:
/// 1. Take a context type as input
/// 2. Return a wrapper type implementing the context protocol
///
/// # Examples
///
/// ```verum
/// @transform
/// fn my_custom_transform<C: Database>(ctx: C) -> MyWrapper<C> {
///     MyWrapper::new(ctx)
/// }
///
/// // Usage in function signature:
/// fn batch_update() using [Database.my_custom_transform()] {
///     Database.execute("UPDATE ...");
/// }
/// ```
///
/// # Custom Transform with Arguments
///
/// ```verum
/// @transform
/// fn with_timeout<C: Database>(ctx: C, timeout: Duration) -> TimedWrapper<C> {
///     TimedWrapper::new(ctx, timeout)
/// }
///
/// // Usage:
/// fn query() using [Database.with_timeout(5_seconds)] {
///     Database.execute("SELECT ...");
/// }
/// ```
///
/// # Semantics
///
/// 1. Transform functions are registered in the TransformRegistry at compile time
/// 2. When a function uses `using [Context.transform()]`, the transform is applied
/// 3. Transforms can be composed: `using [Database.transactional().traced()]`
/// 4. Custom transforms must implement the ContextTransform protocol
///
/// # Specification
///
/// Context transformation attribute (e.g., .transactional(), .scoped()).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransformAttr {
    /// Optional transform name (defaults to function name)
    pub name: Maybe<Text>,
    /// The context protocol this transform applies to (inferred from first param)
    pub context_protocol: Maybe<Text>,
    /// Whether this transform is composable with other transforms
    pub composable: bool,
    /// Priority when multiple transforms apply (lower = applied first)
    pub priority: i32,
    /// Source location
    pub span: Span,
}

impl TransformAttr {
    /// Create a new transform attribute with default settings
    pub fn new(span: Span) -> Self {
        Self {
            name: Maybe::None,
            context_protocol: Maybe::None,
            composable: true,
            priority: 0,
            span,
        }
    }

    /// Create with explicit name
    pub fn with_name(name: Text, span: Span) -> Self {
        Self {
            name: Maybe::Some(name),
            context_protocol: Maybe::None,
            composable: true,
            priority: 0,
            span,
        }
    }

    /// Set the context protocol constraint
    pub fn for_protocol(mut self, protocol: Text) -> Self {
        self.context_protocol = Maybe::Some(protocol);
        self
    }

    /// Set priority (lower = applied first in chain)
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Mark as non-composable (cannot be chained with other transforms)
    pub fn non_composable(mut self) -> Self {
        self.composable = false;
        self
    }

    /// Get the transform name (or None if should use function name)
    pub fn get_name(&self) -> Maybe<&Text> {
        self.name.as_ref()
    }

    /// Check if this transform can be composed with others
    pub fn is_composable(&self) -> bool {
        self.composable
    }
}

impl Spanned for TransformAttr {
    fn span(&self) -> Span {
        self.span
    }
}

impl Default for TransformAttr {
    fn default() -> Self {
        Self::new(Span::default())
    }
}

// ============================================================================
// Dependency Injection Attributes
// ============================================================================
//
// These attributes enable compile-time dependency injection with zero runtime
// overhead for singleton scope. The DI system validates scope hierarchies and
// detects circular dependencies at compile time.
//
// Static dependency injection attributes: @injectable and @inject.

/// Dependency injection scope.
///
/// Defines the lifecycle of injectable services:
/// - Singleton: Created once, shared across all uses (0ns overhead)
/// - Request: Created per request scope (~3ns overhead)
/// - Transient: Created fresh on each injection (~8ns overhead)
///
/// # Scope Hierarchy
///
/// Scope hierarchy must be respected:
/// - Singleton cannot depend on Request or Transient
/// - Request can only depend on Singleton
/// - Transient can depend on Singleton or Request
///
/// # Specification
///
/// @inject attribute for field-level dependency injection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InjectionScope {
    /// Created once, shared across all uses (0ns overhead)
    Singleton,
    /// Created per request scope (~3ns overhead)
    Request,
    /// Created fresh on each injection (~8ns overhead)
    Transient,
}

impl InjectionScope {
    /// Get the scope from a string name
    pub fn from_str(s: &str) -> Maybe<Self> {
        match s {
            "Singleton" | "singleton" => Maybe::Some(InjectionScope::Singleton),
            "Request" | "request" => Maybe::Some(InjectionScope::Request),
            "Transient" | "transient" => Maybe::Some(InjectionScope::Transient),
            _ => Maybe::None,
        }
    }

    /// Get the string name of this scope
    pub fn as_str(&self) -> &'static str {
        match self {
            InjectionScope::Singleton => "Singleton",
            InjectionScope::Request => "Request",
            InjectionScope::Transient => "Transient",
        }
    }

    /// Check if this scope can depend on another scope
    ///
    /// Scope hierarchy: Singleton → Request → Transient
    /// Lower scopes cannot depend on higher scopes
    pub fn can_depend_on(&self, other: &InjectionScope) -> bool {
        use InjectionScope::*;
        match (self, other) {
            // Singleton can only depend on other singletons
            (Singleton, Singleton) => true,
            (Singleton, _) => false,
            // Request can depend on Singleton or Request
            (Request, Singleton) => true,
            (Request, Request) => true,
            (Request, Transient) => false,
            // Transient can depend on anything
            (Transient, _) => true,
        }
    }
}

/// Marks a type as injectable with dependency injection.
///
/// The `@injectable` attribute marks a type as a DI-managed service
/// with a specific lifecycle scope.
///
/// # Examples
///
/// ```verum
/// @injectable(Singleton)
/// type UserService is {
///     users: Map<Int, User>,
/// }
///
/// @injectable(Request)
/// type RequestHandler is {
///     user_service: UserService,
/// }
/// ```
///
/// # Specification
///
/// @injectable attribute for marking types as injectable with scope control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InjectableAttr {
    /// The injection scope for this type
    pub scope: InjectionScope,
    /// Source location
    pub span: Span,
}

impl InjectableAttr {
    /// Create a new injectable attribute with the given scope
    pub fn new(scope: InjectionScope, span: Span) -> Self {
        Self { scope, span }
    }

    /// Create a singleton-scoped injectable
    pub fn singleton(span: Span) -> Self {
        Self::new(InjectionScope::Singleton, span)
    }

    /// Create a request-scoped injectable
    pub fn request(span: Span) -> Self {
        Self::new(InjectionScope::Request, span)
    }

    /// Create a transient-scoped injectable
    pub fn transient(span: Span) -> Self {
        Self::new(InjectionScope::Transient, span)
    }
}

impl Spanned for InjectableAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Marks a function as the injection constructor.
///
/// The `@inject` attribute marks a function (typically `new`) as the
/// constructor to use when injecting dependencies. Dependencies are
/// automatically resolved from the DI container.
///
/// # Examples
///
/// ```verum
/// @injectable(Singleton)
/// type UserService is {
///     db: Database,
///     logger: Logger,
/// }
///
/// implement UserService {
///     @inject
///     fn new(db: Database, logger: Logger) -> Self {
///         Self { db, logger }
///     }
/// }
/// ```
///
/// # Semantics
///
/// When `@inject` is used:
/// 1. All parameters are resolved from the DI container
/// 2. Parameters can use context with `using [...]` clause
/// 3. Circular dependencies are detected at compile time
/// 4. Scope violations are compile-time errors
///
/// # Specification
///
/// Injection scope: singleton, scoped, or transient lifetime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InjectAttr {
    /// Source location
    pub span: Span,
}

impl InjectAttr {
    /// Create a new inject attribute
    pub fn new(span: Span) -> Self {
        Self { span }
    }
}

impl Spanned for InjectAttr {
    fn span(&self) -> Span {
        self.span
    }
}

impl Default for InjectAttr {
    fn default() -> Self {
        Self::new(Span::default())
    }
}

// =============================================================================
// BITFIELD ATTRIBUTES
// Low-level bit manipulation for hardware drivers and network protocols
// =============================================================================

/// Bit width attribute: @bits(N)
///
/// Specifies the number of bits a field occupies within a bitfield type.
/// Used in conjunction with `@bitfield` on the parent type.
///
/// # Examples
///
/// ```verum
/// @bitfield
/// type Flags is {
///     @bits(4) version: U8,    // 4 bits for version (0-15)
///     @bits(4) ihl: U8,        // 4 bits for IHL (0-15)
///     @bits(1) flag: Bool,     // 1 bit for boolean flag
///     @bits(15) data: U16,     // 15 bits for data
/// };
/// ```
///
/// # Validation
///
/// The type checker validates:
/// - Bit width is positive (> 0)
/// - Bit width does not exceed storage type's bit width
/// - For Bool fields, bit width must be 1
///
/// # Code Generation
///
/// The compiler generates accessor methods:
/// - `get_field() -> T`: Extract with proper masking/shifting
/// - `set_field(value: T)`: Update with bounds checking
/// - `with_field(value: T) -> Self`: Builder-style immutable update
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitsAttr {
    /// Number of bits this field occupies
    pub width: u32,
    /// Source location
    pub span: Span,
}

impl BitsAttr {
    /// Create a new bits attribute with the specified width.
    pub fn new(width: u32, span: Span) -> Self {
        Self { width, span }
    }

    /// Get the bit width value.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Check if this width is valid for a boolean field.
    pub fn is_valid_for_bool(&self) -> bool {
        self.width == 1
    }

    /// Check if this width is valid for the given storage type bit width.
    pub fn is_valid_for(&self, storage_bits: u32) -> bool {
        self.width > 0 && self.width <= storage_bits
    }
}

impl Spanned for BitsAttr {
    fn span(&self) -> Span {
        self.span
    }
}

impl std::fmt::Display for BitsAttr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "@bits({})", self.width)
    }
}

/// Bit offset attribute: @offset(N)
///
/// Specifies an explicit bit offset from the start of the container.
/// Optional - if omitted, fields are placed sequentially.
///
/// # Examples
///
/// ```verum
/// @bitfield
/// type SparseFlags is {
///     @bits(8) @offset(0)  low: U8,   // Bits 0-7
///     @bits(8) @offset(24) high: U8,  // Bits 24-31 (skip 8-23)
/// };
/// ```
///
/// # Use Cases
///
/// - Hardware registers with reserved/unused bit ranges
/// - Protocol fields at non-contiguous positions
/// - Overlay patterns where fields share the same bits
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitOffsetAttr {
    /// Bit offset from start of container (0-indexed)
    pub offset: u32,
    /// Source location
    pub span: Span,
}

impl BitOffsetAttr {
    /// Create a new offset attribute.
    pub fn new(offset: u32, span: Span) -> Self {
        Self { offset, span }
    }

    /// Get the offset value.
    pub fn offset(&self) -> u32 {
        self.offset
    }
}

impl Spanned for BitOffsetAttr {
    fn span(&self) -> Span {
        self.span
    }
}

impl std::fmt::Display for BitOffsetAttr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "@offset({})", self.offset)
    }
}

/// Bitfield type attribute: @bitfield
///
/// Marks a record type as using packed bitfield layout instead of
/// standard struct layout with natural alignment.
///
/// # Examples
///
/// ```verum
/// @bitfield
/// @endian(big)
/// type IpHeader is {
///     @bits(4)  version: U8,
///     @bits(4)  ihl: U8,
///     @bits(8)  dscp_ecn: U8,
///     @bits(16) total_length: U16,
/// };
/// ```
///
/// # Layout Semantics
///
/// When a type has `@bitfield`:
/// - Fields are packed according to `@bits(N)` specifications
/// - No implicit padding between fields
/// - Byte order determined by `@endian` attribute
/// - Total size is rounded up to byte boundary
///
/// # Validation
///
/// The type checker ensures:
/// - All fields have `@bits(N)` specification
/// - No fields overlap (unless `@allow_overlap` is present)
/// - Total bit count fits within container alignment
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BitfieldAttr {
    /// Allow overlapping fields (union-like semantics)
    pub allow_overlap: bool,
    /// Optional explicit total size in bits
    pub total_bits: Maybe<u32>,
    /// Source location
    pub span: Span,
}

impl BitfieldAttr {
    /// Create a new bitfield attribute with default settings.
    pub fn new(span: Span) -> Self {
        Self {
            allow_overlap: false,
            total_bits: Maybe::None,
            span,
        }
    }

    /// Create a bitfield attribute that allows overlapping fields.
    pub fn with_overlap(span: Span) -> Self {
        Self {
            allow_overlap: true,
            total_bits: Maybe::None,
            span,
        }
    }

    /// Create a bitfield attribute with explicit total size.
    pub fn with_total_bits(total_bits: u32, span: Span) -> Self {
        Self {
            allow_overlap: false,
            total_bits: Maybe::Some(total_bits),
            span,
        }
    }

    /// Check if overlapping fields are allowed.
    pub fn allows_overlap(&self) -> bool {
        self.allow_overlap
    }
}

impl Spanned for BitfieldAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// Endian attribute: @endian(big|little|native)
///
/// Specifies byte order for multi-byte bitfield containers.
/// Critical for hardware interfaces and network protocols.
///
/// # Examples
///
/// ```verum
/// @bitfield
/// @endian(big)  // Network byte order
/// type NetworkHeader is { ... };
///
/// @bitfield
/// @endian(little)  // x86/ARM hardware registers
/// type HardwareRegister is { ... };
///
/// @bitfield
/// @endian(native)  // Platform-specific (use with caution)
/// type LocalData is { ... };
/// ```
///
/// # Byte Order Modes
///
/// - `big`: Most significant byte at lowest address (network byte order)
/// - `little`: Least significant byte at lowest address (x86, ARM)
/// - `native`: Platform-specific (avoid for portable code)
///
/// # Default Behavior
///
/// If `@endian` is omitted, `little` is used as the default,
/// matching common hardware platforms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndianAttr {
    /// The byte order mode
    pub byte_order: crate::bitfield::ByteOrder,
    /// Source location
    pub span: Span,
}

impl EndianAttr {
    /// Create a new endian attribute with the specified byte order.
    pub fn new(byte_order: crate::bitfield::ByteOrder, span: Span) -> Self {
        Self { byte_order, span }
    }

    /// Create a big-endian attribute.
    pub fn big(span: Span) -> Self {
        Self::new(crate::bitfield::ByteOrder::Big, span)
    }

    /// Create a little-endian attribute.
    pub fn little(span: Span) -> Self {
        Self::new(crate::bitfield::ByteOrder::Little, span)
    }

    /// Create a native-endian attribute.
    pub fn native(span: Span) -> Self {
        Self::new(crate::bitfield::ByteOrder::Native, span)
    }

    /// Get the byte order.
    pub fn byte_order(&self) -> crate::bitfield::ByteOrder {
        self.byte_order
    }

    /// Check if this is network byte order (big-endian).
    pub fn is_network_order(&self) -> bool {
        self.byte_order.is_network_order()
    }
}

impl Spanned for EndianAttr {
    fn span(&self) -> Span {
        self.span
    }
}

impl Default for EndianAttr {
    fn default() -> Self {
        Self {
            byte_order: crate::bitfield::ByteOrder::Little,
            span: Span::default(),
        }
    }
}

impl std::fmt::Display for EndianAttr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "@endian({})", self.byte_order)
    }
}

// =============================================================================
// MMIO/REGISTER ATTRIBUTES
// =============================================================================

/// Register block attribute: @register_block(base = ADDRESS, stride = VALUE)
///
/// Marks a struct type as a memory-mapped register block with a base address
/// and optional stride for peripheral instances.
///
/// # Examples
///
/// ```verum
/// @repr(C)
/// @register_block(base = 0x4002_0000, stride = 0x400)
/// public type GpioRegisters is {
///     @offset(0x00) moder: Register<UInt32, ReadWrite>,
///     @offset(0x04) otyper: Register<UInt32, ReadWrite>,
///     @offset(0x10) idr: Register<UInt32, ReadOnly>,
///     @offset(0x18) bsrr: Register<UInt32, WriteOnly>,
/// };
///
/// // Access via generated constants:
/// public const GPIOA: &GpioRegisters = GpioRegisters.at(0x4002_0000);
/// public const GPIOB: &GpioRegisters = GpioRegisters.at(0x4002_0400);
/// ```
///
/// # Specification
///
/// Register block attribute for memory-mapped hardware register groups.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RegisterBlockAttr {
    /// Base address of the register block
    pub base_address: u64,
    /// Optional stride between peripheral instances
    pub stride: Maybe<u64>,
    /// Source location
    pub span: Span,
}

impl RegisterBlockAttr {
    /// Create a new register block attribute with base address only.
    pub fn new(base_address: u64, span: Span) -> Self {
        Self {
            base_address,
            stride: Maybe::None,
            span,
        }
    }

    /// Create a register block attribute with base address and stride.
    pub fn with_stride(base_address: u64, stride: u64, span: Span) -> Self {
        Self {
            base_address,
            stride: Maybe::Some(stride),
            span,
        }
    }

    /// Get the base address.
    pub fn base_address(&self) -> u64 {
        self.base_address
    }

    /// Get the stride, if specified.
    pub fn stride(&self) -> Maybe<u64> {
        self.stride
    }

    /// Calculate the address of an instance at the given index.
    pub fn instance_address(&self, index: u64) -> u64 {
        match self.stride {
            Maybe::Some(s) => self.base_address + index * s,
            Maybe::None => self.base_address,
        }
    }
}

impl Spanned for RegisterBlockAttr {
    fn span(&self) -> Span {
        self.span
    }
}

impl std::fmt::Display for RegisterBlockAttr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.stride {
            Maybe::Some(s) => write!(f, "@register_block(base = 0x{:X}, stride = 0x{:X})", self.base_address, s),
            Maybe::None => write!(f, "@register_block(base = 0x{:X})", self.base_address),
        }
    }
}

/// Register offset attribute: @offset(N)
///
/// Specifies the byte offset of a register field from the start of the
/// register block. This is separate from BitOffsetAttr which specifies
/// bit offset within a bitfield.
///
/// # Examples
///
/// ```verum
/// @register_block(base = 0x4002_0000)
/// type GpioRegisters is {
///     @offset(0x00) moder: Register<UInt32, ReadWrite>,   // At base + 0x00
///     @offset(0x04) otyper: Register<UInt32, ReadWrite>,  // At base + 0x04
///     @offset(0x10) idr: Register<UInt32, ReadOnly>,      // At base + 0x10
/// };
/// ```
///
/// # Note
///
/// This attribute uses the same name as BitOffsetAttr but serves a different
/// purpose. Context determines which is applicable:
/// - On @bitfield types: BitOffsetAttr (bit offset)
/// - On @register_block types: RegisterOffsetAttr (byte offset)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterOffsetAttr {
    /// Byte offset from the start of the register block
    pub offset: u64,
    /// Source location
    pub span: Span,
}

impl RegisterOffsetAttr {
    /// Create a new register offset attribute.
    pub fn new(offset: u64, span: Span) -> Self {
        Self { offset, span }
    }

    /// Get the offset value.
    pub fn offset(&self) -> u64 {
        self.offset
    }
}

impl Spanned for RegisterOffsetAttr {
    fn span(&self) -> Span {
        self.span
    }
}

impl std::fmt::Display for RegisterOffsetAttr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "@offset(0x{:X})", self.offset)
    }
}

/// Access mode for MMIO registers.
///
/// Defines the read/write capabilities of a register, enabling compile-time
/// enforcement of access restrictions.
///
/// # Examples
///
/// ```verum
/// // Read-write register
/// moder: Register<UInt32, ReadWrite>,
///
/// // Read-only status register
/// idr: Register<UInt32, ReadOnly>,
///
/// // Write-only set/reset register
/// bsrr: Register<UInt32, WriteOnly>,
///
/// // Write-one-to-clear interrupt flags
/// status: Register<UInt32, WriteOneToClear>,
/// ```
///
/// # Specification
///
/// Register access mode: read-only, write-only, or read-write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AccessMode {
    /// Read-only: only read() is allowed
    ReadOnly,
    /// Write-only: only write() is allowed
    WriteOnly,
    /// Read-write: both read() and write() allowed
    ReadWrite,
    /// Write-one-to-clear: read() and clear_bits() allowed (common for interrupt status)
    WriteOneToClear,
    /// Write-one-to-set: read() and set_bits() allowed
    WriteOneToSet,
    /// Reserved: no access allowed (placeholder for reserved registers)
    Reserved,
}

impl AccessMode {
    /// Parse access mode from a string identifier.
    pub fn from_str(s: &str) -> Maybe<Self> {
        match s {
            "ReadOnly" | "read_only" => Maybe::Some(AccessMode::ReadOnly),
            "WriteOnly" | "write_only" => Maybe::Some(AccessMode::WriteOnly),
            "ReadWrite" | "read_write" => Maybe::Some(AccessMode::ReadWrite),
            "WriteOneToClear" | "write_one_to_clear" => Maybe::Some(AccessMode::WriteOneToClear),
            "WriteOneToSet" | "write_one_to_set" => Maybe::Some(AccessMode::WriteOneToSet),
            "Reserved" | "reserved" => Maybe::Some(AccessMode::Reserved),
            _ => Maybe::None,
        }
    }

    /// Get the string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            AccessMode::ReadOnly => "ReadOnly",
            AccessMode::WriteOnly => "WriteOnly",
            AccessMode::ReadWrite => "ReadWrite",
            AccessMode::WriteOneToClear => "WriteOneToClear",
            AccessMode::WriteOneToSet => "WriteOneToSet",
            AccessMode::Reserved => "Reserved",
        }
    }

    /// Check if this mode allows reading.
    pub fn can_read(&self) -> bool {
        matches!(self, AccessMode::ReadOnly | AccessMode::ReadWrite |
                 AccessMode::WriteOneToClear | AccessMode::WriteOneToSet)
    }

    /// Check if this mode allows writing.
    pub fn can_write(&self) -> bool {
        matches!(self, AccessMode::WriteOnly | AccessMode::ReadWrite |
                 AccessMode::WriteOneToClear | AccessMode::WriteOneToSet)
    }

    /// Check if this is a write-modify mode.
    pub fn is_write_modify(&self) -> bool {
        matches!(self, AccessMode::WriteOneToClear | AccessMode::WriteOneToSet)
    }
}

impl std::fmt::Display for AccessMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ============================================================================
// Interrupt Handling Attributes (Phase 3)
// ============================================================================

/// Types of interrupt handlers.
///
/// Different interrupt types require different handling at the hardware level.
/// The type affects prologue/epilogue generation and calling conventions.
///
/// # Specification
///
/// Interrupt handler attribute with handler type and priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InterruptKind {
    /// Regular interrupt: standard prologue saves all caller-saved registers.
    Regular,
    /// Non-maskable interrupt: cannot be disabled, highest priority.
    #[allow(clippy::upper_case_acronyms)]
    NMI,
    /// Fast interrupt (FIQ on ARM): uses banked registers for faster response.
    Fast,
    /// Exception handler: for CPU exceptions (divide by zero, page fault, etc.).
    Exception,
    /// System call/trap: software-triggered interrupt.
    Trap,
    /// Reset handler: system startup, special stack handling.
    Reset,
}

impl InterruptKind {
    /// Parse interrupt kind from a string identifier.
    pub fn from_str(s: &str) -> Maybe<Self> {
        match s {
            "regular" | "Regular" | "irq" | "IRQ" => Maybe::Some(InterruptKind::Regular),
            "nmi" | "NMI" => Maybe::Some(InterruptKind::NMI),
            "fast" | "Fast" | "fiq" | "FIQ" => Maybe::Some(InterruptKind::Fast),
            "exception" | "Exception" => Maybe::Some(InterruptKind::Exception),
            "trap" | "Trap" | "syscall" | "svc" | "SVC" => Maybe::Some(InterruptKind::Trap),
            "reset" | "Reset" => Maybe::Some(InterruptKind::Reset),
            _ => Maybe::None,
        }
    }

    /// Get the string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            InterruptKind::Regular => "regular",
            InterruptKind::NMI => "nmi",
            InterruptKind::Fast => "fast",
            InterruptKind::Exception => "exception",
            InterruptKind::Trap => "trap",
            InterruptKind::Reset => "reset",
        }
    }

    /// Check if this interrupt type can be masked/disabled.
    pub fn is_maskable(&self) -> bool {
        matches!(self, InterruptKind::Regular | InterruptKind::Fast)
    }

    /// Check if this requires special stack handling.
    pub fn needs_special_stack(&self) -> bool {
        matches!(self, InterruptKind::NMI | InterruptKind::Reset | InterruptKind::Exception)
    }
}

impl std::fmt::Display for InterruptKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Interrupt handler attribute for marking functions as interrupt service routines.
///
/// This attribute enables proper interrupt handler code generation including:
/// - Saving/restoring registers according to the target ABI
/// - Using the correct return instruction (iret, rfi, etc.)
/// - Stack alignment requirements
/// - Critical section handling
///
/// # Examples
///
/// ```verum
/// @interrupt(regular)
/// fn timer_isr() {
///     // Handle timer interrupt
///     TIMER.status.clear_bits(TIMER_OVERFLOW);
/// }
///
/// @interrupt(nmi)
/// fn non_maskable_handler(frame: &ExceptionFrame) {
///     // NMI cannot be disabled - handle critical errors
///     panic("NMI occurred");
/// }
///
/// @interrupt(exception)
/// fn divide_by_zero(frame: &ExceptionFrame) -> ! {
///     panic(f"Division by zero at {frame.pc:x}");
/// }
///
/// // Naked interrupt for custom prologue
/// @interrupt(fast, naked: true)
/// fn fast_irq() {
///     @asm {
///         // Custom fast path handling
///         ldr r0, =GPIOA_BASE
///         str r1, [r0, #ODR]
///         bx lr
///     }
/// }
/// ```
///
/// # Specification
///
/// Naked function: compiler generates no prologue/epilogue. (Interrupt Handler Codegen)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterruptAttr {
    /// The kind of interrupt handler.
    pub kind: InterruptKind,
    /// If true, no prologue/epilogue is generated (for hand-written assembly).
    pub naked: bool,
    /// Optional interrupt vector number (for linking to vector table).
    pub vector: Maybe<u32>,
    /// Optional priority level (for NVIC-style interrupt controllers).
    pub priority: Maybe<u8>,
    /// Whether to save floating-point registers.
    pub save_fpu: bool,
    /// Source location.
    pub span: Span,
}

impl InterruptAttr {
    /// Create a new interrupt attribute with default settings.
    pub fn new(kind: InterruptKind, span: Span) -> Self {
        Self {
            kind,
            naked: false,
            vector: Maybe::None,
            priority: Maybe::None,
            save_fpu: false,
            span,
        }
    }

    /// Create a naked interrupt handler attribute.
    pub fn naked(kind: InterruptKind, span: Span) -> Self {
        Self {
            kind,
            naked: true,
            vector: Maybe::None,
            priority: Maybe::None,
            save_fpu: false,
            span,
        }
    }

    /// Set the vector number.
    pub fn with_vector(mut self, vector: u32) -> Self {
        self.vector = Maybe::Some(vector);
        self
    }

    /// Set the priority level.
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = Maybe::Some(priority);
        self
    }

    /// Enable FPU register saving.
    pub fn with_fpu_save(mut self) -> Self {
        self.save_fpu = true;
        self
    }

    /// Get the interrupt kind.
    pub fn kind(&self) -> InterruptKind {
        self.kind
    }

    /// Check if this is a naked interrupt handler.
    pub fn is_naked(&self) -> bool {
        self.naked
    }

    /// Get the vector number if specified.
    pub fn vector(&self) -> Maybe<u32> {
        self.vector
    }

    /// Get the priority level if specified.
    pub fn priority(&self) -> Maybe<u8> {
        self.priority
    }

    /// Check if FPU registers should be saved.
    pub fn saves_fpu(&self) -> bool {
        self.save_fpu
    }
}

impl Spanned for InterruptAttr {
    fn span(&self) -> Span {
        self.span
    }
}

impl std::fmt::Display for InterruptAttr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "@interrupt({}", self.kind)?;
        if self.naked {
            write!(f, ", naked: true")?;
        }
        if let Maybe::Some(v) = self.vector {
            write!(f, ", vector: {}", v)?;
        }
        if let Maybe::Some(p) = self.priority {
            write!(f, ", priority: {}", p)?;
        }
        if self.save_fpu {
            write!(f, ", save_fpu: true")?;
        }
        write!(f, ")")
    }
}

/// Critical section attribute for functions that disable interrupts.
///
/// This attribute marks functions that must run atomically with interrupts
/// disabled. The compiler ensures proper interrupt save/restore around
/// the function body.
///
/// # Examples
///
/// ```verum
/// @critical_section
/// fn update_shared_counter(delta: Int) {
///     // Interrupts are disabled here
///     SHARED_COUNTER += delta;
/// }
///
/// @critical_section(priority_mask: 0x10)
/// fn update_with_partial_mask() {
///     // Only interrupts below priority 0x10 are masked
///     LOW_PRIORITY_DATA += 1;
/// }
/// ```
///
/// # Specification
///
/// Critical section attribute: disables interrupts within the annotated scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CriticalSectionAttr {
    /// If Some, only mask interrupts below this priority (BASEPRI on ARM).
    /// If None, disable all maskable interrupts.
    pub priority_mask: Maybe<u8>,
    /// Source location.
    pub span: Span,
}

impl CriticalSectionAttr {
    /// Create a new critical section attribute that disables all interrupts.
    pub fn new(span: Span) -> Self {
        Self {
            priority_mask: Maybe::None,
            span,
        }
    }

    /// Create a critical section with a priority mask.
    pub fn with_priority_mask(priority: u8, span: Span) -> Self {
        Self {
            priority_mask: Maybe::Some(priority),
            span,
        }
    }

    /// Get the priority mask if specified.
    pub fn priority_mask(&self) -> Maybe<u8> {
        self.priority_mask
    }

    /// Check if this masks all interrupts.
    pub fn masks_all(&self) -> bool {
        self.priority_mask.is_none()
    }
}

impl Spanned for CriticalSectionAttr {
    fn span(&self) -> Span {
        self.span
    }
}

impl std::fmt::Display for CriticalSectionAttr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.priority_mask {
            Maybe::Some(p) => write!(f, "@critical_section(priority_mask: {})", p),
            Maybe::None => write!(f, "@critical_section"),
        }
    }
}

// =============================================================================
// FRAMEWORK-AXIOM ATTRIBUTION
//
// `@framework(name, "citation")` marks an axiom or theorem as a trusted
// postulate coming from an external formal-mathematics framework (Lurie's
// Higher Topos Theory, Schreiber's Differential Cohesion, Connes's
// reconstruction theorem, Petz classification, Arnold–Mather catastrophe
// normal forms, Baez–Dolan tricategory coherence, etc.).
//
// Every `@framework` marker produces a typed entry in the compiler's
// framework-axiom registry (`verum_smt::framework_registry`). The registry
// drives two downstream features:
//   * `verum audit --framework-axioms` — enumerates the trusted boundary
//     of any proof, so external reviewers see exactly which external
//     results are relied on.
//   * `.verum-cert` export — certificates carry their framework-axiom
//     dependency list, allowing Coq / Lean / Isabelle / Dedukti / Metamath
//     consumers to replay the proof under their own axioms.
//
// Grammar: the existing generic attribute production
//   attribute = identifier , [ '(' , attribute_args , ')' ]
// already accepts `@framework(lurie_htt, "HTT 6.2.2.7")`. This typed
// extractor pulls a structured pair out of the generic `Attribute`.
// =============================================================================

/// Typed form of `@framework(name, "citation")`.
///
/// `name` is the framework identifier (e.g. `lurie_htt`, `schreiber_dcct`,
/// `connes_reconstruction`, `petz_classification`, `arnold_catastrophe`,
/// `baez_dolan`). `citation` is a free-form string literal pointing at the
/// specific theorem / section / page in the cited work.
///
/// Attached to `axiom` or `theorem` declarations — or to a user-defined
/// lemma that depends transitively on one of these external results and
/// wants to make the dependency explicit.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameworkAttr {
    /// Framework identifier — short, stable, machine-readable.
    /// Convention: `snake_case` matching the file under
    /// `core/math/frameworks/<name>.vr`.
    pub name: Text,
    /// Citation string — human-readable reference to the specific result,
    /// e.g. `"HTT 6.2.2.7"`, `"DCCT §3.9"`, `"Connes 2008 axiom (vii)"`.
    pub citation: Text,
    /// Source span of the `@framework(...)` form.
    pub span: Span,
}

impl FrameworkAttr {
    /// Construct directly. Prefer `from_attribute` at parse time.
    pub fn new(name: Text, citation: Text, span: Span) -> Self {
        Self { name, citation, span }
    }

    /// Try to extract a `FrameworkAttr` from a generic `Attribute`.
    ///
    /// Returns `None` when the attribute name is not `"framework"` or the
    /// argument shape does not match `(identifier, string_literal)`. A
    /// shape mismatch on a `@framework(...)` is a user error — the caller
    /// (elaboration / diagnostic pass) should emit a diagnostic in that
    /// case rather than silently discarding the attribution.
    pub fn from_attribute(attr: &Attribute) -> Maybe<Self> {
        if !attr.is_named("framework") {
            return Maybe::None;
        }
        let args = match &attr.args {
            Maybe::Some(a) => a,
            Maybe::None => return Maybe::None,
        };
        // Expect exactly two args: an identifier and a string literal.
        if args.len() != 2 {
            return Maybe::None;
        }
        use crate::expr::{Expr, ExprKind};
        use crate::literal::{LiteralKind, StringLit};
        let name: Option<Text> = match args.get(0) {
            Some(Expr { kind: ExprKind::Path(path), .. }) => {
                path.segments.last().and_then(|seg| match seg {
                    crate::ty::PathSegment::Name(ident) => Some(ident.name.clone()),
                    _ => None,
                })
            }
            _ => None,
        };
        let citation: Option<Text> = match args.get(1) {
            Some(Expr { kind: ExprKind::Literal(lit), .. }) => match &lit.kind {
                LiteralKind::Text(StringLit::Regular(s))
                | LiteralKind::Text(StringLit::MultiLine(s)) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        };
        match (name, citation) {
            (Some(n), Some(c)) => Maybe::Some(FrameworkAttr::new(n, c, attr.span)),
            _ => Maybe::None,
        }
    }
}

impl Spanned for FrameworkAttr {
    fn span(&self) -> Span {
        self.span
    }
}

impl std::fmt::Display for FrameworkAttr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "@framework({}, \"{}\")", self.name, self.citation)
    }
}

// =============================================================================
// ENACTMENT ATTRIBUTION (VUVA §11.4)
//
// `@enact(epsilon = "ε_prove")` tags a function / theorem / lemma / axiom
// declaration with a Diakrisis primitive act on the Actic (DC) side of
// the OC/DC duality (108.T). Every proof assistant before Verum ships
// only the object-centric (OC) layer; the `@enact` marker is how Verum
// exposes the dependency-centric (DC) coordinate of a function as
// first-class audit data.
//
// The seven canonical primitives — ε_math, ε_compute, ε_observe,
// ε_prove, ε_decide, ε_translate, ε_construct — are defined in
// `core.action.primitives` and documented in VUVA §11.2. User-defined
// ε-contracts (composites under `core.action.enactments`) are not
// supported by this attribute in Phase 5 E3; they are tracked through
// the per-function enactment inferred from the body.
//
// Grammar shape: `@enact(epsilon = <string-literal>)`.
//
// Downstream consumers:
//   * `verum audit --epsilon` — enumerates the ε-distribution across
//     the corpus, parallel to `verum audit --framework-axioms` for
//     the OC side.
//   * `core.action.verify.verify_epsilon` — compile-time consistency
//     check that the declared ε matches the enactment inferred from
//     the function body (Phase 5 E3b).
// =============================================================================

/// Typed form of `@enact(epsilon = "<primitive>")`.
///
/// The primitive is one of the seven Diakrisis ε-tags (VUVA §11.2).
/// Unicode (`ε_prove`) and ASCII (`epsilon_prove`) spellings are both
/// accepted at parse time; the typed form stores the canonical Unicode
/// string so `verum audit --epsilon` renders deterministically.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EnactAttr {
    /// Canonical ε-primitive identifier, e.g. `"ε_prove"`, `"ε_compute"`.
    pub epsilon: Text,
    /// Source span of the `@enact(...)` form.
    pub span: Span,
}

impl EnactAttr {
    /// Construct directly. Prefer `from_attribute` at parse time.
    pub fn new(epsilon: Text, span: Span) -> Self {
        Self { epsilon, span }
    }

    /// Canonicalise an ε-primitive string. Accepts both Unicode
    /// (`ε_prove`) and ASCII-fallback (`epsilon_prove`) spellings;
    /// returns the canonical Unicode form, or `None` if the input
    /// does not match a known primitive.
    ///
    /// Mirrors `core.action.primitives.primitive_from_text` but lives
    /// in AST-layer because the attribute needs to canonicalise at
    /// parse time before any stdlib fn has been compiled.
    pub fn canonicalise_primitive(raw: &str) -> Option<&'static str> {
        match raw {
            "ε_math"      | "epsilon_math"      => Some("ε_math"),
            "ε_compute"   | "epsilon_compute"   => Some("ε_compute"),
            "ε_observe"   | "epsilon_observe"   => Some("ε_observe"),
            "ε_prove"     | "epsilon_prove"     => Some("ε_prove"),
            "ε_decide"    | "epsilon_decide"    => Some("ε_decide"),
            "ε_translate" | "epsilon_translate" => Some("ε_translate"),
            "ε_construct" | "epsilon_construct" => Some("ε_construct"),
            _ => None,
        }
    }

    /// Try to extract an `EnactAttr` from a generic `Attribute`.
    ///
    /// Returns `None` when the attribute name is not `"enact"` or the
    /// argument shape does not match `epsilon = <string_literal>`. The
    /// parser accepts named-argument form `epsilon = "..."`; downstream
    /// emits a diagnostic if the string is not a recognised primitive.
    pub fn from_attribute(attr: &Attribute) -> Maybe<Self> {
        if !attr.is_named("enact") {
            return Maybe::None;
        }
        let args = match &attr.args {
            Maybe::Some(a) => a,
            Maybe::None => return Maybe::None,
        };
        if args.len() != 1 {
            return Maybe::None;
        }
        use crate::expr::{BinOp, Expr, ExprKind};
        use crate::literal::{LiteralKind, StringLit};
        // Accepted shapes (all equivalent):
        //   1. `@enact(epsilon: "ε_prove")` — parser lowers `key: value`
        //      attribute named-args to `Binary(Assign, Path("epsilon"), Literal)`.
        //   2. `@enact(epsilon = "ε_prove")` — same lowering via Binary Assign.
        //   3. `@enact("ε_prove")` — bare positional string literal.
        // All three land in `args[0]`; walk the shape and pull the string.
        let raw_opt: Option<Text> = match args.get(0) {
            // Named form: `epsilon: "..."` or `epsilon = "..."`.
            Some(Expr {
                kind: ExprKind::Binary { op: BinOp::Assign, left, right },
                ..
            }) => {
                let key_is_epsilon = match &left.kind {
                    ExprKind::Path(p) => p
                        .segments
                        .last()
                        .and_then(|seg| match seg {
                            crate::ty::PathSegment::Name(ident) => {
                                Some(ident.name.as_str() == "epsilon")
                            }
                            _ => None,
                        })
                        .unwrap_or(false),
                    _ => false,
                };
                if !key_is_epsilon {
                    None
                } else {
                    match &right.kind {
                        ExprKind::Literal(lit) => match &lit.kind {
                            LiteralKind::Text(StringLit::Regular(s))
                            | LiteralKind::Text(StringLit::MultiLine(s)) => Some(s.clone()),
                            _ => None,
                        },
                        _ => None,
                    }
                }
            }
            // Positional form: bare string literal.
            Some(Expr { kind: ExprKind::Literal(lit), .. }) => match &lit.kind {
                LiteralKind::Text(StringLit::Regular(s))
                | LiteralKind::Text(StringLit::MultiLine(s)) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        };
        let canonical = raw_opt
            .as_ref()
            .and_then(|s| Self::canonicalise_primitive(s.as_str()))
            .map(Text::from);
        match canonical {
            Some(c) => Maybe::Some(EnactAttr::new(c, attr.span)),
            None => Maybe::None,
        }
    }
}

impl Spanned for EnactAttr {
    fn span(&self) -> Span {
        self.span
    }
}

impl std::fmt::Display for EnactAttr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "@enact(epsilon = \"{}\")", self.epsilon)
    }
}

// =============================================================================
// OWL 2 ATTRIBUTION FAMILY (VUVA §21.6 Phase 3 C8 V1)
//
// Vocabulary-preserving typed attributes for the OWL 2 Direct Semantics
// surface. Each attribute installs (1) a `CoreTerm::FrameworkAxiom`
// reference into `core.math.frameworks.owl2_fs.*`, (2) a `@verify(...)`
// obligation per VUVA §21.3 Table, and (3) a round-trip marker consumed
// by `verum export --to owl2-fs` (B5, deferred).
//
// V1 ships four attributes covering the most common OWL 2 surface forms;
// V2 adds the richer `@owl2_property(domain = ..., range = ...,
// characteristic = [...], inverse_of = ...)` named-arg form per §21.6.
//
//   @owl2_class[(semantics = "OpenWorld" | "ClosedWorld")]
//   @owl2_subclass_of(<class_name>)
//   @owl2_disjoint_with([<class_name>, ...])
//   @owl2_characteristic("Transitive" | "Symmetric" | "Asymmetric"
//                       | "Reflexive" | "Irreflexive" | "Functional"
//                       | "InverseFunctional")
//
// All four follow the FrameworkAttr / EnactAttr pattern: a struct with
// a `from_attribute(attr: &Attribute) -> Maybe<Self>` parser. Syntax
// errors (wrong arg count, unknown enum variant) return Maybe::None;
// the elaboration pass is responsible for emitting a diagnostic in
// that case rather than silently dropping the attribute.
// =============================================================================

/// Open-world / closed-world semantics flag.  OWL 2 DS is normally
/// open-world; Verum's typed refinement system is closed-world.
/// VUVA §21.4 picks CWA as the default and admits OWA only when
/// the user explicitly opts in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Owl2Semantics {
    /// Open World Assumption — absence of an assertion does not imply
    /// negation. Queries return `Maybe<Bool>` with `Unknown` when the
    /// class membership is neither provable nor refutable.
    OpenWorld,
    /// Closed World Assumption — a predicate either holds or fails.
    /// Queries return plain `Bool`. Default for `@owl2_class` without
    /// an explicit `semantics` argument.
    ClosedWorld,
}

impl Owl2Semantics {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenWorld   => "OpenWorld",
            Self::ClosedWorld => "ClosedWorld",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "OpenWorld" | "open_world" | "OWA"   => Some(Self::OpenWorld),
            "ClosedWorld" | "closed_world" | "CWA" => Some(Self::ClosedWorld),
            _ => None,
        }
    }
}

/// `@owl2_class` — marks a Verum type as an OWL 2 Class. Optional
/// `semantics = "OpenWorld" | "ClosedWorld"` argument selects the
/// semantic regime per VUVA §21.4.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Owl2ClassAttr {
    /// Optional semantics qualifier; default is ClosedWorld per VUVA §21.4.
    pub semantics: Maybe<Owl2Semantics>,
    pub span: Span,
}

impl Owl2ClassAttr {
    pub fn new(semantics: Maybe<Owl2Semantics>, span: Span) -> Self {
        Self { semantics, span }
    }

    pub fn from_attribute(attr: &Attribute) -> Maybe<Self> {
        if !attr.is_named("owl2_class") {
            return Maybe::None;
        }
        let args = match &attr.args {
            Maybe::Some(a) => a,
            Maybe::None    => {
                // `@owl2_class` without args is valid — defaults to
                // ClosedWorld semantics.
                return Maybe::Some(Self::new(Maybe::None, attr.span));
            }
        };
        if args.is_empty() {
            return Maybe::Some(Self::new(Maybe::None, attr.span));
        }
        if args.len() != 1 {
            return Maybe::None;
        }
        // Expect a single named-arg `semantics: "..."` lowered to
        // Binary { Assign, Path("semantics"), Literal("...") }.
        let semantics = parse_named_string_arg(args.get(0)?, "semantics")
            .and_then(|s| Owl2Semantics::parse(s.as_str()));
        match semantics {
            Some(sem) => Maybe::Some(Self::new(Maybe::Some(sem), attr.span)),
            None      => Maybe::None,
        }
    }
}

impl Spanned for Owl2ClassAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// `@owl2_subclass_of(<class_name>)` — marks the decorated type as a
/// subclass of the named OWL 2 class.  The class name resolves through
/// the surrounding module's import graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Owl2SubClassOfAttr {
    pub parent: Text,
    pub span: Span,
}

impl Owl2SubClassOfAttr {
    pub fn new(parent: Text, span: Span) -> Self {
        Self { parent, span }
    }

    pub fn from_attribute(attr: &Attribute) -> Maybe<Self> {
        if !attr.is_named("owl2_subclass_of") {
            return Maybe::None;
        }
        let args = match &attr.args {
            Maybe::Some(a) => a,
            Maybe::None    => return Maybe::None,
        };
        if args.len() != 1 {
            return Maybe::None;
        }
        // Single positional argument — either an identifier path or a
        // string literal naming the parent class.
        let parent = parse_class_name_arg(args.get(0)?);
        match parent {
            Some(p) => Maybe::Some(Self::new(p, attr.span)),
            None    => Maybe::None,
        }
    }
}

impl Spanned for Owl2SubClassOfAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// `@owl2_disjoint_with([<class_name>, ...])` — marks the decorated
/// type as disjoint from each of the listed OWL 2 classes.  Lowers to
/// a `DisjointClasses(self, c1, c2, ...)` axiom invocation per
/// Shkotin Table 5.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Owl2DisjointWithAttr {
    pub disjoint_classes: Vec<Text>,
    pub span: Span,
}

impl Owl2DisjointWithAttr {
    pub fn new(disjoint_classes: Vec<Text>, span: Span) -> Self {
        Self { disjoint_classes, span }
    }

    pub fn from_attribute(attr: &Attribute) -> Maybe<Self> {
        if !attr.is_named("owl2_disjoint_with") {
            return Maybe::None;
        }
        let args = match &attr.args {
            Maybe::Some(a) => a,
            Maybe::None    => return Maybe::None,
        };
        // Accept two equivalent shapes:
        //   1. `@owl2_disjoint_with([Foo, Bar])` — single Array list arg
        //   2. `@owl2_disjoint_with(Foo, Bar)`   — multiple positional args
        // The parser surfaces (1) as a single ExprKind::Array(ArrayExpr::List)
        // and (2) as multiple Path / Literal args; both lower to the
        // same disjoint_classes set. Repeat-form arrays `[x; n]` are
        // not legal in this context — disjoint-class lists are
        // semantically a set, repeat would collapse to one class.
        use crate::expr::{ArrayExpr, ExprKind};
        let mut classes: Vec<Text> = Vec::new();
        if args.len() == 1 {
            let single = args.get(0).unwrap();
            match &single.kind {
                ExprKind::Array(ArrayExpr::List(elems)) => {
                    let mut i: usize = 0;
                    while i < elems.len() {
                        if let Some(elem_ref) = elems.get(i) {
                            if let Some(name) = parse_class_name_arg(elem_ref) {
                                classes.push(name);
                            }
                        }
                        i += 1;
                    }
                }
                _ => {
                    // Degenerate single-class form (2).
                    if let Some(name) = parse_class_name_arg(single) {
                        classes.push(name);
                    }
                }
            }
        } else {
            // Form (2): multiple positional args.
            for arg in args.iter() {
                if let Some(name) = parse_class_name_arg(arg) {
                    classes.push(name);
                }
            }
        }
        if classes.is_empty() {
            return Maybe::None;
        }
        Maybe::Some(Self::new(classes, attr.span))
    }
}

impl Spanned for Owl2DisjointWithAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// One of the seven OWL 2 object-property characteristics from
/// Shkotin 2019 Table 6 / VUVA §21.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Owl2Characteristic {
    Transitive,
    Symmetric,
    Asymmetric,
    Reflexive,
    Irreflexive,
    Functional,
    InverseFunctional,
}

impl Owl2Characteristic {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Transitive        => "Transitive",
            Self::Symmetric         => "Symmetric",
            Self::Asymmetric        => "Asymmetric",
            Self::Reflexive         => "Reflexive",
            Self::Irreflexive       => "Irreflexive",
            Self::Functional        => "Functional",
            Self::InverseFunctional => "InverseFunctional",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "Transitive"        => Some(Self::Transitive),
            "Symmetric"         => Some(Self::Symmetric),
            "Asymmetric"        => Some(Self::Asymmetric),
            "Reflexive"         => Some(Self::Reflexive),
            "Irreflexive"       => Some(Self::Irreflexive),
            "Functional"        => Some(Self::Functional),
            "InverseFunctional" => Some(Self::InverseFunctional),
            _                   => None,
        }
    }
}

/// `@owl2_characteristic("Transitive" | "Symmetric" | ...)` — marks
/// the decorated function (interpreted as an OWL 2 ObjectProperty) with
/// one of the seven Shkotin Table 6 characteristic flags.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Owl2CharacteristicAttr {
    pub characteristic: Owl2Characteristic,
    pub span: Span,
}

impl Owl2CharacteristicAttr {
    pub fn new(characteristic: Owl2Characteristic, span: Span) -> Self {
        Self { characteristic, span }
    }

    pub fn from_attribute(attr: &Attribute) -> Maybe<Self> {
        if !attr.is_named("owl2_characteristic") {
            return Maybe::None;
        }
        let args = match &attr.args {
            Maybe::Some(a) => a,
            Maybe::None    => return Maybe::None,
        };
        if args.len() != 1 {
            return Maybe::None;
        }
        // Accept either a Path (`Transitive`) or a string literal
        // (`"Transitive"`); both resolve to the canonical enum.
        let arg = args.get(0).unwrap();
        let name: Option<Text> = parse_class_name_arg(arg);
        let parsed = name.and_then(|s| Owl2Characteristic::parse(s.as_str()));
        match parsed {
            Some(c) => Maybe::Some(Self::new(c, attr.span)),
            None    => Maybe::None,
        }
    }
}

impl Spanned for Owl2CharacteristicAttr {
    fn span(&self) -> Span {
        self.span
    }
}

// -----------------------------------------------------------------------------
// Internal helpers for OwlAttr parsers.
// -----------------------------------------------------------------------------

/// Parse a named-arg shape `name: "value"` (the parser lowers `:` to
/// Binary Assign; `=` lowers the same way) where `value` is a string
/// literal. Returns the literal text when the key matches; None
/// otherwise.
fn parse_named_string_arg(
    arg: &crate::expr::Expr,
    expected_key: &str,
) -> Option<Text> {
    use crate::expr::{BinOp, ExprKind};
    use crate::literal::{LiteralKind, StringLit};
    let (left, right) = match &arg.kind {
        ExprKind::Binary { op: BinOp::Assign, left, right } => (left, right),
        _ => return None,
    };
    let key_ok = match &left.kind {
        ExprKind::Path(p) => p
            .segments
            .last()
            .and_then(|seg| match seg {
                crate::ty::PathSegment::Name(id) => Some(id.name.as_str() == expected_key),
                _ => None,
            })
            .unwrap_or(false),
        _ => false,
    };
    if !key_ok {
        return None;
    }
    match &right.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Text(StringLit::Regular(s))
            | LiteralKind::Text(StringLit::MultiLine(s)) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Parse a class-name argument: either a Path expression
/// (`@owl2_subclass_of(Animal)`) or a string literal
/// (`@owl2_subclass_of("Animal")`). Returns the last path segment or
/// the literal text. Returns None for any other shape.
fn parse_class_name_arg(arg: &crate::expr::Expr) -> Option<Text> {
    use crate::expr::ExprKind;
    use crate::literal::{LiteralKind, StringLit};
    match &arg.kind {
        ExprKind::Path(p) => p.segments.last().and_then(|seg| match seg {
            crate::ty::PathSegment::Name(id) => Some(id.name.clone()),
            _ => None,
        }),
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Text(StringLit::Regular(s))
            | LiteralKind::Text(StringLit::MultiLine(s)) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

// =============================================================================
// OWL 2 ATTRIBUTION FAMILY — Full §21.6 surface (Phase 3 C8 V1 closeout)
// =============================================================================
//
// V1+ extension covering the three remaining attributes from the
// VUVA §21.6 catalogue:
//
//   @owl2_property(domain = ..., range = ...,
//                  characteristic = [...], inverse_of = ...)
//   @owl2_equivalent_class(<class_expr>)
//   @owl2_has_key(<property>, ...)
//
// These complete the four-attribute V1 set (Owl2ClassAttr,
// Owl2SubClassOfAttr, Owl2DisjointWithAttr, Owl2CharacteristicAttr)
// shipped earlier in this file. Together the seven attributes round-trip
// the full OWL 2 Functional-Style Syntax surface that
// `verum export --to owl2-fs` (B5) produces.

/// `@owl2_property(domain = X, range = Y[, characteristic = [Trans, Sym, ...]][, inverse_of = Foo])`
///
/// Marks a Verum function as an OWL 2 ObjectProperty (or DataProperty)
/// with explicit domain and range types and an optional list of
/// characteristic flags + an optional inverse-property reference.
/// Lowers to a chain of `ObjectPropertyDomain` + `ObjectPropertyRange`
/// + per-flag `<Char>ObjectProperty` axiom invocations + (when
/// inverse_of is supplied) `InverseObjectProperties`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Owl2PropertyAttr {
    pub domain: Maybe<Text>,
    pub range:  Maybe<Text>,
    pub characteristics: Vec<Owl2Characteristic>,
    pub inverse_of: Maybe<Text>,
    pub span: Span,
}

impl Owl2PropertyAttr {
    pub fn new(
        domain: Maybe<Text>,
        range: Maybe<Text>,
        characteristics: Vec<Owl2Characteristic>,
        inverse_of: Maybe<Text>,
        span: Span,
    ) -> Self {
        Self { domain, range, characteristics, inverse_of, span }
    }

    pub fn from_attribute(attr: &Attribute) -> Maybe<Self> {
        if !attr.is_named("owl2_property") {
            return Maybe::None;
        }
        let args = match &attr.args {
            Maybe::Some(a) => a,
            Maybe::None    => return Maybe::None,
        };
        if args.is_empty() {
            return Maybe::None;
        }
        let mut domain: Maybe<Text> = Maybe::None;
        let mut range:  Maybe<Text> = Maybe::None;
        let mut characteristics: Vec<Owl2Characteristic> = Vec::new();
        let mut inverse_of: Maybe<Text> = Maybe::None;

        // Walk each named argument; unknown keys are treated as a
        // shape error and the whole attribute is rejected — silent
        // discard would let typos slip through unnoticed.
        for arg in args.iter() {
            // Try each known key; first-match wins.
            if let Some(value) = parse_named_string_arg(arg, "domain") {
                domain = Maybe::Some(value);
                continue;
            }
            if let Some(value) = parse_named_class_arg(arg, "domain") {
                domain = Maybe::Some(value);
                continue;
            }
            if let Some(value) = parse_named_string_arg(arg, "range") {
                range = Maybe::Some(value);
                continue;
            }
            if let Some(value) = parse_named_class_arg(arg, "range") {
                range = Maybe::Some(value);
                continue;
            }
            if let Some(value) = parse_named_string_arg(arg, "inverse_of") {
                inverse_of = Maybe::Some(value);
                continue;
            }
            if let Some(value) = parse_named_class_arg(arg, "inverse_of") {
                inverse_of = Maybe::Some(value);
                continue;
            }
            if let Some(list) = parse_named_characteristic_list_arg(arg, "characteristic") {
                characteristics = list;
                continue;
            }
            // Unknown / malformed key — reject the whole attribute.
            return Maybe::None;
        }
        // Domain and range are MANDATORY per VUVA §21.6 grammar — the
        // attribute is meaningless without them.
        match (&domain, &range) {
            (Maybe::Some(_), Maybe::Some(_)) => Maybe::Some(Self::new(
                domain, range, characteristics, inverse_of, attr.span,
            )),
            _ => Maybe::None,
        }
    }
}

impl Spanned for Owl2PropertyAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// `@owl2_equivalent_class(<class_expr>)` — declares the decorated
/// class equivalent to another class expression. Lowers to
/// `EquivalentClasses(self, expr)` per Shkotin Table 5.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Owl2EquivalentClassAttr {
    pub equivalent_to: Text,
    pub span: Span,
}

impl Owl2EquivalentClassAttr {
    pub fn new(equivalent_to: Text, span: Span) -> Self {
        Self { equivalent_to, span }
    }

    pub fn from_attribute(attr: &Attribute) -> Maybe<Self> {
        if !attr.is_named("owl2_equivalent_class") {
            return Maybe::None;
        }
        let args = match &attr.args {
            Maybe::Some(a) => a,
            Maybe::None    => return Maybe::None,
        };
        if args.len() != 1 {
            return Maybe::None;
        }
        let class = parse_class_name_arg(args.get(0).unwrap());
        match class {
            Some(c) => Maybe::Some(Self::new(c, attr.span)),
            None    => Maybe::None,
        }
    }
}

impl Spanned for Owl2EquivalentClassAttr {
    fn span(&self) -> Span {
        self.span
    }
}

/// `@owl2_has_key(<prop>, <prop>, ...)` — the NAMED-restricted key
/// constraint per Shkotin Table 9. The decorated class has a key
/// composed of the listed properties; two NamedIndividuals agreeing on
/// every key property must be the same individual.
///
/// The NAMED restriction means this constraint applies only to
/// `NamedIndividual`s, not anonymous individuals — a deliberate OWL 2
/// design decision per Shkotin §2.3.5; VUVA §21.3 routes HasKey
/// obligations to `@verify(proof)` because the DL reasoner case
/// requires a user-supplied tactic.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Owl2HasKeyAttr {
    pub key_properties: Vec<Text>,
    pub span: Span,
}

impl Owl2HasKeyAttr {
    pub fn new(key_properties: Vec<Text>, span: Span) -> Self {
        Self { key_properties, span }
    }

    pub fn from_attribute(attr: &Attribute) -> Maybe<Self> {
        if !attr.is_named("owl2_has_key") {
            return Maybe::None;
        }
        let args = match &attr.args {
            Maybe::Some(a) => a,
            Maybe::None    => return Maybe::None,
        };
        if args.is_empty() {
            return Maybe::None;
        }
        // Accept positional list of identifiers (the spec grammar form)
        // or a single bracketed list. Two equivalent shapes by analogy
        // with @owl2_disjoint_with.
        use crate::expr::{ArrayExpr, ExprKind};
        let mut props: Vec<Text> = Vec::new();
        if args.len() == 1 {
            let single = args.get(0).unwrap();
            match &single.kind {
                ExprKind::Array(ArrayExpr::List(elems)) => {
                    for elem in elems.iter() {
                        if let Some(name) = parse_class_name_arg(elem) {
                            props.push(name);
                        }
                    }
                }
                _ => {
                    if let Some(name) = parse_class_name_arg(single) {
                        props.push(name);
                    }
                }
            }
        } else {
            for arg in args.iter() {
                if let Some(name) = parse_class_name_arg(arg) {
                    props.push(name);
                }
            }
        }
        if props.is_empty() {
            return Maybe::None;
        }
        Maybe::Some(Self::new(props, attr.span))
    }
}

impl Spanned for Owl2HasKeyAttr {
    fn span(&self) -> Span {
        self.span
    }
}

// -----------------------------------------------------------------------------
// Internal helpers — extension of the OwlAttr parsing infrastructure
// -----------------------------------------------------------------------------

/// Parse a named-arg `key = ClassName` shape where the value is a Path
/// (rather than a string literal). Mirrors `parse_named_string_arg`
/// for the typed-class-reference case used by `@owl2_property(domain
/// = Animal)` etc.
fn parse_named_class_arg(
    arg: &crate::expr::Expr,
    expected_key: &str,
) -> Option<Text> {
    use crate::expr::{BinOp, ExprKind};
    let (left, right) = match &arg.kind {
        ExprKind::Binary { op: BinOp::Assign, left, right } => (left, right),
        _ => return None,
    };
    let key_ok = match &left.kind {
        ExprKind::Path(p) => p
            .segments
            .last()
            .and_then(|seg| match seg {
                crate::ty::PathSegment::Name(id) => Some(id.name.as_str() == expected_key),
                _ => None,
            })
            .unwrap_or(false),
        _ => false,
    };
    if !key_ok {
        return None;
    }
    parse_class_name_arg(right)
}

/// Parse a named-arg `characteristic = [Transitive, Symmetric, ...]`
/// shape. Returns the parsed list when the key matches and every
/// element is a recognised characteristic; None otherwise.
fn parse_named_characteristic_list_arg(
    arg: &crate::expr::Expr,
    expected_key: &str,
) -> Option<Vec<Owl2Characteristic>> {
    use crate::expr::{ArrayExpr, BinOp, ExprKind};
    let (left, right) = match &arg.kind {
        ExprKind::Binary { op: BinOp::Assign, left, right } => (left, right),
        _ => return None,
    };
    let key_ok = match &left.kind {
        ExprKind::Path(p) => p
            .segments
            .last()
            .and_then(|seg| match seg {
                crate::ty::PathSegment::Name(id) => Some(id.name.as_str() == expected_key),
                _ => None,
            })
            .unwrap_or(false),
        _ => false,
    };
    if !key_ok {
        return None;
    }
    let elems = match &right.kind {
        ExprKind::Array(ArrayExpr::List(es)) => es,
        _ => return None,
    };
    let mut chars: Vec<Owl2Characteristic> = Vec::new();
    for elem in elems.iter() {
        let name = parse_class_name_arg(elem)?;
        let ch = Owl2Characteristic::parse(name.as_str())?;
        chars.push(ch);
    }
    Some(chars)
}
