//! Example: Unified Performance Dashboard
//!
//! Demonstrates creating and using the unified performance dashboard.
//!
//! Run with:
//! ```bash
//! cargo run --example unified_dashboard_example
//! ```

use std::time::Duration;
use verum_compiler::compilation_metrics::{CompilationProfileReport, PhasePerformanceMetrics};
use verum_compiler::profile_cmd::{CbgrStats, FunctionProfile, ProfileReport};
use verum_compiler::unified_dashboard::UnifiedDashboard;
use verum_common::Map;

fn main() {
    println!("=== Unified Performance Dashboard Example ===\n");

    // Create sample compilation metrics
    let compilation_report = create_sample_compilation_report();

    // Create sample profile report
    let profile_report = create_sample_profile_report();

    // Build unified dashboard
    let dashboard = UnifiedDashboard::from_data(&compilation_report, &profile_report);

    // Display in terminal
    println!("TERMINAL OUTPUT:");
    println!("{}", "━".repeat(60));
    dashboard.display();

    // Export to JSON
    println!("\nJSON EXPORT:");
    println!("{}", "━".repeat(60));
    match dashboard.to_json() {
        Ok(json) => {
            // Print first 500 characters
            let preview = if json.len() > 500 {
                format!("{}...\n(truncated)", &json[..500])
            } else {
                json.to_string()
            };
            println!("{}", preview);
        }
        Err(e) => eprintln!("Failed to export JSON: {}", e),
    }

    // Export to HTML
    println!("\nHTML EXPORT:");
    println!("{}", "━".repeat(60));
    let html = dashboard.to_html();
    let preview = if html.len() > 300 {
        format!("{}...\n(truncated)", &html[..300])
    } else {
        html.to_string()
    };
    println!("{}", preview);

    println!("\n✓ Example complete!");
    println!("\nTo save outputs:");
    println!("  JSON: dashboard.write_to_file(\"profile.json\", OutputFormat::Json)");
    println!("  HTML: dashboard.write_to_file(\"profile.html\", OutputFormat::Html)");
}

fn create_sample_compilation_report() -> CompilationProfileReport {
    let mut report = CompilationProfileReport::new();
    report.total_duration = Duration::from_millis(45200);

    report.phase_metrics = vec![
        PhasePerformanceMetrics {
            phase_name: "Parsing".into(),
            duration: Duration::from_millis(2100),
            memory_allocated: 1024 * 1024,
            items_processed: 150,
            time_percentage: 4.6,
            memory_percentage: 10.0,
            custom_metrics: Map::new(),
        },
        PhasePerformanceMetrics {
            phase_name: "Type Checking".into(),
            duration: Duration::from_millis(8700),
            memory_allocated: 2 * 1024 * 1024,
            items_processed: 200,
            time_percentage: 19.2,
            memory_percentage: 20.0,
            custom_metrics: Map::new(),
        },
        PhasePerformanceMetrics {
            phase_name: "Verification".into(),
            duration: Duration::from_millis(28300),
            memory_allocated: 5 * 1024 * 1024,
            items_processed: 75,
            time_percentage: 62.6,
            memory_percentage: 50.0,
            custom_metrics: Map::new(),
        },
        PhasePerformanceMetrics {
            phase_name: "Codegen".into(),
            duration: Duration::from_millis(6100),
            memory_allocated: 2 * 1024 * 1024,
            items_processed: 100,
            time_percentage: 13.5,
            memory_percentage: 20.0,
            custom_metrics: Map::new(),
        },
    ]
    .into();

    report
}

fn create_sample_profile_report() -> ProfileReport {
    let mut report = ProfileReport::new();

    // Function 1: Slow verification
    report.add_function(
        "complex_algorithm".into(),
        FunctionProfile {
            stats: CbgrStats {
                num_cbgr_refs: 45,
                num_ownership_refs: 12,
                num_checks: 450,
                total_time_ns: 30_000_000_000, // 30s
                cbgr_time_ns: 28_300_000_000,  // 28.3s
            },
            overhead_pct: 94.3,
            is_hot: true,
        },
    );

    // Function 2: High CBGR overhead
    report.add_function(
        "process_matrix".into(),
        FunctionProfile {
            stats: CbgrStats {
                num_cbgr_refs: 18,
                num_ownership_refs: 6,
                num_checks: 180,
                total_time_ns: 45_300_000, // 45.3ms
                cbgr_time_ns: 28_700_000,  // 28.7ms
            },
            overhead_pct: 63.4,
            is_hot: true,
        },
    );

    // Function 3: Good performance
    report.add_function(
        "safe_parse".into(),
        FunctionProfile {
            stats: CbgrStats {
                num_cbgr_refs: 3,
                num_ownership_refs: 10,
                num_checks: 15,
                total_time_ns: 1_000_000, // 1ms
                cbgr_time_ns: 1_000,      // 0.001ms
            },
            overhead_pct: 0.1,
            is_hot: false,
        },
    );

    // Function 4: Balanced performance
    report.add_function(
        "data_transform".into(),
        FunctionProfile {
            stats: CbgrStats {
                num_cbgr_refs: 8,
                num_ownership_refs: 4,
                num_checks: 40,
                total_time_ns: 5_000_000, // 5ms
                cbgr_time_ns: 200_000,    // 0.2ms
            },
            overhead_pct: 4.0,
            is_hot: false,
        },
    );

    report
}
