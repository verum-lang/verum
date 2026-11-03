use verum_common::{List, Shared, Text};
use verum_types::TypeError;

fn suggestions(target: &str, candidates: &[&str]) -> Text {
    let diagnostic = TypeError::ImportItemNotFound {
        item_name: target.into(),
        module_path: "project.api".into(),
        available_items: Shared::new(candidates.iter().map(|name| Text::from(*name)).collect()),
        declared_in: List::new(),
        span: verum_ast::Span::default(),
    }
    .to_diagnostic();
    assert_eq!(diagnostic.code(), Some("E401"));
    diagnostic
        .helps()
        .iter()
        .find(|help| help.message.starts_with("did you mean"))
        .map(|help| help.message.clone())
        .unwrap_or_default()
}

#[test]
fn import_hints_keep_prefix_candidates_exact_scores_and_stable_ties() {
    assert_eq!(
        suggestions("cat", &["catalogue", "cut", "cart", "care", "unrelated"]).as_str(),
        "did you mean one of these? `cut`, `cart`, `care`"
    );
    assert_eq!(
        suggestions("cat", &["catalogue", "c", "unrelated"]).as_str(),
        "did you mean one of these? `c`, `catalogue`"
    );
    assert_eq!(
        suggestions("Жук", &["Жукология", "жак", "лук", "unrelated"]).as_str(),
        "did you mean one of these? `жак`, `лук`, `Жукология`"
    );
    assert!(suggestions("missing", &["completely_unrelated"]).is_empty());
}

#[test]
fn module_suggestions_are_ranked_at_rendering_and_notes_are_not_names() {
    let error = TypeError::ImportModuleNotFound {
        module_path: "cat".into(),
        available_modules: ["catalogue", "cut", "cart", "care", "unrelated"]
            .into_iter().map(Text::from).collect(),
        // Even a single word is a note; its role is not inferred from spaces.
        notes: [Text::from("cat")].into_iter().collect(),
        span: verum_ast::Span::default(),
    };
    let TypeError::ImportModuleNotFound { available_modules, .. } = &error else { unreachable!() };
    assert_eq!(available_modules.len(), 5);
    let diagnostic = error.to_diagnostic();
    assert_eq!(diagnostic.code(), Some("E402"));
    assert_eq!(diagnostic.helps()[0].message.as_str(),
        "did you mean one of these modules? `cut`, `cart`, `care`");
    assert!(diagnostic.notes().iter().any(|note| note.message.as_str() == "cat"));
}

#[test]
fn actual_module_resolution_preserves_failure_candidates_and_keyword_notes() {
    use verum_ast::ItemKind;
    use verum_fast_parser::Parser;
    use verum_modules::{ModuleId, ModuleInfo, ModulePath, ModuleRegistry};
    let provider = "module core.util;";
    let mut registry = ModuleRegistry::new();
    registry.register(ModuleInfo::new(ModuleId::new(0), ModulePath::from_str("core.util"),
        Parser::new(provider).parse_module().unwrap(), verum_ast::FileId::new(0), provider.into()));
    let mut checker = verum_types::TypeChecker::new();
    checker.set_current_module_path("project.main");
    checker.set_module_registry_direct(registry.clone());
    for (source, valid, keyword) in [
        ("mount core.util.*;", true, false),
        ("mount core.utli.*;", false, false),
        ("mount cog.utli.*;", false, true),
    ] {
        let ast = Parser::new(source).parse_module().unwrap();
        let ItemKind::Mount(import) = &ast.items[0].kind else { unreachable!() };
        let result = checker.process_import(import, "project.main", &registry);
        if valid {
            assert!(result.is_ok(), "{source}: {result:?}");
            continue;
        }
        let error = result.unwrap_err();
        let TypeError::ImportModuleNotFound { available_modules, notes, .. } = &error else {
            panic!("wrong resolution error: {error:?}")
        };
        assert!(available_modules.iter().any(|name| name.as_str() == "core.util"));
        assert_eq!(!notes.is_empty(), keyword);
        let diagnostic = error.to_diagnostic();
        assert_eq!(diagnostic.code(), Some("E402"));
        if keyword {
            assert!(diagnostic.notes().iter().any(|note| note.message.contains("path keyword")));
        } else {
            assert!(diagnostic.helps().iter().any(|help| help.message.contains("`core.util`")));
        }
    }
}
