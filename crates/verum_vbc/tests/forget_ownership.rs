//! The source wrapper transfers its actual argument to the non-dropping sink.
#![cfg(feature = "codegen")]
#[path = "fixtures/canonical_drop.rs"]
mod canonical_drop;

use verum_ast::ItemKind;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

#[test]
fn forgetting_one_affine_owner_preserves_its_siblings_destructor() {
    let mut memory = Parser::new(include_str!("../../../core/intrinsics/memory.vr"))
        .parse_module()
        .unwrap();
    memory.items.retain(|item| matches!(&item.kind, ItemKind::Module(owner) if owner.items.is_none())
        || matches!(&item.kind, ItemKind::Function(function) if function.name.as_str() == "forget"));
    let client = Parser::new(
        r#"
        module forget_test;
        mount core.base.protocols.Drop;
        mount core.intrinsics.memory.forget;
        static mut RELEASED: Int = 0;
        type affine Ticket is { count: Int };
        implement Drop for Ticket {
            fn drop(&mut self) { unsafe { RELEASED = RELEASED + self.count; } }
        }
        fn forward<T>(value: T) { forget(value); }
        fn check() -> Int {
            {
                let kept = Ticket { count: 73 };
                let consumed = Ticket { count: 19 };
                forward(consumed);
                if unsafe { RELEASED } != 0 { return 0; }
            }
            unsafe { RELEASED }
        }
    "#,
    )
    .parse_module()
    .unwrap();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new(""));
    canonical_drop::initialize(&mut compiler);
    compiler
        .collect_unit_declarations(&[&memory, &client])
        .unwrap();
    compiler.resolve_pending_imports();
    compiler
        .compile_unit_items(&[&memory, &client], ItemFailurePolicy::Strict)
        .unwrap();
    let module = compiler.finalize_module().unwrap();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module_validated(&wire).unwrap();
    for image in [module, wire] {
        let entry = image.find_function_by_name("forget_test.check").unwrap();
        let mut vm = Interpreter::try_new_validated(std::sync::Arc::new(image)).unwrap();
        vm.run_global_ctors().unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    }
}
