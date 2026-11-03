//! Wrapper constructors retain generic witnesses without bypassing CallM.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

#[test]
fn wrapper_static_initializer_and_generic_forwarder_receive_witnesses() {
    for wrapper in ["Shared", "Heap"] {
        let source = format!(
            r#"
module wrapper_witness;
type Payload is {{ value: Int }};
type {wrapper}<T> is {{ value: T }};
implement<T> {wrapper}<T> {{
    fn new(value: T) -> {wrapper}<T> {{ {wrapper} {{ value: value }} }}
}}
static ROOT: {wrapper}<Payload> = {wrapper}.new(Payload {{ value: 42 }});
fn forward<T>(value: T) -> {wrapper}<T> {{ {wrapper}.new(value) }}
fn probe() -> Int {{
    let local = forward<Payload>(Payload {{ value: 17 }});
    42
}}
"#
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        for bootstrap in [false, true] {
            let mut codegen = VbcCodegen::with_config(CodegenConfig::new("wrapper_witness"));
            let module = if bootstrap {
                codegen.collect_unit_declarations(&[&ast]).unwrap();
                codegen
                    .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
                    .unwrap();
                codegen.finalize_module_from_state().unwrap()
            } else {
                codegen.compile_module(&ast).unwrap()
            };
            let module = verum_vbc::deserialize::deserialize_module(
                &verum_vbc::serialize::serialize_module(&module).unwrap(),
            )
            .unwrap();
            let probe = module
                .find_function_by_name("wrapper_witness.probe")
                .unwrap();
            let mut interpreter = Interpreter::new(Arc::new(module));
            interpreter.run_global_ctors().unwrap();
            assert_eq!(
                interpreter.execute_function(probe).unwrap().as_i64(),
                42,
                "{wrapper}, bootstrap={bootstrap}"
            );
        }
    }
}

#[test]
fn dereferenced_arguments_forward_the_declared_protocol_target() {
    use verum_vbc::{Instruction, bytecode::decode_instructions, types::TypeRef};

    let source = r#"module core.base.protocols;
        type Deref is protocol { type Target; fn deref(&self) -> &Self.Target; };
        type Other is protocol { type Target; };
        type Project<A, B> is { context: A, item: B };
        implement<A, B> Other for Project<A, B> { type Target = A; }
        implement<A, B> Deref for Project<A, B> {
            type Target = B;
            fn deref(&self) -> &B { &self.item }
        }
        type Measure is protocol { fn measure(&self) -> Int; };
        type Payload is { value: Int };
        implement Measure for Payload { fn measure(&self) -> Int { self.value } }
        fn inner<X, Y: Measure>(stream: &mut X, value: &Y) -> Int { value.measure() }
        fn forward<T, S: Measure>(mut stream: T, app: Project<Int, S>) -> Int {
            inner(&mut stream, &*app)
        }
        fn probe() -> Int { forward(7, Project { context: 12, item: Payload { value: 37 } }) }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    for bootstrap in [false, true] {
        let mut compiler = VbcCodegen::with_config(CodegenConfig::new("core.base.protocols"));
        let compiled = if bootstrap {
            compiler.collect_unit_declarations(&[&ast]).unwrap();
            compiler.compile_unit_items(&[&ast], ItemFailurePolicy::Strict).unwrap();
            compiler.finalize_module_from_state().unwrap()
        } else {
            compiler.compile_module(&ast).unwrap()
        };
        let module = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&compiled).unwrap(),
        ).unwrap();
        let inner = module.find_function_by_name("core.base.protocols.inner").unwrap();
        let forward = module.find_function_by_name("core.base.protocols.forward").unwrap();
        let function = module.get_function(forward).unwrap();
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let instructions = decode_instructions(&module.bytecode[start..end]).unwrap();
        let expected: Vec<_> = function.type_params.iter().map(|param| TypeRef::Generic(param.id)).collect();
        assert!(instructions.iter().any(|instruction| matches!(instruction,
            Instruction::CallG { func_id, type_args, .. } if *func_id == inner.0 && type_args.as_slice() == expected
        )), "bootstrap={bootstrap}: {instructions:?}");
        let probe = module.find_function_by_name("core.base.protocols.probe").unwrap();
        let result = Interpreter::new(Arc::new(module)).execute_function(probe).unwrap();
        assert_eq!(result.as_i64(), 37, "bootstrap={bootstrap}");
    }
}
