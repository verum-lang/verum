//! Exception ownership must survive recovery, propagation and value handoff.
#![cfg(feature = "codegen")]
#[path = "fixtures/canonical_drop.rs"]
mod canonical_drop;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module_validated,
    interpreter::Interpreter,
    serialize::serialize_module,
};

fn check(source: &str) {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("exception_ownership"));
    canonical_drop::initialize(&mut codegen);
    let module = codegen.compile_additional_module(&ast).expect("source VBC");
    let wire = deserialize_module_validated(&serialize_module(&module).unwrap()).unwrap();
    for (route, module) in [("source", module), ("wire", wire)] {
        let entry = module
            .find_function_by_name("exception_ownership.probe")
            .unwrap();
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        assert_eq!(
            vm.execute_function(entry).expect(route).try_as_i64(),
            Some(73)
        );
        assert_eq!(
            vm.state.heap.stats().total_frees,
            vm.state.heap.stats().total_allocs,
            "all exception payload owners must be retired, {route}"
        );
    }
}

#[test]
fn a_recovered_owned_value_can_escape_the_handler() {
    check(
        r#"
        mount core.base.protocols.Drop;
        type Count is { value: Int };
        type Payload is { count: &mut Count, value: Int };
        implement Drop for Payload {
            fn drop(&mut self) { self.count.value = self.count.value + 1; }
        }
        fn probe()->Int {
            let mut count = Count { value: 0 };
            {
                let recovered: Payload = try {
                    throw Payload { count: &mut count, value: 73 }
                } recover |payload| { payload };
                if count.value != 0 { return 1; }
                if recovered.value != 73 { return 2; }
            }
            if count.value == 1 { 73 } else { 3 }
        }
    "#,
    );
}
