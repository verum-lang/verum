//! The type checker and interpreter must use the same selected cog test source.
use serde_json::Value;
use std::{fs, path::Path, process::Command};

const GOOD: &str = "module cog.lib;\npublic fn answer() -> Int { 17 }\n";
const TEST: &str = "@test\nfn selected() { assert_eq(answer(), 17); }\n";

fn run(root: &str, tests: &str, selection: &[&str]) -> (bool, Vec<Value>, String) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::create_dir(dir.path().join("tests")).unwrap();
    fs::write(
        dir.path().join("Verum.toml"),
        "[cog]\nname='probe'\nversion='0.1.0'\n",
    )
    .unwrap();
    fs::write(dir.path().join("src/lib.vr"), root).unwrap();
    fs::write(dir.path().join("tests/probe.vr"), tests).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_verum"))
        .args([
            "test",
            "--interp",
            "--test-threads",
            "1",
            "--format",
            "json",
        ])
        .args(selection)
        // Also exercise source-only SDK builds against the real standard library.
        .env(
            "VERUM_STDLIB_PATH",
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core"),
        )
        .env("VERUM_TEST_LENIENT_TYPES", "1")
        .current_dir(dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let events = stdout
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let message = format!("{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    (out.status.success(), events, message)
}

#[test]
fn checks_cog_root_and_test_file_even_when_invalid_function_is_unused() {
    for (root, tests, source, error) in [
        (
            format!("{GOOD}fn other() -> Bool {{ 47 }}"),
            TEST.into(),
            "src/lib.vr:3:",
            "expected 'Bool', found 'Int'",
        ),
        (
            GOOD.into(),
            format!("{TEST}fn other() -> Bool {{ 47 }}"),
            "tests/probe.vr:3:",
            "expected 'Bool', found 'Int'",
        ),
        (
            format!("{GOOD}fn malformed("),
            TEST.into(),
            "src/lib.vr:3:",
            "Parse error",
        ),
    ] {
        let (success, events, message) = run(&root, &tests, &[]);
        assert!(!success, "{message}");
        assert!(
            events
                .iter()
                .any(|e| e["event"] == "summary" && e["failed"] == 1 && e["passed"] == 0),
            "{message}"
        );
        assert!(
            message.contains(source) && message.contains(error),
            "{message}"
        );
    }
}

#[test]
fn ignored_declarations_are_checked_when_selected_and_do_not_poison_active_tests() {
    let tests = format!("{TEST}\n@test\n@ignore\nfn pinned() {{ let wrong: Bool = 47; }}");
    let (success, events, message) = run(GOOD, &tests, &[]);
    assert!(
        success
            && events
                .iter()
                .any(|e| e["event"] == "summary" && e["passed"] == 1),
        "{message}"
    );
    let (success, events, message) = run(GOOD, &tests, &["--include-ignored"]);
    assert!(!success, "{message}");
    assert!(
        events
            .iter()
            .any(|e| e["event"] == "summary" && e["passed"] == 1 && e["failed"] == 1),
        "{message}"
    );
    assert!(message.contains("tests/probe.vr"), "{message}");
}
