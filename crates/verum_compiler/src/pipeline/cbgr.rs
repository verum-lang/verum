//! CBGR (Capability-Based Generation References) tier analysis.
//!
//! Extracted from `pipeline.rs` (#106 Phase 9). Performs tier
//! analysis on all functions in the module:
//!
//!   1. Builds control flow graphs (CFGs) for each function.
//!   2. Runs escape analysis to determine reference tier selection.
//!   3. Decides which references can be promoted from Tier 0
//!      (~15ns managed) to Tier 1 (0ns compiler-proven safe,
//!      `&checked T`).
//!   4. Logs analysis statistics for optimisation feedback.
//!
//! The cluster is the second-largest single concern in pipeline.rs
//! (~1794 LOC across 14 methods): one orchestrator
//! (`phase_cbgr_analysis`) plus the AST-to-CFG construction
//! family (`build_function_cfg`, `build_body_cfg`,
//! `build_block_cfg`, `build_if_cfg`, `build_match_cfg`,
//! `build_loop_cfg`, `build_while_cfg`, `build_for_cfg`) and the
//! def-use extraction family
//! (`extract_defs_and_uses_from_condition`,
//! `extract_defs_and_uses_from_expr`,
//! `extract_defs_and_uses_from_stmt`,
//! `collect_exit_predecessors`).  Plus two leaf utilities
//! (`param_is_reference`, `hash_function_name`).

use std::time::Instant;

use anyhow::Result;
use tracing::{debug, info};

use verum_ast::{decl::ItemKind, Module};
use verum_common::List;

use super::{CfgBuildContext, CompilationPipeline};

impl<'s> CompilationPipeline<'s> {
    /// Phase 4a: Tier analysis
    ///
    /// Performs tier analysis on all functions in the module. This phase:
    /// 1. Builds control flow graphs (CFGs) for each function
    /// 2. Runs escape analysis to determine reference tier selection
    /// 3. Decides which references can be promoted from Tier 0 (~15ns) to Tier 1 (0ns)
    /// 4. Logs analysis statistics for optimization feedback
    ///
    /// CBGR analysis: builds CFGs, runs escape analysis to promote references from
    /// Tier 0 (~15ns managed) to Tier 1 (0ns compiler-proven safe, `&checked T`).
    pub(super) fn phase_cbgr_analysis(&self, module: &Module) -> Result<()> {
        use verum_cbgr::tier_analysis::{TierAnalysisConfig, TierAnalyzer};
        use verum_cbgr::tier_types::TierStatistics;
        use crate::session::FunctionId;

        // Gate on [runtime].cbgr_mode:
        //   "unsafe" → skip analysis entirely (all refs are raw)
        //   "managed" → skip promotion (all refs stay at Tier 0)
        //   "checked" / "mixed" → full analysis (current behavior)
        let cbgr_mode = self
            .session
            .language_features()
            .runtime
            .cbgr_mode
            .as_str()
            .to_string();
        if cbgr_mode == "unsafe" {
            tracing::debug!(
                "CBGR analysis SKIPPED ([runtime] cbgr_mode = \"unsafe\")"
            );
            return Ok(());
        }

        debug!("Running tier analysis (cbgr_mode = {})", cbgr_mode);
        let start = Instant::now();

        // Create tier analysis configuration based on [runtime].cbgr_mode.
        // "managed" → disable promotion (nothing can be promoted to checked).
        // "checked" / "mixed" → full analysis.
        // (enable_promotion would gate tier promotion if the analyzer API
        //  accepted it; TierAnalysisConfig does not currently expose a
        //  flag for it, so we record the decision here for documentation.)
        let _enable_promotion = cbgr_mode != "managed";
        let config = TierAnalysisConfig {
            confidence_threshold: 0.95,
            analyze_async_boundaries: true,
            analyze_exception_paths: true,
            enable_ownership_analysis: true,
            enable_concurrency_analysis: true,
            enable_lifetime_analysis: true,
            enable_nll_analysis: true,
            max_iterations: 1000,
            timeout_ms: 5000,
        };

        // Process each function in the module.
        //
        // #118 follow-up — per-function tier analysis is embarrassingly
        // parallel: each `TierAnalyzer` runs its 9-phase escape /
        // dominance / ownership / concurrency / lifetime / NLL /
        // tier-determination / cross-fn / final passes over an
        // independent CFG, with no shared mutable state.  Statistics
        // and cache writes both go through `&self` Session methods
        // (`cache_tier_analysis` uses an internal RwLock, statistics
        // merge is sequential under one Mutex).
        //
        // Opt-out via `VERUM_NO_PARALLEL_CBGR=1` for diagnostic
        // ordering and parallel-only regression triage.
        let parallel_cbgr = std::env::var("VERUM_NO_PARALLEL_CBGR").is_err();

        let funcs: Vec<&verum_ast::decl::FunctionDecl> = module
            .items
            .iter()
            .filter_map(|item| {
                if let ItemKind::Function(func) = &item.kind {
                    if func.is_meta { None } else { Some(func) }
                } else {
                    None
                }
            })
            .collect();

        let stats_mu = std::sync::Mutex::new(TierStatistics::new());
        let analyse_one = |func: &verum_ast::decl::FunctionDecl| {
            let cfg = self.build_function_cfg(func);
            let function_id = FunctionId(Self::hash_function_name(&func.name.name));
            let analyzer = TierAnalyzer::with_config(cfg, config.clone());
            let result = analyzer.analyze();

            if result.stats.total_refs > 0 {
                debug!(
                    "  Function '{}': {} refs, {} T1, {} T0 ({:.1}% promoted)",
                    func.name.name,
                    result.stats.total_refs,
                    result.stats.tier1_count,
                    result.stats.tier0_count,
                    result.stats.promotion_rate() * 100.0
                );
            }
            // Merge statistics under a single short-held Mutex.
            stats_mu.lock().unwrap().merge(&result.stats);
            // Cache for codegen phase. `cache_tier_analysis` is `&self`
            // with internal RwLock — safe under concurrent writes.
            self.session.cache_tier_analysis(function_id, result);
        };

        if parallel_cbgr && funcs.len() > 1 {
            use rayon::prelude::*;
            funcs.par_iter().copied().for_each(analyse_one);
        } else {
            for func in &funcs {
                analyse_one(func);
            }
        }

        let global_stats = stats_mu.into_inner().unwrap();

        let elapsed = start.elapsed();

        // Report summary statistics
        if global_stats.functions_analyzed > 0 {
            debug!(
                "Tier analysis completed in {:.2}ms: {} functions, {} refs, {} promoted ({:.1}%)",
                elapsed.as_millis(),
                global_stats.functions_analyzed,
                global_stats.total_refs,
                global_stats.tier1_count,
                global_stats.promotion_rate() * 100.0
            );

            // At higher verbosity, show full statistics
            if self.session.options().verbose >= 2 {
                info!("{}", global_stats);
            }
        } else {
            debug!(
                "Tier analysis completed in {:.2}ms (no functions analyzed)",
                elapsed.as_millis()
            );
        }

        // Update global statistics in session for reporting
        self.session.merge_tier_statistics(&global_stats);

        Ok(())
    }

    /// Build a control flow graph from a function declaration
    ///
    /// Creates a complete CFG for escape analysis. This builds:
    /// - Entry block for function entry with parameter definitions
    /// - Blocks for if/else branches
    /// - Blocks for match arms
    /// - Loop header and body blocks
    /// - Exit blocks
    /// - Control flow edges between blocks
    ///
    /// CFG construction for escape analysis: creates basic blocks for branches,
    /// match arms, loops, with control flow edges for dataflow analysis.
    pub(super) fn build_function_cfg(
        &self,
        func: &verum_ast::decl::FunctionDecl,
    ) -> verum_cbgr::analysis::ControlFlowGraph {
        use verum_cbgr::CfgBuilder;
        use verum_cbgr::analysis::{DefSite, RefId};

        let mut builder = CfgBuilder::new();
        let mut ref_counter = 0u64;

        // Entry and exit blocks
        let entry_id = builder.new_block_id();
        let exit_id = builder.new_block_id();

        // Create entry block with parameter definitions
        let mut param_defs = List::new();
        for param in func.params.iter() {
            // Each parameter is a reference definition
            if Self::param_is_reference(param) {
                param_defs.push(DefSite {
                    block: entry_id,
                    reference: RefId(ref_counter),
                    is_stack_allocated: true, // Parameters are stack-allocated
                    span: None,
                });
                ref_counter += 1;
            }
        }

        // Start building CFG with basic structure
        let mut cfg = builder.build_cfg(entry_id, exit_id);

        // Build function body CFG if present
        if let Some(ref body) = func.body {
            // Create a context for building blocks
            let mut ctx = CfgBuildContext {
                builder: &mut builder,
                ref_counter: &mut ref_counter,
                entry_id,
                exit_id,
                pending_blocks: List::new(),
                closure_captures: List::new(),
            };

            // Build the body, which returns the first block after entry
            let body_start = self.build_body_cfg(body, &mut ctx, &mut cfg);

            // Connect entry to body start
            let entry_successors = if body_start != entry_id {
                let mut succs = verum_common::Set::new();
                succs.insert(body_start);
                succs
            } else {
                let mut succs = verum_common::Set::new();
                succs.insert(exit_id);
                succs
            };

            // Build entry block with proper successors
            let entry_block = ctx.builder.build_block(
                entry_id,
                verum_common::Set::new(), // No predecessors for entry
                entry_successors,
                param_defs,
                List::new(),
            );
            cfg.add_block(entry_block);

            // Add all pending blocks to CFG
            for block in ctx.pending_blocks.drain(..) {
                cfg.add_block(block);
            }

            // Build exit block with collected predecessors
            let exit_preds = self.collect_exit_predecessors(&cfg, exit_id);
            let exit_block = ctx.builder.build_block(
                exit_id,
                exit_preds,
                verum_common::Set::new(), // No successors for exit
                List::new(),
                List::new(),
            );
            cfg.add_block(exit_block);
        } else {
            // No body - entry connects directly to exit
            let entry_block = builder.build_block(
                entry_id,
                verum_common::Set::new(),
                {
                    let mut succs = verum_common::Set::new();
                    succs.insert(exit_id);
                    succs
                },
                param_defs,
                List::new(),
            );

            let exit_block = builder.build_block(
                exit_id,
                {
                    let mut preds = verum_common::Set::new();
                    preds.insert(entry_id);
                    preds
                },
                verum_common::Set::new(),
                List::new(),
                List::new(),
            );

            cfg.add_block(entry_block);
            cfg.add_block(exit_block);
        }

        cfg
    }

    /// Build CFG for a function body
    pub(super) fn build_body_cfg(
        &self,
        body: &verum_ast::decl::FunctionBody,
        ctx: &mut CfgBuildContext<'_>,
        cfg: &mut verum_cbgr::analysis::ControlFlowGraph,
    ) -> verum_cbgr::analysis::BlockId {
        use verum_ast::decl::FunctionBody;

        match body {
            FunctionBody::Block(block) => self.build_block_cfg(block, ctx, cfg, ctx.exit_id),
            FunctionBody::Expr(expr) => {
                // Single expression body - create a block for it
                let block_id = ctx.builder.new_block_id();
                let mut defs = List::new();
                let mut uses = List::new();

                self.extract_defs_and_uses_from_expr(
                    expr,
                    block_id,
                    &mut defs,
                    &mut uses,
                    ctx.ref_counter,
                    &mut ctx.closure_captures,
                );

                let block = ctx.builder.build_block(
                    block_id,
                    {
                        let mut preds = verum_common::Set::new();
                        preds.insert(ctx.entry_id);
                        preds
                    },
                    {
                        let mut succs = verum_common::Set::new();
                        succs.insert(ctx.exit_id);
                        succs
                    },
                    defs,
                    uses,
                );
                ctx.pending_blocks.push(block);
                block_id
            }
        }
    }

    /// Build CFG for a block expression, returning the starting block ID
    pub(super) fn build_block_cfg(
        &self,
        block: &verum_ast::expr::Block,
        ctx: &mut CfgBuildContext<'_>,
        cfg: &mut verum_cbgr::analysis::ControlFlowGraph,
        continuation: verum_cbgr::analysis::BlockId,
    ) -> verum_cbgr::analysis::BlockId {
        use verum_ast::expr::ExprKind;
        use verum_ast::stmt::StmtKind;

        if block.stmts.is_empty() && block.expr.is_none() {
            // Empty block - just return entry, will connect to continuation
            return ctx.entry_id;
        }

        // Create block for the sequential statements
        let block_id = ctx.builder.new_block_id();
        let mut defs = List::new();
        let mut uses = List::new();
        let mut current_block_id = block_id;
        let mut successors = verum_common::Set::new();

        // Process statements
        for stmt in block.stmts.iter() {
            match &stmt.kind {
                // Handle control flow statements that create new blocks
                StmtKind::Expr { expr, .. } => {
                    match &expr.kind {
                        ExprKind::If {
                            condition,
                            then_branch,
                            else_branch,
                            ..
                        } => {
                            // Build if/else CFG
                            let (if_start, _if_end) = self.build_if_cfg(
                                condition,
                                then_branch,
                                else_branch.as_ref().map(|e| e.as_ref()),
                                ctx,
                                cfg,
                                continuation,
                            );

                            // Current block leads to if start
                            successors.insert(if_start);

                            // Emit current block and start new one for statements after if
                            if !defs.is_empty() || !uses.is_empty() {
                                let stmt_block = ctx.builder.build_block(
                                    current_block_id,
                                    verum_common::Set::new(),
                                    successors.clone(),
                                    std::mem::take(&mut defs),
                                    std::mem::take(&mut uses),
                                );
                                ctx.pending_blocks.push(stmt_block);
                            }

                            // Continue with a new block after the if
                            current_block_id = ctx.builder.new_block_id();
                            successors.clear();
                        }
                        ExprKind::Match {
                            expr: scrutinee,
                            arms,
                        } => {
                            // Build match CFG
                            let match_start =
                                self.build_match_cfg(scrutinee, arms, ctx, cfg, continuation);

                            successors.insert(match_start);

                            if !defs.is_empty() || !uses.is_empty() {
                                let stmt_block = ctx.builder.build_block(
                                    current_block_id,
                                    verum_common::Set::new(),
                                    successors.clone(),
                                    std::mem::take(&mut defs),
                                    std::mem::take(&mut uses),
                                );
                                ctx.pending_blocks.push(stmt_block);
                            }

                            current_block_id = ctx.builder.new_block_id();
                            successors.clear();
                        }
                        ExprKind::Loop {
                            body: loop_body, ..
                        } => {
                            let loop_start = self.build_loop_cfg(loop_body, ctx, cfg, continuation);

                            successors.insert(loop_start);

                            if !defs.is_empty() || !uses.is_empty() {
                                let stmt_block = ctx.builder.build_block(
                                    current_block_id,
                                    verum_common::Set::new(),
                                    successors.clone(),
                                    std::mem::take(&mut defs),
                                    std::mem::take(&mut uses),
                                );
                                ctx.pending_blocks.push(stmt_block);
                            }

                            current_block_id = ctx.builder.new_block_id();
                            successors.clear();
                        }
                        ExprKind::While {
                            condition,
                            body: while_body,
                            ..
                        } => {
                            let while_start =
                                self.build_while_cfg(condition, while_body, ctx, cfg, continuation);

                            successors.insert(while_start);

                            if !defs.is_empty() || !uses.is_empty() {
                                let stmt_block = ctx.builder.build_block(
                                    current_block_id,
                                    verum_common::Set::new(),
                                    successors.clone(),
                                    std::mem::take(&mut defs),
                                    std::mem::take(&mut uses),
                                );
                                ctx.pending_blocks.push(stmt_block);
                            }

                            current_block_id = ctx.builder.new_block_id();
                            successors.clear();
                        }
                        ExprKind::For {
                            pattern: _,
                            iter,
                            body: for_body,
                            ..
                        } => {
                            let for_start =
                                self.build_for_cfg(iter, for_body, ctx, cfg, continuation);

                            successors.insert(for_start);

                            if !defs.is_empty() || !uses.is_empty() {
                                let stmt_block = ctx.builder.build_block(
                                    current_block_id,
                                    verum_common::Set::new(),
                                    successors.clone(),
                                    std::mem::take(&mut defs),
                                    std::mem::take(&mut uses),
                                );
                                ctx.pending_blocks.push(stmt_block);
                            }

                            current_block_id = ctx.builder.new_block_id();
                            successors.clear();
                        }
                        ExprKind::Return(_) => {
                            // Return jumps to exit
                            self.extract_defs_and_uses_from_expr(
                                expr,
                                current_block_id,
                                &mut defs,
                                &mut uses,
                                ctx.ref_counter,
                                &mut ctx.closure_captures,
                            );
                            successors.insert(ctx.exit_id);
                        }
                        _ => {
                            // Regular expression - collect defs and uses
                            self.extract_defs_and_uses_from_expr(
                                expr,
                                current_block_id,
                                &mut defs,
                                &mut uses,
                                ctx.ref_counter,
                                &mut ctx.closure_captures,
                            );
                        }
                    }
                }
                StmtKind::Let {
                    pattern: _,
                    ty: _,
                    value,
                    ..
                } => {
                    // Let bindings may define references
                    if let Some(val) = value {
                        self.extract_defs_and_uses_from_expr(
                            val,
                            current_block_id,
                            &mut defs,
                            &mut uses,
                            ctx.ref_counter,
                            &mut ctx.closure_captures,
                        );
                    }
                }
                StmtKind::LetElse {
                    pattern: _,
                    value,
                    else_block,
                    ..
                } => {
                    self.extract_defs_and_uses_from_expr(
                        value,
                        current_block_id,
                        &mut defs,
                        &mut uses,
                        ctx.ref_counter,
                        &mut ctx.closure_captures,
                    );
                    // Process else block
                    for else_stmt in else_block.stmts.iter() {
                        if let StmtKind::Expr { expr, .. } = &else_stmt.kind {
                            self.extract_defs_and_uses_from_expr(
                                expr,
                                current_block_id,
                                &mut defs,
                                &mut uses,
                                ctx.ref_counter,
                                &mut ctx.closure_captures,
                            );
                        }
                    }
                }
                StmtKind::Defer(expr) => {
                    self.extract_defs_and_uses_from_expr(
                        expr,
                        current_block_id,
                        &mut defs,
                        &mut uses,
                        ctx.ref_counter,
                        &mut ctx.closure_captures,
                    );
                }
                StmtKind::Errdefer(expr) => {
                    self.extract_defs_and_uses_from_expr(
                        expr,
                        current_block_id,
                        &mut defs,
                        &mut uses,
                        ctx.ref_counter,
                        &mut ctx.closure_captures,
                    );
                }
                StmtKind::Provide { value, .. } => {
                    self.extract_defs_and_uses_from_expr(
                        value,
                        current_block_id,
                        &mut defs,
                        &mut uses,
                        ctx.ref_counter,
                        &mut ctx.closure_captures,
                    );
                }
                _ => {}
            }
        }

        // Process trailing expression
        if let Some(ref trailing_expr) = block.expr {
            self.extract_defs_and_uses_from_expr(
                trailing_expr,
                current_block_id,
                &mut defs,
                &mut uses,
                ctx.ref_counter,
                &mut ctx.closure_captures,
            );
        }

        // If we haven't added any successors, connect to continuation
        if successors.is_empty() {
            successors.insert(continuation);
        }

        // Emit the final block
        let final_block = ctx.builder.build_block(
            current_block_id,
            verum_common::Set::new(),
            successors,
            defs,
            uses,
        );
        ctx.pending_blocks.push(final_block);

        block_id
    }

    /// Build CFG for if/else expression
    pub(super) fn build_if_cfg(
        &self,
        condition: &verum_ast::expr::IfCondition,
        then_branch: &verum_ast::expr::Block,
        else_branch: Option<&verum_ast::expr::Expr>,
        ctx: &mut CfgBuildContext<'_>,
        cfg: &mut verum_cbgr::analysis::ControlFlowGraph,
        continuation: verum_cbgr::analysis::BlockId,
    ) -> (verum_cbgr::analysis::BlockId, verum_cbgr::analysis::BlockId) {
        use verum_ast::expr::ExprKind;

        // Condition block
        let cond_block_id = ctx.builder.new_block_id();
        let mut cond_defs = List::new();
        let mut cond_uses = List::new();

        // Extract uses from condition
        self.extract_defs_and_uses_from_condition(
            condition,
            cond_block_id,
            &mut cond_defs,
            &mut cond_uses,
            ctx.ref_counter,
            &mut ctx.closure_captures,
        );

        // Then block
        let then_block_id = self.build_block_cfg(then_branch, ctx, cfg, continuation);

        // Else block (or continuation if no else)
        let else_block_id = if let Some(else_expr) = else_branch {
            match &else_expr.kind {
                ExprKind::Block(else_block) => {
                    self.build_block_cfg(else_block, ctx, cfg, continuation)
                }
                ExprKind::If {
                    condition: else_cond,
                    then_branch: else_then,
                    else_branch: else_else,
                    ..
                } => {
                    let (else_if_start, _) = self.build_if_cfg(
                        else_cond,
                        else_then,
                        else_else.as_ref().map(|e| e.as_ref()),
                        ctx,
                        cfg,
                        continuation,
                    );
                    else_if_start
                }
                _ => {
                    // Create block for else expression
                    let else_id = ctx.builder.new_block_id();
                    let mut else_defs = List::new();
                    let mut else_uses = List::new();
                    self.extract_defs_and_uses_from_expr(
                        else_expr,
                        else_id,
                        &mut else_defs,
                        &mut else_uses,
                        ctx.ref_counter,
                        &mut ctx.closure_captures,
                    );
                    let else_block = ctx.builder.build_block(
                        else_id,
                        {
                            let mut preds = verum_common::Set::new();
                            preds.insert(cond_block_id);
                            preds
                        },
                        {
                            let mut succs = verum_common::Set::new();
                            succs.insert(continuation);
                            succs
                        },
                        else_defs,
                        else_uses,
                    );
                    ctx.pending_blocks.push(else_block);
                    else_id
                }
            }
        } else {
            continuation
        };

        // Build condition block with successors to both branches
        let cond_block = ctx.builder.build_block(
            cond_block_id,
            verum_common::Set::new(),
            {
                let mut succs = verum_common::Set::new();
                succs.insert(then_block_id);
                succs.insert(else_block_id);
                succs
            },
            cond_defs,
            cond_uses,
        );
        ctx.pending_blocks.push(cond_block);

        (cond_block_id, continuation)
    }

    /// Build CFG for match expression
    pub(super) fn build_match_cfg(
        &self,
        scrutinee: &verum_ast::expr::Expr,
        arms: &verum_common::List<verum_ast::pattern::MatchArm>,
        ctx: &mut CfgBuildContext<'_>,
        _cfg: &mut verum_cbgr::analysis::ControlFlowGraph,
        continuation: verum_cbgr::analysis::BlockId,
    ) -> verum_cbgr::analysis::BlockId {
        // Scrutinee evaluation block
        let scrutinee_block_id = ctx.builder.new_block_id();
        let mut scrutinee_defs = List::new();
        let mut scrutinee_uses = List::new();

        self.extract_defs_and_uses_from_expr(
            scrutinee,
            scrutinee_block_id,
            &mut scrutinee_defs,
            &mut scrutinee_uses,
            ctx.ref_counter,
            &mut ctx.closure_captures,
        );

        // Build blocks for each arm
        let mut arm_block_ids = List::new();
        for arm in arms.iter() {
            let arm_block_id = ctx.builder.new_block_id();
            let mut arm_defs = List::new();
            let mut arm_uses = List::new();

            // Extract uses from guard if present
            if let Some(ref guard) = arm.guard {
                self.extract_defs_and_uses_from_expr(
                    guard,
                    arm_block_id,
                    &mut arm_defs,
                    &mut arm_uses,
                    ctx.ref_counter,
                    &mut ctx.closure_captures,
                );
            }

            // Extract uses from arm body
            self.extract_defs_and_uses_from_expr(
                &arm.body,
                arm_block_id,
                &mut arm_defs,
                &mut arm_uses,
                ctx.ref_counter,
                &mut ctx.closure_captures,
            );

            let arm_block = ctx.builder.build_block(
                arm_block_id,
                {
                    let mut preds = verum_common::Set::new();
                    preds.insert(scrutinee_block_id);
                    preds
                },
                {
                    let mut succs = verum_common::Set::new();
                    succs.insert(continuation);
                    succs
                },
                arm_defs,
                arm_uses,
            );
            ctx.pending_blocks.push(arm_block);
            arm_block_ids.push(arm_block_id);
        }

        // Build scrutinee block with successors to all arms
        let scrutinee_successors: verum_common::Set<_> = arm_block_ids.into_iter().collect();
        let scrutinee_block = ctx.builder.build_block(
            scrutinee_block_id,
            verum_common::Set::new(),
            scrutinee_successors,
            scrutinee_defs,
            scrutinee_uses,
        );
        ctx.pending_blocks.push(scrutinee_block);

        scrutinee_block_id
    }

    /// Build CFG for loop expression
    pub(super) fn build_loop_cfg(
        &self,
        body: &verum_ast::expr::Block,
        ctx: &mut CfgBuildContext<'_>,
        cfg: &mut verum_cbgr::analysis::ControlFlowGraph,
        continuation: verum_cbgr::analysis::BlockId,
    ) -> verum_cbgr::analysis::BlockId {
        // Loop header block
        let header_block_id = ctx.builder.new_block_id();

        // Loop body - continues back to header
        let body_block_id = self.build_block_cfg(body, ctx, cfg, header_block_id);

        // Build header block with back-edge from body
        let header_block = ctx.builder.build_block(
            header_block_id,
            verum_common::Set::new(),
            {
                let mut succs = verum_common::Set::new();
                succs.insert(body_block_id);
                succs.insert(continuation); // Break exits to continuation
                succs
            },
            List::new(),
            List::new(),
        );
        ctx.pending_blocks.push(header_block);

        header_block_id
    }

    /// Build CFG for while loop
    pub(super) fn build_while_cfg(
        &self,
        condition: &verum_ast::expr::Expr,
        body: &verum_ast::expr::Block,
        ctx: &mut CfgBuildContext<'_>,
        cfg: &mut verum_cbgr::analysis::ControlFlowGraph,
        continuation: verum_cbgr::analysis::BlockId,
    ) -> verum_cbgr::analysis::BlockId {
        // Condition block (loop header)
        let cond_block_id = ctx.builder.new_block_id();
        let mut cond_defs = List::new();
        let mut cond_uses = List::new();

        self.extract_defs_and_uses_from_expr(
            condition,
            cond_block_id,
            &mut cond_defs,
            &mut cond_uses,
            ctx.ref_counter,
            &mut ctx.closure_captures,
        );

        // Body block - loops back to condition
        let body_block_id = self.build_block_cfg(body, ctx, cfg, cond_block_id);

        // Build condition block
        let cond_block = ctx.builder.build_block(
            cond_block_id,
            verum_common::Set::new(),
            {
                let mut succs = verum_common::Set::new();
                succs.insert(body_block_id); // True branch
                succs.insert(continuation); // False branch (exit)
                succs
            },
            cond_defs,
            cond_uses,
        );
        ctx.pending_blocks.push(cond_block);

        cond_block_id
    }

    /// Build CFG for for loop
    pub(super) fn build_for_cfg(
        &self,
        iter: &verum_ast::expr::Expr,
        body: &verum_ast::expr::Block,
        ctx: &mut CfgBuildContext<'_>,
        cfg: &mut verum_cbgr::analysis::ControlFlowGraph,
        continuation: verum_cbgr::analysis::BlockId,
    ) -> verum_cbgr::analysis::BlockId {
        // Iterator initialization block
        let init_block_id = ctx.builder.new_block_id();
        let mut init_defs = List::new();
        let mut init_uses = List::new();

        self.extract_defs_and_uses_from_expr(
            iter,
            init_block_id,
            &mut init_defs,
            &mut init_uses,
            ctx.ref_counter,
            &mut ctx.closure_captures,
        );

        // Loop header block (iterator next check)
        let header_block_id = ctx.builder.new_block_id();

        // Body block - loops back to header
        let body_block_id = self.build_block_cfg(body, ctx, cfg, header_block_id);

        // Build init block
        let init_block = ctx.builder.build_block(
            init_block_id,
            verum_common::Set::new(),
            {
                let mut succs = verum_common::Set::new();
                succs.insert(header_block_id);
                succs
            },
            init_defs,
            init_uses,
        );
        ctx.pending_blocks.push(init_block);

        // Build header block
        let header_block = ctx.builder.build_block(
            header_block_id,
            verum_common::Set::new(),
            {
                let mut succs = verum_common::Set::new();
                succs.insert(body_block_id); // Has more items
                succs.insert(continuation); // Iterator exhausted
                succs
            },
            List::new(),
            List::new(),
        );
        ctx.pending_blocks.push(header_block);

        init_block_id
    }

    /// Extract definitions and uses from an if condition
    pub(super) fn extract_defs_and_uses_from_condition(
        &self,
        condition: &verum_ast::expr::IfCondition,
        block_id: verum_cbgr::analysis::BlockId,
        defs: &mut List<verum_cbgr::analysis::DefSite>,
        uses: &mut List<verum_cbgr::analysis::UseeSite>,
        ref_counter: &mut u64,
        closure_captures: &mut List<(verum_cbgr::analysis::RefId, bool)>,
    ) {
        use verum_ast::expr::ConditionKind;

        for cond in condition.conditions.iter() {
            match cond {
                ConditionKind::Expr(expr) => {
                    self.extract_defs_and_uses_from_expr(
                        expr,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
                ConditionKind::Let { value, .. } => {
                    // Let in condition may create a reference binding
                    self.extract_defs_and_uses_from_expr(
                        value,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }
        }
    }

    /// Extract reference definitions and uses from an expression
    pub(super) fn extract_defs_and_uses_from_expr(
        &self,
        expr: &verum_ast::expr::Expr,
        block_id: verum_cbgr::analysis::BlockId,
        defs: &mut List<verum_cbgr::analysis::DefSite>,
        uses: &mut List<verum_cbgr::analysis::UseeSite>,
        ref_counter: &mut u64,
        closure_captures: &mut List<(verum_cbgr::analysis::RefId, bool)>,
    ) {
        use verum_ast::expr::{ExprKind, UnOp};
        use verum_cbgr::analysis::{DefSite, RefId, UseeSite};

        match &expr.kind {
            // Reference creation - this is a definition
            ExprKind::Unary { op, expr: inner } => {
                match op {
                    UnOp::Ref | UnOp::RefChecked | UnOp::RefUnsafe => {
                        // Immutable reference definition
                        let ref_id = RefId(*ref_counter);
                        *ref_counter += 1;
                        defs.push(DefSite {
                            block: block_id,
                            reference: ref_id,
                            is_stack_allocated: true,
                            span: None,
                        });
                    }
                    UnOp::RefMut | UnOp::RefCheckedMut | UnOp::RefUnsafeMut => {
                        // Mutable reference definition
                        let ref_id = RefId(*ref_counter);
                        *ref_counter += 1;
                        defs.push(DefSite {
                            block: block_id,
                            reference: ref_id,
                            is_stack_allocated: true,
                            span: None,
                        });
                    }
                    UnOp::Deref => {
                        // Dereference is a use
                        uses.push(UseeSite {
                            block: block_id,
                            reference: RefId(*ref_counter),
                            is_mutable: false,
                            span: None,
                        });
                        *ref_counter += 1;
                    }
                    _ => {}
                }
                self.extract_defs_and_uses_from_expr(
                    inner,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Field access on references is a use
            ExprKind::Field { expr: base, .. } => {
                uses.push(UseeSite {
                    block: block_id,
                    reference: RefId(*ref_counter),
                    is_mutable: false,
                    span: None,
                });
                *ref_counter += 1;
                self.extract_defs_and_uses_from_expr(
                    base,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Index access is a use
            ExprKind::Index { expr: base, index } => {
                uses.push(UseeSite {
                    block: block_id,
                    reference: RefId(*ref_counter),
                    is_mutable: false,
                    span: None,
                });
                *ref_counter += 1;
                self.extract_defs_and_uses_from_expr(
                    base,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                self.extract_defs_and_uses_from_expr(
                    index,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Method call - receiver is a use
            ExprKind::MethodCall { receiver, args, .. } => {
                uses.push(UseeSite {
                    block: block_id,
                    reference: RefId(*ref_counter),
                    is_mutable: false, // Could be mutable if method takes &mut self
                    span: None,
                });
                *ref_counter += 1;
                self.extract_defs_and_uses_from_expr(
                    receiver,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                for arg in args.iter() {
                    self.extract_defs_and_uses_from_expr(
                        arg,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            // Function call - args may be reference uses
            ExprKind::Call { func, args, .. } => {
                self.extract_defs_and_uses_from_expr(
                    func,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                for arg in args.iter() {
                    self.extract_defs_and_uses_from_expr(
                        arg,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            // Closure - track captures for escape analysis
            ExprKind::Closure {
                params: _,
                return_type: _,
                body,
                ..
            } => {
                // Mark any references captured by the closure
                let capture_start = *ref_counter;
                self.extract_defs_and_uses_from_expr(
                    body,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );

                // References used in closure body are potential captures
                // We mark them as escaping via closure capture
                for i in capture_start..*ref_counter {
                    closure_captures.push((RefId(i), false));
                }
            }

            // Return - reference may escape
            ExprKind::Return(value) => {
                if let Some(val) = value {
                    // Mark this as a potential escape point
                    uses.push(UseeSite {
                        block: block_id,
                        reference: RefId(*ref_counter),
                        is_mutable: false,
                        span: None,
                    });
                    *ref_counter += 1;
                    self.extract_defs_and_uses_from_expr(
                        val.as_ref(),
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            // Binary operations
            ExprKind::Binary { left, right, .. } => {
                self.extract_defs_and_uses_from_expr(
                    left,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                self.extract_defs_and_uses_from_expr(
                    right,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Block expression
            ExprKind::Block(inner_block) => {
                for stmt in inner_block.stmts.iter() {
                    self.extract_defs_and_uses_from_stmt(
                        stmt,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
                if let Some(ref result_expr) = inner_block.expr {
                    self.extract_defs_and_uses_from_expr(
                        result_expr,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            // Tuple and array literals
            ExprKind::Tuple(elements) => {
                for elem in elements.iter() {
                    self.extract_defs_and_uses_from_expr(
                        elem,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            ExprKind::Array(array_expr) => {
                use verum_ast::expr::ArrayExpr;
                match array_expr {
                    ArrayExpr::List(elements) => {
                        for elem in elements.iter() {
                            self.extract_defs_and_uses_from_expr(
                                elem,
                                block_id,
                                defs,
                                uses,
                                ref_counter,
                                closure_captures,
                            );
                        }
                    }
                    ArrayExpr::Repeat { value, count } => {
                        self.extract_defs_and_uses_from_expr(
                            value,
                            block_id,
                            defs,
                            uses,
                            ref_counter,
                            closure_captures,
                        );
                        self.extract_defs_and_uses_from_expr(
                            count,
                            block_id,
                            defs,
                            uses,
                            ref_counter,
                            closure_captures,
                        );
                    }
                }
            }

            // Record literals
            ExprKind::Record { fields, base, .. } => {
                for field in fields.iter() {
                    if let Some(ref val) = field.value {
                        self.extract_defs_and_uses_from_expr(
                            val,
                            block_id,
                            defs,
                            uses,
                            ref_counter,
                            closure_captures,
                        );
                    }
                }
                if let Some(base_expr) = base {
                    self.extract_defs_and_uses_from_expr(
                        base_expr.as_ref(),
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            // Await expressions
            ExprKind::Await(operand) => {
                self.extract_defs_and_uses_from_expr(
                    operand,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Async blocks
            ExprKind::Async(async_block) => {
                for stmt in async_block.stmts.iter() {
                    self.extract_defs_and_uses_from_stmt(
                        stmt,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
                if let Some(ref result_expr) = async_block.expr {
                    self.extract_defs_and_uses_from_expr(
                        result_expr,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            // Try expressions
            ExprKind::Try(inner) => {
                self.extract_defs_and_uses_from_expr(
                    inner,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Cast expressions
            ExprKind::Cast { expr: inner, .. } => {
                self.extract_defs_and_uses_from_expr(
                    inner,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Pipeline expressions
            ExprKind::Pipeline { left, right } => {
                self.extract_defs_and_uses_from_expr(
                    left,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                self.extract_defs_and_uses_from_expr(
                    right,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Null coalescing
            ExprKind::NullCoalesce { left, right } => {
                self.extract_defs_and_uses_from_expr(
                    left,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                self.extract_defs_and_uses_from_expr(
                    right,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Parenthesized expressions
            ExprKind::Paren(inner) => {
                self.extract_defs_and_uses_from_expr(
                    inner,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Optional chaining
            ExprKind::OptionalChain { expr: base, .. } => {
                uses.push(UseeSite {
                    block: block_id,
                    reference: RefId(*ref_counter),
                    is_mutable: false,
                    span: None,
                });
                *ref_counter += 1;
                self.extract_defs_and_uses_from_expr(
                    base,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Tuple index
            ExprKind::TupleIndex { expr: base, .. } => {
                self.extract_defs_and_uses_from_expr(
                    base,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Yield expressions
            ExprKind::Yield(inner) => {
                self.extract_defs_and_uses_from_expr(
                    inner,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Break with value
            ExprKind::Break { value, .. } => {
                if let Some(val) = value {
                    self.extract_defs_and_uses_from_expr(
                        val.as_ref(),
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            // Literals, paths, continue - no references to track
            ExprKind::Literal(_) | ExprKind::Path(_) | ExprKind::Continue { .. } => {}

            // Control flow expressions handled at block level, but process their contents
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.extract_defs_and_uses_from_condition(
                    condition,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                for stmt in then_branch.stmts.iter() {
                    self.extract_defs_and_uses_from_stmt(
                        stmt,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
                if let Some(ref then_expr) = then_branch.expr {
                    self.extract_defs_and_uses_from_expr(
                        then_expr,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
                if let Some(else_expr) = else_branch {
                    self.extract_defs_and_uses_from_expr(
                        else_expr.as_ref(),
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            ExprKind::Match {
                expr: scrutinee,
                arms,
            } => {
                self.extract_defs_and_uses_from_expr(
                    scrutinee,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                for arm in arms.iter() {
                    if let Some(ref guard) = arm.guard {
                        self.extract_defs_and_uses_from_expr(
                            guard,
                            block_id,
                            defs,
                            uses,
                            ref_counter,
                            closure_captures,
                        );
                    }
                    self.extract_defs_and_uses_from_expr(
                        &arm.body,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            ExprKind::Loop {
                body: loop_body, ..
            } => {
                for stmt in loop_body.stmts.iter() {
                    self.extract_defs_and_uses_from_stmt(
                        stmt,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
                if let Some(ref loop_expr) = loop_body.expr {
                    self.extract_defs_and_uses_from_expr(
                        loop_expr,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            ExprKind::While {
                condition,
                body: while_body,
                ..
            } => {
                self.extract_defs_and_uses_from_expr(
                    condition,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                for stmt in while_body.stmts.iter() {
                    self.extract_defs_and_uses_from_stmt(
                        stmt,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
                if let Some(ref while_expr) = while_body.expr {
                    self.extract_defs_and_uses_from_expr(
                        while_expr,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            ExprKind::For {
                iter,
                body: for_body,
                ..
            } => {
                self.extract_defs_and_uses_from_expr(
                    iter,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                for stmt in for_body.stmts.iter() {
                    self.extract_defs_and_uses_from_stmt(
                        stmt,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
                if let Some(ref for_expr) = for_body.expr {
                    self.extract_defs_and_uses_from_expr(
                        for_expr,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            // Handle remaining expression types conservatively
            _ => {}
        }
    }

    /// Extract definitions and uses from a statement
    pub(super) fn extract_defs_and_uses_from_stmt(
        &self,
        stmt: &verum_ast::stmt::Stmt,
        block_id: verum_cbgr::analysis::BlockId,
        defs: &mut List<verum_cbgr::analysis::DefSite>,
        uses: &mut List<verum_cbgr::analysis::UseeSite>,
        ref_counter: &mut u64,
        closure_captures: &mut List<(verum_cbgr::analysis::RefId, bool)>,
    ) {
        use verum_ast::stmt::StmtKind;

        match &stmt.kind {
            StmtKind::Expr { expr, .. } => {
                self.extract_defs_and_uses_from_expr(
                    expr,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }
            StmtKind::Let { value, .. } => {
                if let Some(val) = value {
                    self.extract_defs_and_uses_from_expr(
                        val,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }
            StmtKind::LetElse {
                value, else_block, ..
            } => {
                self.extract_defs_and_uses_from_expr(
                    value,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                for else_stmt in else_block.stmts.iter() {
                    self.extract_defs_and_uses_from_stmt(
                        else_stmt,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }
            StmtKind::Defer(expr) => {
                self.extract_defs_and_uses_from_expr(
                    expr,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }
            StmtKind::Errdefer(expr) => {
                self.extract_defs_and_uses_from_expr(
                    expr,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }
            StmtKind::Provide { value, .. } => {
                self.extract_defs_and_uses_from_expr(
                    value,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }
            _ => {}
        }
    }

    /// Collect all predecessors of the exit block
    pub(super) fn collect_exit_predecessors(
        &self,
        cfg: &verum_cbgr::analysis::ControlFlowGraph,
        exit_id: verum_cbgr::analysis::BlockId,
    ) -> verum_common::Set<verum_cbgr::analysis::BlockId> {
        let mut preds = verum_common::Set::new();

        // Find all blocks that have exit_id as a successor
        for (block_id, block) in &cfg.blocks {
            if block.successors.contains(&exit_id) {
                preds.insert(*block_id);
            }
        }

        preds
    }

    /// Check if a function parameter contains reference types
    pub(super) fn param_is_reference(param: &verum_ast::decl::FunctionParam) -> bool {
        use verum_ast::decl::FunctionParamKind;
        use verum_ast::ty::TypeKind;

        match &param.kind {
            FunctionParamKind::Regular { ty, .. } => {
                matches!(ty.kind, TypeKind::Reference { .. })
            }
            // Self reference parameters
            FunctionParamKind::SelfRef | FunctionParamKind::SelfRefMut |
            FunctionParamKind::SelfRefChecked | FunctionParamKind::SelfRefCheckedMut |
            FunctionParamKind::SelfRefUnsafe | FunctionParamKind::SelfRefUnsafeMut => true,
            FunctionParamKind::SelfOwn | FunctionParamKind::SelfOwnMut => true,
            // Self value parameters are not references
            FunctionParamKind::SelfValue | FunctionParamKind::SelfValueMut => false,
        }
    }

    /// Hash function name to create a stable function ID
    pub(super) fn hash_function_name(name: &str) -> u64 {
        let mut hasher = crate::hash::ContentHash::new();
        hasher.update_str(name);
        hasher.finalize().to_u64()
    }
}
