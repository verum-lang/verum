//! Graceful Fallback System
//!
//! Verum uses a two-tier execution model:
//! - Interpreter: VBC bytecode for development/debugging
//! - Aot: Native code via LLVM for production
//!
//! The fallback system ensures the compiler can always make progress:
//! - LLVM unavailable → Interpreter
//! - AOT compilation failure → Interpreter
//!
//! Note: JIT infrastructure is preserved as an internal component of the
//! AOT pipeline for REPL, incremental compilation, and hot reload.
//!
//! Graceful Fallback Guarantee: The compiler always produces a runnable result.
//! If AOT compilation fails (e.g., LLVM unavailable or codegen error), the
//! compiler falls back to VBC interpretation. This ensures developers always
//! get feedback, even in degraded environments. Fallback events are logged
//! as warnings so users know they are running at a lower tier.

use tracing::warn;
use verum_common::{List, Text};

// Import ExecutionTier from phases module (single source of truth)
use super::phases::ExecutionTier;

/// Fallback manager
pub struct GracefulFallback {
    /// Preferred execution tier
    preferred_tier: ExecutionTier,

    /// Current active tier
    active_tier: ExecutionTier,

    /// Fallback history
    fallback_history: List<FallbackEvent>,
}

#[derive(Debug, Clone)]
pub struct FallbackEvent {
    pub from: ExecutionTier,
    pub to: ExecutionTier,
    pub reason: Text,
}

impl GracefulFallback {
    pub fn new(preferred_tier: ExecutionTier) -> Self {
        Self {
            preferred_tier,
            active_tier: preferred_tier,
            fallback_history: List::new(),
        }
    }

    /// Attempt to use preferred tier
    pub fn try_preferred(&mut self) -> ExecutionTier {
        self.active_tier = self.preferred_tier;
        self.active_tier
    }

    /// Fallback to a lower tier (AOT → Interpreter)
    pub fn fallback(&mut self, reason: impl Into<Text>) -> ExecutionTier {
        let reason = reason.into();
        let from = self.active_tier;

        self.active_tier = match self.active_tier {
            ExecutionTier::Aot => {
                warn!("Falling back from AOT to Interpreter: {}", reason);
                ExecutionTier::Interpreter
            }
            ExecutionTier::Interpreter => {
                warn!("Already at interpreter tier: {}", reason);
                ExecutionTier::Interpreter
            }
        };

        self.fallback_history.push(FallbackEvent {
            from,
            to: self.active_tier,
            reason,
        });

        self.active_tier
    }

    /// Get current active tier
    pub fn active_tier(&self) -> ExecutionTier {
        self.active_tier
    }

    /// Check if LLVM is available
    pub fn llvm_available(&self) -> bool {
        #[cfg(feature = "llvm-codegen")]
        {
            use std::sync::Once;
            static LLVM_INIT: Once = Once::new();
            static mut LLVM_AVAILABLE: bool = false;

            LLVM_INIT.call_once(|| {
                use inkwell::targets::{InitializationConfig, Target};
                Target::initialize_native(&InitializationConfig::default())
                    .map(|_| unsafe { LLVM_AVAILABLE = true })
                    .ok();
            });

            unsafe { LLVM_AVAILABLE }
        }

        #[cfg(not(feature = "llvm-codegen"))]
        {
            false
        }
    }

    /// Check if JIT is available (internal AOT pipeline component)
    pub fn jit_available(&self) -> bool {
        if !self.llvm_available() {
            return false;
        }

        #[cfg(feature = "llvm-codegen")]
        {
            use std::sync::Once;
            static JIT_CHECK: Once = Once::new();
            static mut JIT_AVAILABLE: bool = false;

            JIT_CHECK.call_once(|| {
                use inkwell::OptimizationLevel;
                use inkwell::context::Context;

                let context = Context::create();
                let module = context.create_module("jit_check");

                match module.create_jit_execution_engine(OptimizationLevel::None) {
                    Ok(_) => unsafe { JIT_AVAILABLE = true },
                    Err(e) => {
                        tracing::debug!("JIT not available: {}", e.to_string());
                    }
                }
            });

            unsafe { JIT_AVAILABLE }
        }

        #[cfg(not(feature = "llvm-codegen"))]
        {
            false
        }
    }

    /// Get fallback history
    pub fn history(&self) -> &[FallbackEvent] {
        &self.fallback_history
    }

    /// Generate fallback report
    pub fn report(&self) -> Text {
        let mut report = Text::from("=== Graceful Fallback Report ===\n\n");

        report.push_str(&format!("Preferred Tier: {:?}\n", self.preferred_tier));
        report.push_str(&format!("Active Tier: {:?}\n\n", self.active_tier));

        if !self.fallback_history.is_empty() {
            report.push_str("Fallback History:\n");
            for event in &self.fallback_history {
                report.push_str(&format!(
                    "  {:?} → {:?}: {}\n",
                    event.from, event.to, event.reason
                ));
            }
        } else {
            report.push_str("No fallbacks occurred\n");
        }

        report
    }
}

impl Default for GracefulFallback {
    fn default() -> Self {
        Self::new(ExecutionTier::Interpreter)
    }
}
