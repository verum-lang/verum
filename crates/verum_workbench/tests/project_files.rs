use serde_json::json;
use std::{path::Path, process::Command};
use verum_workbench::{Config, Studio, project_git, workspace::Workspace};

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=Studio Test",
            "-c",
            "user.email=studio@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn lazy_tree_exposes_project_text_without_traversing_hidden_or_symlink_paths() {
    let dir = tempfile::tempdir().unwrap();
    for path in ["src/deep", "empty", ".private", "target"] {
        std::fs::create_dir_all(dir.path().join(path)).unwrap();
    }
    for path in [
        "README.md",
        "src/deep/main.vr",
        ".private/secret.vr",
        "target/generated.vr",
    ] {
        std::fs::write(dir.path().join(path), "content").unwrap();
    }
    let workspace = Workspace::open(dir.path()).unwrap();
    let listing = workspace.browse("").unwrap();
    assert_eq!(
        serde_json::to_value(&listing).unwrap()["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["empty", "src", "README.md"]
    );
    assert_eq!(
        workspace.browse("src").unwrap().entries[0].path.as_str(),
        "src/deep"
    );
    assert_eq!(
        workspace.search_files("MAIN").unwrap().entries[0]
            .path
            .as_str(),
        "src/deep/main.vr"
    );
    for path in ["../", ".private", "/tmp"] {
        assert!(workspace.browse(path).is_err(), "{path}");
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.path().join(".private"), dir.path().join("linked")).unwrap();
        assert!(workspace.browse("linked").is_err());
        assert_eq!(workspace.browse("").unwrap().entries.len(), 3);
    }
    let first = workspace.read_document("README.md").unwrap();
    workspace
        .write_document("README.md", "updated", first["hash"].as_str())
        .unwrap();
    assert!(
        workspace
            .write_document("README.md", "stale", first["hash"].as_str())
            .is_err()
    );
    assert!(workspace.read("README.md").is_err()); // Verum agent/source capability stays narrow.
    std::fs::write(dir.path().join("binary.dat"), b"a\0b").unwrap();
    assert!(workspace.read_document("binary.dat").is_err());
    assert!(workspace.read_document("src").is_err());
}

#[test]
fn path_search_reports_truncation_and_keeps_deep_directories_discoverable() {
    let dir = tempfile::tempdir().unwrap();
    for index in 0..205 {
        std::fs::write(dir.path().join(format!("match-{index}.txt")), "text").unwrap();
    }
    let deep = "nested/".repeat(20);
    std::fs::create_dir_all(dir.path().join(&deep)).unwrap();
    std::fs::write(dir.path().join(format!("{deep}proof.vr")), "text").unwrap();
    let workspace = Workspace::open(dir.path()).unwrap();
    let found = workspace.search_files("match-").unwrap();
    assert!(found.truncated);
    assert_eq!(found.entries.len(), 200);
    let found = workspace.search_files("proof.vr").unwrap();
    assert!(!found.truncated);
    assert_eq!(found.entries.len(), 1);
}

#[tokio::test]
async fn actual_git_preserves_index_worktree_rename_deletion_and_nested_workspace_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir(root.join("app")).unwrap();
    for path in [
        "outside.vr",
        "app/both.vr",
        "app/old name.vr",
        "app/deleted.vr",
        "app/[literal].vr",
    ] {
        std::fs::write(root.join(path), "base\n").unwrap();
    }
    git(root, &["init", "-q"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "base"]);
    std::fs::write(root.join("app/both.vr"), "staged\n").unwrap();
    git(root, &["add", "app/both.vr"]);
    std::fs::write(root.join("app/both.vr"), "working\n").unwrap();
    git(root, &["mv", "app/old name.vr", "app/new name.vr"]);
    std::fs::remove_file(root.join("app/deleted.vr")).unwrap();
    std::fs::write(root.join("app/new\nfile.vr"), "new\n").unwrap();
    std::fs::write(root.join("outside.vr"), "outside change\n").unwrap();
    std::fs::write(root.join("app/[literal].vr"), "literal changed\n").unwrap();
    let app = root.join("app");
    let snapshot = project_git::changes(&app).await;
    assert_eq!(snapshot.state, "ready", "{:?}", snapshot.error);
    let find = |path: &str| {
        snapshot
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .unwrap()
    };
    assert_eq!(find("both.vr").index, "M");
    assert_eq!(find("both.vr").worktree, "M");
    assert_eq!(
        find("new name.vr").original_path.as_deref(),
        Some("old name.vr")
    );
    assert_eq!(find("deleted.vr").worktree, "D");
    assert_eq!(find("new\nfile.vr").index, "?");
    assert!(
        !snapshot
            .entries
            .iter()
            .any(|entry| entry.path.contains("outside"))
    );
    let index = project_git::diff(&app, "both.vr", "index").await.unwrap();
    let working = project_git::diff(&app, "both.vr", "worktree")
        .await
        .unwrap();
    assert!(index.contains("+staged"));
    assert!(!index.contains("+working"));
    assert!(working.contains("-staged"));
    assert!(working.contains("+working"));
    let literal = project_git::diff(&app, "[literal].vr", "worktree")
        .await
        .unwrap();
    assert!(literal.contains("+literal changed"));
    assert!(!literal.contains("both.vr"));
    assert!(
        project_git::diff(&app, "../outside.vr", "index")
            .await
            .is_err()
    );
    assert!(project_git::diff(&app, "both.vr", "HEAD").await.is_err());
}

#[tokio::test]
async fn no_repository_is_distinct_from_clean_and_tools_share_the_browse_contract() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        project_git::changes(dir.path()).await.state,
        "not_repository"
    );
    std::fs::write(dir.path().join("cog.toml"), "name = \"example\"\n").unwrap();
    let studio = Studio::open(Config {
        workspace: dir.path().into(),
        verum: "unused".into(),
        proof: "unused".into(),
        codex: "unused".into(),
        port: 0,
    })
    .unwrap();
    let result = studio.call("workspace.browse", json!({})).await.unwrap();
    assert_eq!(result["entries"][0]["path"], "cog.toml");
    for method in ["document.write", "workspace.write"] {
        for invalid in [json!(false), json!(1), json!({})] {
            assert!(studio.call(method, json!({"path":"new.vr","source":"","expected_hash":invalid})).await.is_err());
        }
    }
    assert!(!dir.path().join("new.vr").exists());
    assert!(
        studio
            .call("document.write", json!({"path":"cog.toml","source":"bad"}))
            .await
            .is_err()
    );
    for name in ["verum_files", "verum_changes"] {
        assert!(
            verum_workbench::codex::tools()
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == name)
        );
    }
    assert!(project_git::parse_status(b"R  incomplete\0", "").is_err());
}
