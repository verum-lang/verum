//! Source admission must not read files excluded by stdlib discovery.
//! The selected module is the complete production SemVer source; this test
//! observes loader I/O and source identity, not SemVer runtime acceptance.

use super::CompilationPipeline;
use crate::{CompilerOptions, Session};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SOURCE: &str = include_str!("../../../../core/base/semver.vr");
const CHILD_ROOT: &str = "VERUM_SOURCE_STDLIB_IO_ROOT";
const TEST: &str =
    "pipeline::source_stdlib_io_tests::source_loader_does_not_read_excluded_artifacts";

#[test]
fn source_loader_does_not_read_excluded_artifacts() {
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = PathBuf::from(root);
        let mut session = Session::new(CompilerOptions::default());
        let mut pipeline = CompilationPipeline::new_interpreter(&mut session);
        pipeline.load_stdlib_modules_scoped(None).unwrap();
        let registry = pipeline.session.module_registry();
        let registry = registry.read();
        assert_eq!(registry.len(), 1);
        let module = registry.get_by_path("core.base.semver").unwrap();
        assert_eq!(module.source.as_str(), SOURCE);
        let source = pipeline.session.get_source(module.ast.file_id).unwrap();
        assert_eq!(source.source.as_str(), SOURCE);
        assert_eq!(source.path.as_ref(), Some(&root.join("base/semver.vr")));
        return;
    }

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("core");
    std::fs::create_dir_all(root.join("base")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("base/semver.vr"), SOURCE).unwrap();
    // A read of this excluded build artifact cannot finish: no process opens
    // the FIFO for writing. Supervise a child so regressions fail boundedly.
    assert!(
        Command::new("mkfifo")
            .arg(root.join("target/excluded.vr"))
            .status()
            .unwrap()
            .success()
    );
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
        .env(CHILD_ROOT, &root)
        .env("VERUM_STDLIB_PATH", &root)
        .env_remove("VERUM_NO_DISK_CACHE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let timed_out = loop {
        if child.try_wait().unwrap().is_some() {
            break false;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            break true;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output = child.wait_with_output().unwrap();
    assert!(
        !timed_out && output.status.success(),
        "source loader read an excluded artifact or lost source identity: timeout={timed_out}, status={}\n{}\n{}",
        output.status,
        verum_common::Text::from_utf8_lossy(&output.stdout),
        verum_common::Text::from_utf8_lossy(&output.stderr),
    );
}
