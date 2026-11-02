//! A published stdlib archive must implement every active source body.
use std::fs;
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};

#[test]
fn failed_source_body_prevents_archive_publication() {
    for declaration in [
        "public fn broken() -> Int { undefined_value }",
        "type A is { n: Int }; implement A { fn broken(&self) -> Int { undefined_value } }",
        "public module inner { public fn broken() -> Int { undefined_value } }",
        "type P is protocol { fn broken(&self) -> Int { undefined_value } }; type A is { n: Int }; implement P for A {}",
    ] {
        let root = tempfile::tempdir().unwrap();
        let core = root.path().join("core");
        fs::create_dir(&core).unwrap();
        let input = core.join("mod.vr");
        fs::write(&input, format!("module core; {declaration}")).unwrap();
        let output = root.path().join("runtime.vbca");
        let config = CoreConfig::new(&core).with_output(&output);
        let mut session = Session::new(CompilerOptions {
            input,
            ..Default::default()
        });
        let error = CompilationPipeline::new_core(&mut session, config)
            .compile_core()
            .expect_err(declaration);
        let error = format!("{error:#}");
        assert!(
            error.contains("broken") && error.contains("undefined_value"),
            "{declaration}: {error}"
        );
        assert!(!output.exists(), "{declaration} published {output:?}");
    }
}

#[test]
fn malformed_source_file_prevents_publication_of_its_siblings() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir(&core).unwrap();
    let input = core.join("mod.vr");
    fs::write(
        &input,
        "module core; public module sibling; fn ok() -> Int { 7 }",
    )
    .unwrap();
    fs::write(
        core.join("sibling.vr"),
        "module core.sibling; public fn unfinished(",
    )
    .unwrap();
    let output = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input,
        ..Default::default()
    });
    let error =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
            .compile_core()
            .expect_err("invalid source file");
    assert!(format!("{error:#}").contains("sibling.vr"), "{error:#}");
    assert!(!output.exists());
}

#[test]
fn missing_file_mount_prevents_archive_publication() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir(&core).unwrap();
    let input = core.join("mod.vr");
    fs::write(&input, "module core; mount ./missing.vr;").unwrap();
    let output = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input,
        ..Default::default()
    });
    let error =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
            .compile_core()
            .expect_err("required mount must resolve");
    assert!(format!("{error:#}").contains("missing.vr"), "{error:#}");
    assert!(!output.exists());
}
