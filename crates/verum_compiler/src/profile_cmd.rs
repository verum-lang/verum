//! CBGR Profiling Command
//!
//! P0 Feature for v1.0: Profile CBGR overhead and suggest optimizations
//!
//! # Example Output
//!
//! ```text
//! $ verum profile --memory app.vr
//!
//! Performance Report:
//!   hot_loop(): 15% time in CBGR checks
//!     → Convert to %T for zero-cost: `fn hot_loop(data: %List<Int>)`
//!
//!   safe_parse(): 0.1% CBGR overhead
//!     → Keep &T, overhead negligible
//! ```

use anyhow::{Context, Result};
use colored::Colorize;
use std::path::Path;
use tracing::{debug, info};
use verum_ast::decl::ItemKind;
use verum_ast::{Expr, ExprKind, FunctionDecl, FunctionParamKind, Module, Type, TypeKind};
use verum_common::{List, Map, Text, ToText};

use crate::pipeline::CompilationPipeline;
use crate::session::Session;

/// Profiling mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfilingMode {
    /// Static analysis (AST-based estimation)
    Static,
    /// Runtime sampling (requires execution)
    Runtime,
}

/// CBGR profiling command handler
pub struct ProfileCommand<'s> {
    session: &'s mut Session,
    mode: ProfilingMode,
}

impl<'s> ProfileCommand<'s> {
    /// Create new profiling command with static analysis mode
    pub fn new(session: &'s mut Session) -> Self {
        Self {
            session,
            mode: ProfilingMode::Static,
        }
    }

    /// Create profiling command with runtime mode
    pub fn new_runtime(session: &'s mut Session) -> Self {
        Self {
            session,
            mode: ProfilingMode::Runtime,
        }
    }

    /// Set profiling mode
    pub fn with_mode(mut self, mode: ProfilingMode) -> Self {
        // Phase-not-realised tracing: `ProfileCommand::with_mode`
        // selects between `Static` (AST-based estimation) and
        // `Runtime` (requires execution sampling). The CLI
        // `verum profile --memory <file>` entry point at
        // commands/file.rs:1474 calls `ProfileCommand::new` which
        // defaults to Static — no caller threads `--memory` into
        // a Runtime selection. This builder exists for embedders
        // but the CLI surface doesn't expose mode selection yet.
        // Surface a debug trace when the user picks Runtime so
        // the gap is visible (Static stays quiet — that's the
        // default already).
        if matches!(mode, ProfilingMode::Runtime) {
            tracing::debug!(
                "ProfileCommand::with_mode(Runtime) — Runtime mode requires \
                 execution sampling, but the CLI's `verum profile` command \
                 doesn't yet expose a mode selector. The default Static path \
                 (AST-based estimation) runs regardless of the --memory flag. \
                 Forward-looking: hook this builder into a future \
                 `verum profile --runtime` flag."
            );
        }
        self.mode = mode;
        self
    }

    /// Run CBGR profiling
    pub fn run(&mut self, output: Option<&Path>, suggest: bool) -> Result<()> {
        match self.mode {
            ProfilingMode::Static => self.run_static(output, suggest),
            ProfilingMode::Runtime => self.run_runtime(output, suggest),
        }
    }

    /// Run static analysis profiling
    fn run_static(&mut self, output: Option<&Path>, suggest: bool) -> Result<()> {
        // Load and parse source
        let input = self.session.options().input.clone();
        let file_id = self
            .session
            .load_file(&input)
            .with_context(|| format!("Failed to load: {}", input.display()))?;

        // Parse and type check
        let mut pipeline = CompilationPipeline::new(self.session);
        pipeline.run_check_only()?;

        let module = self
            .session
            .get_module(file_id)
            .map(|m| (*m).clone())
            .ok_or_else(|| anyhow::anyhow!("Module not found"))?;

        // Profile CBGR usage
        let report = self.profile_module(&module)?;

        // Display report
        self.display_report(&report);

        // Display suggestions if enabled
        if suggest || self.session.options().hot_path_threshold > 0.0 {
            self.display_suggestions(&report);
        }

        // Write to file if requested
        if let Some(path) = output {
            self.write_report(&report, path)?;
        }

        Ok(())
    }

    /// Run runtime sampling profiling
    ///
    /// This mode executes the program with profiling enabled and collects
    /// actual runtime statistics using sampling.
    fn run_runtime(&mut self, _output: Option<&Path>, _suggest: bool) -> Result<()> {
        println!("{}", "=== Runtime CBGR Profiling ===".bold());
        println!("Mode: Sampling-based (1% default)");
        println!();

        // Note: This requires runtime integration which would be implemented
        // in the actual execution tier. For now, we provide a placeholder.
        println!(
            "{}",
            "Note: Runtime profiling requires execution tier integration.".yellow()
        );
        println!("Use static mode for compile-time analysis:");
        println!("  $ verum profile --memory <file>");
        println!();
        println!("For runtime profiling, use:");
        println!("  $ verum run --profile-cbgr <file>");

        Err(anyhow::anyhow!(
            "Runtime profiling not yet implemented in this command. Use 'verum run --profile-cbgr' instead."
        ))
    }

    /// Profile CBGR usage in module
    fn profile_module(&mut self, module: &Module) -> Result<ProfileReport> {
        let mut report = ProfileReport::new();
        let threshold = self.session.options().hot_path_threshold;

        for item in &module.items {
            if let ItemKind::Function(func) = &item.kind {
                debug!("Profiling function: {}", func.name);
                let stats = self.profile_function(func)?;

                // Calculate overhead percentage
                let overhead_pct = self.calculate_overhead(&stats);

                report.add_function(
                    func.name.as_str().to_text(),
                    FunctionProfile {
                        stats,
                        overhead_pct,
                        is_hot: overhead_pct >= threshold,
                    },
                );
            }
        }

        Ok(report)
    }

    /// Profile a single function using real AST analysis
    ///
    /// This walks the entire function AST to count:
    /// - CBGR managed references (&T, &mut T)
    /// - Checked references (&checked T)
    /// - Unsafe references (&unsafe T)
    /// - Ownership references (%T)
    fn profile_function(&self, func: &FunctionDecl) -> Result<CbgrStats> {
        let mut stats = RefStats::default();

        // Analyze parameter types
        for param in &func.params {
            if let FunctionParamKind::Regular { pattern: _, ty, .. } = &param.kind {
                self.analyze_type(ty, &mut stats);
            }
        }

        // Analyze return type
        if let Some(ref ret_ty) = func.return_type {
            self.analyze_type(ret_ty, &mut stats);
        }

        // Analyze function body if available
        if let Some(ref body) = func.body {
            self.analyze_body(body, &mut stats);
        }

        // Calculate estimated checks and timing
        // Each CBGR ref access requires ~1 check, loops multiply this
        let estimated_checks = stats.cbgr_refs * stats.estimated_loop_factor.max(1);

        // CBGR check overhead: ~15ns per check
        const CBGR_CHECK_NS: u64 = 15;
        let cbgr_time_ns = (estimated_checks as u64) * CBGR_CHECK_NS;

        // Estimate total time: base + CBGR overhead
        // Base: 100ns per expression (rough estimate)
        let base_time_ns = (stats.total_expressions as u64) * 100;
        let total_time_ns = base_time_ns + cbgr_time_ns;

        Ok(CbgrStats {
            num_cbgr_refs: stats.cbgr_refs,
            num_ownership_refs: stats.ownership_refs,
            num_checks: estimated_checks,
            total_time_ns,
            cbgr_time_ns,
        })
    }

    /// Analyze a type for reference categories
    fn analyze_type(&self, ty: &Type, stats: &mut RefStats) {
        match &ty.kind {
            // CBGR managed reference (&T, &mut T)
            TypeKind::Reference { inner, .. } => {
                stats.cbgr_refs += 1;
                self.analyze_type(inner, stats);
            }
            // Checked reference (&checked T) - no CBGR overhead
            TypeKind::CheckedReference { inner, .. } => {
                stats.checked_refs += 1;
                self.analyze_type(inner, stats);
            }
            // Unsafe reference (&unsafe T) - no CBGR overhead
            TypeKind::UnsafeReference { inner, .. } => {
                stats.unsafe_refs += 1;
                self.analyze_type(inner, stats);
            }
            // Ownership (%T) - no CBGR overhead
            TypeKind::Ownership { inner, .. } => {
                stats.ownership_refs += 1;
                self.analyze_type(inner, stats);
            }
            // Generic types - analyze type arguments
            TypeKind::Generic { args, .. } => {
                for arg in args {
                    if let verum_ast::ty::GenericArg::Type(t) = arg {
                        self.analyze_type(t, stats);
                    }
                }
            }
            // Tuple types
            TypeKind::Tuple(types) => {
                for t in types {
                    self.analyze_type(t, stats);
                }
            }
            // Function types
            TypeKind::Function {
                params,
                return_type,
                ..
            } => {
                for p in params {
                    self.analyze_type(p, stats);
                }
                self.analyze_type(return_type, stats);
            }
            // Array types
            TypeKind::Array { element, .. } => {
                self.analyze_type(element, stats);
            }
            // Slice types
            TypeKind::Slice(inner) => {
                self.analyze_type(inner, stats);
            }
            _ => {}
        }
    }

    /// Analyze function body for expressions and loop factors
    fn analyze_body(&self, body: &verum_ast::FunctionBody, stats: &mut RefStats) {
        match body {
            verum_ast::FunctionBody::Block(block) => {
                self.analyze_block(block, stats, 1);
            }
            verum_ast::FunctionBody::Expr(expr) => {
                self.analyze_expr(expr, stats, 1);
            }
        }
    }

    /// Analyze a block for expressions
    fn analyze_block(&self, block: &verum_ast::Block, stats: &mut RefStats, loop_depth: usize) {
        for stmt in &block.stmts {
            self.analyze_stmt(stmt, stats, loop_depth);
        }
        if let Some(ref expr) = block.expr {
            self.analyze_expr(expr, stats, loop_depth);
        }
    }

    /// Analyze a statement
    fn analyze_stmt(&self, stmt: &verum_ast::Stmt, stats: &mut RefStats, loop_depth: usize) {
        use verum_ast::StmtKind;
        match &stmt.kind {
            StmtKind::Let { value, ty, .. } => {
                if let Some(t) = ty {
                    self.analyze_type(t, stats);
                }
                if let Some(v) = value {
                    self.analyze_expr(v, stats, loop_depth);
                }
            }
            StmtKind::Expr { expr, .. } => {
                self.analyze_expr(expr, stats, loop_depth);
            }
            _ => {}
        }
    }

    /// Analyze an expression for CBGR operations
    fn analyze_expr(&self, expr: &Expr, stats: &mut RefStats, loop_depth: usize) {
        stats.total_expressions += 1;

        // Update loop factor if we're in a loop
        if loop_depth > 1 {
            // Estimate 10 iterations per loop level
            let factor = 10_usize.pow((loop_depth - 1) as u32);
            stats.estimated_loop_factor = stats.estimated_loop_factor.max(factor);
        }

        match &expr.kind {
            // Unary operations - check for dereference
            ExprKind::Unary { op, expr: inner } => {
                // Deref (*) triggers a CBGR check
                if matches!(op, verum_ast::UnOp::Deref) {
                    stats.cbgr_refs += loop_depth;
                }
                self.analyze_expr(inner, stats, loop_depth);
            }
            // Field access on reference
            ExprKind::Field { expr: object, .. } => {
                self.analyze_expr(object, stats, loop_depth);
            }
            // Index access - triggers bounds check + CBGR check
            ExprKind::Index {
                expr: object,
                index,
            } => {
                stats.cbgr_refs += loop_depth; // CBGR check on container
                self.analyze_expr(object, stats, loop_depth);
                self.analyze_expr(index, stats, loop_depth);
            }
            // Loop constructs increase loop depth
            ExprKind::For { iter, body, .. } => {
                self.analyze_expr(iter, stats, loop_depth);
                self.analyze_block(body, stats, loop_depth + 1);
            }
            ExprKind::While {
                condition, body, ..
            } => {
                self.analyze_expr(condition, stats, loop_depth + 1);
                self.analyze_block(body, stats, loop_depth + 1);
            }
            ExprKind::Loop { body, .. } => {
                self.analyze_block(body, stats, loop_depth + 1);
            }
            // Recursively analyze subexpressions
            ExprKind::Block(block) => {
                self.analyze_block(block, stats, loop_depth);
            }
            ExprKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                // condition is IfCondition, we just analyze the branches
                self.analyze_block(then_branch, stats, loop_depth);
                if let Some(e) = else_branch {
                    self.analyze_expr(e, stats, loop_depth);
                }
            }
            ExprKind::Match {
                expr: scrutinee,
                arms,
            } => {
                self.analyze_expr(scrutinee, stats, loop_depth);
                for arm in arms {
                    self.analyze_expr(&arm.body, stats, loop_depth);
                }
            }
            ExprKind::Call { func, args, .. } => {
                self.analyze_expr(func, stats, loop_depth);
                for arg in args {
                    self.analyze_expr(arg, stats, loop_depth);
                }
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                self.analyze_expr(receiver, stats, loop_depth);
                for arg in args {
                    self.analyze_expr(arg, stats, loop_depth);
                }
            }
            ExprKind::Binary { left, right, .. } => {
                self.analyze_expr(left, stats, loop_depth);
                self.analyze_expr(right, stats, loop_depth);
            }
            ExprKind::Tuple(exprs) => {
                for e in exprs {
                    self.analyze_expr(e, stats, loop_depth);
                }
            }
            ExprKind::Array(array_expr) => match array_expr {
                verum_ast::ArrayExpr::List(exprs) => {
                    for e in exprs {
                        self.analyze_expr(e, stats, loop_depth);
                    }
                }
                verum_ast::ArrayExpr::Repeat { value, count } => {
                    self.analyze_expr(value, stats, loop_depth);
                    self.analyze_expr(count, stats, loop_depth);
                }
            },
            ExprKind::Cast { expr: inner, .. } => {
                self.analyze_expr(inner, stats, loop_depth);
            }
            ExprKind::Return(maybe_inner) => {
                if let Some(inner) = maybe_inner {
                    self.analyze_expr(inner, stats, loop_depth);
                }
            }
            ExprKind::Await(inner) => {
                self.analyze_expr(inner, stats, loop_depth);
            }
            _ => {}
        }
    }

    /// Calculate overhead percentage
    fn calculate_overhead(&self, stats: &CbgrStats) -> f64 {
        if stats.total_time_ns == 0 {
            return 0.0;
        }
        (stats.cbgr_time_ns as f64 / stats.total_time_ns as f64) * 100.0
    }

    /// Display profiling report
    fn display_report(&self, report: &ProfileReport) {
        println!("{}", "\nCBGR Performance Report:".bold());
        println!("{}", "=".repeat(60));

        // Sort by overhead (highest first)
        let mut functions: List<_> = report.functions.iter().collect();
        functions.sort_by(|a, b| {
            b.1.overhead_pct
                .partial_cmp(&a.1.overhead_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for (name, profile) in functions {
            let overhead_str = format!("{:.1}%", profile.overhead_pct);
            let colored_overhead = if profile.is_hot {
                overhead_str.red()
            } else if profile.overhead_pct > 1.0 {
                overhead_str.yellow()
            } else {
                overhead_str.green()
            };

            println!(
                "  {} {}: {} CBGR overhead",
                if profile.is_hot { "🔥" } else { " " },
                name.as_str().bold(),
                colored_overhead
            );

            println!(
                "      {} CBGR refs, {} ownership refs, {} checks",
                profile.stats.num_cbgr_refs,
                profile.stats.num_ownership_refs,
                profile.stats.num_checks
            );

            if profile.is_hot {
                println!(
                    "      {} Hot path detected! Consider optimization",
                    "⚠".yellow()
                );
            }
        }

        println!();
        println!(
            "Total: {} functions, {} hot paths",
            report.functions.len(),
            report.num_hot_paths()
        );
    }

    /// Display optimization suggestions
    fn display_suggestions(&self, report: &ProfileReport) {
        let hot_functions: List<_> = report.functions.iter().filter(|(_, p)| p.is_hot).collect();

        if hot_functions.is_empty() {
            println!(
                "{}",
                "\n✓ No hot paths detected. CBGR overhead is minimal.".green()
            );
            return;
        }

        println!("{}", "\nOptimization Suggestions:".bold());
        println!("{}", "=".repeat(60));

        for (name, profile) in hot_functions {
            println!(
                "\n  {} {}: {:.1}% overhead",
                "•".yellow(),
                name.as_str().bold(),
                profile.overhead_pct
            );

            if profile.stats.num_cbgr_refs > 0 {
                println!("    {} Convert CBGR refs to ownership:", "→".cyan());
                println!(
                    "      {}",
                    format!("fn {}(data: %T) instead of &T", name).cyan()
                );
                println!("      Benefit: {:.1}% → 0% overhead", profile.overhead_pct);
            }

            if profile.stats.num_checks > 100 {
                println!("    {} Reduce reference checks:", "→".cyan());
                println!("      - Cache references outside loops");
                println!("      - Use raw pointers in trusted code");
            }
        }

        println!(
            "\n{}",
            "For more details, run: verum profile --help".dimmed()
        );
    }

    /// Write report to file
    fn write_report(&self, report: &ProfileReport, path: &Path) -> Result<()> {
        use std::fs::File;
        use std::io::Write;

        let json = serde_json::to_string_pretty(report)?;
        let mut file = File::create(path)?;
        write!(file, "{}", json)?;

        info!("Profiling report written to: {}", path.display());
        Ok(())
    }
}

/// Internal statistics for AST analysis
#[derive(Debug, Default)]
struct RefStats {
    /// Count of CBGR managed references (&T, &mut T)
    cbgr_refs: usize,
    /// Count of checked references (&checked T)
    checked_refs: usize,
    /// Count of unsafe references (&unsafe T)
    unsafe_refs: usize,
    /// Count of ownership references (%T)
    ownership_refs: usize,
    /// Total expression count for timing estimates
    total_expressions: usize,
    /// Estimated loop factor for hot path detection
    estimated_loop_factor: usize,
}

/// CBGR statistics for a function
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CbgrStats {
    pub num_cbgr_refs: usize,
    pub num_ownership_refs: usize,
    pub num_checks: usize,
    pub total_time_ns: u64,
    pub cbgr_time_ns: u64,
}

/// Profile information for a function
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FunctionProfile {
    pub stats: CbgrStats,
    pub overhead_pct: f64,
    pub is_hot: bool,
}

/// Complete profiling report
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProfileReport {
    pub functions: Map<Text, FunctionProfile>,
}

impl ProfileReport {
    pub fn new() -> Self {
        Self {
            functions: Map::new(),
        }
    }

    pub fn add_function(&mut self, name: Text, profile: FunctionProfile) {
        self.functions.insert(name, profile);
    }

    pub fn num_hot_paths(&self) -> usize {
        self.functions.values().filter(|p| p.is_hot).count()
    }
}
