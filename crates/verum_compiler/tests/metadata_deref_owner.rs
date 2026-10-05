//! The production metadata's exact nominal owner must retain its Deref impl.
use std::sync::Arc;
use verum_ast::ItemKind;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, core_metadata::CoreMetadata};

#[test]
fn shared_atomic_methods_resolve_from_the_embedded_metadata() {
    let metadata: CoreMetadata = bincode::deserialize(include_bytes!(concat!(
        env!("OUT_DIR"),
        "/stdlib_runtime.core_metadata"
    )))
    .expect("coherent embedded metadata");
    let module = Parser::new(
        r#"
        mount core.base.memory.{Shared};
        mount core.sync.atomic.{AtomicBool, MemoryOrdering};
        fn probe()->Bool {
            let shared=Shared.new(AtomicBool.new(true));
            let old=shared.swap(false,MemoryOrdering.SeqCst);
            shared.load(MemoryOrdering.SeqCst)
        }
    "#,
    )
    .parse_module()
    .unwrap();
    let mut checker = TypeChecker::new_with_core(Arc::new(metadata));
    checker.register_stdlib_types_for_module(&module);
    for item in &module.items {
        if let ItemKind::Function(decl) = &item.kind {
            checker.register_function_signature(decl).unwrap();
        }
    }
    let mut errors: List<Text> = module
        .items
        .iter()
        .filter_map(|item| {
            checker
                .check_item(item)
                .err()
                .map(|e| Text::from(format!("{e:?}")))
        })
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    assert!(errors.is_empty(), "{errors:?}");
}
