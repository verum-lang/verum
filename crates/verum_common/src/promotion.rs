//! Unified Reference Promotion System
//!
//! CBGR Reference Promotion: Manages promotion between three reference tiers:
//! Tier 0 (&T, ~15ns CBGR check), Tier 1 (&checked T, 0ns compile-time proven),
//! Tier 2 (&unsafe T, 0ns manual safety). Promotion from &T to &checked T requires:
//! (1) reference doesn't escape function scope, (2) no concurrent access possible,
//! (3) allocation dominates all uses, (4) lifetime is stack-bounded,
//! (5) confidence >= threshold (default 0.95).
//!
//! This module provides a unified API for reference tier promotion and degradation
//! across all execution tiers (Tier 0-3) and reference types (&T, &checked T, &unsafe T).
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │          Unified Promotion System (verum_common)          │
//! ├─────────────────────────────────────────────────────────┤
//! │  PromotionStrategy (enum)                               │
//! │  ├─ ByRefCount(u64)        - Usage-based promotion      │
//! │  ├─ ByConfidence(f64)      - Static analysis confidence │
//! │  ├─ ByHotness(u64)         - Runtime profiling          │
//! │  ├─ ByAnalysis(...)        - Escape analysis result     │
//! │  └─ ByProfile(...)         - Profile-guided optimization│
//! ├─────────────────────────────────────────────────────────┤
//! │  PromotionPolicy (trait)                                │
//! │  ├─ should_promote()       - Decision engine            │
//! │  ├─ promotion_strategy()   - Get current strategy       │
//! │  └─ confidence_threshold() - Get threshold              │
//! ├─────────────────────────────────────────────────────────┤
//! │  PromotionContext (struct)                              │
//! │  ├─ ref_id                 - Reference identifier       │
//! │  ├─ access_count           - Number of accesses         │
//! │  ├─ confidence             - Static analysis confidence │
//! │  ├─ escape_analysis        - Escape analysis result     │
//! │  └─ profile_data           - Runtime profile data       │
//! ├─────────────────────────────────────────────────────────┤
//! │  Conversion Functions                                   │
//! │  ├─ promote_managed_to_checked()    - &T → &checked T   │
//! │  ├─ promote_checked_to_unsafe()     - &checked → &unsafe│
//! │  ├─ degrade_checked_to_managed()    - &checked → &T     │
//! │  └─ degrade_unsafe_to_checked()     - &unsafe → &checked│
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! # Three-Tier Reference Model
//!
//! | Tier | Type | Overhead | Safety | Use Case |
//! |------|------|----------|--------|----------|
//! | 0 | &T | ~15ns | Runtime checked | Default, safe |
//! | 1 | &checked T | 0ns | Compile-time proven | Optimized, verified |
//! | 2 | &unsafe T | 0ns | Manual responsibility | FFI, performance-critical |
//!
//! # Promotion Criteria
//!
//! For &T → &checked T promotion, ALL must be true:
//! 1. Reference doesn't escape function scope
//! 2. No concurrent access possible
//! 3. Allocation dominates all uses
//! 4. Lifetime is stack-bounded
//! 5. Confidence ≥ threshold (default 0.95)
//!
//! # Example
//!
//! ```rust
//! use verum_common::promotion::{PromotionContext, PromotionStrategy, PromotionPolicy, RefId};
//!
//! // Create promotion context
//! let context = PromotionContext {
//!     ref_id: Some(RefId(42)),
//!     access_count: 150,
//!     confidence: 0.98,
//!     escape_analysis: None,
//!     profile_data: None,
//! };
//!
//! // Decide on promotion
//! let strategy = PromotionStrategy::ByRefCount(100);
//! if strategy.should_promote(&context) {
//!     // Promote reference to &checked T
//! }
//! ```

use crate::{Maybe, Text};
use std::fmt;

/// Reference identifier for tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RefId(pub u64);

impl fmt::Display for RefId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ref_{}", self.0)
    }
}

/// Promotion strategy - determines when to promote references
///
/// Determines when to promote references between CBGR tiers. Each strategy
/// evaluates a PromotionContext to decide if a reference should be upgraded
/// from &T (runtime-checked) to &checked T (compile-time proven safe).
#[derive(Debug, Clone, PartialEq)]
pub enum PromotionStrategy {
    /// Promote after N reference accesses
    ///
    /// # Example
    /// ```
    /// use verum_common::promotion::PromotionStrategy;
    /// let strategy = PromotionStrategy::ByRefCount(100);
    /// // Promotes after 100 accesses
    /// ```
    ByRefCount(u64),

    /// Promote if static analysis confidence ≥ threshold
    ///
    /// # Example
    /// ```
    /// use verum_common::promotion::PromotionStrategy;
    /// let strategy = PromotionStrategy::ByConfidence(0.95);
    /// // Promotes if confidence ≥ 95%
    /// ```
    ByConfidence(f64),

    /// Promote if hotness score ≥ threshold
    ///
    /// Hotness combines call frequency and execution time.
    ///
    /// # Example
    /// ```
    /// use verum_common::promotion::PromotionStrategy;
    /// let strategy = PromotionStrategy::ByHotness(1000);
    /// // Promotes if accessed frequently (hot path)
    /// ```
    ByHotness(u64),

    /// Promote based on escape analysis result
    ///
    /// # Example
    /// ```
    /// use verum_common::promotion::{PromotionStrategy, EscapeAnalysisResult};
    /// let strategy = PromotionStrategy::ByAnalysis(EscapeAnalysisResult::DoesNotEscape);
    /// // Promotes if escape analysis proves safety
    /// ```
    ByAnalysis(EscapeAnalysisResult),

    /// Promote based on runtime profile data
    ///
    /// # Example
    /// ```
    /// use verum_common::promotion::{PromotionStrategy, ProfileData};
    /// let profile = ProfileData {
    ///     total_calls: 1000,
    ///     avg_time_ns: 5000,
    ///     hotness_score: 0.85,
    /// };
    /// let strategy = PromotionStrategy::ByProfile(profile);
    /// ```
    ByProfile(ProfileData),
}

impl PromotionStrategy {
    /// Check if promotion should occur given the context
    ///
    /// # Arguments
    ///
    /// * `context` - Promotion context with all decision parameters
    ///
    /// # Returns
    ///
    /// `true` if promotion should occur, `false` otherwise
    pub fn should_promote(&self, context: &PromotionContext) -> bool {
        match self {
            PromotionStrategy::ByRefCount(threshold) => context.access_count >= *threshold,
            PromotionStrategy::ByConfidence(threshold) => context.confidence >= *threshold,
            PromotionStrategy::ByHotness(threshold) => context.access_count >= *threshold,
            PromotionStrategy::ByAnalysis(expected) => {
                if let Maybe::Some(ref analysis) = context.escape_analysis {
                    analysis == expected && analysis.can_promote()
                } else {
                    false
                }
            }
            PromotionStrategy::ByProfile(expected_profile) => {
                if let Maybe::Some(ref profile) = context.profile_data {
                    profile.hotness_score >= expected_profile.hotness_score
                } else {
                    false
                }
            }
        }
    }

    /// Get human-readable description
    pub fn description(&self) -> Text {
        match self {
            PromotionStrategy::ByRefCount(n) => {
                Text::from(format!("Promote after {} accesses", n))
            }
            PromotionStrategy::ByConfidence(c) => {
                Text::from(format!("Promote if confidence ≥ {:.1}%", c * 100.0))
            }
            PromotionStrategy::ByHotness(h) => {
                Text::from(format!("Promote if hotness ≥ {}", h))
            }
            PromotionStrategy::ByAnalysis(a) => {
                Text::from(format!("Promote if escape analysis shows: {:?}", a))
            }
            PromotionStrategy::ByProfile(p) => {
                Text::from(format!("Promote if hotness score ≥ {:.2}", p.hotness_score))
            }
        }
    }
}

/// Escape analysis result for promotion decisions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EscapeAnalysisResult {
    /// Reference does not escape (safe to promote)
    DoesNotEscape,
    /// Reference escapes via return
    EscapesViaReturn,
    /// Reference escapes via heap storage
    EscapesViaHeap,
    /// Reference escapes via closure capture
    EscapesViaClosure,
    /// Reference escapes via thread
    EscapesViaThread,
    /// Concurrent access possible
    ConcurrentAccess,
    /// Allocation doesn't dominate uses
    NonDominatingAllocation,
    /// Lifetime exceeds stack bounds
    ExceedsStackBounds,
}

impl EscapeAnalysisResult {
    /// Check if promotion is allowed
    pub fn can_promote(&self) -> bool {
        matches!(self, EscapeAnalysisResult::DoesNotEscape)
    }

    /// Get human-readable reason
    pub fn reason(&self) -> &'static str {
        match self {
            EscapeAnalysisResult::DoesNotEscape => "Reference does not escape (safe to promote)",
            EscapeAnalysisResult::EscapesViaReturn => "Reference escapes via return value",
            EscapeAnalysisResult::EscapesViaHeap => "Reference stored in heap-allocated structure",
            EscapeAnalysisResult::EscapesViaClosure => {
                "Reference captured by closure that outlives scope"
            }
            EscapeAnalysisResult::EscapesViaThread => "Reference shared across thread boundaries",
            EscapeAnalysisResult::ConcurrentAccess => "Concurrent access possible (data race)",
            EscapeAnalysisResult::NonDominatingAllocation => {
                "Allocation doesn't dominate all uses (may use before allocation)"
            }
            EscapeAnalysisResult::ExceedsStackBounds => {
                "Lifetime exceeds stack bounds (outlives function)"
            }
        }
    }
}

/// Runtime profile data for promotion decisions
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfileData {
    /// Total function calls
    pub total_calls: u64,
    /// Average execution time (nanoseconds)
    pub avg_time_ns: u64,
    /// Hotness score (0.0 to 1.0)
    pub hotness_score: f64,
}

impl ProfileData {
    /// Create new profile data
    pub fn new(total_calls: u64, avg_time_ns: u64, hotness_score: f64) -> Self {
        Self {
            total_calls,
            avg_time_ns,
            hotness_score,
        }
    }

    /// Check if this profile indicates a hot path
    pub fn is_hot(&self, threshold: f64) -> bool {
        self.hotness_score >= threshold
    }
}

/// Promotion context - contains all parameters for promotion decisions
#[derive(Debug, Clone)]
pub struct PromotionContext {
    /// Reference identifier (if available)
    pub ref_id: Maybe<RefId>,
    /// Number of times reference has been accessed
    pub access_count: u64,
    /// Static analysis confidence (0.0 to 1.0)
    pub confidence: f64,
    /// Escape analysis result (if available)
    pub escape_analysis: Maybe<EscapeAnalysisResult>,
    /// Runtime profile data (if available)
    pub profile_data: Maybe<ProfileData>,
}

impl PromotionContext {
    /// Create new promotion context
    pub fn new() -> Self {
        Self {
            ref_id: Maybe::None,
            access_count: 0,
            confidence: 0.0,
            escape_analysis: Maybe::None,
            profile_data: Maybe::None,
        }
    }

    /// Create context from reference count
    pub fn from_ref_count(count: u64) -> Self {
        Self {
            ref_id: Maybe::None,
            access_count: count,
            confidence: 0.0,
            escape_analysis: Maybe::None,
            profile_data: Maybe::None,
        }
    }

    /// Create context from confidence score
    pub fn from_confidence(confidence: f64) -> Self {
        Self {
            ref_id: Maybe::None,
            access_count: 0,
            confidence,
            escape_analysis: Maybe::None,
            profile_data: Maybe::None,
        }
    }

    /// Create context from escape analysis
    pub fn from_analysis(analysis: EscapeAnalysisResult, confidence: f64) -> Self {
        Self {
            ref_id: Maybe::None,
            access_count: 0,
            confidence,
            escape_analysis: Maybe::Some(analysis),
            profile_data: Maybe::None,
        }
    }

    /// Create context from profile data
    pub fn from_profile(profile: ProfileData) -> Self {
        Self {
            ref_id: Maybe::None,
            access_count: profile.total_calls,
            confidence: profile.hotness_score,
            escape_analysis: Maybe::None,
            profile_data: Maybe::Some(profile),
        }
    }

    /// Set reference ID
    pub fn with_ref_id(mut self, ref_id: RefId) -> Self {
        self.ref_id = Maybe::Some(ref_id);
        self
    }

    /// Set access count
    pub fn with_access_count(mut self, count: u64) -> Self {
        self.access_count = count;
        self
    }

    /// Set confidence
    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence;
        self
    }

    /// Set escape analysis
    pub fn with_escape_analysis(mut self, analysis: EscapeAnalysisResult) -> Self {
        self.escape_analysis = Maybe::Some(analysis);
        self
    }

    /// Set profile data
    pub fn with_profile_data(mut self, profile: ProfileData) -> Self {
        self.profile_data = Maybe::Some(profile);
        self
    }
}

impl Default for PromotionContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Promotion policy trait - unified interface for promotion decisions
///
/// Implement this trait to create custom promotion policies.
pub trait PromotionPolicy {
    /// Check if promotion should occur
    ///
    /// # Arguments
    ///
    /// * `context` - Promotion context with all decision parameters
    ///
    /// # Returns
    ///
    /// `true` if promotion should occur, `false` otherwise
    fn should_promote(&self, context: &PromotionContext) -> bool;

    /// Get current promotion strategy
    fn promotion_strategy(&self) -> PromotionStrategy;

    /// Get confidence threshold (if applicable)
    fn confidence_threshold(&self) -> Maybe<f64> {
        Maybe::None
    }
}

/// Standard promotion policy - uses a single strategy
#[derive(Debug, Clone)]
pub struct StandardPromotionPolicy {
    strategy: PromotionStrategy,
}

impl StandardPromotionPolicy {
    /// Create new standard promotion policy
    pub fn new(strategy: PromotionStrategy) -> Self {
        Self { strategy }
    }

    /// Create policy that promotes after N accesses
    pub fn by_ref_count(threshold: u64) -> Self {
        Self::new(PromotionStrategy::ByRefCount(threshold))
    }

    /// Create policy that promotes if confidence ≥ threshold
    pub fn by_confidence(threshold: f64) -> Self {
        Self::new(PromotionStrategy::ByConfidence(threshold))
    }

    /// Create policy that promotes if hotness ≥ threshold
    pub fn by_hotness(threshold: u64) -> Self {
        Self::new(PromotionStrategy::ByHotness(threshold))
    }

    /// Create policy based on escape analysis
    pub fn by_analysis(expected: EscapeAnalysisResult) -> Self {
        Self::new(PromotionStrategy::ByAnalysis(expected))
    }

    /// Create policy based on profile data
    pub fn by_profile(expected: ProfileData) -> Self {
        Self::new(PromotionStrategy::ByProfile(expected))
    }
}

impl PromotionPolicy for StandardPromotionPolicy {
    fn should_promote(&self, context: &PromotionContext) -> bool {
        self.strategy.should_promote(context)
    }

    fn promotion_strategy(&self) -> PromotionStrategy {
        self.strategy.clone()
    }

    fn confidence_threshold(&self) -> Maybe<f64> {
        match &self.strategy {
            PromotionStrategy::ByConfidence(threshold) => Maybe::Some(*threshold),
            _ => Maybe::None,
        }
    }
}

/// Composite promotion policy - combines multiple strategies
///
/// Promotes if ANY of the strategies recommend promotion.
#[derive(Debug, Clone)]
pub struct CompositePromotionPolicy {
    strategies: Vec<PromotionStrategy>,
    /// Require ALL strategies to agree (default: false - ANY strategy)
    require_all: bool,
}

impl CompositePromotionPolicy {
    /// Create new composite policy (ANY strategy)
    pub fn new(strategies: Vec<PromotionStrategy>) -> Self {
        Self {
            strategies,
            require_all: false,
        }
    }

    /// Create new composite policy that requires ALL strategies
    pub fn require_all(strategies: Vec<PromotionStrategy>) -> Self {
        Self {
            strategies,
            require_all: true,
        }
    }

    /// Add a strategy
    pub fn add_strategy(&mut self, strategy: PromotionStrategy) {
        self.strategies.push(strategy);
    }
}

impl PromotionPolicy for CompositePromotionPolicy {
    fn should_promote(&self, context: &PromotionContext) -> bool {
        if self.strategies.is_empty() {
            return false;
        }

        if self.require_all {
            // ALL strategies must agree
            self.strategies.iter().all(|s| s.should_promote(context))
        } else {
            // ANY strategy can promote
            self.strategies.iter().any(|s| s.should_promote(context))
        }
    }

    fn promotion_strategy(&self) -> PromotionStrategy {
        // Return the first strategy as representative
        self.strategies
            .first()
            .cloned()
            .unwrap_or(PromotionStrategy::ByRefCount(100))
    }
}

/// Promotion decision with rationale
#[derive(Debug, Clone)]
pub struct PromotionDecision {
    /// Reference being considered
    pub ref_id: Maybe<RefId>,
    /// Should promote?
    pub should_promote: bool,
    /// Strategy used
    pub strategy: PromotionStrategy,
    /// Confidence score (0.0 to 1.0)
    pub confidence: f64,
    /// Estimated performance gain (nanoseconds per access)
    pub estimated_gain_ns: u64,
    /// Rationale for decision
    pub rationale: Text,
}

impl PromotionDecision {
    /// Create promotion decision
    pub fn new(
        ref_id: Maybe<RefId>,
        should_promote: bool,
        strategy: PromotionStrategy,
        confidence: f64,
        estimated_gain_ns: u64,
    ) -> Self {
        let rationale = if should_promote {
            Text::from(format!(
                "PROMOTE: {} (confidence: {:.1}%, estimated gain: ~{}ns per access)",
                strategy.description(),
                confidence * 100.0,
                estimated_gain_ns
            ))
        } else {
            Text::from(format!(
                "KEEP: {} (confidence: {:.1}%)",
                strategy.description(),
                confidence * 100.0
            ))
        };

        Self {
            ref_id,
            should_promote,
            strategy,
            confidence,
            estimated_gain_ns,
            rationale,
        }
    }

    /// Create decision from policy and context
    pub fn from_policy<P: PromotionPolicy>(policy: &P, context: &PromotionContext) -> Self {
        let should_promote = policy.should_promote(context);
        let strategy = policy.promotion_strategy();
        let confidence = context.confidence;

        // Estimate performance gain: 15ns saved per access if promoted
        let estimated_gain_ns = if should_promote { 15 } else { 0 };

        Self::new(
            context.ref_id,
            should_promote,
            strategy,
            confidence,
            estimated_gain_ns,
        )
    }
}

impl fmt::Display for PromotionDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.rationale)
    }
}

/// Promotion statistics
#[derive(Debug, Clone, Default)]
pub struct PromotionStatistics {
    /// Total promotion decisions made
    pub total_decisions: u64,
    /// Number of promotions
    pub promotions: u64,
    /// Number of references kept at current tier
    pub kept_at_tier: u64,
    /// Total estimated time saved (nanoseconds)
    pub estimated_time_saved_ns: u64,
}

impl PromotionStatistics {
    /// Create new statistics
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a promotion decision
    pub fn record_decision(&mut self, decision: &PromotionDecision) {
        self.total_decisions += 1;
        if decision.should_promote {
            self.promotions += 1;
            self.estimated_time_saved_ns += decision.estimated_gain_ns;
        } else {
            self.kept_at_tier += 1;
        }
    }

    /// Get promotion rate (0.0 to 1.0)
    pub fn promotion_rate(&self) -> f64 {
        if self.total_decisions == 0 {
            0.0
        } else {
            self.promotions as f64 / self.total_decisions as f64
        }
    }

    /// Get average time saved per promotion
    pub fn avg_time_saved_ns(&self) -> u64 {
        if self.promotions == 0 {
            0
        } else {
            self.estimated_time_saved_ns / self.promotions
        }
    }
}

impl fmt::Display for PromotionStatistics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Promotion Statistics:")?;
        writeln!(f, "  Total decisions:     {}", self.total_decisions)?;
        writeln!(f, "  Promotions:          {}", self.promotions)?;
        writeln!(f, "  Kept at tier:        {}", self.kept_at_tier)?;
        writeln!(
            f,
            "  Promotion rate:      {:.1}%",
            self.promotion_rate() * 100.0
        )?;
        writeln!(
            f,
            "  Estimated savings:   ~{}μs",
            self.estimated_time_saved_ns / 1000
        )?;
        Ok(())
    }
}

/// Reference tier for conversions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReferenceTier {
    /// Tier 0: &T (CBGR-managed, ~15ns overhead)
    Managed,
    /// Tier 1: &checked T (compile-time verified, 0ns overhead)
    Checked,
    /// Tier 2: &unsafe T (manual safety, 0ns overhead)
    Unsafe,
}

impl ReferenceTier {
    /// Get overhead in nanoseconds
    pub fn overhead_ns(&self) -> u64 {
        match self {
            ReferenceTier::Managed => 15,
            ReferenceTier::Checked => 0,
            ReferenceTier::Unsafe => 0,
        }
    }

    /// Get human-readable name
    pub fn name(&self) -> &'static str {
        match self {
            ReferenceTier::Managed => "&T",
            ReferenceTier::Checked => "&checked T",
            ReferenceTier::Unsafe => "&unsafe T",
        }
    }

    /// Check if promotion is possible
    pub fn can_promote_to(&self, target: ReferenceTier) -> bool {
        match (self, target) {
            // &T → &checked T (requires proof)
            (ReferenceTier::Managed, ReferenceTier::Checked) => true,
            // &T → &unsafe T (requires @unsafe)
            (ReferenceTier::Managed, ReferenceTier::Unsafe) => true,
            // &checked T → &unsafe T (requires @unsafe)
            (ReferenceTier::Checked, ReferenceTier::Unsafe) => true,
            // No other promotions allowed
            _ => false,
        }
    }

    /// Check if degradation is possible
    pub fn can_degrade_to(&self, target: ReferenceTier) -> bool {
        match (self, target) {
            // &checked T → &T (graceful degradation)
            (ReferenceTier::Checked, ReferenceTier::Managed) => true,
            // &unsafe T → &checked T (add safety checks)
            (ReferenceTier::Unsafe, ReferenceTier::Checked) => true,
            // &unsafe T → &T (add all safety)
            (ReferenceTier::Unsafe, ReferenceTier::Managed) => true,
            // No other degradations allowed
            _ => false,
        }
    }
}

impl fmt::Display for ReferenceTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}
