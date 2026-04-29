//! `verum benchmark` subcommand — head-to-head comparison surface
//! for the continuous-benchmarking trait module.  Non-interactive
//! batch driver: runs the configured suite against one or more
//! systems and emits a comparison matrix.

use crate::error::{CliError, Result};
use verum_verification::benchmark::{
    mock_runner_for, BenchmarkMetric, BenchmarkResult, BenchmarkRunner, BenchmarkSuite,
    BenchmarkSystem, ComparisonMatrix,
};

fn parse_system(s: &str) -> Result<BenchmarkSystem> {
    BenchmarkSystem::from_name(s).ok_or_else(|| {
        CliError::InvalidArgument(format!(
            "--system must be one of verum / coq / lean4 / mizar (no Mizar yet — use lean / mathlib / isabelle / agda; aliases: rocq / mathlib / hol), got '{}'",
            s
        ))
    })
}

fn parse_metric(s: &str) -> Result<BenchmarkMetric> {
    BenchmarkMetric::from_name(s).ok_or_else(|| {
        CliError::InvalidArgument(format!(
            "--metric must be one of kernel_loc / lines_per_second / theorems_per_second / peak_rss_bytes / elapsed_ms / cross_format_exports / tactic_coverage_percent / trust_diversification_count / llm_acceptance_percent, got '{}'",
            s
        ))
    })
}

fn validate_format(s: &str) -> Result<()> {
    if s != "plain" && s != "json" && s != "markdown" && s != "csv" {
        return Err(CliError::InvalidArgument(format!(
            "--format must be 'plain', 'json', 'markdown', or 'csv', got '{}'",
            s
        )));
    }
    Ok(())
}

fn build_suite(name: &str, theorems: &[String]) -> Result<BenchmarkSuite> {
    if name.is_empty() {
        return Err(CliError::InvalidArgument(
            "--suite-name must be non-empty".into(),
        ));
    }
    let mut suite = BenchmarkSuite::new(name);
    for t in theorems {
        if t.is_empty() {
            return Err(CliError::InvalidArgument(
                "--theorem must be non-empty".into(),
            ));
        }
        suite = suite.add_theorem(t.as_str());
    }
    Ok(suite)
}

/// Run the suite against a single system (mock runner).  V1
/// production runners that call out to the real tools plug in via
/// the same trait without changing this dispatch.
pub fn run_run(
    system: &str,
    suite_name: &str,
    theorems: &[String],
    format: &str,
) -> Result<()> {
    validate_format(format)?;
    let sys = parse_system(system)?;
    let suite = build_suite(suite_name, theorems)?;
    let runner = mock_runner_for(sys);
    let results = runner.run(&suite).map_err(|e| {
        CliError::VerificationFailed(format!("benchmark.{}: {}", sys.name(), e))
    })?;
    emit_results(&results, format)?;
    Ok(())
}

/// Run the suite against every requested system and emit a
/// comparison matrix.  When `systems` is empty, runs against every
/// known system.
pub fn run_compare(
    systems: &[String],
    suite_name: &str,
    theorems: &[String],
    format: &str,
) -> Result<()> {
    validate_format(format)?;
    let suite = build_suite(suite_name, theorems)?;
    let parsed_systems: Vec<BenchmarkSystem> = if systems.is_empty() {
        BenchmarkSystem::all().to_vec()
    } else {
        systems.iter().map(|s| parse_system(s)).collect::<Result<Vec<_>>>()?
    };

    let mut matrix = ComparisonMatrix::new(suite.name.as_str().to_string());
    let mut all_results: Vec<BenchmarkResult> = Vec::new();
    for sys in parsed_systems {
        let runner = mock_runner_for(sys);
        let results = runner.run(&suite).map_err(|e| {
            CliError::VerificationFailed(format!("benchmark.{}: {}", sys.name(), e))
        })?;
        all_results.extend(results);
    }
    matrix.ingest_aggregated(&all_results);

    match format {
        "plain" => emit_matrix_plain(&matrix),
        "markdown" => print!("{}", matrix.to_markdown().as_str()),
        "json" => emit_matrix_json(&matrix, &all_results),
        "csv" => emit_matrix_csv(&matrix),
        _ => unreachable!(),
    }
    Ok(())
}

/// Print every supported metric with its `higher_is_better` flag —
/// useful for CI scripts that need to know which direction means
/// "better" for a given metric.
pub fn run_metrics(format: &str) -> Result<()> {
    validate_format(format)?;
    let metrics = BenchmarkMetric::all();
    match format {
        "plain" => {
            println!("Benchmark metrics ({}):", metrics.len());
            println!();
            println!("  {:<32}  {}", "Name", "Direction");
            println!("  {:<32}  {}", "─".repeat(32), "─".repeat(20));
            for m in metrics {
                println!(
                    "  {:<32}  {}",
                    m.name(),
                    if m.higher_is_better() {
                        "higher is better"
                    } else {
                        "lower is better"
                    }
                );
            }
        }
        "json" => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"count\": {},\n", metrics.len()));
            out.push_str("  \"metrics\": [\n");
            for (i, m) in metrics.iter().enumerate() {
                out.push_str(&format!(
                    "    {{ \"name\": \"{}\", \"higher_is_better\": {} }}{}\n",
                    m.name(),
                    m.higher_is_better(),
                    if i + 1 < metrics.len() { "," } else { "" }
                ));
            }
            out.push_str("  ]\n}");
            println!("{}", out);
        }
        "markdown" => {
            println!("# Benchmark metrics\n");
            println!("| Metric | Direction |");
            println!("|---|---|");
            for m in metrics {
                println!(
                    "| `{}` | {} |",
                    m.name(),
                    if m.higher_is_better() {
                        "higher is better"
                    } else {
                        "lower is better"
                    }
                );
            }
        }
        "csv" => {
            println!("metric,higher_is_better");
            for m in metrics {
                println!("{},{}", m.name(), m.higher_is_better());
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

// =============================================================================
// emitters
// =============================================================================

fn emit_results(results: &[BenchmarkResult], format: &str) -> Result<()> {
    match format {
        "plain" => {
            println!(
                "Benchmark transcript ({} result(s)):",
                results.len()
            );
            println!();
            for r in results {
                println!(
                    "  {:<10} {:<28} {:<14} {}",
                    r.system.name(),
                    r.theorem
                        .as_ref()
                        .map(|t| t.as_str().to_string())
                        .unwrap_or_else(|| "(suite)".to_string()),
                    r.metric.name(),
                    r.value
                );
            }
        }
        "json" => {
            let body = serde_json::to_string_pretty(results).unwrap_or_default();
            println!("{}", body);
        }
        "markdown" => {
            println!(
                "# Benchmark transcript ({} result(s))\n",
                results.len()
            );
            println!("| System | Theorem | Metric | Value |");
            println!("|---|---|---|---|");
            for r in results {
                println!(
                    "| `{}` | `{}` | `{}` | {} |",
                    r.system.name(),
                    r.theorem
                        .as_ref()
                        .map(|t| t.as_str().to_string())
                        .unwrap_or_else(|| "(suite)".to_string()),
                    r.metric.name(),
                    r.value
                );
            }
        }
        "csv" => {
            println!("system,theorem,metric,value,timestamp");
            for r in results {
                println!(
                    "{},{},{},{},{}",
                    r.system.name(),
                    r.theorem.as_ref().map(|t| t.as_str()).unwrap_or(""),
                    r.metric.name(),
                    r.value,
                    r.timestamp
                );
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn emit_matrix_plain(m: &ComparisonMatrix) {
    println!("Comparison matrix — suite: `{}`", m.suite.as_str());
    println!();
    let metrics: Vec<BenchmarkMetric> = {
        let mut s: std::collections::BTreeSet<BenchmarkMetric> =
            std::collections::BTreeSet::new();
        for (mt, _) in m.by_metric_and_system.keys() {
            s.insert(*mt);
        }
        s.into_iter().collect()
    };
    let systems: Vec<BenchmarkSystem> = {
        let mut s: std::collections::BTreeSet<BenchmarkSystem> =
            std::collections::BTreeSet::new();
        for (_, sys) in m.by_metric_and_system.keys() {
            s.insert(*sys);
        }
        s.into_iter().collect()
    };
    print!("  {:<32}", "Metric");
    for s in &systems {
        print!(" {:>14}", s.name());
    }
    print!(" {:>10}", "leader");
    println!();
    print!("  {}", "─".repeat(32));
    for _ in &systems {
        print!(" {}", "─".repeat(14));
    }
    print!(" {}", "─".repeat(10));
    println!();
    for metric in &metrics {
        print!("  {:<32}", metric.name());
        for s in &systems {
            match m.by_metric_and_system.get(&(*metric, *s)) {
                Some(v) => print!(" {:>14.1}", v),
                None => print!(" {:>14}", "—"),
            }
        }
        let leader = m.leader(*metric);
        print!(
            " {:>10}",
            leader.map(|s| s.name()).unwrap_or("—")
        );
        println!();
    }
}

fn emit_matrix_json(m: &ComparisonMatrix, raw_results: &[BenchmarkResult]) {
    let mut out = String::from("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!(
        "  \"suite\": \"{}\",\n",
        json_escape(m.suite.as_str())
    ));
    out.push_str("  \"results\": ");
    let body = serde_json::to_string(raw_results).unwrap_or_default();
    out.push_str(&body);
    out.push_str(",\n");
    out.push_str("  \"matrix\": [\n");
    let entries: Vec<((BenchmarkMetric, BenchmarkSystem), f64)> = m
        .by_metric_and_system
        .iter()
        .map(|((mt, sys), v)| ((*mt, *sys), *v))
        .collect();
    for (i, ((metric, sys), value)) in entries.iter().enumerate() {
        out.push_str(&format!(
            "    {{ \"metric\": \"{}\", \"system\": \"{}\", \"value\": {} }}{}\n",
            metric.name(),
            sys.name(),
            value,
            if i + 1 < entries.len() { "," } else { "" }
        ));
    }
    out.push_str("  ],\n");
    out.push_str("  \"leaders\": [\n");
    let mut metrics: Vec<BenchmarkMetric> = entries.iter().map(|(k, _)| k.0).collect();
    metrics.sort();
    metrics.dedup();
    for (i, metric) in metrics.iter().enumerate() {
        let leader = m.leader(*metric);
        out.push_str(&format!(
            "    {{ \"metric\": \"{}\", \"leader\": {} }}{}\n",
            metric.name(),
            match leader {
                Some(s) => format!("\"{}\"", s.name()),
                None => "null".to_string(),
            },
            if i + 1 < metrics.len() { "," } else { "" }
        ));
    }
    out.push_str("  ]\n}");
    println!("{}", out);
}

fn emit_matrix_csv(m: &ComparisonMatrix) {
    println!("metric,system,value,is_leader");
    let metrics: Vec<BenchmarkMetric> = {
        let mut s: std::collections::BTreeSet<BenchmarkMetric> =
            std::collections::BTreeSet::new();
        for (mt, _) in m.by_metric_and_system.keys() {
            s.insert(*mt);
        }
        s.into_iter().collect()
    };
    for metric in &metrics {
        let leader = m.leader(*metric);
        for ((mt, sys), v) in &m.by_metric_and_system {
            if *mt != *metric {
                continue;
            }
            println!(
                "{},{},{},{}",
                mt.name(),
                sys.name(),
                v,
                leader == Some(*sys)
            );
        }
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- parsers -----

    #[test]
    fn parse_system_canonical() {
        for s in ["verum", "coq", "lean4", "isabelle", "agda"] {
            assert!(parse_system(s).is_ok());
        }
    }

    #[test]
    fn parse_system_aliases() {
        assert!(parse_system("rocq").is_ok());
        assert!(parse_system("lean").is_ok());
        assert!(parse_system("mathlib").is_ok());
    }

    #[test]
    fn parse_system_rejects_unknown() {
        assert!(matches!(
            parse_system("garbage"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    #[test]
    fn parse_metric_round_trip() {
        for m in [
            "kernel_loc",
            "lines_per_second",
            "theorems_per_second",
            "peak_rss_bytes",
            "elapsed_ms",
            "cross_format_exports",
            "tactic_coverage_percent",
            "trust_diversification_count",
            "llm_acceptance_percent",
        ] {
            assert!(parse_metric(m).is_ok(), "{}", m);
        }
    }

    #[test]
    fn validate_format_round_trip() {
        for f in ["plain", "json", "markdown", "csv"] {
            assert!(validate_format(f).is_ok());
        }
        assert!(matches!(
            validate_format("yaml"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    #[test]
    fn build_suite_validates_inputs() {
        assert!(matches!(
            build_suite("", &[]),
            Err(CliError::InvalidArgument(_))
        ));
        assert!(matches!(
            build_suite("ok", &["".into()]),
            Err(CliError::InvalidArgument(_))
        ));
        let s = build_suite("name", &["thm1".into(), "thm2".into()]).unwrap();
        assert_eq!(s.theorems.len(), 2);
    }

    // ----- run_run -----

    #[test]
    fn run_run_validates_inputs() {
        assert!(matches!(
            run_run("garbage", "suite", &[], "plain"),
            Err(CliError::InvalidArgument(_))
        ));
        assert!(matches!(
            run_run("verum", "", &[], "plain"),
            Err(CliError::InvalidArgument(_))
        ));
        assert!(matches!(
            run_run("verum", "suite", &[], "yaml"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    #[test]
    fn run_run_emits_smoke() {
        let r = run_run(
            "verum",
            "suite",
            &["thm1".into(), "thm2".into()],
            "plain",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn run_run_json_format_works() {
        assert!(run_run("verum", "suite", &["thm1".into()], "json").is_ok());
        assert!(run_run("verum", "suite", &["thm1".into()], "markdown").is_ok());
        assert!(run_run("verum", "suite", &["thm1".into()], "csv").is_ok());
    }

    // ----- run_compare -----

    #[test]
    fn run_compare_no_systems_runs_all() {
        // Empty systems list → run all 5.  Smoke test only — we
        // can't easily capture stdout in unit tests.
        let r = run_compare(
            &[],
            "suite",
            &["thm1".into()],
            "plain",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn run_compare_explicit_systems() {
        let r = run_compare(
            &["verum".into(), "coq".into()],
            "suite",
            &["thm1".into()],
            "json",
        );
        assert!(r.is_ok());
    }

    #[test]
    fn run_compare_validates_inputs() {
        assert!(matches!(
            run_compare(&["garbage".into()], "s", &[], "plain"),
            Err(CliError::InvalidArgument(_))
        ));
        assert!(matches!(
            run_compare(&[], "", &[], "plain"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    // ----- run_metrics -----

    #[test]
    fn run_metrics_every_format() {
        for f in ["plain", "json", "markdown", "csv"] {
            assert!(run_metrics(f).is_ok());
        }
    }

    #[test]
    fn run_metrics_rejects_unknown_format() {
        assert!(matches!(
            run_metrics("yaml"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    // ----- json_escape -----

    #[test]
    fn json_escape_handles_control_chars() {
        assert_eq!(json_escape("a\"b\nc"), "a\\\"b\\nc");
    }
}
