//! Static CBGR source profiling. Reference annotations are counted from the
//! syntax tree. The cost model assigns explicit weights to potential accesses
//! and loop iterations. Only executing the program can establish its runtime cost.

use anyhow::{Context, Result};
use colored::Colorize;
use std::path::Path;
use tracing::{debug, info};
use verum_ast::decl::ItemKind;
use verum_ast::visitor::{Visitor, walk_expr, walk_type};
use verum_ast::{Expr, ExprKind, FunctionDecl, Module, Type, TypeKind};
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
        let report = ProfileReport::estimate(&module, self.session.options().hot_path_threshold)?;

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
        anyhow::bail!("Runtime sampling is unavailable in the static CBGR profiler")
    }
}

impl ProfileReport {
    /// Profile CBGR usage in module
    pub fn estimate(module: &Module, threshold: f64) -> Result<ProfileReport> {
        let mut report = ProfileReport::new();

        for item in &module.items {
            if let ItemKind::Function(func) = &item.kind {
                debug!("Profiling function: {}", func.name);
                let stats = Self::profile_function(func)?;

                // Calculate overhead percentage
                let overhead_pct = Self::calculate_overhead(&stats);

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
    fn profile_function(func: &FunctionDecl) -> Result<CbgrStats> {
        let mut stats = RefStats::default();
        stats.visit_function(func);

        let model = CbgrCostModel::DEFAULT;
        let (estimated_checks, cbgr_time_ns, total_time_ns) =
            model.estimate_costs(&stats).ok_or_else(|| {
                anyhow::anyhow!(
                    "CBGR cost estimate exceeds the report range for {}",
                    func.name
                )
            })?;

        Ok(CbgrStats {
            num_cbgr_refs: stats.cbgr_refs,
            num_ownership_refs: stats.ownership_refs,
            num_checks: estimated_checks,
            total_time_ns,
            cbgr_time_ns,
        })
    }

    /// Calculate overhead percentage
    fn calculate_overhead(stats: &CbgrStats) -> f64 {
        if stats.total_time_ns == 0 {
            return 0.0;
        }
        (stats.cbgr_time_ns as f64 / stats.total_time_ns as f64) * 100.0
    }
}

impl ProfileCommand<'_> {
    /// Display the static CBGR estimate.
    fn display_report(&self, report: &ProfileReport) {
        println!("{}", "\nStatic CBGR cost estimates:".bold());
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
                "  {} {}: {} modeled CBGR overhead",
                if profile.is_hot { "🔥" } else { " " },
                name.as_str().bold(),
                colored_overhead
            );

            println!(
                "      {} CBGR refs, {} ownership refs, {} modeled checks",
                profile.stats.num_cbgr_refs,
                profile.stats.num_ownership_refs,
                profile.stats.num_checks
            );

            if profile.is_hot {
                println!(
                    "      {} Function exceeds the model threshold",
                    "⚠".yellow()
                );
            }
        }

        println!();
        println!(
            "Total: {} functions, {} above the model threshold",
            report.functions.len(),
            report.num_hot_paths()
        );
    }

    /// Display optimization suggestions
    fn display_suggestions(&self, report: &ProfileReport) {
        for (name, profile) in report.functions.iter().filter(|(_, p)| p.is_hot) {
            println!(
                "  {name}: benchmark the reference accesses behind the {:.1}% estimate",
                profile.overhead_pct
            );
        }
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

/// Assumptions of the static cost model, published with its estimates.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct CbgrCostModel {
    pub assumed_check_ns: u64,
    pub assumed_expression_ns: u64,
    pub assumed_iterations_per_loop: usize,
}

impl CbgrCostModel {
    pub const DEFAULT: Self = Self {
        assumed_check_ns: 15,
        assumed_expression_ns: 100,
        assumed_iterations_per_loop: 10,
    };

    fn estimate_costs(self, stats: &RefStats) -> Option<(usize, u64, u64)> {
        let checks =
            stats
                .access_sites_by_depth
                .iter()
                .try_fold(0usize, |sum, (&depth, &sites)| {
                    let iterations = self
                        .assumed_iterations_per_loop
                        .checked_pow(depth.try_into().ok()?)?;
                    sum.checked_add(sites.checked_mul(iterations)?)
                })?;
        let cbgr_ns = u64::try_from(checks)
            .ok()?
            .checked_mul(self.assumed_check_ns)?;
        let expression_ns = u64::try_from(stats.total_expressions)
            .ok()?
            .checked_mul(self.assumed_expression_ns)?;
        Some((checks, cbgr_ns, expression_ns.checked_add(cbgr_ns)?))
    }
}

/// Internal statistics for AST analysis
#[derive(Debug, Default)]
struct RefStats {
    /// Count of CBGR managed references (&T, &mut T)
    cbgr_refs: usize,
    /// Count of ownership references (%T)
    ownership_refs: usize,
    /// Total expression count for timing estimates
    total_expressions: usize,
    loop_depth: usize,
    access_sites_by_depth: std::collections::BTreeMap<usize, usize>,
}

impl Visitor for RefStats {
    fn visit_type(&mut self, ty: &Type) {
        match ty.kind {
            TypeKind::Reference { .. } => self.cbgr_refs += 1,
            TypeKind::Ownership { .. } => self.ownership_refs += 1,
            _ => {}
        }
        walk_type(self, ty);
    }

    fn visit_expr(&mut self, expr: &Expr) {
        self.total_expressions += 1;
        // Each dereference or index is a potential check. An unused reference
        // is still a source annotation, with no corresponding access site.
        if matches!(
            expr.kind,
            ExprKind::Unary {
                op: verum_ast::UnOp::Deref,
                ..
            } | ExprKind::Index { .. }
        ) {
            *self
                .access_sites_by_depth
                .entry(self.loop_depth)
                .or_default() += 1;
        }
        let nested = matches!(
            expr.kind,
            ExprKind::Loop { .. }
                | ExprKind::While { .. }
                | ExprKind::For { .. }
                | ExprKind::ForAwait { .. }
        );
        self.loop_depth += usize::from(nested);
        walk_expr(self, expr);
        self.loop_depth -= usize::from(nested);
    }
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
