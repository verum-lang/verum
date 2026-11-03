use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn profile(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_verum"))
        .current_dir(dir)
        .arg("profile")
        .args(args)
        .output()
        .unwrap()
}

fn json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("one JSON report on stdout")
}

#[test]
fn file_timing_is_measured_and_the_source_model_is_opt_in() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("main.vr"),
        "fn probe(n: &Int) -> Int { 1 }\nfn main() -> Int { 0 }",
    )
    .unwrap();
    let timing = json(&profile(
        dir.path(),
        &["--cpu", "--precision", "ns", "--json", "main.vr"],
    ));
    assert_eq!(timing["compilation"]["measurement"], "compiler_wall_clock");
    assert!(
        timing["compilation"]["phases"]
            .as_array()
            .unwrap()
            .iter()
            .any(|phase| phase["name"] == "Stdlib Loading")
    );
    let elapsed = &timing["compilation"]["elapsed"];
    assert!(elapsed["secs"].as_u64().unwrap() > 0 || elapsed["nanos"].as_u64().unwrap() > 0);
    assert!(timing.get("functions").is_none());
    let all = json(&profile(
        dir.path(),
        &[
            "--all",
            "--functions",
            "probe",
            "--json",
            "-o",
            "report.json",
            "main.vr",
        ],
    ));
    assert_eq!(
        all,
        serde_json::from_slice::<Value>(&fs::read(dir.path().join("report.json")).unwrap())
            .unwrap()
    );
    assert_eq!(all["function_measurement"], "static_cbgr_cost_model");
    assert_eq!(all["functions"].as_object().unwrap().len(), 1);
    assert_eq!(all["functions"]["probe"]["stats"]["num_cbgr_refs"], 1);
    assert!(all["cbgr_analysis_cache_entries"].is_u64());

    fs::write(dir.path().join("main.vr"), "fn main( -> Int {").unwrap();
    let bad = profile(
        dir.path(),
        &["--cpu", "--json", "-o", "invalid.json", "main.vr"],
    );
    assert!(!bad.status.success());
    assert!(!dir.path().join("invalid.json").exists());
}

#[test]
fn project_check_profiles_each_real_module_under_its_own_name() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("Verum.toml"), "[cog]\nname = \"profile_test\"\nversion = \"0.1.0\"\n[language]\nprofile = \"application\"\n").unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    for name in ["left", "right"] {
        fs::write(
            dir.path().join("src").join(format!("{name}.vr")),
            "public fn same() -> Int { 7 }",
        )
        .unwrap();
    }
    let report = json(&profile(
        dir.path(),
        &["--all", "--functions", "same", "--json"],
    ));
    assert_eq!(report["files_checked"], 2);
    let functions = report["functions"].as_object().unwrap();
    assert_eq!(functions.len(), 2, "{report}");
    assert!(functions.contains_key("profile_test.left.same"));
    assert!(functions.contains_key("profile_test.right.same"));
}
