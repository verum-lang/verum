//! T1672: exercise the handlers dispatched by workspace list/add/remove/exec.
//! Case-distinct manifest controls require VERUM_T1672_FIXTURE_ROOT on a
//! case-sensitive filesystem. Child commands write observable per-member files.

use super::{Config, add, exec, list, remove};
use crate::error::CliError;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;
use verum_common::{List, Text};

const CHILD: &str = "commands::workspace::management_manifests::isolated_management_case";
const EXEC_CHILD: &str = "commands::workspace::management_manifests::executed_member_command";
const CASE_ENV: &str = "VERUM_T1672_WORKSPACE_CASE";
const EXEC_ENV: &str = "VERUM_T1672_EXEC_CHILD";
const ROOT_ENV: &str = "VERUM_T1672_FIXTURE_ROOT";

fn fixture() -> (TempDir, bool) {
    let root = std::env::var_os(ROOT_ENV);
    let mut builder = tempfile::Builder::new();
    builder.prefix("verum-t1672-");
    let dir = match &root {
        Some(path) => builder.tempdir_in(path),
        None => builder.tempdir(),
    }
    .unwrap();
    let probe = dir.path().join("probe");
    fs::create_dir(&probe).unwrap();
    fs::write(probe.join("case"), b"lower").unwrap();
    fs::write(probe.join("CASE"), b"upper").unwrap();
    let distinct = fs::read_dir(&probe).unwrap().count() == 2
        && fs::read(probe.join("case")).unwrap() == b"lower"
        && fs::read(probe.join("CASE")).unwrap() == b"upper";
    fs::remove_dir_all(probe).unwrap();
    println!("T1672_CASE_DISTINCT={distinct}");
    assert!(
        root.is_none() || distinct,
        "{ROOT_ENV} must be case-sensitive"
    );
    (dir, distinct)
}

fn member(root: &Path, dir: &str, filename: &str, name: &str) {
    fs::create_dir(root.join(dir)).unwrap();
    fs::write(
        root.join(dir).join(filename),
        format!("[cog]\nname = \"{name}\"\nversion = \"1.2.3\"\n"),
    )
    .unwrap();
}

fn workspace(root: &Path, filename: &str, members: &[&str]) -> Text {
    let members = members
        .iter()
        .map(|member| format!("\"{member}\""))
        .collect::<List<_>>()
        .join(", ");
    let bytes: Text = format!(
        "[cog]\nname = \"management-workspace\"\nversion = \"1.0.0\"\n\
         [workspace]\nmembers = [{members}]\n"
    )
    .into();
    fs::write(root.join(filename), bytes.as_bytes()).unwrap();
    bytes
}

#[test]
fn list_discovers_canonical_and_legacy_members() {
    child_case("list");
}

#[test]
fn list_reports_invalid_and_missing_members_without_legacy_fallback() {
    child_case("list_invalid");
}

#[test]
fn exec_runs_the_child_command_in_canonical_and_legacy_members() {
    child_case("exec");
}

#[test]
fn exec_cannot_succeed_after_skipping_its_only_missing_member() {
    child_case("exec_missing");
}

#[test]
fn exec_runs_valid_members_and_reports_invalid_and_missing_failures() {
    child_case("exec_mixed");
}

#[test]
fn exec_propagates_a_real_child_command_failure() {
    child_case("exec_failure");
}

#[test]
fn add_persists_a_canonical_member_in_the_canonical_workspace() {
    child_case("add_canonical");
}

#[test]
fn add_preserves_legacy_workspace_and_member_compatibility() {
    child_case("add_legacy");
}

#[test]
fn add_updates_canonical_workspace_without_overwriting_legacy_shadow() {
    child_case("add_precedence");
}

#[test]
fn add_refuses_invalid_canonical_member_without_mutating_workspace() {
    child_case("add_invalid");
}

#[test]
fn remove_by_cog_name_persists_in_canonical_workspace() {
    child_case("remove_canonical");
}

#[test]
fn remove_by_member_path_preserves_legacy_workspace_compatibility() {
    child_case("remove_legacy");
}

#[test]
fn remove_updates_canonical_workspace_without_overwriting_legacy_shadow() {
    child_case("remove_precedence");
}

fn child_case(case: &str) {
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD, "--nocapture"])
        .env(CASE_ENV, case)
        .env(EXEC_ENV, "1")
        .output()
        .unwrap();
    let output = format!(
        "{}\n{}",
        std::str::from_utf8(&result.stdout).unwrap(),
        std::str::from_utf8(&result.stderr).unwrap()
    );
    println!("T1672_CASE={case}\n{output}");
    assert!(result.status.success(), "{case}: {output}");
    match case {
        "list" => {
            assert!(output.contains("canonical-member"), "{output}");
            assert!(output.contains("legacy-member"), "{output}");
            assert!(!output.contains("Missing"), "{output}");
        }
        "list_invalid" => {
            assert!(output.contains("Invalid config"), "{output}");
            assert!(output.contains("Missing manifest"), "{output}");
            assert!(!output.contains("shadow-member"), "{output}");
        }
        "exec" => assert!(output.contains("successfully in all 2 members"), "{output}"),
        "exec_missing" | "exec_mixed" | "exec_failure" => {
            assert!(
                !output.contains("completed successfully in all"),
                "{output}"
            );
            let failed = if case == "exec_mixed" { 2 } else { 1 };
            assert!(
                output.contains(&format!("Command failed in {failed} members")),
                "{output}"
            );
        }
        _ => {}
    }
}

#[test]
fn isolated_management_case() {
    let Ok(case) = std::env::var(CASE_ENV) else {
        return;
    };
    let (fixture, distinct) = fixture();
    let root = fixture.path();
    std::env::set_current_dir(root).unwrap();
    crate::ui::init(false, false, "never").unwrap();
    match case.as_str() {
        "list" | "exec" | "exec_failure" => {
            member(root, "canonical", "Verum.toml", "canonical-member");
            member(root, "legacy", "verum.toml", "legacy-member");
            workspace(root, "Verum.toml", &["canonical", "legacy"]);
            if case == "list" {
                list().unwrap();
            } else {
                if case == "exec_failure" {
                    fs::write(root.join("legacy/command-fails"), b"fail").unwrap();
                }
                let result = run_in_members();
                assert_executed(root, "canonical");
                assert_executed(root, "legacy");
                if case == "exec" {
                    result.unwrap();
                } else {
                    assert_exec_failure(result, 1);
                }
            }
        }
        "list_invalid" | "exec_mixed" => {
            member(root, "canonical", "Verum.toml", "canonical-member");
            member(root, "invalid", "Verum.toml", "invalid-member");
            fs::write(root.join("invalid/Verum.toml"), "[cog]\nname = [").unwrap();
            shadow_member(&root.join("invalid"), distinct);
            fs::create_dir(root.join("missing")).unwrap();
            workspace(root, "Verum.toml", &["canonical", "invalid", "missing"]);
            if case == "list_invalid" {
                list().unwrap();
            } else {
                assert_exec_failure(run_in_members(), 2);
                assert_executed(root, "canonical");
                assert!(!root.join("invalid/executed.txt").exists());
                assert!(!root.join("missing/executed.txt").exists());
            }
        }
        "exec_missing" => {
            fs::create_dir(root.join("missing")).unwrap();
            workspace(root, "Verum.toml", &["missing"]);
            assert_exec_failure(run_in_members(), 1);
            assert!(!root.join("missing/executed.txt").exists());
        }
        _ if case.starts_with("add_") || case.starts_with("remove_") => {
            modify_workspace(&case, root, distinct);
        }
        _ => panic!("unknown workspace management case: {case}"),
    }
}

fn modify_workspace(case: &str, root: &Path, distinct: bool) {
    let legacy = case.ends_with("legacy");
    let filename = if legacy { "verum.toml" } else { "Verum.toml" };
    let removing = case.starts_with("remove_");
    let before = workspace(root, filename, if removing { &["member"] } else { &[] });
    let shadow = if case.ends_with("precedence") && distinct {
        println!("T1672_DISTINCT_PRECEDENCE=exercised");
        Some(workspace(root, "verum.toml", &["shadow"]))
    } else {
        if case.ends_with("precedence") {
            println!("T1672_DISTINCT_PRECEDENCE=unavailable-on-case-folded-filesystem");
        }
        None
    };
    // A lowercase member in the root-precedence add control isolates the
    // workspace write bug from the independent canonical-member discovery bug.
    let member_filename = if case == "add_precedence" {
        "verum.toml"
    } else {
        filename
    };
    member(root, "member", member_filename, "fixture-member");
    if case == "add_invalid" {
        fs::write(root.join("member/Verum.toml"), "[cog]\nname = [").unwrap();
        shadow_member(&root.join("member"), distinct);
        let error = add("member".into()).unwrap_err();
        assert!(matches!(error, CliError::ConfigParse(_)), "{error}");
        assert_eq!(fs::read(root.join(filename)).unwrap(), before.as_bytes());
        assert_members(root, &[]);
        return;
    }
    if removing {
        remove(if legacy {
            "member".into()
        } else {
            "fixture-member".into()
        })
        .unwrap();
        assert_members(root, &[]);
    } else {
        add("member".into()).unwrap();
        assert_members(root, &["member"]);
    }
    if let Some(bytes) = shadow {
        assert_eq!(fs::read(root.join("verum.toml")).unwrap(), bytes.as_bytes());
    } else {
        let names = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name == "Verum.toml" || name == "verum.toml")
            .collect::<List<_>>();
        assert_eq!(
            names.len(),
            1,
            "mutation created a competing workspace manifest"
        );
        assert_eq!(names[0], filename);
    }
}

fn shadow_member(path: &Path, distinct: bool) {
    if distinct {
        fs::write(
            path.join("verum.toml"),
            "[cog]\nname = \"shadow-member\"\nversion = \"9.9.9\"\n",
        )
        .unwrap();
        println!("T1672_DISTINCT_PRECEDENCE=exercised");
    } else {
        println!("T1672_DISTINCT_PRECEDENCE=unavailable-on-case-folded-filesystem");
    }
}

fn assert_members(root: &Path, expected: &[&str]) {
    let actual = Config::load(root).unwrap().workspace.unwrap().members;
    assert_eq!(
        actual
            .iter()
            .map(Text::as_str)
            .collect::<List<_>>()
            .as_slice(),
        expected
    );
}

fn run_in_members() -> crate::error::Result<()> {
    exec(vec![
        std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        "--exact".into(),
        EXEC_CHILD.into(),
        "--nocapture".into(),
    ])
}

fn assert_exec_failure(result: crate::error::Result<()>, failed: usize) {
    let error = result.expect_err("declared members that did not run must not yield success");
    assert_eq!(
        error.to_string(),
        format!("Command failed in {failed} members")
    );
}

fn assert_executed(root: &Path, member: &str) {
    let cwd = fs::canonicalize(root.join(member)).unwrap();
    assert_eq!(
        fs::read_to_string(root.join(member).join("executed.txt")).unwrap(),
        cwd.to_string_lossy()
    );
}

#[test]
fn executed_member_command() {
    if std::env::var(EXEC_ENV).as_deref() != Ok("1") {
        return;
    }
    let cwd = fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
    fs::write("executed.txt", cwd.to_string_lossy().as_bytes()).unwrap();
    assert!(
        !cwd.join("command-fails").exists(),
        "intentional child command failure"
    );
}
