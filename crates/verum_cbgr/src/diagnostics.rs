//! CBGR Diagnostics Integration
//!
//! This module converts CBGR analysis results into `verum_diagnostics::Diagnostic`
//! instances for consistent error reporting throughout the compiler.
//!
//! # Diagnostic Codes
//!
//! | Code | Category | Description |
//! |------|----------|-------------|
//! | E1001 | Memory | Use-after-free detected |
//! | E1002 | Memory | Double-free detected |
//! | W1003 | Memory | Potential memory leak |
//! | E1004 | Concurrency | Data race detected |
//! | E1005 | Concurrency | Potential deadlock |
//! | E1006 | Concurrency | Thread safety violation |
//! | E1007 | Lifetime | Lifetime violation |
//! | E1008 | Borrow | Borrow violation (NLL) |
//! | W1009 | Tier | Reference kept at Tier 0 |
//!
//! # Architecture
//!
//! ```text
//! TierAnalysisResult
//!        │
//!        ▼
//! CbgrDiagnostics::from_analysis_result()
//!        │
//!        ▼
//! Vec<verum_diagnostics::Diagnostic>
//! ```
//!
//! Converts CBGR tier analysis results into standardized verum_diagnostics::Diagnostic
//! instances. Reports memory safety issues (use-after-free E1001, double-free E1002,
//! leaks W1003), concurrency issues (data race E1004, deadlock E1005, thread safety
//! E1006), lifetime violations (E1007), borrow violations (E1008), and tier warnings
//! (W1009 for references that remain at Tier 0 when promotion might be possible).

use crate::analysis::Span as CbgrSpan;
use crate::concurrency_analysis::{
    ConcurrencyAnalysisResult, DataRaceWarning, DeadlockWarning, ThreadSafetyKind,
    ThreadSafetyViolation,
};
use crate::lifetime_analysis::{LifetimeAnalysisResult, LifetimeViolation, ViolationKind};
use crate::nll_analysis::{NllAnalysisResult, NllViolation, NllViolationKind};
use crate::ownership_analysis::{
    DoubleFreeWarning, LeakReason, LeakWarning, OwnershipAnalysisResult, UseAfterFreeWarning,
};
use crate::tier_analysis::TierAnalysisResult;
use crate::tier_types::{ReferenceTier, Tier0Reason};
use verum_common::{List, Text};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Span as DiagSpan};

// ============================================================================
// Diagnostic Codes
// ============================================================================

/// CBGR diagnostic codes.
pub mod codes {
    /// Use-after-free detected.
    pub const USE_AFTER_FREE: &str = "E1001";
    /// Double-free detected.
    pub const DOUBLE_FREE: &str = "E1002";
    /// Potential memory leak.
    pub const MEMORY_LEAK: &str = "W1003";
    /// Data race detected.
    pub const DATA_RACE: &str = "E1004";
    /// Potential deadlock.
    pub const DEADLOCK: &str = "E1005";
    /// Thread safety violation.
    pub const THREAD_SAFETY: &str = "E1006";
    /// Lifetime violation.
    pub const LIFETIME_VIOLATION: &str = "E1007";
    /// Borrow violation (NLL).
    pub const BORROW_VIOLATION: &str = "E1008";
    /// Reference kept at Tier 0.
    pub const TIER0_REFERENCE: &str = "W1009";
}

// ============================================================================
// Span Conversion
// ============================================================================

/// Convert CBGR Span to diagnostic Span.
fn cbgr_span_to_diag_span(span: CbgrSpan, file: &str) -> DiagSpan {
    DiagSpan {
        file: Text::from(file),
        line: span.0 as usize,
        column: span.1 as usize,
        end_line: None, // Single-line span
        end_column: (span.1 + 1) as usize,
    }
}

/// Create a synthetic span for a block ID (when source span unavailable).
fn block_id_span(block_id: crate::analysis::BlockId, file: &str) -> DiagSpan {
    DiagSpan {
        file: Text::from(file),
        line: block_id.0 as usize + 1, // 1-indexed
        column: 1,
        end_line: None, // Single-line span
        end_column: 1,
    }
}

/// Create a minimal fallback span.
fn fallback_span(file: &str) -> DiagSpan {
    DiagSpan {
        file: Text::from(file),
        line: 1,
        column: 1,
        end_line: None, // Single-line span
        end_column: 1,
    }
}

// ============================================================================
// CBGR Diagnostics Generator
// ============================================================================

/// Configuration for diagnostic generation.
#[derive(Debug, Clone)]
pub struct DiagnosticsConfig {
    /// Source file name for span generation.
    pub file_name: Text,
    /// Include Tier 0 reasons as notes.
    pub include_tier_reasons: bool,
    /// Minimum confidence threshold for warnings.
    pub confidence_threshold: f64,
    /// Include detailed help messages.
    pub include_help: bool,
    /// Include documentation URLs.
    pub include_doc_urls: bool,
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            file_name: Text::from("<source>"),
            include_tier_reasons: true,
            confidence_threshold: 0.7,
            include_help: true,
            include_doc_urls: true,
        }
    }
}

impl DiagnosticsConfig {
    /// Create config with file name.
    #[must_use]
    pub fn with_file(file: impl Into<Text>) -> Self {
        Self {
            file_name: file.into(),
            ..Default::default()
        }
    }
}

/// CBGR diagnostics generator.
///
/// Converts analysis results into standardized diagnostics.
pub struct CbgrDiagnostics {
    config: DiagnosticsConfig,
}

impl CbgrDiagnostics {
    /// Create new diagnostics generator with default config.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: DiagnosticsConfig::default(),
        }
    }

    /// Create with custom config.
    #[must_use]
    pub fn with_config(config: DiagnosticsConfig) -> Self {
        Self { config }
    }

    /// Generate all diagnostics from tier analysis result.
    #[must_use]
    pub fn from_analysis_result(&self, result: &TierAnalysisResult) -> List<Diagnostic> {
        let mut diagnostics = List::new();

        // Ownership analysis diagnostics
        if let Some(ref ownership) = result.ownership {
            diagnostics.extend(self.from_ownership_result(ownership));
        }

        // Concurrency analysis diagnostics
        if let Some(ref concurrency) = result.concurrency {
            diagnostics.extend(self.from_concurrency_result(concurrency));
        }

        // Lifetime analysis diagnostics
        if let Some(ref lifetime) = result.lifetime {
            diagnostics.extend(self.from_lifetime_result(lifetime));
        }

        // NLL analysis diagnostics
        if let Some(ref nll) = result.nll {
            diagnostics.extend(self.from_nll_result(nll));
        }

        // Tier 0 reason diagnostics (optional)
        if self.config.include_tier_reasons {
            diagnostics.extend(self.from_tier_decisions(&result.decisions));
        }

        diagnostics
    }

    /// Generate diagnostics from ownership analysis.
    #[must_use]
    pub fn from_ownership_result(&self, result: &OwnershipAnalysisResult) -> List<Diagnostic> {
        let mut diagnostics = List::new();

        // Use-after-free warnings
        for warning in &result.use_after_free_warnings {
            if warning.confidence >= self.config.confidence_threshold {
                diagnostics.push(self.use_after_free_diagnostic(warning));
            }
        }

        // Double-free warnings
        for warning in &result.double_free_warnings {
            if warning.confidence >= self.config.confidence_threshold {
                diagnostics.push(self.double_free_diagnostic(warning));
            }
        }

        // Leak warnings
        for warning in &result.leak_warnings {
            if warning.confidence >= self.config.confidence_threshold {
                diagnostics.push(self.leak_diagnostic(warning));
            }
        }

        diagnostics
    }

    /// Generate use-after-free diagnostic.
    fn use_after_free_diagnostic(&self, warning: &UseAfterFreeWarning) -> Diagnostic {
        let file = self.config.file_name.as_str();

        let primary_span = warning
            .use_span
            .map(|s| cbgr_span_to_diag_span(s, file))
            .unwrap_or_else(|| block_id_span(warning.use_site, file));

        let mut builder = DiagnosticBuilder::error()
            .code(codes::USE_AFTER_FREE)
            .message("use of memory after it has been freed")
            .span_label(primary_span.clone(), "memory used here after being freed");

        // Add secondary span for free site
        let free_span = block_id_span(warning.free_site, file);
        builder = builder.secondary_span(free_span, "memory was freed here");

        // Add help message
        if self.config.include_help {
            builder = builder.help(
                "ensure that references are not used after the memory they point to has been deallocated",
            );
            builder = builder.add_note(format!(
                "CBGR detected this issue with {:.0}% confidence",
                warning.confidence * 100.0
            ));
        }

        // Add documentation URL
        if self.config.include_doc_urls {
            builder = builder.doc_url("https://verum-lang.org/docs/errors/E1001");
        }

        builder.build()
    }

    /// Generate double-free diagnostic.
    fn double_free_diagnostic(&self, warning: &DoubleFreeWarning) -> Diagnostic {
        let file = self.config.file_name.as_str();

        let primary_span = warning
            .second_free_span
            .map(|s| cbgr_span_to_diag_span(s, file))
            .unwrap_or_else(|| block_id_span(warning.second_free_site, file));

        let mut builder = DiagnosticBuilder::error()
            .code(codes::DOUBLE_FREE)
            .message("memory freed more than once")
            .span_label(primary_span.clone(), "second free occurs here");

        // Add secondary spans
        if let Some(alloc_span) = warning.allocation_span {
            builder = builder.secondary_span(
                cbgr_span_to_diag_span(alloc_span, file),
                "memory allocated here",
            );
        }

        let first_free_span = warning
            .first_free_span
            .map(|s| cbgr_span_to_diag_span(s, file))
            .unwrap_or_else(|| block_id_span(warning.first_free_site, file));
        builder = builder.secondary_span(first_free_span, "first free occurs here");

        // Add help
        if self.config.include_help {
            builder = builder.help(
                "ensure each allocation is freed exactly once; consider using RAII patterns or smart pointers",
            );
        }

        if self.config.include_doc_urls {
            builder = builder.doc_url("https://verum-lang.org/docs/errors/E1002");
        }

        builder.build()
    }

    /// Generate memory leak diagnostic.
    fn leak_diagnostic(&self, warning: &LeakWarning) -> Diagnostic {
        let file = self.config.file_name.as_str();

        let primary_span = warning
            .allocation_span
            .map(|s| cbgr_span_to_diag_span(s, file))
            .unwrap_or_else(|| block_id_span(warning.allocation_site, file));

        let reason_msg = match warning.reason {
            LeakReason::NoDeallocation => "no deallocation found on any code path",
            LeakReason::PartialPaths => "some code paths do not deallocate this memory",
            LeakReason::OwnershipEscapes => "ownership escapes the function without being freed",
            LeakReason::StoredInContainer => "stored in a container that may outlive its scope",
            LeakReason::Unknown => "ownership analysis could not verify deallocation",
        };

        let mut builder = DiagnosticBuilder::warning()
            .code(codes::MEMORY_LEAK)
            .message("potential memory leak detected")
            .span_label(primary_span.clone(), "memory allocated here may not be freed")
            .add_note(reason_msg);

        if self.config.include_help {
            builder = builder.help(
                "ensure all allocated memory is properly deallocated; use RAII or ensure ownership transfer",
            );
        }

        if self.config.include_doc_urls {
            builder = builder.doc_url("https://verum-lang.org/docs/errors/W1003");
        }

        builder.build()
    }

    /// Generate diagnostics from concurrency analysis.
    #[must_use]
    pub fn from_concurrency_result(&self, result: &ConcurrencyAnalysisResult) -> List<Diagnostic> {
        let mut diagnostics = List::new();

        // Data race warnings
        for warning in &result.data_race_warnings {
            if warning.confidence >= self.config.confidence_threshold {
                diagnostics.push(self.data_race_diagnostic(warning));
            }
        }

        // Deadlock warnings
        for warning in &result.deadlock_warnings {
            diagnostics.push(self.deadlock_diagnostic(warning));
        }

        // Thread safety violations
        for violation in &result.thread_safety_violations {
            diagnostics.push(self.thread_safety_diagnostic(violation));
        }

        diagnostics
    }

    /// Generate data race diagnostic.
    fn data_race_diagnostic(&self, warning: &DataRaceWarning) -> Diagnostic {
        let file = self.config.file_name.as_str();

        let primary_span = warning
            .access1
            .span
            .map(|s| cbgr_span_to_diag_span(s, file))
            .unwrap_or_else(|| block_id_span(warning.access1.block, file));

        let access1_kind = if warning.access1.is_write() {
            "write"
        } else {
            "read"
        };
        let access2_kind = if warning.access2.is_write() {
            "write"
        } else {
            "read"
        };

        let mut builder = DiagnosticBuilder::error()
            .code(codes::DATA_RACE)
            .message(format!(
                "potential data race: {} and {} to same memory location without synchronization",
                access1_kind, access2_kind
            ))
            .span_label(primary_span.clone(), format!("{} access here", access1_kind));

        // Secondary span for the other access
        if let Some(span2) = warning.access2.span {
            builder = builder.secondary_span(
                cbgr_span_to_diag_span(span2, file),
                format!("conflicting {} access here", access2_kind),
            );
        }

        builder = builder.add_note(format!(
            "access1 in thread {:?}, access2 in thread {:?}",
            warning.access1.thread, warning.access2.thread
        ));

        if self.config.include_help {
            builder = builder.help(
                "use atomic operations, locks, or channels to synchronize access between threads",
            );
        }

        if self.config.include_doc_urls {
            builder = builder.doc_url("https://verum-lang.org/docs/errors/E1004");
        }

        builder.build()
    }

    /// Generate deadlock diagnostic.
    fn deadlock_diagnostic(&self, warning: &DeadlockWarning) -> Diagnostic {
        let file = self.config.file_name.as_str();

        // Deadlock doesn't have spans, use fallback
        let primary_span = fallback_span(file);

        let lock_count = warning.lock_cycle.len();
        let kind_msg = match warning.kind {
            crate::concurrency_analysis::DeadlockKind::SelfDeadlock => {
                "self-deadlock: thread tries to acquire lock it already holds"
            }
            crate::concurrency_analysis::DeadlockKind::LockOrderViolation => {
                "lock order violation: threads acquire locks in inconsistent order"
            }
            crate::concurrency_analysis::DeadlockKind::CyclicWait => {
                "cyclic wait: circular dependency detected in lock acquisition"
            }
            crate::concurrency_analysis::DeadlockKind::Unknown => {
                "potential deadlock detected in lock usage pattern"
            }
        };

        let mut builder = DiagnosticBuilder::error()
            .code(codes::DEADLOCK)
            .message(format!(
                "potential deadlock involving {} lock(s)",
                lock_count
            ))
            .span_label(primary_span.clone(), kind_msg);

        builder = builder.add_note(format!(
            "{} thread(s) involved in potential deadlock",
            warning.threads.len()
        ));

        if self.config.include_help {
            builder = builder.help(
                "establish a consistent lock ordering across all threads to prevent deadlock",
            );
        }

        if self.config.include_doc_urls {
            builder = builder.doc_url("https://verum-lang.org/docs/errors/E1005");
        }

        builder.build()
    }

    /// Generate thread safety violation diagnostic.
    fn thread_safety_diagnostic(&self, violation: &ThreadSafetyViolation) -> Diagnostic {
        let file = self.config.file_name.as_str();

        let primary_span = violation
            .span
            .map(|s| cbgr_span_to_diag_span(s, file))
            .unwrap_or_else(|| block_id_span(violation.block, file));

        let kind_msg = match violation.kind {
            ThreadSafetyKind::NotSend => "type is not Send: cannot be safely transferred between threads",
            ThreadSafetyKind::NotSync => "type is not Sync: cannot be safely shared between threads",
            ThreadSafetyKind::UnsafeMutableAlias => "unsafe mutable aliasing across threads",
        };

        let mut builder = DiagnosticBuilder::error()
            .code(codes::THREAD_SAFETY)
            .message(format!("thread safety violation: {}", kind_msg))
            .span_label(primary_span.clone(), kind_msg);

        if self.config.include_help {
            let help_msg = match violation.kind {
                ThreadSafetyKind::NotSend => {
                    "use types that implement Send, or use Arc<Mutex<T>> for thread-safe sharing"
                }
                ThreadSafetyKind::NotSync => {
                    "use types that implement Sync, or protect access with Mutex/RwLock"
                }
                ThreadSafetyKind::UnsafeMutableAlias => {
                    "avoid sharing mutable references across threads; use atomic types or synchronization"
                }
            };
            builder = builder.help(help_msg);
        }

        if self.config.include_doc_urls {
            builder = builder.doc_url("https://verum-lang.org/docs/errors/E1006");
        }

        builder.build()
    }

    /// Generate diagnostics from lifetime analysis.
    #[must_use]
    pub fn from_lifetime_result(&self, result: &LifetimeAnalysisResult) -> List<Diagnostic> {
        let mut diagnostics = List::new();

        for violation in &result.violations {
            diagnostics.push(self.lifetime_violation_diagnostic(violation));
        }

        diagnostics
    }

    /// Generate lifetime violation diagnostic.
    fn lifetime_violation_diagnostic(&self, violation: &LifetimeViolation) -> Diagnostic {
        let file = self.config.file_name.as_str();

        let primary_span = violation
            .span
            .map(|s| cbgr_span_to_diag_span(s, file))
            .unwrap_or_else(|| fallback_span(file));

        let kind_msg = match violation.kind {
            ViolationKind::RefOutlivesReferent => "reference outlives the value it refers to",
            ViolationKind::BorrowedValueNotLongEnough => "borrowed value does not live long enough",
            ViolationKind::UseAfterMove => "value used after being moved",
            ViolationKind::DanglingReference => "dangling reference detected",
            ViolationKind::ConflictingLifetimes => "conflicting lifetime requirements",
            ViolationKind::CannotReturnLocalRef => "cannot return reference to local variable",
            ViolationKind::ClosureCaptureLifetime => {
                "closure captures reference with insufficient lifetime"
            }
            ViolationKind::UnsatisfiableConstraint => "unsatisfiable lifetime constraint",
        };

        let mut builder = DiagnosticBuilder::error()
            .code(codes::LIFETIME_VIOLATION)
            .message(kind_msg)
            .span_label(primary_span.clone(), kind_msg);

        if self.config.include_help {
            let help_msg = match violation.kind {
                ViolationKind::RefOutlivesReferent | ViolationKind::BorrowedValueNotLongEnough => {
                    "consider extending the lifetime of the owner or cloning the value"
                }
                ViolationKind::UseAfterMove => {
                    "consider cloning the value before the move or restructuring ownership"
                }
                ViolationKind::DanglingReference => {
                    "ensure references point to valid memory throughout their lifetime"
                }
                ViolationKind::ConflictingLifetimes => {
                    "review the lifetime relationships and consider using explicit lifetime annotations"
                }
                ViolationKind::CannotReturnLocalRef => {
                    "return an owned value instead or use a parameter lifetime"
                }
                ViolationKind::ClosureCaptureLifetime => {
                    "use 'move' to take ownership or ensure captured references live long enough"
                }
                ViolationKind::UnsatisfiableConstraint => {
                    "review the lifetime constraints and restructure the code"
                }
            };
            builder = builder.help(help_msg);
        }

        if self.config.include_doc_urls {
            builder = builder.doc_url("https://verum-lang.org/docs/errors/E1007");
        }

        builder.build()
    }

    /// Generate diagnostics from NLL analysis.
    #[must_use]
    pub fn from_nll_result(&self, result: &NllAnalysisResult) -> List<Diagnostic> {
        let mut diagnostics = List::new();

        for violation in &result.violations {
            diagnostics.push(self.nll_violation_diagnostic(violation));
        }

        diagnostics
    }

    /// Generate NLL violation diagnostic.
    fn nll_violation_diagnostic(&self, violation: &NllViolation) -> Diagnostic {
        let file = self.config.file_name.as_str();

        let primary_span = violation
            .span
            .map(|s| cbgr_span_to_diag_span(s, file))
            .unwrap_or_else(|| fallback_span(file));

        let kind_msg = match &violation.kind {
            NllViolationKind::ConflictingBorrow { .. } => "conflicting borrows detected",
            NllViolationKind::UseWhileMutablyBorrowed { .. } => "value used while mutably borrowed",
            NllViolationKind::MutationWhileBorrowed { .. } => "mutation while value is borrowed",
            NllViolationKind::MoveWhileBorrowed { .. } => "value moved while borrowed",
            NllViolationKind::BorrowOutlivesData { .. } => "borrow outlives the borrowed data",
            NllViolationKind::ReturnLocalRef { .. } => {
                "cannot return reference to local variable"
            }
        };

        let mut builder = DiagnosticBuilder::error()
            .code(codes::BORROW_VIOLATION)
            .message(kind_msg)
            .span_label(primary_span.clone(), kind_msg);

        if self.config.include_help {
            let help_msg = match &violation.kind {
                NllViolationKind::ConflictingBorrow { .. } => {
                    "ensure borrows don't overlap; consider restructuring to avoid simultaneous borrows"
                }
                NllViolationKind::UseWhileMutablyBorrowed { .. } => {
                    "wait for the mutable borrow to end before using the value"
                }
                NllViolationKind::MutationWhileBorrowed { .. } => {
                    "wait for the borrow to end before mutating the value"
                }
                NllViolationKind::MoveWhileBorrowed { .. } => {
                    "wait for the borrow to end before moving the value"
                }
                NllViolationKind::BorrowOutlivesData { .. } => {
                    "ensure the borrowed data lives at least as long as the borrow"
                }
                NllViolationKind::ReturnLocalRef { .. } => {
                    "return an owned value or extend the data's lifetime with a parameter"
                }
            };
            builder = builder.help(help_msg);
        }

        if self.config.include_doc_urls {
            builder = builder.doc_url("https://verum-lang.org/docs/errors/E1008");
        }

        builder.build()
    }

    /// Generate diagnostics from tier decisions (informational).
    #[must_use]
    pub fn from_tier_decisions(
        &self,
        decisions: &verum_common::Map<crate::analysis::RefId, ReferenceTier>,
    ) -> List<Diagnostic> {
        let mut diagnostics = List::new();

        for (ref_id, tier) in decisions {
            if let ReferenceTier::Tier0 { reason } = tier {
                // Only emit for non-trivial reasons
                if !matches!(reason, Tier0Reason::NotAnalyzed | Tier0Reason::Conservative) {
                    diagnostics.push(self.tier0_diagnostic(*ref_id, reason));
                }
            }
        }

        diagnostics
    }

    /// Generate Tier 0 reason diagnostic (informational note).
    fn tier0_diagnostic(
        &self,
        _ref_id: crate::analysis::RefId,
        reason: &Tier0Reason,
    ) -> Diagnostic {
        let file = self.config.file_name.as_str();

        // Create a minimal span (this is informational)
        let span = fallback_span(file);

        let reason_msg = reason.description();

        let mut builder = DiagnosticBuilder::note_diag()
            .code(codes::TIER0_REFERENCE)
            .message(format!("reference kept at Tier 0: {}", reason_msg))
            .span(span);

        // Add reason-specific help
        if self.config.include_help {
            let help_msg = match reason {
                Tier0Reason::Escapes => {
                    "avoid storing references in data structures that outlive the current scope"
                }
                Tier0Reason::AsyncBoundary => {
                    "references used across await points require runtime validation"
                }
                Tier0Reason::ExceptionPath => {
                    "references on exception paths are kept at Tier 0 for safety"
                }
                Tier0Reason::ConcurrentAccess => {
                    "use atomics or synchronization for concurrent access"
                }
                Tier0Reason::ExternalCall => {
                    "references passed to external functions cannot be promoted"
                }
                Tier0Reason::UseAfterFree | Tier0Reason::DoubleFree | Tier0Reason::DataRace => {
                    "fix the memory safety issue to enable tier promotion"
                }
                _ => "this reference requires runtime CBGR validation (~15ns overhead)",
            };
            builder = builder.help(help_msg);
        }

        builder.build()
    }
}

impl Default for CbgrDiagnostics {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Convenience Functions
// ============================================================================

/// Generate all diagnostics from a tier analysis result.
#[must_use]
pub fn generate_diagnostics(result: &TierAnalysisResult) -> List<Diagnostic> {
    CbgrDiagnostics::new().from_analysis_result(result)
}

/// Generate diagnostics with custom config.
#[must_use]
pub fn generate_diagnostics_with_config(
    result: &TierAnalysisResult,
    config: DiagnosticsConfig,
) -> List<Diagnostic> {
    CbgrDiagnostics::with_config(config).from_analysis_result(result)
}

/// Check if tier analysis result has any errors.
#[must_use]
pub fn has_errors(result: &TierAnalysisResult) -> bool {
    result.has_safety_warnings()
}

/// Count total diagnostics that would be generated.
#[must_use]
pub fn diagnostic_count(result: &TierAnalysisResult) -> usize {
    result.total_warnings()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostic_codes() {
        assert_eq!(codes::USE_AFTER_FREE, "E1001");
        assert_eq!(codes::DOUBLE_FREE, "E1002");
        assert_eq!(codes::MEMORY_LEAK, "W1003");
        assert_eq!(codes::DATA_RACE, "E1004");
    }

    #[test]
    fn test_diagnostics_config_default() {
        let config = DiagnosticsConfig::default();
        assert!(config.include_tier_reasons);
        assert!(config.include_help);
        assert!(config.include_doc_urls);
        assert_eq!(config.confidence_threshold, 0.7);
    }

    #[test]
    fn test_cbgr_diagnostics_new() {
        let diag = CbgrDiagnostics::new();
        assert_eq!(diag.config.file_name.as_str(), "<source>");
    }

    #[test]
    fn test_empty_analysis_result() {
        let result = TierAnalysisResult::empty();
        let diagnostics = generate_diagnostics(&result);
        assert!(diagnostics.is_empty());
    }
}
