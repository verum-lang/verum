//! T2800: declaration owners and source units survive public checking.
//! Coherence diagnostics are currently advisory, even in strict mode. These
//! controls assert their presence, not a compile refusal that does not exist.

use super::*;
use crate::{CompilerOptions, Session, VerifyMode};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use verum_fast_parser::VerumParser;
use verum_modules::ModuleInfo;

const IMPLEMENTATION: &str = r#"
module fixture.model;
public type Tagged is protocol { fn tag(&self) -> Int; };
public type Token is { public value: Int };
implement Tagged for Token { fn tag(&self) -> Int { self.value } }
"#;

fn fixture() -> TempDir {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("Verum.toml"),
        "[cog]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::create_dir(directory.path().join("src")).unwrap();
    directory
}

fn write(directory: &TempDir, name: &str, source: &str) -> PathBuf {
    let path = directory.path().join("src").join(name);
    std::fs::write(&path, source).unwrap();
    path
}

fn new_session(input: &Path) -> Session {
    Session::new(CompilerOptions {
        input: input.to_owned(),
        check_only: true,
        verify_mode: VerifyMode::Runtime,
        ..Default::default()
    })
}

fn load(session: &Session, path: &Path) -> Module {
    let id = session.load_file(path).unwrap();
    let source = session.get_source(id).unwrap();
    VerumParser::new()
        .parse_module_str(source.source.as_str(), id)
        .unwrap()
}

fn coherence_messages(session: &Session) -> List<Text> {
    session
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.message().contains("[coherence]"))
        .map(|diagnostic| diagnostic.message().into())
        .collect()
}

#[test]
fn declared_owner_preserves_qualified_short_and_headerless_sources() {
    for (source, selected, expected) in [
        ("module actual.model;", "fixture.physical", "actual.model"),
        ("module model;", "fixture.model", "fixture.model"),
        (
            "public type Token is Int;",
            "fixture.model",
            "fixture.model",
        ),
    ] {
        let module = verum_fast_parser::Parser::new(source)
            .parse_module()
            .unwrap();
        assert_eq!(
            crate::owned_source_semantics::declared_owner(&module, selected).as_str(),
            expected
        );
    }
}

#[test]
fn one_physical_source_is_registered_once_across_entry_and_both_maps() {
    let directory = fixture();
    let path = write(&directory, "model.vr", IMPLEMENTATION);
    let mut session = new_session(&path);
    let module = load(&session, &path);
    std::fs::create_dir(path.parent().unwrap().join("alias")).unwrap();
    let alias_path = path.parent().unwrap().join("alias/../model.vr");
    assert_eq!(session.load_file(&alias_path).unwrap(), module.file_id);
    // Independent in-memory snapshots may still reach coherence under distinct IDs.
    let alias_id = session.load_source_string(IMPLEMENTATION, alias_path).unwrap();
    let alias = VerumParser::new().parse_module_str(IMPLEMENTATION, alias_id).unwrap();
    assert_ne!(module.file_id, alias_id);
    let mut pipeline = CompilationPipeline::new_check(&mut session);
    pipeline
        .modules
        .insert("library.model".into(), Arc::new(module.clone()));
    pipeline
        .project_modules
        .insert("project.model".into(), Arc::new(alias));
    pipeline
        .check_protocol_coherence(&module, "fixture.model")
        .unwrap();
    assert!(
        coherence_messages(pipeline.session).is_empty(),
        "{}",
        pipeline.session.format_diagnostics()
    );
}

#[test]
fn distinct_files_with_the_same_owner_keep_their_overlap_diagnostic() {
    let directory = fixture();
    let entry_path = write(
        &directory,
        "main.vr",
        "module fixture.main; fn main() -> Int { 0 }",
    );
    let left = write(&directory, "left.vr", IMPLEMENTATION);
    let right = write(&directory, "right.vr", IMPLEMENTATION);
    let mut session = new_session(&entry_path);
    let entry = load(&session, &entry_path);
    let left = load(&session, &left);
    let right = load(&session, &right);
    assert_ne!(left.file_id, right.file_id);
    let mut pipeline = CompilationPipeline::new_check(&mut session);
    pipeline
        .modules
        .insert("fixture.left".into(), Arc::new(left));
    pipeline
        .project_modules
        .insert("fixture.right".into(), Arc::new(right));
    pipeline
        .check_protocol_coherence(&entry, "fixture.main")
        .unwrap();
    let messages = coherence_messages(pipeline.session);
    assert!(
        messages
            .iter()
            .any(|message| message.contains("Overlapping implementations")
                && message.contains("fixture.model")),
        "{messages:?}"
    );
}

#[test]
fn distinct_virtual_sources_and_same_file_implementations_are_not_deduplicated() {
    let directory = fixture();
    let input = directory.path().join("virtual.vr");
    let mut session = new_session(&input);
    let first = session
        .load_source_string(IMPLEMENTATION, input.clone())
        .unwrap();
    let second = session.load_source_string(IMPLEMENTATION, input).unwrap();
    let parser = VerumParser::new();
    let entry = parser.parse_module_str(IMPLEMENTATION, first).unwrap();
    let other = parser.parse_module_str(IMPLEMENTATION, second).unwrap();
    let mut pipeline = CompilationPipeline::new_check(&mut session);
    pipeline
        .project_modules
        .insert("fixture.model".into(), Arc::new(other));
    pipeline
        .check_protocol_coherence(&entry, "fixture.model")
        .unwrap();
    assert!(
        coherence_messages(pipeline.session)
            .iter()
            .any(|message| message.contains("Overlapping implementations"))
    );

    let path = write(
        &directory,
        "repeated.vr",
        &format!("{IMPLEMENTATION}\nimplement Tagged for Token {{ fn tag(&self) -> Int {{ 0 }} }}"),
    );
    let mut session = new_session(&path);
    let entry = load(&session, &path);
    let pipeline = CompilationPipeline::new_check(&mut session);
    pipeline
        .check_protocol_coherence(&entry, "fixture.model")
        .unwrap();
    assert!(
        coherence_messages(pipeline.session)
            .iter()
            .any(|message| message.contains("Overlapping implementations"))
    );
}

#[test]
fn absolute_public_entry_uses_the_declared_owner_in_registry_and_coherence() {
    let directory = fixture();
    let path = write(&directory, "physical.vr", IMPLEMENTATION);
    assert!(path.is_absolute());
    let mut session = new_session(&path);
    let result = CompilationPipeline::new_check(&mut session).run_check_only();
    assert!(
        result.is_ok() && !session.has_errors(),
        "{result:?}\n{}",
        session.format_diagnostics()
    );
    let registry = session.module_registry();
    let registry = registry.read();
    let actual = registry
        .get_by_path("fixture.model")
        .expect("explicit header owns entry declarations");
    assert_eq!(
        session.get_source(actual.file_id).unwrap().path.as_deref(),
        Some(path.as_path())
    );
    assert!(
        registry.get_by_path("fixture.physical").is_none(),
        "physical filename invented another owner"
    );
    assert!(
        coherence_messages(&session)
            .iter()
            .all(|message| !message.contains("fixture.model")),
        "{}",
        session.format_diagnostics()
    );
}

#[test]
fn public_entry_cannot_replace_a_different_source_with_the_same_owner() {
    for registered_source in [true, false] {
        let directory = fixture();
        let original_path = write(&directory, "original.vr", IMPLEMENTATION);
        let entry_path = write(&directory, "replacement.vr", IMPLEMENTATION);
        let mut session = new_session(&entry_path);
        let original = load(&session, &original_path);
        let registry = session.module_registry();
        let owner = ModulePath::from_str("fixture.model");
        let id = registry.read().allocate_id();
        let mut info = ModuleInfo::new(
            id,
            owner.clone(),
            original.clone(),
            original.file_id,
            session.get_source(original.file_id).unwrap().source.clone(),
        );
        if !registered_source {
            info.ast.file_id = verum_ast::FileId::dummy();
        }
        info.exports = verum_modules::extract_exports_from_module(&original, id, &owner).unwrap();
        registry.write().register(info);
        let result = CompilationPipeline::new_check(&mut session).run_check_only();
        let error = result.expect_err("different source units cannot claim the same owner");
        assert!(
            format!("{error:#}").contains("conflicting source parts for owner fixture.model"),
            "{error:#}"
        );
        assert_eq!(
            registry
                .read()
                .get_by_path("fixture.model")
                .unwrap()
                .file_id,
            original.file_id
        );
    }
}

#[test]
fn prefetched_source_cannot_reuse_another_revisions_spans() {
    let directory = fixture();
    let path = write(&directory, "model.vr", IMPLEMENTATION);
    let session = new_session(&path);
    let id = session.load_file_snapshot(&path, Some(IMPLEMENTATION)).unwrap();
    assert_eq!(session.load_file(&path).unwrap(), id);
    let other = IMPLEMENTATION.replace("self.value", "self.other");
    assert_eq!(IMPLEMENTATION.len(), other.len());
    assert!(session.load_file_snapshot(&path, Some(&other)).is_err());
    assert_eq!(session.get_source(id).unwrap().source.as_str(), IMPLEMENTATION);
    assert_eq!(session.stats().num_files, 1);
}

#[test]
fn different_revisions_at_the_same_physical_path_are_not_deduplicated() {
    let directory = fixture();
    let path = write(&directory, "model.vr", IMPLEMENTATION);
    let mut session = new_session(&path);
    let entry = load(&session, &path);
    let changed = IMPLEMENTATION.replace("self.value", "42        ");
    assert_eq!(changed.len(), IMPLEMENTATION.len());
    let revision = session.load_source_string(&changed, path).unwrap();
    let other = VerumParser::new()
        .parse_module_str(&changed, revision)
        .unwrap();
    let mut pipeline = CompilationPipeline::new_check(&mut session);
    pipeline
        .project_modules
        .insert("fixture.model".into(), Arc::new(other));
    let error = pipeline
        .check_protocol_coherence(&entry, "fixture.model")
        .expect_err("a newer source snapshot must not be discarded as a file alias");
    assert!(format!("{error:#}").contains("conflicting source snapshots"));
}

#[test]
fn inline_declarations_in_the_same_file_are_distinct_source_units() {
    let directory = fixture();
    let path = write(
        &directory,
        "model.vr",
        &format!(
            "{IMPLEMENTATION}\npublic module child {{ implement Tagged for Token {{ fn tag(&self) -> Int {{ 0 }} }} }}"
        ),
    );
    let mut session = new_session(&path);
    let entry = load(&session, &path);
    let item = entry
        .items
        .iter()
        .find(|item| {
            matches!(
                &item.kind, verum_ast::ItemKind::Module(declaration) if declaration.items.is_some()
            )
        })
        .unwrap();
    let verum_ast::ItemKind::Module(declaration) = &item.kind else {
        unreachable!()
    };
    let child = Module::new(declaration.items.clone().unwrap(), entry.file_id, item.span);
    let mut pipeline = CompilationPipeline::new_check(&mut session);
    pipeline
        .project_modules
        .insert("fixture.model.child".into(), Arc::new(child));
    pipeline
        .check_protocol_coherence(&entry, "fixture.model")
        .unwrap();
    assert!(
        coherence_messages(pipeline.session)
            .iter()
            .any(|message| message.contains("Overlapping implementations")
                && message.contains("fixture.model.child")),
        "{}",
        pipeline.session.format_diagnostics()
    );
}
