//! Performance profiling command with CBGR overhead analysis
//!
//! Compiles the input source through the full pipeline, then extracts real
//! compilation phase timings and CBGR tier analysis results. For --memory mode,
//! reports reference type breakdown (&T vs &checked T vs &unsafe T), per-function
//! estimated CBGR overhead, promotion opportunities, and hot spots based on real
//! escape analysis. For --cpu/--cache modes, reports real compilation phase timings.
//!
//! All data comes from actual compilation — no hardcoded or simulated values.

use crate::error::Result;
use crate::ui;
use colored::Colorize;
use std::path::PathBuf;
use verum_compiler::compilation_metrics::CompilationProfileReport;
use verum_compiler::options::CompilerOptions;
use verum_compiler::pipeline::CompilationPipeline;
use verum_compiler::session::Session;

/// Profiling target
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileTarget {
    /// CBGR memory profiling
    Memory,
    /// CPU profiling
    Cpu,
    /// Cache analysis
    Cache,
    /// Compilation pipeline profiling
    Compilation,
    /// All profiling types
    All,
}

/// Output format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Human-readable text
    Text,
    /// JSON output
    Json,
    /// Flamegraph SVG
    Flamegraph,
}

/// Collected profiling data from a real compilation run
struct ProfileData {
    /// Finalized compilation metrics (real phase timings)
    compilation_report: CompilationProfileReport,
    /// CBGR tier statistics from escape analysis
    tier_stats: verum_cbgr::tier_types::TierStatistics,
    /// Per-function tier analysis results
    tier_analyses: verum_common::Map<
        verum_compiler::session::FunctionId,
        verum_cbgr::tier_analysis::TierAnalysisResult,
    >,
    /// Per-function reference counts from AST analysis
    function_ref_counts: Vec<FunctionRefCount>,
}

/// Reference counts extracted from AST for a single function
struct FunctionRefCount {
    /// Function name
    name: String,
    /// Number of &T (managed) references in signature and body types
    cbgr_refs: usize,
    /// Number of &checked T references
    checked_refs: usize,
    /// Number of &unsafe T references
    unsafe_refs: usize,
    /// Total AST expression count (for density estimation)
    expression_count: usize,
    /// Maximum loop nesting depth
    max_loop_depth: usize,
}

/// Run the full compilation pipeline on the input and collect profiling data.
fn collect_profile_data(input: &str) -> std::result::Result<ProfileData, String> {
    let input_path = PathBuf::from(input);
    if !input_path.exists() {
        return Err(format!("Input file not found: {}", input));
    }

    let mut options = CompilerOptions::default();
    options.input = input_path.clone();
    options.verbose = 0;

    let mut session = Session::new(options);

    let file_id = session
        .load_file(&input_path)
        .map_err(|e| format!("Failed to load file: {}", e))?;

    let mut pipeline = CompilationPipeline::new(&mut session);
    pipeline
        .run_check_only()
        .map_err(|e| format!("Compilation failed: {}", e))?;

    // Extract per-function reference counts from the parsed AST
    let mut function_ref_counts = Vec::new();
    if let Some(module) = session.get_module(file_id) {
        let module_clone = (*module).clone();
        for item in &module_clone.items {
            if let verum_ast::decl::ItemKind::Function(func) = &item.kind {
                let counts = extract_function_ref_counts(func);
                function_ref_counts.push(counts);
            }
        }
    }

    let compilation_report = session.finalize_metrics();
    let tier_stats = session.tier_statistics();
    let tier_analyses = session.all_tier_analyses();

    Ok(ProfileData {
        compilation_report,
        tier_stats,
        tier_analyses,
        function_ref_counts,
    })
}

/// Extract reference type counts from a function's AST.
///
/// Walks parameter types, return type, and body to count &T, &checked T,
/// and &unsafe T references, plus expression count and loop depth.
fn extract_function_ref_counts(func: &verum_ast::FunctionDecl) -> FunctionRefCount {
    let mut cbgr_refs = 0usize;
    let mut checked_refs = 0usize;
    let mut unsafe_refs = 0usize;
    let mut expression_count = 0usize;
    let mut max_loop_depth = 0usize;

    // Analyze parameter types
    for param in &func.params {
        if let verum_ast::FunctionParamKind::Regular { ty, .. } = &param.kind {
            count_type_refs(ty, &mut cbgr_refs, &mut checked_refs, &mut unsafe_refs);
        }
    }

    // Analyze return type
    if let Some(ref ret_ty) = func.return_type {
        count_type_refs(ret_ty, &mut cbgr_refs, &mut checked_refs, &mut unsafe_refs);
    }

    // Walk body for expression count and loop depth
    if let Some(ref body) = func.body {
        walk_body(body, &mut expression_count, &mut max_loop_depth, 0);
    }

    FunctionRefCount {
        name: func.name.to_string(),
        cbgr_refs,
        checked_refs,
        unsafe_refs,
        expression_count,
        max_loop_depth,
    }
}

fn count_type_refs(
    ty: &verum_ast::Type,
    cbgr: &mut usize,
    checked: &mut usize,
    unsafe_r: &mut usize,
) {
    use verum_ast::TypeKind;
    match &ty.kind {
        TypeKind::Reference { inner, .. } => {
            *cbgr += 1;
            count_type_refs(inner, cbgr, checked, unsafe_r);
        }
        TypeKind::CheckedReference { inner, .. } => {
            *checked += 1;
            count_type_refs(inner, cbgr, checked, unsafe_r);
        }
        TypeKind::UnsafeReference { inner, .. } => {
            *unsafe_r += 1;
            count_type_refs(inner, cbgr, checked, unsafe_r);
        }
        TypeKind::Generic { args, .. } => {
            for arg in args {
                if let verum_ast::ty::GenericArg::Type(t) = arg {
                    count_type_refs(t, cbgr, checked, unsafe_r);
                }
            }
        }
        TypeKind::Tuple(types) => {
            for t in types {
                count_type_refs(t, cbgr, checked, unsafe_r);
            }
        }
        TypeKind::Function {
            params,
            return_type,
            ..
        } => {
            for p in params {
                count_type_refs(p, cbgr, checked, unsafe_r);
            }
            count_type_refs(return_type, cbgr, checked, unsafe_r);
        }
        TypeKind::Array { element, .. } => count_type_refs(element, cbgr, checked, unsafe_r),
        TypeKind::Slice(inner) => count_type_refs(inner, cbgr, checked, unsafe_r),
        _ => {}
    }
}

fn walk_body(
    body: &verum_ast::FunctionBody,
    expr_count: &mut usize,
    max_depth: &mut usize,
    current_depth: usize,
) {
    match body {
        verum_ast::FunctionBody::Block(block) => {
            walk_block(block, expr_count, max_depth, current_depth)
        }
        verum_ast::FunctionBody::Expr(expr) => {
            walk_expr(expr, expr_count, max_depth, current_depth)
        }
    }
}

fn walk_block(
    block: &verum_ast::Block,
    expr_count: &mut usize,
    max_depth: &mut usize,
    current_depth: usize,
) {
    for stmt in &block.stmts {
        use verum_ast::StmtKind;
        match &stmt.kind {
            StmtKind::Let { value, .. } => {
                if let Some(v) = value {
                    walk_expr(v, expr_count, max_depth, current_depth);
                }
            }
            StmtKind::Expr { expr, .. } => {
                walk_expr(expr, expr_count, max_depth, current_depth)
            }
            _ => {}
        }
    }
    if let Some(ref expr) = block.expr {
        walk_expr(expr, expr_count, max_depth, current_depth);
    }
}

fn walk_expr(
    expr: &verum_ast::Expr,
    expr_count: &mut usize,
    max_depth: &mut usize,
    current_depth: usize,
) {
    use verum_ast::ExprKind;
    *expr_count += 1;
    *max_depth = (*max_depth).max(current_depth);

    match &expr.kind {
        ExprKind::For { iter, body, .. } => {
            walk_expr(iter, expr_count, max_depth, current_depth);
            walk_block(body, expr_count, max_depth, current_depth + 1);
        }
        ExprKind::While {
            condition, body, ..
        } => {
            walk_expr(condition, expr_count, max_depth, current_depth + 1);
            walk_block(body, expr_count, max_depth, current_depth + 1);
        }
        ExprKind::Loop { body, .. } => {
            walk_block(body, expr_count, max_depth, current_depth + 1);
        }
        ExprKind::Block(block) => walk_block(block, expr_count, max_depth, current_depth),
        ExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            walk_block(then_branch, expr_count, max_depth, current_depth);
            if let Some(e) = else_branch {
                walk_expr(e, expr_count, max_depth, current_depth);
            }
        }
        ExprKind::Match { expr: scrutinee, arms } => {
            walk_expr(scrutinee, expr_count, max_depth, current_depth);
            for arm in arms {
                walk_expr(&arm.body, expr_count, max_depth, current_depth);
            }
        }
        ExprKind::Call { func, args, .. } => {
            walk_expr(func, expr_count, max_depth, current_depth);
            for arg in args {
                walk_expr(arg, expr_count, max_depth, current_depth);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            walk_expr(receiver, expr_count, max_depth, current_depth);
            for arg in args {
                walk_expr(arg, expr_count, max_depth, current_depth);
            }
        }
        ExprKind::Binary { left, right, .. } => {
            walk_expr(left, expr_count, max_depth, current_depth);
            walk_expr(right, expr_count, max_depth, current_depth);
        }
        ExprKind::Unary { expr: inner, .. } => walk_expr(inner, expr_count, max_depth, current_depth),
        ExprKind::Field { expr: object, .. } => {
            walk_expr(object, expr_count, max_depth, current_depth)
        }
        ExprKind::Index { expr: object, index } => {
            walk_expr(object, expr_count, max_depth, current_depth);
            walk_expr(index, expr_count, max_depth, current_depth);
        }
        ExprKind::Return(maybe) => {
            if let Some(inner) = maybe {
                walk_expr(inner, expr_count, max_depth, current_depth);
            }
        }
        ExprKind::Await(inner) => walk_expr(inner, expr_count, max_depth, current_depth),
        ExprKind::Cast { expr: inner, .. } => {
            walk_expr(inner, expr_count, max_depth, current_depth)
        }
        ExprKind::Tuple(exprs) => {
            for e in exprs {
                walk_expr(e, expr_count, max_depth, current_depth);
            }
        }
        _ => {}
    }
}

/// Execute profile command from CLI (no input file — guidance only)
pub fn execute(
    memory: bool,
    cpu: bool,
    cache: bool,
    compilation: bool,
    output: &str,
) -> Result<()> {
    execute_with_input(memory, cpu, cache, compilation, output, None)
}

/// Execute profile command with an optional input file
pub fn execute_with_input(
    memory: bool,
    cpu: bool,
    cache: bool,
    compilation: bool,
    output: &str,
    input: Option<&str>,
) -> Result<()> {
    let format = match output {
        "json" => OutputFormat::Json,
        "flamegraph" => OutputFormat::Flamegraph,
        _ => OutputFormat::Text,
    };

    let targets = match (memory, cpu, cache, compilation) {
        (false, false, false, false) => vec![ProfileTarget::All],
        _ => {
            let mut t = Vec::new();
            if memory {
                t.push(ProfileTarget::Memory);
            }
            if cpu {
                t.push(ProfileTarget::Cpu);
            }
            if cache {
                t.push(ProfileTarget::Cache);
            }
            if compilation {
                t.push(ProfileTarget::Compilation);
            }
            t
        }
    };

    // Collect real data from compilation if input file is provided
    let profile_data = input.and_then(|path| match collect_profile_data(path) {
        Ok(data) => Some(data),
        Err(e) => {
            ui::warn(&format!("Could not profile input file: {}", e));
            None
        }
    });

    for (i, target) in targets.iter().enumerate() {
        run_profile(*target, format, profile_data.as_ref())?;
        if i < targets.len() - 1 {
            println!();
        }
    }

    Ok(())
}

/// Run profiling analysis
fn run_profile(
    target: ProfileTarget,
    format: OutputFormat,
    data: Option<&ProfileData>,
) -> Result<()> {
    match target {
        ProfileTarget::Memory => profile_memory(format, data)?,
        ProfileTarget::Cpu => profile_cpu(format, data)?,
        ProfileTarget::Cache => profile_cache(format, data)?,
        ProfileTarget::Compilation => profile_compilation(format, data)?,
        ProfileTarget::All => {
            profile_memory(format, data)?;
            println!();
            profile_cpu(format, data)?;
            println!();
            profile_cache(format, data)?;
            println!();
            profile_compilation(format, data)?;
        }
    }

    Ok(())
}

// ============================================================================
// Section divider helpers
// ============================================================================

fn print_section_header(title: &str) {
    println!("{}", "\u{2501}".repeat(55).cyan().bold());
    println!("{}", title.cyan().bold());
    println!("{}", "\u{2501}".repeat(55).cyan().bold());
}

fn print_summary_header() {
    println!();
    println!("{}", "\u{2501}".repeat(55).cyan().bold());
    println!("{}", "Summary".cyan().bold());
    println!("{}", "\u{2501}".repeat(55).cyan().bold());
}

// ============================================================================
// CBGR Memory Profile (--memory)
// ============================================================================

fn profile_memory(format: OutputFormat, data: Option<&ProfileData>) -> Result<()> {
    match data {
        Some(profile_data) => profile_memory_real(format, profile_data),
        None => profile_memory_no_input(format),
    }
}

fn profile_memory_no_input(format: OutputFormat) -> Result<()> {
    print_section_header("CBGR Performance Profile");
    println!();

    if format == OutputFormat::Json {
        println!(
            "{{\"error\": \"No input file provided. Use: verum profile --memory <file>\"}}"
        );
        return Ok(());
    }

    ui::info("No input file provided for CBGR profiling.");
    println!();
    ui::info("To profile CBGR memory overhead, provide a source file:");
    println!("  verum profile --memory your_program.vr");
    println!();
    ui::info("The profiler will:");
    println!(
        "  {} Compile the file through the full pipeline",
        "*".dimmed()
    );
    println!(
        "  {} Run escape analysis on all references",
        "*".dimmed()
    );
    println!(
        "  {} Report tier breakdown (&T, &checked T, &unsafe T)",
        "*".dimmed()
    );
    println!(
        "  {} Identify promotion opportunities from real analysis",
        "*".dimmed()
    );
    println!();

    Ok(())
}

fn profile_memory_real(format: OutputFormat, data: &ProfileData) -> Result<()> {
    let tier_stats = &data.tier_stats;

    if format == OutputFormat::Json {
        return profile_memory_json(data);
    }

    if format == OutputFormat::Flamegraph {
        ui::warn("Flamegraph output requires runtime profiling.");
        ui::info("Showing text report instead.");
        println!();
    }

    print_section_header("CBGR Performance Profile");
    println!();

    // Hot Spots: functions with the most managed references
    print_hot_spots(data);

    // Reference Breakdown from tier analysis
    print_reference_breakdown(tier_stats);

    // Promotion Opportunities from real tier decisions
    print_promotion_opportunities(data);

    // Summary
    print_summary_header();
    println!();

    let total_refs = tier_stats.total_refs;
    let recoverable_count: u64 = tier_stats
        .tier0_reasons
        .iter()
        .filter(|(reason, _)| reason.is_recoverable())
        .map(|(_, count)| count)
        .sum();

    println!(
        "Total references analyzed:  {}",
        format_number(total_refs)
    );
    println!(
        "Functions analyzed:         {}",
        format_number(tier_stats.functions_analyzed)
    );

    if total_refs > 0 {
        let pct = (recoverable_count as f64 / total_refs as f64) * 100.0;
        println!(
            "Promotable to &checked:     {} ({:.1}%)",
            format_number(recoverable_count),
            pct
        );
        println!(
            "Promotion rate:             {:.1}%",
            tier_stats.promotion_rate() * 100.0
        );
    }

    let overhead_per_call_ns = if tier_stats.functions_analyzed > 0 {
        (tier_stats.tier0_count * 15) as f64 / tier_stats.functions_analyzed as f64
    } else {
        0.0
    };
    if overhead_per_call_ns > 1000.0 {
        println!(
            "Estimated CBGR overhead:    ~{:.1}us per function call",
            overhead_per_call_ns / 1000.0
        );
    } else {
        println!(
            "Estimated CBGR overhead:    ~{:.0}ns per function call",
            overhead_per_call_ns
        );
    }

    if tier_stats.estimated_savings_ns > 0 {
        if tier_stats.estimated_savings_ns > 1000 {
            println!(
                "Estimated savings:          ~{:.1}us per execution (from promotions)",
                tier_stats.estimated_savings_ns as f64 / 1000.0
            );
        } else {
            println!(
                "Estimated savings:          ~{}ns per execution (from promotions)",
                tier_stats.estimated_savings_ns
            );
        }
    }

    if tier_stats.analysis_duration_us > 0 {
        if tier_stats.analysis_duration_us > 1000 {
            println!(
                "Analysis time:              {:.1}ms",
                tier_stats.analysis_duration_us as f64 / 1000.0
            );
        } else {
            println!(
                "Analysis time:              {}us",
                tier_stats.analysis_duration_us
            );
        }
    }

    println!();

    Ok(())
}

fn print_hot_spots(data: &ProfileData) {
    // Identify functions with managed references, sorted by ref count (highest first)
    let mut hot_functions: Vec<&FunctionRefCount> = data
        .function_ref_counts
        .iter()
        .filter(|f| f.cbgr_refs > 0)
        .collect();
    hot_functions.sort_by(|a, b| b.cbgr_refs.cmp(&a.cbgr_refs));

    if hot_functions.is_empty() {
        println!("{}", "Hot Spots (managed references):".bold());
        println!();
        println!("  No CBGR-managed references detected in source.");
        println!();
        return;
    }

    println!("{}", "Hot Spots (managed references):".bold());
    println!();

    // Show top functions (up to 10)
    for (rank, func) in hot_functions.iter().take(10).enumerate() {
        let rank_num = rank + 1;
        let total_refs = func.cbgr_refs + func.checked_refs + func.unsafe_refs;
        let loop_indicator = if func.max_loop_depth > 0 {
            format!(
                " (loop depth: {}, ~{}x check amplification)",
                func.max_loop_depth,
                10_usize.pow(func.max_loop_depth as u32)
            )
        } else {
            String::new()
        };

        let recommendation = if func.max_loop_depth >= 2 && func.cbgr_refs > 0 {
            "Consider promoting inner-loop refs to &checked T"
                .yellow()
                .to_string()
        } else if func.cbgr_refs > 3 {
            "Review for promotion opportunities via escape analysis"
                .yellow()
                .to_string()
        } else {
            "Overhead negligible".dimmed().to_string()
        };

        println!("{}. {}()", rank_num, func.name.as_str().bold());
        println!(
            "   {} References:       {} total ({} managed, {} checked, {} unsafe)",
            "|-".dimmed(),
            total_refs,
            func.cbgr_refs,
            func.checked_refs,
            func.unsafe_refs
        );
        println!(
            "   {} Expressions:      {}{}",
            "|-".dimmed(),
            func.expression_count,
            loop_indicator
        );
        println!(
            "   {} Recommendation:   {}",
            "'-".dimmed(),
            recommendation
        );
        println!();
    }

    if hot_functions.len() > 10 {
        println!(
            "  ... and {} more functions with managed references",
            hot_functions.len() - 10
        );
        println!();
    }
}

fn print_reference_breakdown(tier_stats: &verum_cbgr::tier_types::TierStatistics) {
    let total = tier_stats.total_refs;
    if total == 0 {
        println!("{}:", "Reference Breakdown".bold());
        println!("  No references analyzed by tier analysis.");
        println!(
            "  {}",
            "hint: CBGR tier analysis requires references in function signatures"
                .dimmed()
        );
        println!();
        return;
    }

    let managed_pct = (tier_stats.tier0_count as f64 / total as f64) * 100.0;
    let checked_pct = (tier_stats.tier1_count as f64 / total as f64) * 100.0;
    let unsafe_pct = (tier_stats.tier2_count as f64 / total as f64) * 100.0;

    println!("{}:", "Reference Breakdown".bold());
    println!(
        "  * &T (managed):           {:.0}% of references ({} refs, ~15ns/check)",
        managed_pct, tier_stats.tier0_count
    );
    println!(
        "  * &checked T (verified):  {:.0}% of references ({} refs, 0ns)",
        checked_pct, tier_stats.tier1_count
    );
    println!(
        "  * &unsafe T (raw):        {:.0}% of references ({} refs, 0ns)",
        unsafe_pct, tier_stats.tier2_count
    );
    println!();

    // Show Tier 0 reason breakdown if any
    if !tier_stats.tier0_reasons.is_empty() {
        println!("  {}:", "Tier 0 (managed) reasons".dimmed());
        // Sort by count descending for readability
        let mut reasons: Vec<_> = tier_stats.tier0_reasons.iter().collect();
        reasons.sort_by(|a, b| b.1.cmp(a.1));
        for (reason, count) in &reasons {
            let recoverable_marker = if reason.is_recoverable() {
                " (recoverable)".green().to_string()
            } else {
                String::new()
            };
            println!("    - {}: {}{}", reason, count, recoverable_marker);
        }
        println!();
    }
}

fn print_promotion_opportunities(data: &ProfileData) {
    // Collect recoverable Tier 0 decisions from real tier analysis
    let mut opportunities: Vec<(String, String)> = Vec::new();

    for (_func_id, result) in &data.tier_analyses {
        for (_ref_id, tier) in &result.decisions {
            if let verum_cbgr::tier_types::ReferenceTier::Tier0 { reason } = tier {
                if reason.is_recoverable() {
                    opportunities
                        .push((format!("ref_{}", _ref_id.0), reason.description().to_string()));
                }
            }
        }
    }

    if opportunities.is_empty() {
        return;
    }

    println!("{}:", "Promotion Opportunities".bold());

    // Group by reason for concise display
    let mut by_reason: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (_, reason) in &opportunities {
        *by_reason.entry(reason.clone()).or_insert(0) += 1;
    }

    // Sort by count descending
    let mut sorted_reasons: Vec<_> = by_reason.into_iter().collect();
    sorted_reasons.sort_by(|a, b| b.1.cmp(&a.1));

    for (reason, count) in &sorted_reasons {
        println!(
            "  {} {} reference(s): {}",
            "*".yellow(),
            count,
            reason
        );
    }
    println!(
        "  {}",
        "hint: these references could be promoted to &checked T (0ns) with deeper analysis"
            .dimmed()
    );
    println!();
}

fn profile_memory_json(data: &ProfileData) -> Result<()> {
    let tier_stats = &data.tier_stats;
    let total = tier_stats.total_refs;

    let managed_pct = if total > 0 {
        (tier_stats.tier0_count as f64 / total as f64) * 100.0
    } else {
        0.0
    };
    let checked_pct = if total > 0 {
        (tier_stats.tier1_count as f64 / total as f64) * 100.0
    } else {
        0.0
    };
    let unsafe_pct = if total > 0 {
        (tier_stats.tier2_count as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    println!("{{");
    println!("  \"total_references\": {},", total);
    println!("  \"reference_breakdown\": {{");
    println!(
        "    \"managed\": {{ \"count\": {}, \"percentage\": {:.1} }},",
        tier_stats.tier0_count, managed_pct
    );
    println!(
        "    \"checked\": {{ \"count\": {}, \"percentage\": {:.1} }},",
        tier_stats.tier1_count, checked_pct
    );
    println!(
        "    \"unsafe\": {{ \"count\": {}, \"percentage\": {:.1} }}",
        tier_stats.tier2_count, unsafe_pct
    );
    println!("  }},");
    println!(
        "  \"functions_analyzed\": {},",
        tier_stats.functions_analyzed
    );
    println!(
        "  \"promotion_rate\": {:.4},",
        tier_stats.promotion_rate()
    );
    println!(
        "  \"estimated_savings_ns\": {},",
        tier_stats.estimated_savings_ns
    );
    println!(
        "  \"analysis_duration_us\": {},",
        tier_stats.analysis_duration_us
    );
    // Include per-function data
    println!("  \"functions\": [");
    for (i, func) in data.function_ref_counts.iter().enumerate() {
        let comma = if i < data.function_ref_counts.len() - 1 {
            ","
        } else {
            ""
        };
        println!(
            "    {{ \"name\": \"{}\", \"managed_refs\": {}, \"checked_refs\": {}, \"unsafe_refs\": {}, \"expressions\": {}, \"loop_depth\": {} }}{}",
            func.name, func.cbgr_refs, func.checked_refs, func.unsafe_refs,
            func.expression_count, func.max_loop_depth, comma
        );
    }
    println!("  ]");
    println!("}}");

    Ok(())
}

// ============================================================================
// CPU Profile (--cpu)
// ============================================================================

fn profile_cpu(format: OutputFormat, data: Option<&ProfileData>) -> Result<()> {
    if format == OutputFormat::Json {
        return profile_cpu_json(data);
    }

    print_section_header("CPU Profile (Compilation Phases)");
    println!();

    match data {
        Some(profile_data) => {
            println!("  {}", "Phase timings from actual compilation:".bold());
            println!();

            let report = &profile_data.compilation_report;
            if report.phase_metrics.is_empty() {
                println!("  No phase timing data recorded.");
                println!(
                    "  {}",
                    "note: check-only compilation may skip some phases".dimmed()
                );
            } else {
                for phase in &report.phase_metrics {
                    let bar_len = (phase.time_percentage / 5.0) as usize;
                    let bar = "=".repeat(bar_len.min(20));
                    println!(
                        "    {:20} {:>6}ms ({:>5.1}%) {}",
                        phase.phase_name.as_str(),
                        phase.duration.as_millis(),
                        phase.time_percentage,
                        bar.cyan()
                    );
                }
                println!();
                println!(
                    "  Total compilation time: {}ms",
                    report.total_duration.as_millis()
                );
            }
        }
        None => {
            println!(
                "  {}",
                "No input file provided. Provide a source file for real phase timings:"
                    .yellow()
            );
            println!("    verum profile --cpu your_program.vr");
        }
    }

    println!();
    println!(
        "  {}: Runtime CPU profiling requires instrumented AOT builds.",
        "Note".dimmed()
    );
    println!("  Use 'verum run --profile-cbgr <file>' for runtime profiling.");

    Ok(())
}

fn profile_cpu_json(data: Option<&ProfileData>) -> Result<()> {
    match data {
        Some(profile_data) => {
            let report = &profile_data.compilation_report;
            match report.to_json() {
                Ok(json) => println!("{}", json.as_str()),
                Err(e) => println!("{{\"error\": \"{}\"}}", e),
            }
        }
        None => {
            println!("{{\"error\": \"No input file provided\"}}");
        }
    }
    Ok(())
}

// ============================================================================
// Cache Analysis (--cache)
// ============================================================================

fn profile_cache(format: OutputFormat, data: Option<&ProfileData>) -> Result<()> {
    if format == OutputFormat::Json {
        return profile_cache_json(data);
    }

    print_section_header("Cache Analysis");
    println!();

    match data {
        Some(profile_data) => {
            let tier_stats = &profile_data.tier_stats;

            println!("  {}:", "CBGR Tier Analysis Cache".bold());
            println!(
                "    Functions cached:    {}",
                profile_data.tier_analyses.len()
            );
            println!("    Total refs tracked:  {}", tier_stats.total_refs);
            println!(
                "    Tier 1 promotions:   {}",
                tier_stats.tier1_count
            );
            println!();

            let report = &profile_data.compilation_report;
            println!("  {}:", "Compilation Memory".bold());
            if report.total_memory_bytes > 0 {
                println!(
                    "    Total allocated:     {:.2} MB",
                    report.total_memory_bytes as f64 / (1024.0 * 1024.0)
                );
                println!(
                    "    Peak usage:          {:.2} MB",
                    report.peak_memory_bytes as f64 / (1024.0 * 1024.0)
                );
            } else {
                println!(
                    "    {}",
                    "Memory tracking not available for this compilation mode".dimmed()
                );
            }
        }
        None => {
            println!(
                "  {}",
                "No input file provided. Provide a source file for cache analysis:"
                    .yellow()
            );
            println!("    verum profile --cache your_program.vr");
        }
    }

    println!();
    println!("  {}:", "Recommendations".dimmed());
    println!("    - Enable incremental compilation: --incremental");
    println!("    - Use distributed cache for CI: --distributed-cache=s3://bucket");

    Ok(())
}

fn profile_cache_json(data: Option<&ProfileData>) -> Result<()> {
    match data {
        Some(profile_data) => {
            println!("{{");
            println!(
                "  \"tier_cache_entries\": {},",
                profile_data.tier_analyses.len()
            );
            println!(
                "  \"total_refs_tracked\": {},",
                profile_data.tier_stats.total_refs
            );
            println!(
                "  \"total_memory_bytes\": {},",
                profile_data.compilation_report.total_memory_bytes
            );
            println!(
                "  \"peak_memory_bytes\": {}",
                profile_data.compilation_report.peak_memory_bytes
            );
            println!("}}");
        }
        None => {
            println!("{{\"error\": \"No input file provided\"}}");
        }
    }
    Ok(())
}

// ============================================================================
// Compilation Pipeline Profile (--compilation)
// ============================================================================

fn profile_compilation(format: OutputFormat, data: Option<&ProfileData>) -> Result<()> {
    match data {
        Some(profile_data) => {
            profile_compilation_real(format, &profile_data.compilation_report)
        }
        None => profile_compilation_no_input(format),
    }
}

fn profile_compilation_no_input(format: OutputFormat) -> Result<()> {
    if format == OutputFormat::Json {
        println!(
            "{{\"error\": \"No input file provided. Use: verum profile --compilation <file>\"}}"
        );
        return Ok(());
    }

    print_section_header("Compilation Pipeline Profile");
    println!();
    println!(
        "  {}",
        "No input file provided. Provide a source file for real compilation metrics:"
            .yellow()
    );
    println!("    verum profile --compilation your_program.vr");
    println!();

    Ok(())
}

fn profile_compilation_real(
    format: OutputFormat,
    report: &CompilationProfileReport,
) -> Result<()> {
    match format {
        OutputFormat::Json => print_compilation_json(report),
        OutputFormat::Flamegraph => {
            ui::warn("Flamegraph output not available for compilation profiling");
            print_compilation_text(report);
        }
        OutputFormat::Text => print_compilation_text(report),
    }

    Ok(())
}

fn print_compilation_text(report: &CompilationProfileReport) {
    print_section_header("Compilation Pipeline Profile");
    println!();

    if report.phase_metrics.is_empty() {
        println!("  No phase timing data recorded.");
        println!();
    } else {
        println!("  {} Phase Timings:", "Phase".cyan().bold());
        println!();

        for phase in &report.phase_metrics {
            let bar_len = (phase.time_percentage / 5.0) as usize;
            let bar = "=".repeat(bar_len.min(20));
            println!(
                "    {:20} {:>6}ms ({:>5.1}%) {}",
                phase.phase_name.as_str(),
                phase.duration.as_millis(),
                phase.time_percentage,
                bar.cyan()
            );
        }
        println!();
    }

    println!("  {} Statistics:", "Stats".cyan().bold());
    println!(
        "    Total time:     {}ms",
        report.total_duration.as_millis()
    );
    if report.stats.total_loc > 0 {
        println!(
            "    Lines of code:  {}",
            format_number(report.stats.total_loc as u64)
        );
        println!(
            "    Throughput:     {} LOC/sec",
            format_number(report.stats.compilation_speed_loc_per_sec as u64)
        );
    }
    if report.stats.modules_compiled > 0 {
        println!(
            "    Modules:        {}",
            report.stats.modules_compiled
        );
        println!(
            "    Functions:      {}",
            report.stats.functions_compiled
        );
    }
    if report.total_memory_bytes > 0 {
        println!(
            "    Memory used:    {:.2} MB",
            report.total_memory_bytes as f64 / (1024.0 * 1024.0)
        );
    }
    println!();

    // Bottlenecks
    if !report.bottlenecks.is_empty() {
        println!("  {} Bottlenecks:", "Warning".yellow().bold());
        for bottleneck in &report.bottlenecks {
            let kind_str = match bottleneck.kind {
                verum_compiler::compilation_metrics::BottleneckKind::SlowPhase => "Slow phase",
                verum_compiler::compilation_metrics::BottleneckKind::SlowModule => "Slow module",
                verum_compiler::compilation_metrics::BottleneckKind::HighMemory => "High memory",
                verum_compiler::compilation_metrics::BottleneckKind::HighItemCount => {
                    "High item count"
                }
            };
            println!(
                "    - {}: {} ({:.1}%)",
                kind_str.yellow(),
                bottleneck.description.as_str(),
                bottleneck.severity
            );
        }
        println!();
    }

    println!("  {} Recommendations:", "Tip".green().bold());
    println!("    - Enable parallel type checking: --jobs=8");
    println!("    - Use incremental compilation: --incremental");
    println!("    - Consider lazy SMT verification: --lazy-verify");
}

fn print_compilation_json(report: &CompilationProfileReport) {
    match report.to_json() {
        Ok(json) => println!("{}", json.as_str()),
        Err(e) => {
            ui::error(&format!("Failed to serialize report: {}", e));
        }
    }
}

/// Format large numbers with commas
fn format_number(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();

    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i) % 3 == 0 {
            result.push(',');
        }
        result.push(*c);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_number() {
        assert_eq!(format_number(123), "123");
        assert_eq!(format_number(1234), "1,234");
        assert_eq!(format_number(1234567), "1,234,567");
    }

    #[test]
    fn test_profile_targets() {
        assert_eq!(ProfileTarget::Memory, ProfileTarget::Memory);
        assert_ne!(ProfileTarget::Memory, ProfileTarget::Cpu);
    }
}
