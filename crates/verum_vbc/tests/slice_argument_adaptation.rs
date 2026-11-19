//! T2842: selected slice formals preserve a borrow's logical data and extent.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn compile(source: &str, unit: bool) -> Result<VbcModule, verum_vbc::codegen::CodegenError> {
    let ast = Parser::new(source).parse_module().unwrap();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("slice_argument"));
    if unit {
        compiler.collect_unit_declarations(&[&ast])?;
        compiler.compile_unit_items(&[&ast], ItemFailurePolicy::Strict)?;
        compiler.finalize_module_from_state()
    } else {
        compiler.compile_module(&ast)
    }
}

fn run(source: &str, expected: i64) {
    for unit in [false, true] {
        let module = compile(source, unit).unwrap_or_else(|error| panic!("unit={unit}: {error}"));
        let wire = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&module).unwrap(),
        )
        .unwrap();
        for module in [module, wire] {
            verum_vbc::validate::validate_module(&module).unwrap();
            let entry = module
                .find_function_by_name("slice_argument.probe")
                .unwrap();
            let value = Interpreter::new(module.into())
                .execute_function(entry)
                .unwrap();
            assert_eq!(value.try_as_i64(), Some(expected), "unit={unit}");
        }
    }
}

#[test]
fn inferred_and_packed_arrays_reach_the_same_slice_contract() {
    for binding in [
        "let values = [7, 3, 1];",
        "let values: [Int; 3] = [7, 3, 1];",
        "let values: List<Int> = [7, 3, 1];",
    ] {
        run(
            &format!(
                r#"
            fn inspect(values: &[Int]) -> Int {{ values.len() + values[0] + values[2] }}
            fn probe() -> Int {{ {binding} inspect(&values) }}
        "#
            ),
            11,
        );
    }
}

#[test]
fn mutable_slice_argument_updates_the_original_collection() {
    for binding in [
        "let mut values = [7, 3, 1];",
        "let mut values: [Int; 3] = [7, 3, 1];",
    ] {
        run(
            &format!(
                r#"
            fn replace(values: &mut [Int]) -> Int {{ values[1] = 19; values.len() }}
            fn probe() -> Int {{ {binding} let count = replace(&mut values); count + values[1] }}
        "#
            ),
            22,
        );
    }
}

#[test]
fn inherited_method_borrows_an_inferred_fixed_buffer() {
    run(
        r#"
        type Reader is protocol {
            fn read(&mut self, bytes: &mut [Byte]) -> Int;
            fn buffered(&mut self) -> Int {
                let mut tmp = [0_u8; 1024];
                let count = self.read(&mut tmp);
                count + (tmp[1023] as Int)
            }
        };
        type Source is { marker: Byte };
        implement Reader for Source {
            fn read(&mut self, bytes: &mut [Byte]) -> Int {
                bytes[1023] = self.marker;
                bytes.len()
            }
        }
        fn probe() -> Int { let mut source = Source { marker: 73_u8 }; source.buffered() }
        "#,
        1097,
    );
}

#[test]
fn existing_subslices_keep_their_offset_when_forwarded() {
    run(
        r#"
        fn inspect(values: &[Int]) -> Int { values.len() + values[0] }
        fn forward(values: &[Int]) -> Int { inspect(values) }
        fn probe() -> Int {
            let values = [71, 7, 3, 1];
            forward(&values[1..3])
        }
    "#,
        9,
    );
}

#[test]
fn generic_slice_formals_bind_the_actual_element_before_materialization() {
    for call in [
        "measure(&values)",
        "measure<Int>(&values)",
        "forward(&values)",
    ] {
        run(
            &format!(
                r#"
            fn measure<T>(values: &[T]) -> Int {{ values.len() }}
            fn forward<T>(values: &[T]) -> Int {{ measure(values) }}
            fn probe() -> Int {{ let values = [7, 3, 1]; {call} }}
        "#
            ),
            3,
        );
    }
}

#[test]
fn saved_array_and_list_borrows_keep_their_extent_and_data() {
    for binding in [
        "let values = [7, 3, 1];",
        "let values: [Int; 3] = [7, 3, 1];",
        "let values: List<Int> = [7, 3, 1];",
    ] {
        run(
            &format!(
                r#"
            fn inspect(values: &[Int]) -> Int {{ values.len() + values[0] + values[2] }}
            fn probe() -> Int {{
                {binding}
                let view_ref = &values;
                inspect(view_ref)
            }}
        "#
            ),
            11,
        );
    }
}

#[test]
fn declared_aliases_preserve_the_same_borrow_adaptation() {
    for binding in [
        "let values: Words = [7, 3, 1];",
        "let values: Bag = [7, 3, 1];",
    ] {
        run(
            &format!(
                r#"
            type Words is [Int; 3];
            type Bag is List<Int>;
            type View is &[Int];
            fn inspect(values: View) -> Int {{ values.len() + values[0] + values[2] }}
            fn probe() -> Int {{ {binding} let view_ref = &values; inspect(view_ref) }}
        "#
            ),
            11,
        );
    }
}

#[test]
fn generic_slice_arguments_cannot_disagree_on_the_element() {
    let source = r#"
        fn same<T>(left: &[T], right: &[T]) -> Int { left.len() + right.len() }
        fn probe() -> Int {
            let left = [1, 2];
            let right = [true, false];
            same(&left, &right)
        }
    "#;
    for unit in [false, true] {
        let error = compile(source, unit).expect_err("conflicting slice witnesses were accepted");
        assert!(
            error.to_string().contains("generic slice argument"),
            "unit={unit}: {error}"
        );
    }
}

#[test]
fn ordinary_record_references_do_not_become_slices() {
    run(
        r#"
        type Storage is { value: Int };
        fn read(value: &Storage) -> Int { value.value }
        fn probe() -> Int { let value = Storage { value: 73 }; read(&value) }
    "#,
        73,
    );
}

#[test]
fn incompatible_elements_mutability_and_lookalike_storage_are_rejected() {
    // Establish the homonymous record through an ordinary reference before
    // testing slice refusal. Its initializer uses contextual construction.
    run(
        r#"
        type List<T> is { item: T };
        fn read(value: &List<Int>) -> Int { value.item }
        fn probe() -> Int { let value: List<Int> = List { item: 73 }; read(&value) }
    "#,
        73,
    );
    for (source, reason) in [
        (
            r#"fn accept(value: &mut [Byte]) -> Int { value.len() }
            fn probe() -> Int { let mut value = [0_i8; 8]; accept(&mut value) }"#,
            "element differs",
        ),
        (
            r#"fn accept(value: &mut [Byte]) -> Int { value.len() }
            fn probe() -> Int { let value = [0_u8; 8]; accept(&value) }"#,
            "mutable source borrow",
        ),
        (
            r#"fn accept(value: &[Int]) -> Int { value.len() }
            fn probe() -> Int { let value = [true, false]; accept(&value) }"#,
            "element differs",
        ),
        (
            r#"fn accept(value: &mut [Int]) -> Int { value.len() }
            fn probe() -> Int { let value = [1, 2]; accept(&value) }"#,
            "mutable source borrow",
        ),
        (
            r#"type List<T> is { item: T };
            fn accept(value: &[Int]) -> Int { value.len() }
            fn probe() -> Int { let value: List<Int> = List { item: 1 }; accept(&value) }"#,
            "supported collection borrow",
        ),
    ] {
        for unit in [false, true] {
            let error =
                compile(source, unit).expect_err("an incompatible source acquired slice authority");
            assert!(error.to_string().contains(reason), "unit={unit}: {error}");
        }
    }
}
