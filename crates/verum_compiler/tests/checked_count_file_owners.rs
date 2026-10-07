//! File headers name the enclosing checked-count owner, not a nested child.
use verum_compiler::{CompilationPipeline, CompilerOptions, OutputFormat, Session, VerifyMode};

#[test]
fn file_header_keeps_its_qualified_checked_count() {
    for count in ["outer.inner.CAP", "cog.outer.inner.CAP"] {
        let project = tempfile::TempDir::new().unwrap();
        std::fs::write(
            project.path().join("Verum.toml"),
            "[package]\nname=\"count_header\"\n",
        )
        .unwrap();
        let directory = project.path().join("src/outer");
        std::fs::create_dir_all(&directory).unwrap();
        let input = directory.join("inner.vr");
        std::fs::write(&input, format!("module outer.inner; const CAP: Int=3; fn measure<T>()->Int {{T.size}} fn probe()->Int {{measure<[Byte; {count}]>()}}")).unwrap();
        let mut session = Session::new(CompilerOptions {
            input,
            verify_mode: VerifyMode::Runtime,
            output_format: OutputFormat::Human,
            check_only: true,
            ..Default::default()
        });
        let result = CompilationPipeline::new(&mut session).run_check_only();
        assert!(
            result.is_ok(),
            "{count}: {result:?}\n{}",
            session.format_diagnostics()
        );
    }
}
