//! Kernel-rule re-checking verification pass (#187).
//!
//! Invokes the trusted-base K-rules from `verum_kernel` on every
//! function in the module. Currently runs `K-Refine-omega` over
//! every refinement type appearing in parameter / return positions;
//! V6/V7 also descend into impl-block methods, Theorem/Lemma/
//! Corollary/Axiom signatures, Module nested items, and
//! function-body let-binding refinements.
//!
//! # Why this is its own pass
//!
//! Kernel rules are the trusted base of the verification ladder
//! (VUVA §§9.2, 12.4). Routing them through a dedicated pass gives
//! three structural advantages:
//!
//!   1. *Defense-in-depth* — `SmtVerificationPass::verify_function`
//!      *also* runs the recheck preamble; with `KernelRecheckPass`
//!      in the default pipeline the K-rules fire even when SMT is
//!      disabled, so a non-SMT build still sees kernel-level
//!      formation errors.
//!   2. *Fail-fast* — K-rule violations are hard formation errors
//!      (`m_depth_omega(P) ≥ m_depth_omega(A) + 1` cannot be
//!      recovered by any SMT proof). Running the K-pass first and
//!      short-circuiting on failure saves the SMT round.
//!   3. *Diagnostic separation* — the `KernelRecheckResult` is its
//!      own pipeline-result row, not a confusing co-tenant of the
//!      SMT result.

use std::time::Instant;

use verum_ast::{FunctionDecl, Module};
use verum_common::{List, Text};

use crate::context::VerificationContext;
use crate::cost::VerificationCost;
use crate::kernel_recheck::KernelRecheck;
use crate::level::VerificationLevel;

use super::{VerificationError, VerificationPass, VerificationResult};

/// First-class verification pass that runs the K-rule recheck on
/// every refinement-bearing declaration in a module.
#[derive(Debug)]
pub struct KernelRecheckPass {
    /// Per-function rejection counts (recorded for diagnostics).
    rejections: List<Text>,
    /// V8 (#211, B12) — VFE governance policy. Default is
    /// [`ExtensionPolicy::AllRulesActive`], matching the pre-V8
    /// always-on behaviour. The opt-in / opt-out tiers are
    /// configurable via [`Self::with_policy`].
    policy: crate::extension_policy::ExtensionPolicy,
}

impl KernelRecheckPass {
    /// Create a new kernel-recheck pass.
    pub fn new() -> Self {
        Self {
            rejections: List::new(),
            policy: crate::extension_policy::ExtensionPolicy::AllRulesActive,
        }
    }

    /// V8 (#211, B12) — configure the VFE governance policy
    /// applied during this pass. The pass currently only gates
    /// VFE-7 (`vfe_7` = K-Refine-omega). When the policy reports
    /// `vfe_7` as inactive for the surrounding scope, this pass
    /// is a no-op for that scope (returns success without
    /// running the kernel-recheck walker). Pre-V8 always ran;
    /// V8 default still always runs (`AllRulesActive`).
    pub fn with_policy(mut self, policy: crate::extension_policy::ExtensionPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// V8 (#211, B12) — read-only accessor for the configured
    /// VFE governance policy.
    pub fn policy(&self) -> crate::extension_policy::ExtensionPolicy {
        self.policy
    }

    /// The K-rule rejection labels accumulated by the most recent
    /// `run`. Empty list when the module is K-rule clean.
    pub fn rejections(&self) -> &List<Text> {
        &self.rejections
    }

    /// Run the K-rules on a single function and append its
    /// per-call cost record to `costs`. Used by both the
    /// top-level Function arm and the V5 (#192) impl-method
    /// recursion.
    fn recheck_one_function(
        &mut self,
        func: &FunctionDecl,
        level: VerificationLevel,
        costs: &mut List<VerificationCost>,
    ) {
        let func_start = Instant::now();
        let outcomes = KernelRecheck::recheck_function(func);
        self.record_outcomes(&func.name.name, outcomes, level, func_start, costs);
    }

    /// V6 (#200) — dispatch K-rule recheck for a single ItemKind.
    /// Walks Function (V0/V4), Impl-block methods (V5), Theorem /
    /// Lemma / Corollary / Axiom signatures (V6), and Module
    /// nested items (V6). Other ItemKind variants are no-ops —
    /// they don't carry refinement types in syntactic positions
    /// the kernel-recheck cares about.
    fn recheck_one_item(
        &mut self,
        kind: &verum_ast::decl::ItemKind,
        level: VerificationLevel,
        costs: &mut List<VerificationCost>,
    ) {
        use verum_ast::decl::ItemKind as IK;
        match kind {
            IK::Function(func) => {
                self.recheck_one_function(func, level, costs);
            }
            IK::Impl(impl_decl) => {
                for impl_item in impl_decl.items.iter() {
                    if let verum_ast::decl::ImplItemKind::Function(func) =
                        &impl_item.kind
                    {
                        self.recheck_one_function(func, level, costs);
                    }
                }
            }
            IK::Theorem(d) | IK::Lemma(d) | IK::Corollary(d) => {
                let started = Instant::now();
                let outcomes = KernelRecheck::recheck_theorem(d);
                self.record_outcomes(&d.name.name, outcomes, level, started, costs);
            }
            IK::Axiom(a) => {
                let started = Instant::now();
                let outcomes = KernelRecheck::recheck_axiom(a);
                self.record_outcomes(&a.name.name, outcomes, level, started, costs);
            }
            IK::Module(m) => {
                if let verum_common::Maybe::Some(items) = &m.items {
                    for nested in items.iter() {
                        self.recheck_one_item(&nested.kind, level, costs);
                    }
                }
            }
            // Other variants (Const / Static / Mount / Meta /
            // Predicate / Context / ContextGroup / Layer /
            // FFIBoundary / Tactic / View / ExternBlock /
            // Pattern / Protocol) don't carry FunctionParam-shaped
            // signatures or composite types in syntactic positions
            // KernelRecheck visits today. If they later do, add
            // dedicated arms here.
            _ => {}
        }
    }

    /// Common bookkeeping: record per-decl K-rule outcomes into
    /// `costs` and accumulate failure labels into `self.rejections`.
    fn record_outcomes(
        &mut self,
        decl_name: &Text,
        outcomes: List<(Text, Result<(), crate::kernel_recheck::KernelRecheckError>)>,
        level: VerificationLevel,
        started: Instant,
        costs: &mut List<VerificationCost>,
    ) {
        let total = outcomes.len();
        let mut failures = 0usize;
        for (label, outcome) in outcomes.iter() {
            if outcome.is_err() {
                failures += 1;
                self.rejections.push(label.clone());
            }
        }
        costs.push(VerificationCost::new(
            decl_name.clone(),
            level,
            started.elapsed(),
            0,
            failures == 0,
            false,
            total,
        ));
    }
}

impl Default for KernelRecheckPass {
    fn default() -> Self {
        Self::new()
    }
}

impl VerificationPass for KernelRecheckPass {
    fn run(
        &mut self,
        module: &Module,
        ctx: &mut VerificationContext,
    ) -> Result<VerificationResult, VerificationError> {
        let start = Instant::now();
        let mut costs: List<VerificationCost> = List::new();
        self.rejections = List::new();

        // V8 (#211, B12) — read the module's `@require_extension`
        // / `@disable_extension` set, then ask the configured
        // policy whether VFE-7 (K-Refine-omega) is active. When
        // inactive, we skip the recheck walker entirely — no
        // outcomes accumulated, no rejection list populated.
        // The default policy (`AllRulesActive`) always returns
        // true, so existing test corpora continue to run the
        // walker as before.
        let extensions = crate::extension_policy::EnabledExtensions::from_module(module);
        let vfe_7_active = self.policy.is_active(&extensions, "vfe_7");

        let level = ctx.current_level();
        if vfe_7_active {
            for item in &module.items {
                self.recheck_one_item(&item.kind, level, &mut costs);
            }
        }

        let success = self.rejections.is_empty();
        let result = if success {
            VerificationResult::success(VerificationLevel::Runtime, start.elapsed(), costs)
        } else {
            // K-rule rejection is a hard error per the trusted-base
            // contract — the build cannot proceed past this.
            let mut r = VerificationResult::failure(VerificationLevel::Runtime, start.elapsed());
            r.costs = costs;
            r.functions_verified = r.costs.len();
            r
        };
        Ok(result)
    }

    fn name(&self) -> &str {
        "kernel_recheck"
    }
}
