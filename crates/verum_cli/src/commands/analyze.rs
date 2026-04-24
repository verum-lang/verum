//! Deep static analysis command using real compiler infrastructure
//!
//! Runs real escape analysis (verum_cbgr), context checking (verum_types),
//! and refinement type coverage analysis on actual source files.
//!
//! Each analysis mode:
//! - `--escape`: Parses + type checks + CBGR tier analysis on all .vr files
//! - `--context`: Parses + walks AST for context declarations vs usage
//! - `--refinement`: Parses + walks AST for refinement type annotations
//! - `--all`: Runs all three analyses (reusing parsed modules)

use crate::error::Result;
use crate::ui;
use colored::Colorize;
use std::path::{Path, PathBuf};
use verum_common::{Maybe, Text};
use verum_compiler::pipeline::CompilationPipeline;
use verum_compiler::{CompilerOptions, Session};

/// Default confidence threshold for CBGR tier promotion analysis.
/// References with confidence below this threshold remain at Tier 0 (~15ns).
const DEFAULT_CONFIDENCE_THRESHOLD: f64 = 0.95;

/// Analysis type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisType {
    /// Escape analysis for reference promotion
    Escape,
    /// Context system usage analysis
    Context,
    /// Refinement type coverage
    Refinement,
    /// All analyses
    All,
}

/// Execute analysis command
pub fn execute(escape: bool, context: bool, refinement: bool, all: bool) -> Result<()> {
    ui::header("Static Analysis");

    let analysis_type = if all {
        AnalysisType::All
    } else if escape {
        AnalysisType::Escape
    } else if context {
        AnalysisType::Context
    } else if refinement {
        AnalysisType::Refinement
    } else {
        // Default to all if no specific analysis requested
        AnalysisType::All
    };

    // Find .vr source files
    let vr_files = find_vr_files()?;
    if vr_files.is_empty() {
        ui::warn("No .vr source files found in current directory or src/");
        ui::info("Create a .vr file or run from a Verum project directory");
        return Ok(());
    }

    ui::info(&format!("Found {} source file(s)", vr_files.len()));
    println!();

    match analysis_type {
        AnalysisType::Escape => analyze_escape(&vr_files)?,
        AnalysisType::Context => analyze_context(&vr_files)?,
        AnalysisType::Refinement => analyze_refinement(&vr_files)?,
        AnalysisType::All => {
            analyze_escape(&vr_files)?;
            println!();
            analyze_context(&vr_files)?;
            println!();
            analyze_refinement(&vr_files)?;
        }
    }

    Ok(())
}

// =============================================================================
// File discovery
// =============================================================================

/// Find all .vr files in the current directory and src/ subdirectory
fn find_vr_files() -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    // Search current directory
    collect_vr_files(Path::new("."), &mut files, 0)?;

    // Search src/ if it exists
    let src_dir = Path::new("src");
    if src_dir.is_dir() {
        collect_vr_files(src_dir, &mut files, 0)?;
    }

    // Search core/ if it exists
    let core_dir = Path::new("core");
    if core_dir.is_dir() {
        collect_vr_files(core_dir, &mut files, 0)?;
    }

    // Deduplicate (in case src/ or core/ is in cwd)
    files.sort();
    files.dedup();

    Ok(files)
}

fn collect_vr_files(dir: &Path, files: &mut Vec<PathBuf>, depth: usize) -> Result<()> {
    if depth > 5 {
        return Ok(()); // Limit recursion depth
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                // Only .vr extension is valid
                if ext == "vr" {
                    files.push(path.canonicalize().unwrap_or(path));
                }
            }
        } else if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip hidden dirs, target, node_modules, etc.
            if !name.starts_with('.') && name != "target" && name != "node_modules" {
                collect_vr_files(&path, files, depth + 1)?;
            }
        }
    }

    Ok(())
}

/// Helper to get a short display name for a file path
fn file_display_name(path: &Path) -> Text {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?")
        .into()
}

/// Helper to parse a single .vr file into a module AST.
/// Returns (module, pipeline) or an error string.
fn parse_file(
    path: &Path,
) -> std::result::Result<verum_ast::Module, String> {
    let mut options = CompilerOptions::default();
    options.input = path.to_path_buf();
    let mut session = Session::new(options);
    let file_id = session
        .load_file(path)
        .map_err(|e| format!("Failed to load: {}", e))?;
    let mut pipeline = CompilationPipeline::new_check(&mut session);
    let module = pipeline
        .phase_parse(file_id)
        .map_err(|e| format!("Parse error: {}", e))?;
    Ok(module)
}

// =============================================================================
// Escape Analysis (real CBGR tier analysis)
// =============================================================================

fn analyze_escape(vr_files: &[PathBuf]) -> Result<()> {
    use verum_cbgr::tier_analysis::TierAnalysisConfig;
    use verum_cbgr::tier_types::TierStatistics;

    ui::section("Escape Analysis (CBGR Tier Promotion)");
    ui::info("Analyzing references for automatic &T -> &checked T promotion");
    println!();

    let config = TierAnalysisConfig {
        confidence_threshold: DEFAULT_CONFIDENCE_THRESHOLD,
        analyze_async_boundaries: true,
        analyze_exception_paths: true,
        enable_ownership_analysis: true,
        enable_concurrency_analysis: true,
        enable_lifetime_analysis: true,
        enable_nll_analysis: true,
        ..Default::default()
    };

    ui::info(&format!(
        "Tier analysis config: confidence_threshold={:.2}, async={}, exceptions={}",
        config.confidence_threshold,
        config.analyze_async_boundaries,
        config.analyze_exception_paths,
    ));
    println!();

    // Parse each file and run CBGR tier analysis
    let mut global_stats = TierStatistics::new();
    let mut per_function_results: Vec<FunctionEscapeInfo> = Vec::new();
    let mut file_errors = 0usize;

    for path in vr_files {
        match run_escape_analysis_on_file(path, &config) {
            Ok((stats, func_infos)) => {
                global_stats.merge(&stats);
                per_function_results.extend(func_infos);
            }
            Err(e) => {
                file_errors += 1;
                ui::warn(&format!("Skipping {}: {}", file_display_name(path), e));
            }
        }
    }

    if file_errors > 0 && per_function_results.is_empty() {
        ui::warn("No functions could be analyzed. Fix compilation errors first.");
        ui::info("Run `verum check` to see detailed error messages.");
        return Ok(());
    }

    // Display promotable references
    let promotable: Vec<_> = per_function_results
        .iter()
        .filter(|f| f.tier1_count > 0)
        .collect();
    let non_promotable: Vec<_> = per_function_results
        .iter()
        .filter(|f| f.tier0_count > 0)
        .collect();

    if !promotable.is_empty() {
        println!("  {} Promotion Opportunities:", "->".green());
        println!();

        for info in &promotable {
            println!(
                "  {} {} ({})",
                "->".green().bold(),
                info.function_name.as_str().bold(),
                info.file_name.as_str().dimmed(),
            );
            println!(
                "    {} of {} refs promoted to &checked T (0ns)",
                info.tier1_count, info.total_refs,
            );
            if info.total_refs > 0 {
                let rate = info.tier1_count as f64 / info.total_refs as f64 * 100.0;
                println!("    Promotion rate: {:.1}%", rate);
            }
            if info.estimated_savings_ns > 0 {
                println!(
                    "    Estimated savings: ~{}ns/execution",
                    info.estimated_savings_ns
                );
            }
            println!();
        }
    }

    if !non_promotable.is_empty() {
        println!("  {} References Kept at Tier 0 (~15ns CBGR):", "!".yellow());
        println!();

        for info in &non_promotable {
            println!(
                "  {} {} ({})",
                "!".yellow(),
                info.function_name.as_str().bold(),
                info.file_name.as_str().dimmed(),
            );
            println!("    {} ref(s) at Tier 0", info.tier0_count);
            for (reason, count) in &info.tier0_reasons {
                println!("      - {}: {}", format_tier0_reason(reason), count);
            }
            println!();
        }
    }

    if promotable.is_empty() && non_promotable.is_empty() && global_stats.total_refs == 0 {
        println!("  No references found to analyze.");
        println!("  This is normal for code without reference parameters.");
    }

    // Summary
    println!("  Summary:");
    println!(
        "    Functions analyzed: {}",
        global_stats.functions_analyzed
    );
    println!("    Total references:   {}", global_stats.total_refs);
    println!(
        "    Tier 1 (0ns):       {} ({:.1}%)",
        global_stats.tier1_count,
        global_stats.promotion_rate() * 100.0
    );
    println!(
        "    Tier 0 (~15ns):     {}",
        global_stats.tier0_count
    );
    if global_stats.tier2_count > 0 {
        println!("    Tier 2 (unsafe):    {}", global_stats.tier2_count);
    }
    if global_stats.estimated_savings_ns > 0 {
        println!(
            "    Estimated savings:  ~{}ns/execution",
            global_stats.estimated_savings_ns
        );
    }
    println!(
        "    Analysis time:      {}us",
        global_stats.analysis_duration_us
    );

    Ok(())
}

/// Per-function escape analysis result
struct FunctionEscapeInfo {
    function_name: Text,
    file_name: Text,
    total_refs: u64,
    tier0_count: u64,
    tier1_count: u64,
    tier0_reasons: Vec<(verum_cbgr::tier_types::Tier0Reason, u64)>,
    estimated_savings_ns: u64,
}

/// Run escape analysis on a single file by parsing it and running
/// CBGR tier analysis on each function's CFG.
fn run_escape_analysis_on_file(
    path: &Path,
    config: &verum_cbgr::tier_analysis::TierAnalysisConfig,
) -> std::result::Result<
    (verum_cbgr::tier_types::TierStatistics, Vec<FunctionEscapeInfo>),
    String,
> {
    use verum_cbgr::tier_analysis::TierAnalyzer;
    use verum_cbgr::tier_types::TierStatistics;

    let file_name = file_display_name(path);

    // Create a compilation session and pipeline in check-only mode
    let mut options = CompilerOptions::default();
    options.input = path.to_path_buf();
    let mut session = Session::new(options);

    // Load and parse the file
    let file_id = session
        .load_file(path)
        .map_err(|e| format!("Failed to load: {}", e))?;
    let mut pipeline = CompilationPipeline::new_check(&mut session);
    let module = pipeline
        .phase_parse(file_id)
        .map_err(|e| format!("Parse error: {}", e))?;

    // Run type checking (best effort -- errors are non-fatal for escape analysis)
    let _ = pipeline.run_type_check_phase(&module);

    // Build CFGs and run tier analysis on each function
    let mut global_stats = TierStatistics::new();
    let mut func_infos = Vec::new();

    for item in module.items.iter() {
        if let verum_ast::decl::ItemKind::Function(func) = &item.kind {
            if func.is_meta {
                continue;
            }

            // Build CFG from function AST
            let cfg = pipeline.build_function_cfg_public(func);

            // Run tier analysis
            let analyzer = TierAnalyzer::with_config(cfg, config.clone());
            let result = analyzer.analyze();

            if result.stats.total_refs > 0 {
                // Collect tier0 reasons sorted by count
                let mut reasons: Vec<_> = result
                    .stats
                    .tier0_reasons
                    .iter()
                    .map(|(r, c)| (*r, *c))
                    .collect();
                reasons.sort_by(|a, b| b.1.cmp(&a.1));

                func_infos.push(FunctionEscapeInfo {
                    function_name: func.name.name.clone(),
                    file_name: file_name.clone(),
                    total_refs: result.stats.total_refs,
                    tier0_count: result.stats.tier0_count,
                    tier1_count: result.stats.tier1_count,
                    tier0_reasons: reasons,
                    estimated_savings_ns: result.stats.estimated_savings_ns,
                });
            }

            global_stats.merge(&result.stats);
        }
    }

    Ok((global_stats, func_infos))
}

fn format_tier0_reason(reason: &verum_cbgr::tier_types::Tier0Reason) -> &'static str {
    use verum_cbgr::tier_types::Tier0Reason;
    match reason {
        Tier0Reason::Escapes => "Escapes scope",
        Tier0Reason::DominanceFailure => "Allocation does not dominate all uses",
        Tier0Reason::AsyncBoundary => "Crosses async/await boundary",
        Tier0Reason::ExceptionPath => "On exception handling path",
        Tier0Reason::Conservative => "Conservative (analysis uncertain)",
        Tier0Reason::NotAnalyzed => "Not analyzed",
        Tier0Reason::UseAfterFree => "Potential use-after-free detected",
        Tier0Reason::LifetimeViolation => "Lifetime violation detected",
        Tier0Reason::BorrowViolation => "Borrow checking violation",
        Tier0Reason::LowConfidence => "Low confidence score",
        Tier0Reason::ConcurrentAccess => "Concurrent access detected",
        Tier0Reason::MutableFieldStore => "Stored to mutable field",
        Tier0Reason::ExternalCall => "Passed to external function",
        Tier0Reason::DoubleFree => "Double-free detected",
        Tier0Reason::DataRace => "Data race detected",
        Tier0Reason::AnalysisTimeout => "Analysis timed out",
    }
}

// =============================================================================
// Context Analysis (real AST walking)
// =============================================================================

/// Result of context analysis for a single function
struct ContextIssue {
    function_name: Text,
    file_name: Text,
    kind: ContextIssueKind,
    details: Text,
}

enum ContextIssueKind {
    /// Function uses a context but does not declare it in `using [...]`
    MissingDeclaration,
    /// Function declares a context in `using [...]` but never uses it
    UnusedContext,
}

fn analyze_context(vr_files: &[PathBuf]) -> Result<()> {
    ui::section("Context System Analysis");
    ui::info("Analyzing context declarations, usage, and propagation");
    println!();

    let mut issues: Vec<ContextIssue> = Vec::new();
    let mut total_functions = 0usize;
    let mut functions_with_contexts = 0usize;
    let mut total_context_decls = 0usize;
    let mut file_errors = 0usize;

    for path in vr_files {
        match analyze_context_in_file(path) {
            Ok(file_result) => {
                total_functions += file_result.total_functions;
                functions_with_contexts += file_result.functions_with_contexts;
                total_context_decls += file_result.total_context_decls;
                issues.extend(file_result.issues);
            }
            Err(e) => {
                file_errors += 1;
                ui::warn(&format!("Skipping {}: {}", file_display_name(path), e));
            }
        }
    }

    if file_errors > 0 && total_functions == 0 {
        ui::warn("No files could be analyzed. Fix compilation errors first.");
        ui::info("Run `verum check` to see detailed error messages.");
        return Ok(());
    }

    if issues.is_empty() {
        println!(
            "  {} All context declarations are consistent",
            "OK".green()
        );
        println!();
    } else {
        let missing: Vec<_> = issues
            .iter()
            .filter(|i| matches!(i.kind, ContextIssueKind::MissingDeclaration))
            .collect();
        let unused: Vec<_> = issues
            .iter()
            .filter(|i| matches!(i.kind, ContextIssueKind::UnusedContext))
            .collect();

        if !missing.is_empty() {
            println!(
                "  {} Missing Context Declarations ({}):",
                "!".yellow(),
                missing.len()
            );
            println!();
            for issue in &missing {
                println!(
                    "  {} {} ({})",
                    "!".yellow(),
                    issue.function_name.as_str().bold(),
                    issue.file_name.as_str().dimmed(),
                );
                println!("    {}", issue.details);
                println!();
            }
        }

        if !unused.is_empty() {
            println!(
                "  {} Unused Context Declarations ({}):",
                "?".blue(),
                unused.len()
            );
            println!();
            for issue in &unused {
                println!(
                    "  {} {} ({})",
                    "?".blue(),
                    issue.function_name.as_str().bold(),
                    issue.file_name.as_str().dimmed(),
                );
                println!("    {}", issue.details);
                println!();
            }
        }
    }

    println!("  Summary:");
    println!("    Total functions scanned:       {}", total_functions);
    println!(
        "    Functions with using [...]:    {}",
        functions_with_contexts
    );
    println!("    Total context declarations:    {}", total_context_decls);
    println!("    Issues found:                  {}", issues.len());

    if !issues.is_empty() {
        println!();
        println!("  Recommendations:");
        println!("    - Add missing context declarations to avoid runtime resolution overhead");
        println!("    - Remove unused context declarations to reduce API surface");
        println!("    - Ensure context requirements propagate through call chains");
    }

    Ok(())
}

struct FileContextResult {
    total_functions: usize,
    functions_with_contexts: usize,
    total_context_decls: usize,
    issues: Vec<ContextIssue>,
}

/// Extract a display name from a context requirement's path
fn context_req_name(ctx: &verum_ast::ContextRequirement) -> Text {
    use verum_ast::ty::PathSegment;
    ctx.path
        .segments
        .iter()
        .filter_map(|seg| match seg {
            PathSegment::Name(ident) => Some(ident.name.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(".")
        .into()
}

fn analyze_context_in_file(path: &Path) -> std::result::Result<FileContextResult, String> {
    let file_name = file_display_name(path);
    let module = parse_file(path)?;

    let mut result = FileContextResult {
        total_functions: 0,
        functions_with_contexts: 0,
        total_context_decls: 0,
        issues: Vec::new(),
    };

    // Walk all functions in the module
    for item in module.items.iter() {
        if let verum_ast::decl::ItemKind::Function(func) = &item.kind {
            if func.is_meta {
                continue;
            }

            result.total_functions += 1;
            let declared_contexts: Vec<Text> = func
                .contexts
                .iter()
                .map(|c| context_req_name(c))
                .collect();

            if !declared_contexts.is_empty() {
                result.functions_with_contexts += 1;
                result.total_context_decls += declared_contexts.len();
            }

            // Walk the function body for context usage patterns
            if let Maybe::Some(ref body) = func.body {
                let block = match body {
                    verum_ast::decl::FunctionBody::Block(b) => Some(b),
                    verum_ast::decl::FunctionBody::Expr(_) => None,
                };

                let used_contexts = if let Some(block) = block {
                    extract_context_usage_from_block(block)
                } else {
                    Vec::new()
                };

                // Check for missing declarations: used but not declared
                for used in &used_contexts {
                    if !declared_contexts.iter().any(|d| d.as_str() == used.as_str()) {
                        result.issues.push(ContextIssue {
                            function_name: func.name.name.clone(),
                            file_name: file_name.clone(),
                            kind: ContextIssueKind::MissingDeclaration,
                            details: format!(
                                "Uses context '{}' but does not declare it in using [...].\n    Fix: Add 'using [{}]' to the function signature",
                                used, used
                            ).into(),
                        });
                    }
                }

                // Check for unused declarations: declared but not used in body
                for decl in &declared_contexts {
                    if !used_contexts.iter().any(|u| u.as_str() == decl.as_str()) {
                        result.issues.push(ContextIssue {
                            function_name: func.name.name.clone(),
                            file_name: file_name.clone(),
                            kind: ContextIssueKind::UnusedContext,
                            details: format!(
                                "Declares context '{}' in using [...] but never uses it in the function body.\n    Fix: Remove '{}' from the using clause or use it",
                                decl, decl
                            ).into(),
                        });
                    }
                }
            }
        }
    }

    Ok(result)
}

/// Walk a block to find context usage (context method calls).
/// Looks for patterns like `ContextName.method(...)` calls where
/// the receiver is a capitalized identifier (typical of context protocol names).
fn extract_context_usage_from_block(block: &verum_ast::expr::Block) -> Vec<Text> {
    let mut contexts = Vec::new();
    for stmt in &block.stmts {
        walk_stmt_for_contexts(stmt, &mut contexts);
    }
    if let Maybe::Some(ref tail) = block.expr {
        walk_expr_for_contexts(tail, &mut contexts);
    }
    contexts.sort();
    contexts.dedup();
    contexts
}

fn walk_stmt_for_contexts(stmt: &verum_ast::stmt::Stmt, contexts: &mut Vec<Text>) {
    use verum_ast::stmt::StmtKind;
    match &stmt.kind {
        StmtKind::Expr { expr, .. } => walk_expr_for_contexts(expr, contexts),
        StmtKind::Let { value, .. } => {
            if let Maybe::Some(val) = value {
                walk_expr_for_contexts(val, contexts);
            }
        }
        StmtKind::LetElse { value, else_block, .. } => {
            walk_expr_for_contexts(value, contexts);
            walk_block_for_contexts(else_block, contexts);
        }
        StmtKind::Item(_) => {}
        _ => {}
    }
}

fn walk_block_for_contexts(block: &verum_ast::expr::Block, contexts: &mut Vec<Text>) {
    for stmt in &block.stmts {
        walk_stmt_for_contexts(stmt, contexts);
    }
    if let Maybe::Some(ref tail) = block.expr {
        walk_expr_for_contexts(tail, contexts);
    }
}

fn walk_expr_for_contexts(expr: &verum_ast::expr::Expr, contexts: &mut Vec<Text>) {
    use verum_ast::expr::ExprKind;
    use verum_ast::ty::PathSegment;

    match &expr.kind {
        ExprKind::MethodCall { receiver, args, .. } => {
            // Check if receiver is a capitalized path (typical for context protocols)
            if let ExprKind::Path(path) = &receiver.kind {
                if let Some(first_seg) = path.segments.first() {
                    if let PathSegment::Name(ident) = first_seg {
                        let name = ident.name.as_str();
                        if !name.is_empty() && name.chars().next().unwrap().is_uppercase() {
                            contexts.push(ident.name.clone());
                        }
                    }
                }
            }
            walk_expr_for_contexts(receiver, contexts);
            for arg in args.iter() {
                walk_expr_for_contexts(arg, contexts);
            }
        }
        ExprKind::Call { func, args, .. } => {
            walk_expr_for_contexts(func, contexts);
            for arg in args.iter() {
                walk_expr_for_contexts(arg, contexts);
            }
        }
        ExprKind::Block(block) => {
            walk_block_for_contexts(block, contexts);
        }
        ExprKind::If { condition, then_branch, else_branch, .. } => {
            // condition is IfCondition, walk its expr
            walk_if_condition_for_contexts(condition, contexts);
            walk_block_for_contexts(then_branch, contexts);
            if let Maybe::Some(else_expr) = else_branch {
                walk_expr_for_contexts(else_expr, contexts);
            }
        }
        ExprKind::Match { expr: scrutinee, arms, .. } => {
            walk_expr_for_contexts(scrutinee, contexts);
            for arm in arms.iter() {
                walk_expr_for_contexts(&arm.body, contexts);
            }
        }
        ExprKind::While { condition, body, .. } => {
            walk_expr_for_contexts(condition, contexts);
            walk_block_for_contexts(body, contexts);
        }
        ExprKind::For { iter, body, .. } => {
            walk_expr_for_contexts(iter, contexts);
            walk_block_for_contexts(body, contexts);
        }
        ExprKind::Binary { left, right, .. } => {
            walk_expr_for_contexts(left, contexts);
            walk_expr_for_contexts(right, contexts);
        }
        ExprKind::Unary { expr: inner, .. } => {
            walk_expr_for_contexts(inner, contexts);
        }
        ExprKind::Return(maybe_expr) => {
            if let Maybe::Some(inner) = maybe_expr {
                walk_expr_for_contexts(inner, contexts);
            }
        }
        ExprKind::Closure { body, .. } => {
            walk_expr_for_contexts(body, contexts);
        }
        _ => {}
    }
}

fn walk_if_condition_for_contexts(
    cond: &verum_ast::expr::IfCondition,
    contexts: &mut Vec<Text>,
) {
    use verum_ast::expr::ConditionKind;
    for condition in &cond.conditions {
        match condition {
            ConditionKind::Expr(expr) => walk_expr_for_contexts(expr, contexts),
            ConditionKind::Let { value, .. } => walk_expr_for_contexts(value, contexts),
        }
    }
}

// =============================================================================
// Refinement Coverage Analysis (real AST walking)
// =============================================================================

fn analyze_refinement(vr_files: &[PathBuf]) -> Result<()> {
    ui::section("Refinement Type Coverage");
    ui::info("Analyzing refinement type annotations and verification status");
    println!();

    let mut total_type_annotations = 0usize;
    let mut refined_type_count = 0usize;
    let mut functions_with_requires = 0usize;
    let mut functions_with_ensures = 0usize;
    let mut functions_with_verify = 0usize;
    let mut total_functions = 0usize;
    let mut unverified: Vec<UnverifiedRefinement> = Vec::new();
    let mut file_errors = 0usize;

    for path in vr_files {
        match analyze_refinement_in_file(path) {
            Ok(file_result) => {
                total_type_annotations += file_result.total_type_annotations;
                refined_type_count += file_result.refined_type_count;
                functions_with_requires += file_result.functions_with_requires;
                functions_with_ensures += file_result.functions_with_ensures;
                functions_with_verify += file_result.functions_with_verify;
                total_functions += file_result.total_functions;
                unverified.extend(file_result.unverified);
            }
            Err(e) => {
                file_errors += 1;
                ui::warn(&format!("Skipping {}: {}", file_display_name(path), e));
            }
        }
    }

    if file_errors > 0 && total_functions == 0 {
        ui::warn("No files could be analyzed. Fix parse errors first.");
        return Ok(());
    }

    let coverage_pct = if total_type_annotations > 0 {
        refined_type_count as f64 / total_type_annotations as f64 * 100.0
    } else {
        0.0
    };

    println!("  Coverage Statistics:");
    println!("    Total functions:              {}", total_functions);
    println!(
        "    Total type annotations:       {}",
        total_type_annotations
    );
    println!(
        "    Refined types (with pred.):   {} ({:.1}%)",
        refined_type_count, coverage_pct
    );
    println!(
        "    Functions with requires:      {}",
        functions_with_requires
    );
    println!(
        "    Functions with ensures:       {}",
        functions_with_ensures
    );
    println!(
        "    Functions with @verify:       {}",
        functions_with_verify
    );
    println!();

    if !unverified.is_empty() {
        println!(
            "  {} Refinements Without @verify ({}):",
            "!".yellow(),
            unverified.len()
        );
        println!();

        for item in &unverified {
            let status = if item.appears_provable {
                "Likely provable".green()
            } else {
                "May need runtime check".yellow()
            };

            println!(
                "  {} {} ({})",
                if item.appears_provable {
                    "OK".green()
                } else {
                    "!".yellow()
                },
                item.name.as_str().bold(),
                item.file_name.as_str().dimmed(),
            );
            println!("    Predicate: {}", item.predicate);
            println!("    Status: {}", status);
            if !item.suggestion.is_empty() {
                println!("    Suggestion: {}", item.suggestion);
            }
            println!();
        }
    }

    if total_type_annotations > 0 || !unverified.is_empty() {
        println!("  Recommendations:");
        let provable_count = unverified.iter().filter(|u| u.appears_provable).count();
        if provable_count > 0 {
            println!(
                "    - {} refinement(s) appear provable at compile-time",
                provable_count
            );
            println!("      Add @verify annotation or run `verum verify` for SMT checking");
        }

        let complex_count = unverified.len() - provable_count;
        if complex_count > 0 {
            println!(
                "    - {} refinement(s) may need runtime checks",
                complex_count
            );
            println!("      Consider @verify(runtime) for faster iteration");
        }

        if coverage_pct < 50.0 && total_type_annotations > 0 {
            println!(
                "    - Coverage is {:.1}% -- consider adding refinement types to critical paths",
                coverage_pct
            );
        } else if coverage_pct >= 90.0 {
            println!("    - Excellent coverage at {:.1}%", coverage_pct);
        }
    }

    Ok(())
}

struct UnverifiedRefinement {
    name: Text,
    file_name: Text,
    predicate: Text,
    appears_provable: bool,
    suggestion: Text,
}

struct FileRefinementResult {
    total_functions: usize,
    total_type_annotations: usize,
    refined_type_count: usize,
    functions_with_requires: usize,
    functions_with_ensures: usize,
    functions_with_verify: usize,
    unverified: Vec<UnverifiedRefinement>,
}

fn analyze_refinement_in_file(path: &Path) -> std::result::Result<FileRefinementResult, String> {
    let file_name = file_display_name(path);
    let module = parse_file(path)?;

    let mut result = FileRefinementResult {
        total_functions: 0,
        total_type_annotations: 0,
        refined_type_count: 0,
        functions_with_requires: 0,
        functions_with_ensures: 0,
        functions_with_verify: 0,
        unverified: Vec::new(),
    };

    for item in module.items.iter() {
        match &item.kind {
            verum_ast::decl::ItemKind::Function(func) => {
                if func.is_meta {
                    continue;
                }
                result.total_functions += 1;

                // Check for @verify attribute
                let has_verify = func
                    .attributes
                    .iter()
                    .any(|a| a.name.as_str() == "verify");
                if has_verify {
                    result.functions_with_verify += 1;
                }

                // Check for requires/ensures clauses
                if !func.requires.is_empty() {
                    result.functions_with_requires += 1;
                }
                if !func.ensures.is_empty() {
                    result.functions_with_ensures += 1;
                }

                // Count type annotations in parameters
                for param in func.params.iter() {
                    if let Some(ty) = extract_param_type(param) {
                        result.total_type_annotations += 1;
                        if has_refinement(ty) {
                            result.refined_type_count += 1;
                            if !has_verify {
                                let param_name = extract_param_name(param);
                                result.unverified.push(UnverifiedRefinement {
                                    name: format!(
                                        "{}.{}",
                                        func.name.name.as_str(),
                                        param_name,
                                    )
                                    .into(),
                                    file_name: file_name.clone(),
                                    predicate: format_type_refinement(ty),
                                    appears_provable: is_likely_provable(ty),
                                    suggestion: refinement_suggestion(ty),
                                });
                            }
                        }
                    }
                }

                // Count return type annotation
                if let Maybe::Some(ref ret_ty) = func.return_type {
                    result.total_type_annotations += 1;
                    if has_refinement(ret_ty) {
                        result.refined_type_count += 1;
                        if !has_verify {
                            result.unverified.push(UnverifiedRefinement {
                                name: format!("{} (return)", func.name.name.as_str()).into(),
                                file_name: file_name.clone(),
                                predicate: format_type_refinement(ret_ty),
                                appears_provable: is_likely_provable(ret_ty),
                                suggestion: refinement_suggestion(ret_ty),
                            });
                        }
                    }
                }
            }
            verum_ast::decl::ItemKind::Type(type_decl) => {
                count_refinements_in_type_decl(type_decl, &mut result);
            }
            _ => {}
        }
    }

    Ok(result)
}

/// Extract type from a function parameter
fn extract_param_type(param: &verum_ast::decl::FunctionParam) -> Option<&verum_ast::Type> {
    use verum_ast::decl::FunctionParamKind;
    match &param.kind {
        FunctionParamKind::Regular { ty, .. } => Some(ty),
        _ => None, // SelfValue, SelfValueMut, SelfRef, SelfMutRef, SelfChecked, SelfUnsafe
    }
}

/// Extract a display name from a function parameter
fn extract_param_name(param: &verum_ast::decl::FunctionParam) -> &str {
    use verum_ast::decl::FunctionParamKind;
    match &param.kind {
        FunctionParamKind::Regular { pattern, .. } => {
            // Try to extract name from pattern
            use verum_ast::pattern::PatternKind;
            match &pattern.kind {
                PatternKind::Ident { name, .. } => name.name.as_str(),
                _ => "_",
            }
        }
        _ => "self",
    }
}

/// Check if a type AST node contains a refinement predicate
fn has_refinement(ty: &verum_ast::Type) -> bool {
    use verum_ast::ty::TypeKind;
    match &ty.kind {
        // VUVA §5 canonical: Refined covers all three refinement surface forms.
        TypeKind::Refined { .. } => true,
        TypeKind::Function {
            params,
            return_type,
            ..
        } => {
            params.iter().any(|p| has_refinement(p)) || has_refinement(return_type)
        }
        TypeKind::Tuple(types) => types.iter().any(|t| has_refinement(t)),
        TypeKind::Array { element, .. } => has_refinement(element),
        TypeKind::Reference { inner, .. } => has_refinement(inner),
        TypeKind::Ownership { inner, .. } => has_refinement(inner),
        _ => false,
    }
}

/// Format the refinement predicate for display
fn format_type_refinement(ty: &verum_ast::Type) -> Text {
    use verum_ast::ty::TypeKind;
    match &ty.kind {
        TypeKind::Refined { base, predicate } => {
            // VUVA §5: the sigma form lives on Refined with a Some binder.
            match &predicate.binding {
                verum_common::Maybe::Some(binder) => {
                    format!("{}: {}{{...}}", binder.name, format_type_name(base)).into()
                }
                verum_common::Maybe::None => {
                    format!("{}{{...}}", format_type_name(base)).into()
                }
            }
        }
        _ => "(compound type with refinement)".into(),
    }
}

/// Format a type name for display
fn format_type_name(ty: &verum_ast::Type) -> String {
    use verum_ast::ty::{PathSegment, TypeKind};
    match &ty.kind {
        TypeKind::Path(path) => path
            .segments
            .iter()
            .filter_map(|s| match s {
                PathSegment::Name(ident) => Some(ident.name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("."),
        TypeKind::Int => "Int".to_string(),
        TypeKind::Float => "Float".to_string(),
        TypeKind::Bool => "Bool".to_string(),
        TypeKind::Text => "Text".to_string(),
        _ => "<type>".to_string(),
    }
}

/// Heuristic: is this refinement likely provable by SMT?
/// Simple linear predicates on ints/floats are typically provable.
fn is_likely_provable(ty: &verum_ast::Type) -> bool {
    use verum_ast::ty::TypeKind;
    match &ty.kind {
        // VUVA §5 canonical: Refined covers all three refinement surface forms.
        TypeKind::Refined { base, .. } => {
            // Simple base types with linear predicates are usually provable
            matches!(
                base.kind,
                TypeKind::Int | TypeKind::Float | TypeKind::Bool | TypeKind::Path(_)
            )
        }
        _ => false,
    }
}

/// Generate a suggestion for unverified refinements
fn refinement_suggestion(ty: &verum_ast::Type) -> Text {
    if is_likely_provable(ty) {
        "Add @verify to enable compile-time SMT proof, or run `verum verify`".into()
    } else {
        "Complex predicate -- consider @verify(runtime) for runtime checking".into()
    }
}

/// Count refinements in a type declaration's fields
fn count_refinements_in_type_decl(
    type_decl: &verum_ast::decl::TypeDecl,
    result: &mut FileRefinementResult,
) {
    use verum_ast::decl::TypeDeclBody;

    match &type_decl.body {
        TypeDeclBody::Record(fields) => {
            for field in fields.iter() {
                result.total_type_annotations += 1;
                if has_refinement(&field.ty) {
                    result.refined_type_count += 1;
                }
            }
        }
        TypeDeclBody::Tuple(types) => {
            for ty in types.iter() {
                result.total_type_annotations += 1;
                if has_refinement(ty) {
                    result.refined_type_count += 1;
                }
            }
        }
        TypeDeclBody::Newtype(ty) => {
            result.total_type_annotations += 1;
            if has_refinement(ty) {
                result.refined_type_count += 1;
            }
        }
        _ => {}
    }
}
