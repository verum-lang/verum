//! Strict replay of the record telescope model against Rust observations.
use std::{path::Path, process::Command};

pub fn check(observed: &str, theorem_names: &[&str], changes: &[(&str, &str)]) {
    let semantics = include_str!("VerumExternalReplay/RecordTelescope.lean");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verification/external/lean");
    let scratch = std::env::temp_dir().join(format!(
        "verum-record-telescope-{}",
        format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    ));
    std::fs::create_dir(&scratch).unwrap();
    let input = scratch.join("Observed.lean");
    let replay = |claims: &str| {
        std::fs::write(&input, format!("{semantics}\n{claims}")).unwrap();
        Command::new(std::env::var_os("VERUM_LEAN").unwrap_or_else(|| "lean".into()))
            .arg(&input)
            .current_dir(&root)
            .output()
            .expect("pinned Lean toolchain required")
    };
    let clean = replay(observed);
    assert!(
        clean.status.success(),
        "{}{}",
        String::from_utf8_lossy(&clean.stdout),
        String::from_utf8_lossy(&clean.stderr)
    );
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&clean.stdout),
        String::from_utf8_lossy(&clean.stderr)
    );
    for &name in theorem_names {
        assert!(
            report.contains(name),
            "missing audited theorem {name}: {report}"
        );
    }
    for forbidden in ["sorryAx", "Lean.ofReduceBool", "warning:", "error:"] {
        assert!(!report.contains(forbidden), "{report}");
    }
    for line in report
        .lines()
        .filter(|line| line.contains("depends on axioms:"))
    {
        let axioms = line.split_once('[').unwrap().1.strip_suffix(']').unwrap();
        for axiom in axioms.split(',').map(str::trim) {
            assert!(
                ["propext", "Classical.choice", "Quot.sound"].contains(&axiom),
                "unaudited axiom {axiom}"
            );
        }
    }
    for &(before, after) in changes {
        assert!(observed.contains(before), "missing observation {before}");
        let changed = observed.replacen(before, after, 1);
        let negative = replay(&changed);
        let report = format!(
            "{}{}",
            String::from_utf8_lossy(&negative.stdout),
            String::from_utf8_lossy(&negative.stderr)
        );
        assert!(
            !negative.status.success() && report.contains("decide"),
            "tamper result {report}"
        );
    }
    std::fs::remove_file(input).unwrap();
    std::fs::remove_dir(scratch).unwrap();
}
