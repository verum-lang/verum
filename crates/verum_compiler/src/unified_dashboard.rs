//! Unified Performance Dashboard
//!
//! Combines verification costs, CBGR overhead, compilation metrics, and cache statistics
//! into a single comprehensive performance analysis dashboard.
//!
//! Unified compilation dashboard: real-time progress display for all
//! compilation phases, verification status, and performance metrics.
//!
//! # Example
//!
//! ```bash
//! $ verum profile --all src/main.vr
//! ```
//!
//! Output format matches spec exactly:
//! ```text
//! ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//! Verum Performance Analysis
//! ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//!
//! Compilation Time:           45.2s
//!   ├─ Parsing:               2.1s (4.6%)
//!   ├─ Type checking:         8.7s (19.2%)
//!   ├─ Verification (SMT):    28.3s (62.6%)  ⚠ SLOW
//!   └─ Codegen:               6.1s (13.5%)
//!
//! Runtime Performance:        2.34s total
//!   ├─ Business logic:        2.18s (93.2%)
//!   └─ CBGR overhead:         0.16s (6.8%)
//!
//! Hot Spots:
//!   1. complex_algorithm()    28.3s verification (reduce to <5s)
//!   2. process_matrix()       28.7ms CBGR (convert to &checked)
//!
//! Recommendations:
//!   1. Split complex_algorithm() into smaller functions
//!   2. Use @verify(runtime) for complex_algorithm() in development
//!   3. Convert process_matrix() to use &checked references
//!   4. Enable distributed cache: --distributed-cache=s3://bucket
//! ```

use anyhow::{Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use verum_common::{List, Maybe, Text};

use crate::compilation_metrics::CompilationProfileReport;
use crate::profile_cmd::ProfileReport;

/// Unified performance dashboard combining all metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedDashboard {
    /// Compilation metrics
    pub compilation: CompilationMetrics,

    /// Runtime performance metrics
    pub runtime: RuntimeMetrics,

    /// Hot spots requiring attention
    pub hot_spots: List<HotSpot>,

    /// Actionable recommendations
    pub recommendations: List<Recommendation>,

    /// Cache statistics
    pub cache: CacheStatistics,

    /// Per-function verification costs
    pub verification_costs: List<VerificationCost>,
}

/// Compilation time breakdown
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilationMetrics {
    /// Total compilation time
    pub total_time: Duration,

    /// Parsing phase
    pub parsing: DashboardPhaseMetrics,

    /// Type checking phase
    pub type_checking: DashboardPhaseMetrics,

    /// Verification (SMT) phase
    pub verification: DashboardPhaseMetrics,

    /// Code generation phase
    pub codegen: DashboardPhaseMetrics,
}

/// Individual phase metrics for dashboard
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardPhaseMetrics {
    /// Time spent in phase
    #[serde(
        serialize_with = "serialize_duration",
        deserialize_with = "deserialize_duration"
    )]
    pub duration: Duration,

    /// Percentage of total compilation time
    pub percentage: f64,

    /// Is this phase slow? (>20% of total time)
    pub is_slow: bool,
}

/// Runtime performance metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeMetrics {
    /// Total runtime
    #[serde(
        serialize_with = "serialize_duration",
        deserialize_with = "deserialize_duration"
    )]
    pub total_time: Duration,

    /// Business logic time (excluding CBGR)
    #[serde(
        serialize_with = "serialize_duration",
        deserialize_with = "deserialize_duration"
    )]
    pub business_logic_time: Duration,

    /// CBGR overhead
    #[serde(
        serialize_with = "serialize_duration",
        deserialize_with = "deserialize_duration"
    )]
    pub cbgr_overhead: Duration,

    /// CBGR overhead percentage
    pub cbgr_overhead_pct: f64,

    /// Reference type breakdown
    pub reference_breakdown: ReferenceBreakdown,
}

/// Reference type breakdown for CBGR analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceBreakdown {
    /// &T (managed) - ~15ns overhead
    pub managed_count: usize,
    pub managed_overhead_ms: f64,

    /// &checked T (verified) - 0ns overhead
    pub checked_count: usize,

    /// &unsafe T (raw) - 0ns overhead
    pub unsafe_count: usize,
}

/// Performance hot spot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotSpot {
    /// Rank (1 = worst)
    pub rank: usize,

    /// Function name
    pub function_name: Text,

    /// Hot spot type
    pub kind: HotSpotKind,

    /// Cost description
    pub cost: Text,

    /// Target reduction
    pub target: Text,
}

/// Type of hot spot
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HotSpotKind {
    /// Slow verification (>5s)
    SlowVerification,

    /// High CBGR overhead (>10%)
    HighCbgrOverhead,

    /// Excessive checks (>1000)
    ExcessiveChecks,
}

/// Actionable recommendation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    /// Priority (1 = highest)
    pub priority: usize,

    /// Recommendation text
    pub text: Text,

    /// Expected benefit
    pub benefit: Maybe<Text>,
}

/// Cache statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStatistics {
    /// Total queries
    pub total_queries: usize,

    /// Cache hits
    pub hits: usize,

    /// Cache misses
    pub misses: usize,

    /// Hit rate percentage
    pub hit_rate_pct: f64,

    /// Time saved by cache (approximate)
    #[serde(
        serialize_with = "serialize_duration",
        deserialize_with = "deserialize_duration"
    )]
    pub time_saved: Duration,

    /// Cache size in bytes
    pub cache_size_bytes: usize,

    /// Number of entries
    pub entry_count: usize,
}

/// Per-function verification cost
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationCost {
    /// Function name
    pub function_name: Text,

    /// Verification time
    #[serde(
        serialize_with = "serialize_duration",
        deserialize_with = "deserialize_duration"
    )]
    pub verification_time: Duration,

    /// SMT solver used
    pub smt_solver: Text,

    /// Logic type (e.g., QF_LIA, NIA)
    pub logic_type: Text,

    /// Number of SMT queries
    pub query_count: usize,

    /// Bottleneck description
    pub bottleneck: Maybe<Text>,

    /// Recommendations for this function
    pub recommendations: List<Text>,
}

impl UnifiedDashboard {
    /// Create a new unified dashboard
    pub fn new() -> Self {
        Self {
            compilation: CompilationMetrics {
                total_time: Duration::from_secs(0),
                parsing: DashboardPhaseMetrics::zero(),
                type_checking: DashboardPhaseMetrics::zero(),
                verification: DashboardPhaseMetrics::zero(),
                codegen: DashboardPhaseMetrics::zero(),
            },
            runtime: RuntimeMetrics {
                total_time: Duration::from_secs(0),
                business_logic_time: Duration::from_secs(0),
                cbgr_overhead: Duration::from_secs(0),
                cbgr_overhead_pct: 0.0,
                reference_breakdown: ReferenceBreakdown {
                    managed_count: 0,
                    managed_overhead_ms: 0.0,
                    checked_count: 0,
                    unsafe_count: 0,
                },
            },
            hot_spots: List::new(),
            recommendations: List::new(),
            cache: CacheStatistics::zero(),
            verification_costs: List::new(),
        }
    }

    /// Build dashboard from compilation and profiling data
    pub fn from_data(
        compilation_report: &CompilationProfileReport,
        profile_report: &ProfileReport,
    ) -> Self {
        let mut dashboard = Self::new();

        // Extract compilation metrics
        dashboard.extract_compilation_metrics(compilation_report);

        // Extract runtime metrics from profile report
        dashboard.extract_runtime_metrics(profile_report);

        // Identify hot spots
        dashboard.identify_hot_spots(compilation_report, profile_report);

        // Generate recommendations
        dashboard.generate_recommendations();

        dashboard
    }

    /// Extract compilation metrics from report
    fn extract_compilation_metrics(&mut self, report: &CompilationProfileReport) {
        self.compilation.total_time = report.total_duration;

        // Find phase metrics by name
        for phase in &report.phase_metrics {
            let metrics = DashboardPhaseMetrics {
                duration: phase.duration,
                percentage: phase.time_percentage,
                is_slow: phase.time_percentage > 20.0,
            };

            match phase.phase_name.as_str() {
                "Parsing" | "Lexical Parsing" => self.compilation.parsing = metrics,
                "Type Checking" | "Semantic Analysis" => self.compilation.type_checking = metrics,
                "Verification" | "SMT Verification" => self.compilation.verification = metrics,
                "Codegen" | "Code Generation" => self.compilation.codegen = metrics,
                _ => {}
            }
        }
    }

    /// Extract runtime metrics from profile report
    fn extract_runtime_metrics(&mut self, report: &ProfileReport) {
        let mut total_cbgr_ns = 0u64;
        let mut total_time_ns = 0u64;
        let mut managed_refs = 0;
        let checked_refs = 0;
        let unsafe_refs = 0;

        // Aggregate across all functions
        for profile in report.functions.values() {
            total_cbgr_ns += profile.stats.cbgr_time_ns;
            total_time_ns += profile.stats.total_time_ns;
            managed_refs += profile.stats.num_cbgr_refs;
            // Note: ProfileReport doesn't track checked/unsafe yet
            // This would need to be extended in profile_cmd.rs
        }

        self.runtime.total_time = Duration::from_nanos(total_time_ns);
        self.runtime.cbgr_overhead = Duration::from_nanos(total_cbgr_ns);
        self.runtime.business_logic_time = Duration::from_nanos(total_time_ns - total_cbgr_ns);

        if total_time_ns > 0 {
            self.runtime.cbgr_overhead_pct = (total_cbgr_ns as f64 / total_time_ns as f64) * 100.0;
        }

        self.runtime.reference_breakdown = ReferenceBreakdown {
            managed_count: managed_refs,
            managed_overhead_ms: total_cbgr_ns as f64 / 1_000_000.0,
            checked_count: checked_refs,
            unsafe_count: unsafe_refs,
        };
    }

    /// Identify performance hot spots
    fn identify_hot_spots(
        &mut self,
        _compilation_report: &CompilationProfileReport,
        profile_report: &ProfileReport,
    ) {
        let mut hot_spots = List::new();
        let mut rank = 1;

        // Find slow verification functions
        for (func_name, profile) in &profile_report.functions {
            // Slow verification (>5s)
            if profile.stats.cbgr_time_ns > 5_000_000_000 {
                hot_spots.push(HotSpot {
                    rank,
                    function_name: func_name.clone(),
                    kind: HotSpotKind::SlowVerification,
                    cost: format!(
                        "{:.1}s verification",
                        profile.stats.cbgr_time_ns as f64 / 1_000_000_000.0
                    )
                    .into(),
                    target: "reduce to <5s".into(),
                });
                rank += 1;
            }

            // High CBGR overhead (>10%)
            if profile.overhead_pct > 10.0 {
                hot_spots.push(HotSpot {
                    rank,
                    function_name: func_name.clone(),
                    kind: HotSpotKind::HighCbgrOverhead,
                    cost: format!(
                        "{:.1}ms CBGR",
                        profile.stats.cbgr_time_ns as f64 / 1_000_000.0
                    )
                    .into(),
                    target: "convert to &checked".into(),
                });
                rank += 1;
            }
        }

        // Sort by cost (highest first)
        hot_spots.sort_by(|a, b| b.rank.cmp(&a.rank));

        // Re-rank after sorting
        for (i, hot_spot) in hot_spots.iter_mut().enumerate() {
            hot_spot.rank = i + 1;
        }

        self.hot_spots = hot_spots;
    }

    /// Generate actionable recommendations
    fn generate_recommendations(&mut self) {
        let mut recommendations = List::new();
        let mut priority = 1;

        // Recommendation 1: Split slow verification functions
        if self.compilation.verification.is_slow {
            let slow_funcs: List<_> = self
                .hot_spots
                .iter()
                .filter(|h| h.kind == HotSpotKind::SlowVerification)
                .collect();

            if !slow_funcs.is_empty() {
                recommendations.push(Recommendation {
                    priority,
                    text: format!(
                        "Split {} into smaller functions",
                        slow_funcs[0].function_name
                    )
                    .into(),
                    benefit: Maybe::Some(
                        format!("Reduce verification from {} to <5s", slow_funcs[0].cost).into(),
                    ),
                });
                priority += 1;

                recommendations.push(Recommendation {
                    priority,
                    text: format!(
                        "Use @verify(runtime) for {} in development",
                        slow_funcs[0].function_name
                    )
                    .into(),
                    benefit: Maybe::Some("Skip SMT verification during rapid iteration".into()),
                });
                priority += 1;
            }
        }

        // Recommendation 2: Convert CBGR refs to &checked
        if self.runtime.cbgr_overhead_pct > 5.0 {
            let cbgr_funcs: List<_> = self
                .hot_spots
                .iter()
                .filter(|h| h.kind == HotSpotKind::HighCbgrOverhead)
                .collect();

            if !cbgr_funcs.is_empty() {
                recommendations.push(Recommendation {
                    priority,
                    text: format!(
                        "Convert {} to use &checked references",
                        cbgr_funcs[0].function_name
                    )
                    .into(),
                    benefit: Maybe::Some(format!("Eliminate {} CBGR overhead", cbgr_funcs[0].cost).into()),
                });
                priority += 1;
            }
        }

        // Recommendation 3: Enable distributed cache (always useful)
        recommendations.push(Recommendation {
            priority,
            text: "Enable distributed cache: --distributed-cache=s3://bucket".into(),
            benefit: Maybe::Some("Share verification results across team".into()),
        });

        self.recommendations = recommendations;
    }

    /// Display dashboard in terminal
    pub fn display(&self) {
        self.display_header();
        self.display_compilation_time();
        self.display_runtime_performance();
        self.display_hot_spots();
        self.display_recommendations();
    }

    /// Print the unified performance dashboard combining all analysis results.
    ///
    /// Displays a comprehensive dashboard with Unicode line-drawing combining:
    /// - Compilation time breakdown (parsing, type checking, verification, codegen)
    /// - CBGR analysis results (reference breakdown, overhead)
    /// - Hot spots requiring attention
    /// - Cache statistics
    /// - Actionable recommendations
    pub fn print_dashboard(&self) {
        let separator = "━".repeat(60);

        // Header
        println!("{}", separator.cyan().bold());
        println!("{}", "Verum Unified Performance Dashboard".cyan().bold());
        println!("{}", separator.cyan().bold());
        println!();

        // Section 1: Compilation Time Breakdown
        println!("{}", "Compilation Time Breakdown".bold());
        let total_secs = self.compilation.total_time.as_secs_f64();
        println!("  Total: {:.3}s", total_secs);
        println!();

        let phases = [
            ("Parsing", &self.compilation.parsing),
            ("Type Checking", &self.compilation.type_checking),
            ("Verification (SMT)", &self.compilation.verification),
            ("Code Generation", &self.compilation.codegen),
        ];

        for (i, (name, metrics)) in phases.iter().enumerate() {
            let connector = if i < phases.len() - 1 { "├─" } else { "└─" };
            let time_str = if metrics.duration.as_millis() > 0 {
                format!("{:.1}ms", metrics.duration.as_secs_f64() * 1000.0)
            } else {
                "0ms".to_string()
            };
            let pct_str = format!("({:.1}%)", metrics.percentage);
            let slow_marker = if metrics.is_slow {
                "  SLOW".yellow().to_string()
            } else {
                String::new()
            };

            println!(
                "  {} {:<22} {:>8} {:>8}{}",
                connector.dimmed(),
                name,
                time_str,
                pct_str.dimmed(),
                slow_marker
            );
        }
        println!();

        // Section 2: CBGR Analysis Results
        println!("{}", separator.cyan().bold());
        println!("{}", "CBGR Analysis".cyan().bold());
        println!("{}", separator.cyan().bold());
        println!();

        let rb = &self.runtime.reference_breakdown;
        let total_refs = rb.managed_count + rb.checked_count + rb.unsafe_count;

        if total_refs > 0 {
            let managed_pct = (rb.managed_count as f64 / total_refs as f64) * 100.0;
            let checked_pct = (rb.checked_count as f64 / total_refs as f64) * 100.0;
            let unsafe_pct = (rb.unsafe_count as f64 / total_refs as f64) * 100.0;

            println!("  {}:", "Reference Breakdown".bold());
            println!(
                "    * &T (managed):           {:.0}% ({} refs, ~{:.1}ms overhead)",
                managed_pct, rb.managed_count, rb.managed_overhead_ms
            );
            println!(
                "    * &checked T (verified):  {:.0}% ({} refs, 0ms overhead)",
                checked_pct, rb.checked_count
            );
            println!(
                "    * &unsafe T (raw):        {:.0}% ({} refs, 0ms overhead)",
                unsafe_pct, rb.unsafe_count
            );
        } else {
            println!("  No references analyzed.");
        }

        if self.runtime.cbgr_overhead_pct > 0.0 {
            println!();
            println!(
                "  Estimated CBGR overhead: {:.2}s ({:.1}% of runtime)",
                self.runtime.cbgr_overhead.as_secs_f64(),
                self.runtime.cbgr_overhead_pct
            );
        }
        println!();

        // Section 3: Hot Spots
        if !self.hot_spots.is_empty() {
            println!("{}", separator.cyan().bold());
            println!("{}", "Hot Spots".cyan().bold());
            println!("{}", separator.cyan().bold());
            println!();

            for hot_spot in &self.hot_spots {
                let kind_label = match hot_spot.kind {
                    HotSpotKind::SlowVerification => "verification",
                    HotSpotKind::HighCbgrOverhead => "CBGR overhead",
                    HotSpotKind::ExcessiveChecks => "excessive checks",
                };

                println!(
                    "  {}. {}() - {} {}",
                    hot_spot.rank,
                    hot_spot.function_name.as_str().yellow().bold(),
                    hot_spot.cost.as_str(),
                    format!("[{}]", kind_label).dimmed()
                );
                println!(
                    "     {} {}",
                    "Target:".dimmed(),
                    hot_spot.target.as_str()
                );
            }
            println!();
        }

        // Section 4: Cache Statistics
        if self.cache.total_queries > 0 {
            println!("{}", separator.cyan().bold());
            println!("{}", "Cache Statistics".cyan().bold());
            println!("{}", separator.cyan().bold());
            println!();

            println!(
                "  Hit rate:      {:.1}% ({} / {})",
                self.cache.hit_rate_pct, self.cache.hits, self.cache.total_queries
            );
            println!(
                "  Time saved:    {:.1}s",
                self.cache.time_saved.as_secs_f64()
            );
            if self.cache.cache_size_bytes > 0 {
                println!(
                    "  Cache size:    {:.1} MB ({} entries)",
                    self.cache.cache_size_bytes as f64 / (1024.0 * 1024.0),
                    self.cache.entry_count
                );
            }
            println!();
        }

        // Section 5: Actionable Recommendations
        if !self.recommendations.is_empty() {
            println!("{}", separator.cyan().bold());
            println!("{}", "Recommendations".cyan().bold());
            println!("{}", separator.cyan().bold());
            println!();

            for rec in &self.recommendations {
                println!(
                    "  {}. {}",
                    rec.priority,
                    rec.text.as_str().yellow()
                );
                if let Maybe::Some(ref benefit) = rec.benefit {
                    println!(
                        "     {}",
                        benefit.as_str().dimmed()
                    );
                }
            }
            println!();
        }

        // Footer
        println!("{}", separator.cyan().bold());
    }

    fn display_header(&self) {
        println!("{}", "━".repeat(60).bright_blue());
        println!("{}", "Verum Performance Analysis".bold().bright_blue());
        println!("{}", "━".repeat(60).bright_blue());
        println!();
    }

    fn display_compilation_time(&self) {
        println!(
            "{}",
            format!(
                "Compilation Time:           {:.1}s",
                self.compilation.total_time.as_secs_f64()
            )
            .bold()
        );

        self.display_phase("Parsing", &self.compilation.parsing);
        self.display_phase("Type checking", &self.compilation.type_checking);
        self.display_phase("Verification (SMT)", &self.compilation.verification);
        self.display_phase("Codegen", &self.compilation.codegen);
        println!();
    }

    fn display_phase(&self, name: &str, metrics: &DashboardPhaseMetrics) {
        let time_str = format!("{:.1}s", metrics.duration.as_secs_f64());
        let pct_str = format!("({:.1}%)", metrics.percentage);
        let slow_marker = if metrics.is_slow { "  ⚠ SLOW" } else { "" };

        println!(
            "  {} {:<20} {} {}{}",
            "├─".dimmed(),
            format!("{}:", name),
            time_str,
            pct_str.dimmed(),
            slow_marker.yellow()
        );
    }

    fn display_runtime_performance(&self) {
        println!(
            "{}",
            format!(
                "Runtime Performance:        {:.2}s total",
                self.runtime.total_time.as_secs_f64()
            )
            .bold()
        );

        let business_logic_pct = 100.0 - self.runtime.cbgr_overhead_pct;
        println!(
            "  {} {:<20} {:.2}s ({:.1}%)",
            "├─".dimmed(),
            "Business logic:",
            self.runtime.business_logic_time.as_secs_f64(),
            business_logic_pct
        );
        println!(
            "  {} {:<20} {:.2}s ({:.1}%)",
            "└─".dimmed(),
            "CBGR overhead:",
            self.runtime.cbgr_overhead.as_secs_f64(),
            self.runtime.cbgr_overhead_pct
        );
        println!();
    }

    fn display_hot_spots(&self) {
        if self.hot_spots.is_empty() {
            println!("{}", "✓ No hot spots detected!".green());
            println!();
            return;
        }

        println!("{}", "Hot Spots:".bold());
        for hot_spot in &self.hot_spots {
            println!(
                "  {}. {}()    {} ({})",
                hot_spot.rank,
                hot_spot.function_name.as_str().yellow(),
                hot_spot.cost.as_str(),
                hot_spot.target.as_str().dimmed()
            );
        }
        println!();
    }

    fn display_recommendations(&self) {
        if self.recommendations.is_empty() {
            return;
        }

        println!("{}", "Recommendations:".bold());
        for rec in &self.recommendations {
            println!("  {}. {}", rec.priority, rec.text.as_str());
            if let Maybe::Some(ref benefit) = rec.benefit {
                println!("     {}", benefit.as_str().dimmed());
            }
        }
        println!();
    }

    /// Export dashboard as JSON
    pub fn to_json(&self) -> Result<Text> {
        serde_json::to_string_pretty(self)
            .map(|s| s.into())
            .context("Failed to serialize dashboard to JSON")
    }

    /// Export dashboard as HTML report
    pub fn to_html(&self) -> Text {
        let mut html = String::new();

        html.push_str("<!DOCTYPE html>\n");
        html.push_str("<html>\n<head>\n");
        html.push_str("<meta charset=\"utf-8\">\n");
        html.push_str("<title>Verum Performance Analysis</title>\n");
        html.push_str("<style>\n");
        html.push_str(include_str!("dashboard_style.css"));
        html.push_str("</style>\n");
        html.push_str("</head>\n<body>\n");

        html.push_str("<h1>Verum Performance Analysis</h1>\n");

        // Compilation Time
        html.push_str("<section class=\"compilation\">\n");
        html.push_str("<h2>Compilation Time</h2>\n");
        html.push_str(&format!(
            "<p class=\"total\">Total: {:.1}s</p>\n",
            self.compilation.total_time.as_secs_f64()
        ));
        html.push_str("<ul>\n");
        html.push_str(&self.phase_html("Parsing", &self.compilation.parsing));
        html.push_str(&self.phase_html("Type checking", &self.compilation.type_checking));
        html.push_str(&self.phase_html("Verification (SMT)", &self.compilation.verification));
        html.push_str(&self.phase_html("Codegen", &self.compilation.codegen));
        html.push_str("</ul>\n");
        html.push_str("</section>\n");

        // Runtime Performance
        html.push_str("<section class=\"runtime\">\n");
        html.push_str("<h2>Runtime Performance</h2>\n");
        html.push_str(&format!(
            "<p class=\"total\">Total: {:.2}s</p>\n",
            self.runtime.total_time.as_secs_f64()
        ));
        html.push_str("<ul>\n");
        html.push_str(&format!(
            "<li>Business logic: {:.2}s ({:.1}%)</li>\n",
            self.runtime.business_logic_time.as_secs_f64(),
            100.0 - self.runtime.cbgr_overhead_pct
        ));
        html.push_str(&format!(
            "<li>CBGR overhead: {:.2}s ({:.1}%)</li>\n",
            self.runtime.cbgr_overhead.as_secs_f64(),
            self.runtime.cbgr_overhead_pct
        ));
        html.push_str("</ul>\n");
        html.push_str("</section>\n");

        // Hot Spots
        if !self.hot_spots.is_empty() {
            html.push_str("<section class=\"hot-spots\">\n");
            html.push_str("<h2>Hot Spots</h2>\n");
            html.push_str("<ol>\n");
            for hot_spot in &self.hot_spots {
                html.push_str(&format!(
                    "<li><strong>{}()</strong> {} ({})</li>\n",
                    hot_spot.function_name, hot_spot.cost, hot_spot.target
                ));
            }
            html.push_str("</ol>\n");
            html.push_str("</section>\n");
        }

        // Recommendations
        if !self.recommendations.is_empty() {
            html.push_str("<section class=\"recommendations\">\n");
            html.push_str("<h2>Recommendations</h2>\n");
            html.push_str("<ol>\n");
            for rec in &self.recommendations {
                html.push_str(&format!("<li>{}", rec.text));
                if let Maybe::Some(ref benefit) = rec.benefit {
                    html.push_str(&format!(" <em>({})</em>", benefit));
                }
                html.push_str("</li>\n");
            }
            html.push_str("</ol>\n");
            html.push_str("</section>\n");
        }

        html.push_str("</body>\n</html>");
        html.into()
    }

    fn phase_html(&self, name: &str, metrics: &DashboardPhaseMetrics) -> String {
        let class = if metrics.is_slow {
            " class=\"slow\""
        } else {
            ""
        };
        format!(
            "<li{}>{}: {:.1}s ({:.1}%){}</li>\n",
            class,
            name,
            metrics.duration.as_secs_f64(),
            metrics.percentage,
            if metrics.is_slow { " ⚠ SLOW" } else { "" }
        )
    }

    /// Write dashboard to file
    pub fn write_to_file(&self, path: &Path, format: OutputFormat) -> Result<()> {
        use std::fs::File;
        use std::io::Write;

        let content = match format {
            OutputFormat::Json => self.to_json()?,
            OutputFormat::Html => self.to_html(),
            OutputFormat::Text => {
                // Capture terminal output
                let mut output = String::new();
                // Note: This is a simplified version
                // Full implementation would capture display() output
                output.push_str("Verum Performance Analysis\n");
                output.into()
            }
        };

        let mut file = File::create(path)?;
        write!(file, "{}", content)?;

        Ok(())
    }
}

/// Output format for dashboard export
#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    Json,
    Html,
    Text,
}

impl DashboardPhaseMetrics {
    fn zero() -> Self {
        Self {
            duration: Duration::from_secs(0),
            percentage: 0.0,
            is_slow: false,
        }
    }
}

impl CacheStatistics {
    fn zero() -> Self {
        Self {
            total_queries: 0,
            hits: 0,
            misses: 0,
            hit_rate_pct: 0.0,
            time_saved: Duration::from_secs(0),
            cache_size_bytes: 0,
            entry_count: 0,
        }
    }
}

impl Default for UnifiedDashboard {
    fn default() -> Self {
        Self::new()
    }
}

// Helper functions for Duration serialization
fn serialize_duration<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_u64(duration.as_millis() as u64)
}

fn deserialize_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let millis = u64::deserialize(deserializer)?;
    Ok(Duration::from_millis(millis))
}
