//! A nominal runtime layout can carry several distinct method implementations.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::VbcCodegen, deserialize::deserialize_module, interpreter::Interpreter,
    serialize::serialize_module, validate::validate_module,
};

#[test]
fn specialized_self_selects_its_own_body_in_either_declaration_order() {
    let integer = "implement Measure for Cell<Int> { fn width(&self) -> Int { 41 } }";
    let boolean = "implement Measure for Cell<Bool> { fn width(&self) -> Int { 73 } }";
    for (first, second) in [(integer, boolean), (boolean, integer)] {
        let source = format!(
            r#"
            protocol Measure {{ fn width(&self) -> Int; }}
            type Cell<T> is {{ value: T }};
            {first}
            {second}
            fn width_int(c: &Cell<Int>) -> Int {{ c.width() }}
            fn width_bool(c: Cell<Bool>) -> Int {{ c.width() }}
            fn probe() -> Int {{
                let x: Cell<Int> = Cell {{ value: 0 }};
                let y: Cell<Bool> = Cell {{ value: true }};
                width_int(&x) * 100 + width_bool(y)
            }}
        "#
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        let module = VbcCodegen::new().compile_module(&ast).unwrap();
        let mut module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        assert!(module.resolve_external_bands().is_empty());
        validate_module(&module).unwrap();
        let entry = module.find_function_by_name("probe").unwrap();
        let result = Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .unwrap();
        assert_eq!(result.as_i64(), 4173, "{first}");
    }
}

#[test]
fn a_nonmatching_instantiation_cannot_reuse_the_first_body() {
    let ast = Parser::new(
        r#"
        type Cell<T> is { value: T };
        implement Cell<Int> { fn width(&self) -> Int { 1 } }
        implement Cell<Bool> { fn width(&self) -> Int { 2 } }
        fn invalid(c: Cell<Text>) -> Int { c.width() }
    "#,
    )
    .parse_module()
    .unwrap();
    let err = VbcCodegen::new().compile_module(&ast).unwrap_err();
    assert!(
        err.to_string()
            .contains("does not select a unique implementation"),
        "{err}"
    );
}
