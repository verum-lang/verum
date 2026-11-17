#[path = "../src/commands/test/event.rs"]
mod event;

use std::process::{Command, ExitStatus, Output};
use std::time::Duration;

use event::{TestResult, single_worker_result};
use serde_json::{Value, json};

fn exit_status(code: u8) -> ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(i32::from(code) << 8)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(u32::from(code))
    }
}

fn reported(name: &str, outcome: &str) -> Value {
    json!({"event": "test", "name": name, "outcome": outcome, "duration_ms": 7})
}

fn worker(status: ExitStatus, records: &[Value]) -> Output {
    Output {
        status,
        stdout: records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes(),
        stderr: b"worker diagnostic".to_vec(),
    }
}

#[test]
fn single_worker_requires_one_completed_named_test_and_successful_exit() {
    let name = "case::selected";
    let pass = reported(name, "ok");
    let other = reported("case::other", "ok");
    for records in [
        vec![],
        vec![reported(name, "ignored")],
        vec![other.clone()],
        vec![pass.clone(), other],
        vec![pass.clone(), pass.clone()],
        vec![
            pass.clone(),
            json!({"event": "test", "outcome": "ok", "duration_ms": 7}),
        ],
        vec![reported(name, "unknown")],
    ] {
        let result = single_worker_result(name, worker(exit_status(0), &records), Duration::ZERO);
        assert!(
            matches!(result, TestResult::Fail { exit_code: Some(0), stderr, .. } if stderr == "worker diagnostic")
        );
    }
    assert!(matches!(
        single_worker_result(name, worker(exit_status(0), std::slice::from_ref(&pass)), Duration::ZERO),
        TestResult::Pass { duration, .. } if duration == Duration::from_millis(7)
    ));
    assert!(matches!(
        single_worker_result(name, worker(exit_status(24), &[pass]), Duration::ZERO),
        TestResult::Fail {
            exit_code: Some(24),
            ..
        }
    ));
}

#[test]
fn native_failure_retains_the_test_status_and_both_diagnostic_streams() {
    let name = "case::native";
    let mut fail = reported(name, "failed");
    fail["exit_code"] = json!(73);
    fail["stdout"] = json!("test output");
    fail["stderr"] = json!("native diagnostic");
    fail["error"] = json!("process exited with code 73");
    let result = single_worker_result(name, worker(exit_status(102), &[fail]), Duration::ZERO);
    assert!(
        matches!(result, TestResult::Fail { exit_code: Some(73), stdout, stderr, error, .. }
        if stdout == "test output" && stderr.contains("native diagnostic") && stderr.contains("worker diagnostic") && error.contains("73"))
    );

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        let result =
            single_worker_result(name, worker(ExitStatus::from_raw(15), &[]), Duration::ZERO);
        assert!(
            matches!(result, TestResult::Fail { signal: Some(15), exit_code: None, stderr, .. } if stderr == "worker diagnostic")
        );
    }
}

#[test]
fn ordinary_aot_json_retains_the_compiler_refusal_from_its_worker() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Verum.toml"),
        "[cog]\nname=\"errors\"\nversion=\"0.1.0\"\n",
    )
    .unwrap();
    std::fs::create_dir(dir.path().join("tests")).unwrap();
    std::fs::write(
        dir.path().join("tests/report.vr"),
        "@test\nfn test_bad_call() { let result: Int = missing_diagnostic_function(); }\n",
    )
    .unwrap();
    let name = "report::test_bad_call";
    let run = |direct: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_verum"));
        command.current_dir(dir.path()).args([
            "test",
            "--aot",
            "--format",
            "json",
            "--test-threads",
            "1",
            "--exact",
            "--filter",
            name,
        ]);
        command.env_remove("VERUM_TEST_AOT_CHILD");
        if direct {
            command.env("VERUM_TEST_AOT_CHILD", "1");
        }
        let out = command.output().unwrap();
        assert!(!out.status.success());
        let events: Vec<_> = std::str::from_utf8(&out.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|entry| entry["event"] == "test")
            .collect();
        assert_eq!(events.len(), 1);
        let event = events.into_iter().next().unwrap();
        assert_eq!(event["name"], name);
        assert_eq!(event["outcome"], "compile-error");
        assert!(
            event["error"]
                .as_str()
                .unwrap()
                .contains("compilation failed")
        );
        assert!(
            event["error"]
                .as_str()
                .unwrap()
                .contains("E100: unbound variable: missing_diagnostic_function"),
            "{event}"
        );
        let stderr = event["stderr"].as_str().unwrap();
        assert!(
            stderr.contains("E100") && stderr.contains("missing_diagnostic_function"),
            "{event}"
        );
        (out.status, event)
    };
    let (direct_status, direct) = run(true);
    let (_, isolated) = run(false);
    assert_eq!(isolated["error"], direct["error"]);
    assert_eq!(isolated["exit_code"], json!(direct_status.code()));
}
