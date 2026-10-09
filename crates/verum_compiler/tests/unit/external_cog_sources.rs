//! T1637: registered source cogs must participate in ordinary project checking.
//! These exercise the real compiler loader and checker with isolated source trees;
//! they do not stand in for publication, installation, or executable acceptance.

use super::CompilationPipeline;
use crate::{CompilerOptions, Session, VerifyMode};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use verum_common::Text;
use verum_modules::cog_resolver::CogResolver;

struct Fixture {
    directory: TempDir,
    consumer: PathBuf,
    dependency: PathBuf,
}

fn write(path: &Path, source: &str) {
    std::fs::create_dir_all(path.parent().expect("source parent")).unwrap();
    std::fs::write(path, source).unwrap();
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let consumer = directory.path().join("consumer-checkout");
        let dependency = directory.path().join("downloaded-package");
        write(
            &consumer.join("verum.toml"),
            "[cog]\nname = \"consumer\"\nversion = \"0.1.0\"\n",
        );
        write(
            &consumer.join("src/main.vr"),
            "module consumer.main;\nmount greeting.lib.{answer};\nfn main() -> Int { answer() }\n",
        );
        write(
            &dependency.join("Verum.toml"),
            "[cog]\nname = \"greeting\"\nversion = \"1.2.3\"\n",
        );
        write(
            &dependency.join("src/lib.vr"),
            "module greeting.lib;\npublic fn answer() -> Int { 42 }\nfn secret() -> Int { 9 }\n",
        );
        Self {
            directory,
            consumer,
            dependency,
        }
    }

    fn session(&self, root: PathBuf) -> Session {
        let mut session = Session::new(CompilerOptions {
            input: self.consumer.join("src/main.vr"),
            output: self.directory.path().join("output"),
            verify_mode: VerifyMode::Runtime,
            check_only: true,
            ..Default::default()
        });
        let mut resolver = CogResolver::new();
        resolver.register_cog("greeting", "1.2.3", root);
        session.set_cog_resolver(resolver);
        session
    }
}

#[test]
fn source_directory_loads_the_declared_public_function() {
    let fixture = Fixture::new();
    let mut session = fixture.session(fixture.dependency.join("src"));
    let mut pipeline = CompilationPipeline::new_check(&mut session);
    pipeline
        .load_external_cog_modules()
        .expect("valid source cog");
    assert!(pipeline.modules.contains_key(&Text::from("greeting.lib")));
    let registry = pipeline.session.module_registry();
    let guard = registry.read();
    let module = guard
        .get_by_path("greeting.lib")
        .expect("registered library");
    assert!(module.exports.contains("answer"));
}

#[test]
fn package_directory_does_not_add_a_src_module_segment() {
    let fixture = Fixture::new();
    let mut session = fixture.session(fixture.dependency.clone());
    let mut pipeline = CompilationPipeline::new_check(&mut session);
    pipeline
        .load_external_cog_modules()
        .expect("valid archive layout");
    assert!(pipeline.modules.contains_key(&Text::from("greeting.lib")));
    assert!(
        !pipeline
            .modules
            .contains_key(&Text::from("greeting.src.lib"))
    );
}

#[test]
fn dependency_does_not_publish_its_private_function() {
    let fixture = Fixture::new();
    let mut session = fixture.session(fixture.dependency.join("src"));
    let mut pipeline = CompilationPipeline::new_check(&mut session);
    pipeline.load_external_cog_modules().unwrap();
    let registry = pipeline.session.module_registry();
    let guard = registry.read();
    let module = guard.get_by_path("greeting.lib").unwrap();
    assert!(module.exports.contains("answer"), "positive public control");
    assert!(
        !module.exports.contains("secret"),
        "private item crossed cog boundary"
    );
}

#[test]
fn missing_registered_source_directory_is_an_error() {
    let fixture = Fixture::new();
    let mut session = fixture.session(fixture.directory.path().join("missing"));
    let result = CompilationPipeline::new_check(&mut session).load_external_cog_modules();
    assert!(
        result.is_err(),
        "registered missing package was silently skipped"
    );
}

#[test]
fn malformed_dependency_source_is_an_error() {
    let fixture = Fixture::new();
    write(
        &fixture.dependency.join("src/lib.vr"),
        "public fn answer( {\n",
    );
    let mut session = fixture.session(fixture.dependency.join("src"));
    let result = CompilationPipeline::new_check(&mut session).load_external_cog_modules();
    assert!(
        result.is_err(),
        "invalid package source was silently skipped"
    );
}

#[test]
fn colliding_dependency_module_files_are_an_error() {
    let fixture = Fixture::new();
    write(
        &fixture.dependency.join("src/lib/mod.vr"),
        "public fn different() -> Int { 7 }\n",
    );
    let mut session = fixture.session(fixture.dependency.join("src"));
    let result = CompilationPipeline::new_check(&mut session).load_external_cog_modules();
    assert!(
        result.is_err(),
        "lib.vr and lib/mod.vr silently chose one module"
    );
}

#[test]
fn project_check_uses_the_registered_dependency_signature() {
    let fixture = Fixture::new();
    let mut session = fixture.session(fixture.dependency.join("src"));
    let result = CompilationPipeline::new_check(&mut session)
        .check_project()
        .expect("project check completed");
    assert_eq!(result.user_errors, 0, "{}", session.format_diagnostics());

    write(
        &fixture.consumer.join("src/main.vr"),
        "module consumer.main;\nmount greeting.lib.{answer};\nfn main() -> Bool { answer() }\n",
    );
    let mut session = fixture.session(fixture.dependency.join("src"));
    let result = CompilationPipeline::new_check(&mut session)
        .check_project()
        .expect("negative project check completed");
    assert!(
        result.user_errors > 0,
        "dependency return type was not checked"
    );
    assert!(
        !session.format_diagnostics().contains("E402"),
        "dependency was not loaded"
    );
}

#[test]
fn isolated_project_probe_keeps_dependencies_and_leaves_parent_symbols_alone() {
    let fixture = Fixture::new();
    let mut session = fixture.session(fixture.dependency.join("src"));
    let mut pipeline = CompilationPipeline::new_check(&mut session);
    pipeline
        .ensure_project_type_checked()
        .expect("project probe completed");
    assert!(
        !pipeline.session.has_errors(),
        "{}",
        pipeline.session.format_diagnostics()
    );
    assert!(
        pipeline.modules.is_empty(),
        "probe populated parent module state"
    );
    assert!(
        pipeline
            .session
            .module_registry()
            .read()
            .get_by_path("greeting.lib")
            .is_none()
    );
}

#[test]
fn source_loader_leaves_precompiled_cogs_to_the_archive_path() {
    let fixture = Fixture::new();
    let mut session = fixture.session(fixture.dependency.join("src"));
    let mut resolver = CogResolver::new();
    resolver.register_cog_vbca(
        "compiled",
        "1.0.0",
        fixture.directory.path().join("compiled.vbca"),
    );
    session.set_cog_resolver(resolver);
    let mut pipeline = CompilationPipeline::new_check(&mut session);
    pipeline
        .load_external_cog_modules()
        .expect("archive handled by its own path");
    assert!(pipeline.modules.is_empty());
}
