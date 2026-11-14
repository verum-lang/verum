//! Actual local readers bind the declared file API; no ambient context is supplied.
use super::*;
use crate::core_compiler::StdlibModuleResolver;
use crate::phases::semantic_analysis::SemanticAnalysisPhase;
use verum_ast::decl::{ImplItemKind, ImplKind};
use verum_fast_parser::FastParser;

fn check_actual_reader(owner: &str, expected_bodies: usize) -> List<verum_types::TypeError> {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let target = session.cfg_evaluator().config().clone();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core");
    let mut resolver = StdlibModuleResolver::with_target(&root, target.clone());
    resolver.discover().unwrap();
    let mut roster = List::new();
    for group in resolver.modules_in_order() {
        for path in &group.source_files {
            let text = std::fs::read_to_string(path).unwrap();
            let id = session.load_source_string(&text, path.clone()).unwrap();
            let module = FastParser::new().parse_module_str(&text, id).unwrap();
            let owner = crate::module_utils::source_file_module_path(&group.name, path, &module);
            roster.push((
                Text::from(owner),
                Text::from(path.to_string_lossy().as_ref()),
                module,
            ));
        }
    }
    let input: List<_> = roster
        .iter()
        .map(|(owner, path, ast)| (owner.as_str(), path.as_str(), ast))
        .collect();
    let selected = select_source_roster(&input, &target).unwrap();
    let registry = source_registry(
        selected
            .iter()
            .map(|(owner, path, ast)| (owner.as_str(), path.as_str(), ast)),
    )
    .unwrap();
    {
        let (_, _, original) = selected
            .iter()
            .find(|(module, _, _)| module == owner)
            .unwrap();
        let mut module = original.clone();
        let mut checked_bodies = 0;
        // Keep every source declaration in the registry. Select only the bodies
        // whose I/O wiring this test owns; unrelated HTTP/archive-format bodies
        // have their own checks. No signature or body is replaced.
        module.items.retain_mut(|item| match &mut item.kind {
            ItemKind::Function(function) if owner == "core.cog.archive" && function.name.name == "read_archive" => {
                checked_bodies += 1;
                true
            }
            ItemKind::Impl(declaration) if matches!(&declaration.kind, ImplKind::Protocol {for_type, ..} if matches!(&for_type.kind, verum_ast::ty::TypeKind::Path(path) if path.to_string() == "LocalRepository")) => {
                declaration.items.retain(|item| match &item.kind {
                    ImplItemKind::Function(function) if ["fetch_metadata", "fetch_target"].contains(&function.name.name.as_str()) => {
                        checked_bodies += 1;
                        true
                    }
                    _ => false,
                });
                true
            }
            ItemKind::Function(_) | ItemKind::Impl(_) => false,
            _ => true,
        });
        assert_eq!(
            checked_bodies, expected_bodies,
            "actual bodies selected for {owner}"
        );
        let mut checker = SemanticAnalysisPhase::new()
            .with_language_features(session.language_features())
            .with_target(target.clone())
            .configured_checker();
        checker.set_current_cog("core");
        checker.set_current_module_path(owner);
        checker.set_module_registry(registry.clone());
        checker.register_stdlib_types_for_module(original);
        let mut declarations = original.clone();
        verum_ast::prelude::inject_implicit_prelude_mount(&mut declarations);
        if let Some(header) = declarations.file_header_index() {
            declarations.items.remove(header);
        }
        // Prepare the complete original scope, including read_archive_bytes
        // and the role limit helper. Only the subsequent body check is focused.
        let mut errors = checker.prepare_module_declarations(&declarations);
        if let Some(header) = module.file_header_index() {
            module.items.remove(header);
        }
        if errors.is_empty() {
            for item in &module.items {
                if let Err(error) = checker.check_item(item) {
                    errors.push(error);
                    break;
                }
            }
        }
        errors.extend(checker.take_deferred_errors());
        assert!(
            checker.diagnostic_sources().is_empty(),
            "unexpected diagnostic channel: {:#?}",
            checker.diagnostic_sources()
        );
        errors
    }
}

#[test]
fn archive_file_reader_preserves_the_unproved_decoder_borrow_refusal() {
    // File API wiring is checked, but the archive decoder still lacks a proved
    // owned-result contract (T2633). Do not erase this remaining obligation.
    let errors = check_actual_reader("core.cog.archive", 1);
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert!(
        matches!(&errors[0], verum_types::TypeError::DanglingReference { var, .. } if var == "bytes"),
        "{errors:#?}"
    );
}

#[test]
fn local_repository_readers_use_the_actual_stdlib_declarations() {
    let errors = check_actual_reader("core.security.tuf.repository", 2);
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn a_context_requirement_needs_its_exact_imported_declaration() {
    let provider =
        Parser::new("module realm.provider; public context Directory { fn read() -> Int; }")
            .parse_module()
            .unwrap();
    for (mount, allowed) in [("mount realm.provider.Directory;", true), ("", false)] {
        let consumer = Parser::new(&format!("module realm.consumer; {mount} fn read() -> Int using [Directory] {{ Directory.read() }}")).parse_module().unwrap();
        let registry = source_registry([
            ("realm.provider", "provider.vr", &provider),
            ("realm.consumer", "consumer.vr", &consumer),
        ])
        .unwrap();
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_builtins();
        let result = check_source_module(&mut checker, "realm.consumer", &consumer, &registry);
        if allowed {
            result.unwrap_or_else(|errors| panic!("explicit declaration refused: {errors:#?}"));
        } else {
            let errors = result
                .expect_err("an unmounted foreign context cannot satisfy a local requirement");
            assert!(
                errors.iter().any(|error| error.code() == Some("E605")),
                "{errors:#?}"
            );
        }
    }
    let module = Parser::new(
        "module realm.empty; fn read() -> Int using [MissingDirectory] { MissingDirectory.read() }",
    )
    .parse_module()
    .unwrap();
    let registry = source_registry([("realm.empty", "empty.vr", &module)]).unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    let errors = check_source_module(&mut checker, "realm.empty", &module, &registry)
        .expect_err("unknown context accepted");
    assert!(
        errors.iter().any(|error| error.code() == Some("E605")),
        "{errors:#?}"
    );
}
