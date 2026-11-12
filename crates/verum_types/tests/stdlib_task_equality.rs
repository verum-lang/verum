//! Equality evidence from the actual task declarations and protocol headers.
//! This focused contract does not stand in for whole-scheduler execution.
use verum_ast::{FileId, ItemKind, Module, decl::ImplKind, ty::TypeKind};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;

fn parse(source: &str, id: u32) -> Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(id))
        .unwrap()
}

fn sources() -> List<(&'static str, Module, &'static str)> {
    let protocol_source = include_str!("../../../core/base/protocols.vr");
    let mut protocols = parse(protocol_source, 1);
    protocols.items.retain(|item| {
        matches!(&item.kind,
        ItemKind::Type(declaration) if matches!(declaration.name.as_str(), "PartialEq" | "Eq"))
    });

    let task_source = include_str!("../../../core/async/task.vr");
    let mut tasks = parse(task_source, 2);
    tasks.items.retain(|item| match &item.kind {
        ItemKind::Type(declaration) => matches!(declaration.name.as_str(), "TaskId" | "JoinError"),
        ItemKind::Impl(implementation) => matches!(&implementation.kind,
            ImplKind::Protocol { protocol, for_type, .. }
                if protocol.as_ident().is_some_and(|name| matches!(name.as_str(), "PartialEq" | "Eq"))
                && matches!(&for_type.kind, TypeKind::Path(path)
                    if path.as_ident().is_some_and(|name| matches!(name.as_str(), "TaskId" | "JoinError")))),
        _ => false,
    });
    // Bind the actual prelude declarations explicitly in this small graph.
    drop(tasks.items.splice(
        0..0,
        parse("mount core.base.protocols.{PartialEq, Eq};", 2).items,
    ));
    let consumer_source = r#"
        mount core.base.protocols.{PartialEq, Eq};
        mount core.async.task.{TaskId, JoinError};
        fn partial<T: PartialEq>(value: T) {}
        fn total<T: Eq>(value: T) {}
        fn probe(id: TaskId, error: JoinError) {
            partial(id); total(id); partial(error); total(error);
        }
    "#;
    List::from_iter([
        ("core.base.protocols", protocols, protocol_source),
        ("core.async.task", tasks, task_source),
        ("app.main", parse(consumer_source, 3), consumer_source),
    ])
}

#[test]
fn task_identity_and_join_error_publish_partial_and_total_equality() {
    let sources = sources();
    let mut registry = ModuleRegistry::new();
    for (index, (owner, module, source)) in sources.iter().enumerate() {
        let id = ModuleId::new(index as u32 + 1);
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(
            id,
            path.clone(),
            module.clone(),
            module.file_id,
            (*source).into(),
        );
        info.exports = extract_exports_from_module(module, id, &path).unwrap();
        registry.register(info);
    }
    let mut errors = List::<Text>::new();
    for (owner, module, _) in &sources {
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_primitives();
        checker.set_current_module_path(*owner);
        checker.set_module_registry_direct(registry.clone());
        errors.extend(
            checker
                .prepare_module_declarations(module)
                .iter()
                .map(|error| Text::from(format!("{owner}: {error:?}"))),
        );
        for item in &module.items {
            if let Err(error) = checker.check_item(item) {
                errors.push(format!("{owner}: {error:?}").into());
            }
        }
        errors.extend(
            checker
                .take_deferred_errors()
                .iter()
                .map(|error| Text::from(format!("{owner}: {error:?}"))),
        );
        errors.extend(
            checker
                .diagnostic_sources()
                .iter()
                .map(|error| Text::from(format!("{owner}: {error:?}"))),
        );
    }
    assert!(errors.is_empty(), "{errors:#?}");
}
