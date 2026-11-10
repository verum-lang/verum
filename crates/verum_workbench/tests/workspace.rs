use serde_json::json;
use verum_workbench::{Config, Studio, server::authorized, workspace::Workspace};

#[test]
fn edits_require_the_revision_actually_read_and_cannot_escape() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::open(directory.path()).unwrap();
    let first = workspace.write("lesson.vr", "let n = 1;", None).unwrap();
    assert!(workspace.write("lesson.vr", "overwritten", None).is_err());
    let next = workspace
        .write("lesson.vr", "let n = 2;", first["hash"].as_str())
        .unwrap();
    assert!(
        workspace
            .write("lesson.vr", "stale", first["hash"].as_str())
            .is_err()
    );
    assert_eq!(workspace.read("lesson.vr").unwrap(), next);
    for path in ["../outside.vr", "/tmp/outside.vr", ".git/x.vr", "lesson.py"] {
        assert!(workspace.path(path).is_err());
    }
}
#[cfg(unix)]
#[test]
fn symlink_sources_and_directories_are_not_workspace_capabilities() {
    let directory = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    std::fs::write(other.path().join("private.vr"), "secret").unwrap();
    std::os::unix::fs::symlink(other.path(), directory.path().join("outside")).unwrap();
    std::os::unix::fs::symlink(
        other.path().join("private.vr"),
        directory.path().join("link.vr"),
    )
    .unwrap();
    let workspace = Workspace::open(directory.path()).unwrap();
    assert!(workspace.read("outside/private.vr").is_err());
    assert!(workspace.read("link.vr").is_err());
    assert_eq!(workspace.list().unwrap()["files"], json!([]));
}

#[test]
fn paged_inventory_crosses_the_old_file_and_depth_limits_in_path_order() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let deep = "nested/".repeat(24) + "end.vr";
    let mut expected = vec![
        "a.vr".to_owned(),
        "a/b.vr".into(),
        "a0.vr".into(),
        deep.clone(),
    ];
    for i in 0..2050 {
        expected.push(format!("pkg/{:02}/part{i:04}.vr", i % 16));
    }
    for file in expected
        .iter()
        .map(String::as_str)
        .chain([".hidden/private.vr", "pkg/target/build.vr"])
    {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }
    expected.sort();
    let workspace = Workspace::open(root).unwrap();
    let first = workspace.list().unwrap();
    let paths = |page: &serde_json::Value| {
        page["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["path"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(paths(&first), expected[..2000]);
    let cursor = first["next"].as_str().unwrap();
    assert_eq!(cursor, expected[1999]);
    std::fs::remove_file(root.join(cursor)).unwrap();
    let second = workspace.source_page(Some(cursor), 2000).unwrap();
    assert_eq!(paths(&second), expected[2000..]);
    assert!(second["next"].is_null());
    assert_eq!(
        paths(&workspace.source_page(Some("a.vr"), 1).unwrap()),
        ["a/b.vr"]
    );
    for cursor in ["../x.vr", "/x.vr", ".hidden/file.vr", "a//b.vr"] {
        assert!(workspace.source_page(Some(cursor), 1).is_err());
    }
    assert!(workspace.source_page(None, 0).is_err());
    assert!(workspace.source_page(None, 2001).is_err());
}
#[test]
fn local_api_rejects_cross_origin_and_dns_rebinding() {
    assert!(authorized(
        "127.0.0.1:123",
        Some("http://127.0.0.1:123"),
        Some("127.0.0.1:123"),
        Some("Bearer secret"),
        "secret"
    ));
    assert!(!authorized(
        "127.0.0.1:123",
        Some("https://evil.invalid"),
        Some("127.0.0.1:123"),
        Some("Bearer secret"),
        "secret"
    ));
    assert!(!authorized(
        "127.0.0.1:123",
        None,
        Some("evil.invalid:123"),
        Some("Bearer secret"),
        "secret"
    ));
    assert!(!authorized(
        "127.0.0.1:123",
        None,
        Some("127.0.0.1:123"),
        Some("Bearer wrong"),
        "secret"
    ));
}
#[tokio::test]
async fn client_cannot_omit_compare_and_swap_or_invent_a_tool() {
    let directory = tempfile::tempdir().unwrap();
    let studio = Studio::open(Config {
        workspace: directory.path().into(),
        verum: "missing".into(),
        proof: "missing".into(),
        codex: "missing".into(),
        port: 0,
    })
    .unwrap();
    assert!(
        studio
            .call("workspace.write", json!({"path":"lesson.vr","source":""}))
            .await
            .is_err()
    );
    assert!(
        studio
            .call("shell", json!({"command":"true"}))
            .await
            .is_err()
    );
}
