//! Source tooling must accept bounded input and diagnose excessive nesting
//! through the actual CLI, without a caller-provided enlarged worker stack.
use std::process::Command;

fn nested_type(depth: usize) -> verum_common::Text {
    format!(
        "type Envelope<T> is {{ value: T }};\ntype Nested is {}Int{};\n",
        "Envelope<".repeat(depth), ">".repeat(depth),
    ).into()
}

#[test]
fn ordinary_workers_parse_format_and_admit_nested_sources() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    std::fs::create_dir(&core).unwrap();
    let source = core.join("mod.vr");
    std::fs::write(&source, nested_type(60)).unwrap();
    std::fs::write(root.path().join("verum.toml"), "[cog]\nname = \"nested\"\nversion = \"0.1.0\"\n").unwrap();
    let formatted = Command::new(env!("CARGO_BIN_EXE_verum"))
        .env_remove("RUST_MIN_STACK")
        .args(["fmt", "--threads", "2", "--on-parse-error", "error"])
        .current_dir(root.path()).output().unwrap();
    assert!(formatted.status.success(), "{}", verum_common::Text::from_utf8_lossy(&formatted.stderr));
    let archive = root.path().join("nested.vbca");
    let admitted = Command::new(env!("CARGO_BIN_EXE_verum"))
        .env_remove("RUST_MIN_STACK")
        .args(["stdlib", "precompile", "--stdlib-path"])
        .arg(&core).arg("--out").arg(&archive)
        .current_dir(root.path()).output().unwrap();
    // Parsing and elaboration accept this depth. The archive signature codec
    // has its own tighter structural budget: retain this case as a graceful
    // refusal until the codec traversal is aligned with elaboration.
    assert_eq!(admitted.status.code(), Some(1));
    let diagnostic = verum_common::Text::from_utf8_lossy(&admitted.stderr);
    assert!(diagnostic.contains("source signature exceeds its structural budget"), "{diagnostic}");
    assert!(!archive.exists());

    std::fs::write(&source, nested_type(12)).unwrap();
    let admitted = Command::new(env!("CARGO_BIN_EXE_verum"))
        .env_remove("RUST_MIN_STACK")
        .args(["stdlib", "precompile", "--stdlib-path"])
        .arg(&core).arg("--out").arg(&archive)
        .current_dir(root.path()).output().unwrap();
    assert!(admitted.status.success(), "{}", verum_common::Text::from_utf8_lossy(&admitted.stderr));
    assert!(archive.is_file());

    std::fs::write(&source, nested_type(256)).unwrap();
    let refused_archive = root.path().join("refused.vbca");
    let refused = Command::new(env!("CARGO_BIN_EXE_verum"))
        .env_remove("RUST_MIN_STACK")
        .args(["stdlib", "precompile", "--stdlib-path"])
        .arg(&core).arg("--out").arg(&refused_archive)
        .current_dir(root.path()).output().unwrap();
    assert_eq!(refused.status.code(), Some(1), "{}", verum_common::Text::from_utf8_lossy(&refused.stderr));
    let diagnostic = verum_common::Text::from_utf8_lossy(&refused.stderr);
    assert!(diagnostic.contains("nesting") || diagnostic.contains("recursion"), "{diagnostic}");
    assert!(!refused_archive.exists());
}
