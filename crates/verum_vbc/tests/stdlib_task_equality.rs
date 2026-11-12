#![cfg(feature = "codegen")]
//! Execute unchanged task equality bodies, including the serialized VBC path.
//! Scheduler startup and full stdlib admission are separate acceptance gates.
use std::sync::Arc;
use verum_ast::{ItemKind, decl::ImplKind, ty::TypeKind};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

#[test]
fn task_equality_observes_identity_and_join_error_variant() {
    let mut protocols = Parser::new(include_str!("../../../core/base/protocols.vr"))
        .parse_module()
        .unwrap();
    protocols.items.retain(|item| matches!(&item.kind, ItemKind::Module(_))
        || matches!(&item.kind, ItemKind::Type(decl) if matches!(decl.name.as_str(), "PartialEq" | "Eq")));

    let mut tasks = Parser::new(include_str!("../../../core/async/task.vr"))
        .parse_module()
        .unwrap();
    tasks.items.retain(|item| match &item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Type(decl) => matches!(decl.name.as_str(), "TaskId" | "JoinError"),
        ItemKind::Impl(decl) => matches!(&decl.kind,
            ImplKind::Protocol { protocol, for_type, .. }
                if protocol.as_ident().is_some_and(|name| matches!(name.as_str(), "PartialEq" | "Eq"))
                && matches!(&for_type.kind, TypeKind::Path(path)
                    if path.as_ident().is_some_and(|name| matches!(name.as_str(), "TaskId" | "JoinError")))),
        _ => false,
    });
    // This factory lives in the declaring owner so private TaskId storage stays
    // private. The observed equality methods above come from the actual source.
    tasks.items.extend(
        Parser::new(
            r#"
        mount core.base.protocols.{PartialEq, Eq};
        public fn equal_ids(a: Int, b: Int) -> Bool {
            let first = TaskId { id: a };
            let second = TaskId { id: b };
            first.eq(&second)
        }
    "#,
        )
        .parse_module()
        .unwrap()
        .items,
    );
    let consumer = Parser::new(
        r#"
        module task_equality_probe;
        mount core.async.task.{JoinError, equal_ids};
        fn run() -> Int {
            if !equal_ids(42, 42) { return 1; }
            if equal_ids(42, 43) { return 2; }
            if !equal_ids(-1, -1) { return 3; }
            let cancelled = JoinError.Cancelled;
            let panicked = JoinError.Panicked;
            if !cancelled.eq(&cancelled) { return 4; }
            if !panicked.eq(&panicked) { return 5; }
            if cancelled.eq(&panicked) { return 6; }
            73
        }
    "#,
    )
    .parse_module()
    .unwrap();
    let units = [&protocols, &tasks, &consumer];
    let mut generator = VbcCodegen::with_config(CodegenConfig::new("main").with_validation());
    generator.collect_unit_declarations(&units).unwrap();
    generator
        .compile_unit_items(&units, ItemFailurePolicy::Strict)
        .unwrap();
    let module = generator.finalize_module_from_state().unwrap();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    for module in [module, decoded] {
        let entry = module
            .find_function_by_name("task_equality_probe.run")
            .unwrap();
        let result = Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .unwrap();
        assert_eq!(result.try_as_i64(), Some(73));
    }
}
