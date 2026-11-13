//! Reference operations keep their source carrier independently of ambient names.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn run(source: &str) {
    let ast = Parser::new(source).parse_module().unwrap();
    for unit_entry in [false, true] {
        let mut compiler = VbcCodegen::with_config(CodegenConfig::new("app"));
        let module = if unit_entry {
            compiler.collect_unit_declarations(&[&ast]).unwrap();
            compiler
                .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
                .unwrap();
            compiler.finalize_module_from_state().unwrap()
        } else {
            compiler.compile_module(&ast).unwrap()
        };
        let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
        let module = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
        verum_vbc::validate::validate_module(&module).unwrap();
        let entry = module.find_function_by_name("app.main").unwrap();
        let result = Interpreter::new(module.into())
            .execute_function(entry)
            .unwrap_or_else(|error| panic!("unit_entry={unit_entry}: {error:?}"));
        assert_eq!(result.try_as_i64(), Some(73), "unit_entry={unit_entry}");
    }
}

#[test]
fn returned_and_block_bound_raw_pointers_keep_null_semantics() {
    run(r#"
        module app;
        type Cell<T> is { value: T };
        fn address<T>(value: Int) -> &unsafe Cell<T> { value as &unsafe Cell<T> }
        fn inspect<T>(value: Int) -> Bool {
            let pointer = unsafe { address<T>(value) as &unsafe Cell<T> };
            pointer.is_null()
        }
        fn main() -> Int {
            assert(inspect<Int>(0));
            assert(!inspect<Bool>(73));
            73
        }
    "#);
}

#[test]
fn foreign_free_and_nominal_homonyms_do_not_replace_reference_operations() {
    run(r#"
        module app;
        type Cell is { value: Int };
        implement Cell { fn is_null(&self) -> Bool { true } }
        fn is_null<T>(_pointer: &unsafe T) -> Bool { true }
        fn inspect(pointer: &unsafe Cell) -> Bool { pointer.is_null() }
        fn main() -> Int {
            let value = Cell { value: 73 };
            assert(value.is_null());
            assert(!inspect(73 as &unsafe Cell));
            assert(inspect(0 as &unsafe Cell));
            assert(is_null(73 as &unsafe Cell));
            73
        }
    "#);
}

#[test]
fn checked_and_managed_references_inspect_the_reference_itself() {
    run(r#"
        module app;
        type Cell is { value: Int };
        implement Cell { fn is_null(&self) -> Bool { true } }
        fn managed(value: &Cell) -> Bool { value.is_null() }
        fn checked_pointer(value: &checked Cell) -> Bool { value.is_null() }
        fn main() -> Int {
            let value = Cell { value: 73 };
            assert(value.is_null());
            assert(!managed(&value));
            assert(!checked_pointer(&checked value));
            73
        }
    "#);
}
