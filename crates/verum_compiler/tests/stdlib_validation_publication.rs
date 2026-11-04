//! Actual archive production must not publish a failed validation as success.
use std::{fs, path::PathBuf, process::Command};
use verum_compiler::precompile::{PrecompileConfig, precompile_stdlib};

const CHILD: &str = "VERUM_TEST_STDLIB_VALIDATION_CHILD";

fn isolated(modes: &[&str]) -> bool {
    if std::env::var_os(CHILD).is_some() {
        return false;
    }
    let thread = std::thread::current();
    for mode in modes {
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                thread.name().unwrap(),
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .env_remove("VERUM_META_ENGINE");
        if mode.is_empty() {
            child.env_remove("VERUM_BAKE_VALIDATION");
        } else {
            child.env("VERUM_BAKE_VALIDATION", mode);
        }
        let output = child.output().unwrap();
        assert!(output.status.success(), "mode {mode:?}: {output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("PUBLICATION_CONTROL_COMPLETE"));
    }
    true
}

fn config(root: &std::path::Path) -> PrecompileConfig {
    let core = root.join("core");
    fs::create_dir(&core).unwrap();
    PrecompileConfig {
        stdlib_path: core,
        output_path: root.join("runtime.vbca"),
        target_triple: None,
        verbose: false,
    }
}

fn artifacts(config: &PrecompileConfig) -> [PathBuf; 4] {
    [
        config.output_path.clone(),
        config.output_path.with_extension("core_metadata"),
        config.output_path.with_extension("symbol_graph"),
        config.output_path.with_added_extension("receipt"),
    ]
}

#[test]
fn failed_validation_preserves_published_archive_and_sidecars() {
    if isolated(&["", "report", "strict", "verify"]) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    // An open generic telescope is not executed before instantiation. The
    // closed scalar computation below is evaluated and must succeed.
    fs::write(config.stdlib_path.join("mod.vr"),
        "meta fn deferred<T>() -> Int { 1 / 0 } meta fn ready() -> Int { (6 * 7) } public fn answer() -> Int { 42 }").unwrap();
    let produced = precompile_stdlib(&config).unwrap();
    assert!(produced.errors.is_empty());
    let before: Vec<_> = artifacts(&config)
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path).unwrap();
            assert!(!bytes.is_empty(), "{}", path.display());
            (path, bytes)
        })
        .collect();
    fs::write(
        config.stdlib_path.join("mod.vr"),
        "meta fn invalid() -> Int { 1 / (1 - 1) } public fn answer() -> Int { 42 }",
    )
    .unwrap();
    let error = precompile_stdlib(&config).unwrap_err();
    let message = format!("{error:#}");
    assert!(
        message.contains("stdlib validation failed in core"),
        "{message}"
    );
    assert!(message.contains("1 meta failures"), "{message}");
    for (path, bytes) in before {
        assert_eq!(
            fs::read(&path).unwrap(),
            bytes,
            "{} changed after refusal",
            path.display()
        );
    }
    println!("PUBLICATION_CONTROL_COMPLETE");
}

#[test]
fn invalid_modes_cannot_disable_mandatory_validation() {
    if isolated(&["off", "unrecognized"]) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    fs::write(
        config.stdlib_path.join("mod.vr"),
        "public fn answer() -> Int { 42 }",
    )
    .unwrap();
    let error = precompile_stdlib(&config).unwrap_err();
    assert!(format!("{error:#}").contains("mandatory stdlib validation cannot be disabled"));
    assert!(artifacts(&config).iter().all(|path| !path.exists()));
    println!("PUBLICATION_CONTROL_COMPLETE");
}
