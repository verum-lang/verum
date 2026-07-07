//! `meta-engines-diff` — run the META-EXEC-CONVERGENCE-1 differential
//! harness and print honest per-fixture verdicts plus totals.
//!

//! Exit code 0 when every fixture is either agreeing (agreement surface) or
//! a still-matching pin (known divergence); 1 when anything NEW shows up —
//! an agreement fixture diverging, a pin drifting, or a pin disappearing.

use meta_engines::{all_fixtures, overflow_checks_enabled, run_all, with_quiet_panics};

fn main() {
    // Quiet the panic hook: the i128-overflow pin panics the tree-walk
    // engine BY DESIGN (observed via catch_unwind); its backtrace is noise.
    let (fixtures, reports, totals, overflow_checks) = with_quiet_panics(|| {
        let overflow_checks = overflow_checks_enabled();
        let fixtures = all_fixtures();
        let (reports, totals) = run_all(&fixtures);
        (fixtures, reports, totals, overflow_checks)
    });

    println!("META-EXEC-CONVERGENCE-1 — VBC executor vs tree-walk evaluator");
    println!(
        "fixtures: {}   (build profile: overflow-checks {})",
        fixtures.len(),
        if overflow_checks { "ON" } else { "OFF" },
    );
    println!();

    for report in &reports {
        println!("[{:>13}] {} — {}", report.status.label(), report.name, report.description);
        if let (Some(vbc), Some(tree)) = (&report.vbc, &report.tree) {
            println!("                vbc:  {vbc}");
            println!("                tree: {tree}");
        }
        if let Some(verdict) = &report.verdict {
            println!("                verdict: {verdict}");
        }
        if !report.explanation.is_empty() {
            println!("                note: {}", report.explanation);
        }
        println!();
    }

    println!("──────────────────────────────────────────────────────");
    println!(
        "totals: {} agree / {} known-diverge / {} NEW-diverge   ({} fixtures)",
        totals.agree,
        totals.known_diverge,
        totals.new_diverge,
        totals.total()
    );

    if totals.new_diverge > 0 {
        println!("RESULT: FAIL — unexpected divergence(s); see NEW-DIVERGE entries above");
        std::process::exit(1);
    }
    println!("RESULT: OK — agreement surface holds, every known divergence still pinned");
}
