//! Public CLI tests use the actual executable, including fresh/cached runs and
//! refusal paths. A report must not turn absent execution into a clean capture.
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
use verum_common::runtime_diagnostics::{RuntimeObservation, RuntimeReport};

fn run(source: &str, path: &Path, id: &str) -> Output {
    run_at(source, path, id, None)
}

fn run_at(source: &str, path: &Path, id: &str, entry: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_verum"));
    command.args(["run", "--interp"]);
    if let Some(entry) = entry {
        command.args(["--deny-all", "--stdin-filepath"]).arg(entry);
    }
    let mut child = command
        .arg("--runtime-report")
        .arg(path)
        .args(["--runtime-run-id", id, "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn named_stdin_uses_the_project_closure_and_binds_the_unmodified_buffer_even_on_cache_hits() {
    let source =
        "module demo.app;\nmount demo.helper.{answer};\nfn main() { println(answer() + 1); }";
    let hash = hex::encode(Sha256::digest(source.as_bytes()));
    let root = tempfile::tempdir().unwrap();
    for answer in [17, 25] {
        let project = root.path().join(answer.to_string());
        let src = project.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            project.join("verum.toml"),
            "[cog]\nname='demo'\nversion='0.1.0'",
        )
        .unwrap();
        std::fs::write(project.join("mod.vr"), "module demo;").unwrap();
        std::fs::write(
            src.join("helper.vr"),
            format!("module demo.helper; public fn answer() -> Int {{ {answer} }}"),
        )
        .unwrap();
        let entry = src.join("app.vr");
        let saved = "module demo.app; fn main() { println(0); }";
        std::fs::write(&entry, saved).unwrap();
        for id in ["fresh", "warm"] {
            let path = project.join(format!("{id}.json"));
            let output = run_at(source, &path, id, Some(&entry));
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                (answer + 1).to_string()
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stderr).contains("(cached VBC)"),
                id == "warm"
            );
            let report: RuntimeReport =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            report.validate_binding(id, &hash).unwrap();
            assert_eq!(report.observation, RuntimeObservation::Captured);
            assert_eq!(report.wrapper_lines, 0);
            assert_eq!(report.prepared_source_sha256.as_str(), hash);
            assert_eq!(
                report.entry_source_path.as_ref().unwrap().as_str(),
                entry.to_str().unwrap()
            );
        }
        assert_eq!(std::fs::read_to_string(&entry).unwrap(), saved);
    }
}

#[test]
fn actual_cli_reports_exact_stdin_identity_and_refuses_reusing_output() {
    let dir = tempfile::tempdir().unwrap();
    // No trailing newline: the synthetic script adds one as well as a shebang.
    let source = format!("print(42); // runtime report {}", uuid::Uuid::new_v4());
    let hash = hex::encode(Sha256::digest(source.as_bytes()));
    for id in ["first", "cached"] {
        let path = dir.path().join(format!("{id}.json"));
        let output = run(&source, &path, id);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).contains("(cached VBC)"),
            id == "cached",
            "fresh and cached runs must exercise different execution paths"
        );
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|line| line.trim() == "42")
        );
        let report: RuntimeReport = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        report.validate_binding(id, &hash).unwrap();
        assert_eq!(report.observation, RuntimeObservation::Captured);
        assert_eq!(report.wrapper_lines, 1);
        assert!(report.cbgr.unwrap().events.is_empty());
        let retained = std::fs::read(&path).unwrap();
        assert!(!run(&source, &path, "overwrite").status.success());
        assert_eq!(std::fs::read(&path).unwrap(), retained);
    }
}

#[test]
fn compiler_refusal_leaves_unavailable_observation_and_cli_rejects_invalid_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("failed.json");
    let output = run("let = ;", &path, "parse-failure");
    assert!(!output.status.success());
    let report: RuntimeReport = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    report
        .validate_binding("parse-failure", &hex::encode(Sha256::digest(b"let = ;")))
        .unwrap();
    assert_eq!(report.observation, RuntimeObservation::Unavailable);
    assert!(report.cbgr.is_none());
    let rejected = dir.path().join("invalid.json");
    assert!(!run("print(1);", &rejected, "invalid/id").status.success());
    assert!(!rejected.exists());
}
