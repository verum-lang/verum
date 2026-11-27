#![cfg(feature = "codegen")]
#[path = "fixtures/canonical_drop.rs"]
mod canonical_drop;

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::deserialize::deserialize_module;
use verum_vbc::interpreter::Interpreter;
use verum_vbc::serialize::serialize_module;

#[test]
fn record_update_keeps_nested_values_and_retires_displaced_fields() {
    let source = r#"
        module spread;
        mount core.base.protocols.Drop;
        type Payload is { content: Text, number: Int };
        type Title is Empty | Named(Payload);
        type Options is { title: Title, border: Int };
        implement Options {
            fn with_border(self, border: Int) -> Options { Options { border, ..self } }
        }
        static mut DESTROYED: Int = 0;
        type Guard is { weight: Int };
        implement Drop for Guard {
            fn drop(&mut self) { unsafe { DESTROYED = DESTROYED + self.weight; } }
        }
        type affine Owner is { guard: Guard, number: Int };
        fn probe() -> Int {
            let item: Options = Options {
                title: Title.Named(Payload { content: "retained", number: 7 }), border: 0
            };
            let after: Options = item.with_border(1).with_border(2);
            match &after.title {
                Title.Named(p) => { assert(p.number == 7 && p.content == "retained"); }
                Title.Empty => return 0,
            }
            match &item.title {
                Title.Named(p) => { assert(p.number == 7 && item.border == 0); }
                Title.Empty => return 0,
            }
            unsafe { DESTROYED = 0; }
            {
                let held: Owner = Owner { guard: Guard { weight: 1 }, number: 5 };
                let next: Owner = Owner { guard: Guard { weight: 4 }, ..held };
                assert(unsafe { DESTROYED } == 1);
                assert(next.guard.weight == 4 && next.number == 5);
            }
            if after.border == 2 && unsafe { DESTROYED } == 5 { 1 } else { 0 }
        }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let mut generator = VbcCodegen::with_config(CodegenConfig::new("spread"));
    canonical_drop::initialize(&mut generator);
    let module = generator.compile_module(&ast).unwrap();
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, wire] {
        let entry = module
            .functions
            .iter()
            .find(|f| module.get_string(f.name) == Some("spread.probe"))
            .expect("qualified probe")
            .id;
        let mut vm = Interpreter::try_new_validated(Arc::new(module)).unwrap();
        vm.run_global_ctors().unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(1));
    }
}

#[test]
fn record_update_rejects_a_different_nominal_base() {
    let ast = Parser::new(
        r#"
        module spread;
        type A is { n: Int };
        type B is { n: Int };
        fn bad(value: B) -> A { A { n: 1, ..value } }
    "#,
    )
    .parse_module()
    .unwrap();
    let error = VbcCodegen::with_config(CodegenConfig::new("spread"))
        .compile_module(&ast)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("record update base has a different declared owner"),
        "{error}"
    );
}
