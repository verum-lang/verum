//! Operators use the selected source call's value ownership boundary.
#![cfg(feature = "codegen")]
#[path = "fixtures/canonical_drop.rs"]
mod canonical_drop;

use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenResult, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

const DECLARATIONS: &str = r#"
mount core.base.protocols.Drop;
type Mul<Rhs = Self> is protocol {
    type Output;
    fn mul(self, rhs: Rhs) -> Self.Output;
};
type Count is { value: Int };
type Number is { count: &mut Count, value: Int };
implement Drop for Number {
    fn drop(&mut self) { self.count.value = self.count.value + 1; }
}
implement Mul for Number {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Number { count: self.count, value: self.value * rhs.value }
    }
}
fn make(count: &mut Count, value: Int) -> Number { Number { count, value } }
fn generic<T: Mul<Output = T>>(left: T, right: T) -> T { left * right }
fn generic_method<T: Mul<Output = T>>(left: T, right: T) -> T { left.mul(right) }
"#;

fn compile(source: &str, unit: bool) -> CodegenResult<VbcModule> {
    let source = format!("{DECLARATIONS}\n{source}");
    let ast = Parser::new(&source)
        .parse_module()
        .expect("operator grammar");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("operator_ownership"));
    canonical_drop::initialize(&mut codegen);
    if unit {
        codegen.collect_unit_declarations(&[&ast])?;
        codegen.compile_unit_items(&[&ast], ItemFailurePolicy::Strict)?;
        codegen.finalize_module_from_state()
    } else {
        codegen.compile_additional_module(&ast)
    }
}

fn run(source: &str) {
    for unit in [false, true] {
        let module = compile(source, unit).expect("declared operator compilation");
        let wire = verum_vbc::deserialize::deserialize_module_validated(
            &verum_vbc::serialize::serialize_module(&module).unwrap(),
        )
        .unwrap();
        for (route, module) in [("source", module), ("wire", wire)] {
            let entry = module
                .find_function_by_name("operator_ownership.probe")
                .unwrap();
            let result = Interpreter::try_new_validated(module.into())
                .unwrap()
                .execute_function(entry);
            assert_eq!(
                result.unwrap().try_as_i64(),
                Some(73),
                "unit={unit}, {route}"
            );
        }
    }
}

#[test]
fn repeated_named_operands_preserve_their_owner_and_drop_each_copy_once() {
    for (expression, expected) in [
        ("source * source", 4), ("generic(source, source)", 6),
        ("source.mul(source)", 4), ("generic_method(source, source)", 6),
    ] {
        run(&format!(
            r#"
            fn probe() -> Int {{
                let mut count = Count {{ value: 0 }};
                {{
                    let source = Number {{ count: &mut count, value: 3 }};
                    {{
                        let result = {expression};
                        if result.value != 9 {{ return 11; }}
                        if source.value != 3 {{ return 12; }}
                        if count.value != {expected} - 2 {{ return 20 + count.value; }}
                    }}
                    if count.value != {expected} - 1 {{ return 30 + count.value; }}
                    if source.value != 3 {{ return 13; }}
                }}
                if count.value == {expected} {{ 73 }} else {{ 40 + count.value }}
            }}
        "#
        ));
    }
}

#[test]
fn fresh_operator_operands_transfer_without_extra_copies() {
    for expression in [
        "make(&mut count, 3) * make(&mut count, 7)",
        "make(&mut count, 3).mul(make(&mut count, 7))",
    ] {
        run(&format!(r#"
            fn probe() -> Int {{
                let mut count = Count {{ value: 0 }};
                {{
                    let result = {expression};
                    if result.value != 21 {{ return 11; }}
                    if count.value != 2 {{ return 20 + count.value; }}
                }}
                if count.value == 3 {{ 73 }} else {{ 30 + count.value }}
            }}
        "#));
    }
}

#[test]
fn nested_operator_results_transfer_to_the_next_selected_call() {
    run(r#"
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            {
                let result = (make(&mut count, 3) * make(&mut count, 7)) * make(&mut count, 2);
                if result.value != 42 { return 11; }
                if count.value != 4 { return 20 + count.value; }
            }
            if count.value == 5 { 73 } else { 30 + count.value }
        }
    "#);
}

#[test]
fn a_borrowed_affine_operand_cannot_become_an_owned_operator_parameter() {
    let source = r#"
        type affine Ticket is { value: Int };
        implement Mul for Ticket {
            type Output = Self;
            fn mul(self, rhs: Self) -> Self { Ticket { value: self.value * rhs.value } }
        }
        fn invalid(left: &Ticket, right: &Ticket) -> Ticket { *left * *right }
    "#;
    for unit in [false, true] {
        let error = compile(source, unit).expect_err("borrowed exclusive owners must refuse");
        assert!(
            error.to_string().contains("exclusive borrowed owner"),
            "{error}"
        );
    }
}
