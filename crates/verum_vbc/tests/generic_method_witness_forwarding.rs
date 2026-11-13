//! Generic method calls forward the caller's declaration-owned parameter IDs.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, CodegenErrorKind, CodegenResult, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

const TYPES: &str = r#"
type Count is { value: Int };
type affine Ticket is { count: &mut Count };
implement Drop for Ticket { fn drop(&mut self) { self.count.value = self.count.value + 1; } }
type Sink<T> is { marker: Int };
implement<T> Sink<T> {
    fn consume(&self, value: T) { let owned: T = value; drop(owned); }
}
"#;

fn lower(ast: &verum_ast::Module, bootstrap: bool) -> CodegenResult<verum_vbc::module::VbcModule> {
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("forwarding"));
    if bootstrap {
        codegen.collect_unit_declarations(&[ast])?;
        codegen.compile_unit_items(&[ast], ItemFailurePolicy::Strict)?;
        codegen.finalize_module_from_state()
    } else {
        codegen.compile_module(ast)
    }
}

fn run(source: &str) {
    let ast = Parser::new(&format!("{TYPES}\n{source}")).parse_module().unwrap();
    for bootstrap in [false, true] {
        let mut module = lower(&ast, bootstrap).unwrap();
        // The VBC handoff must work independently of optional analysis metadata.
        for function in &mut module.functions {
            function.value_uses = None;
        }
        let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        let entry = module.find_function_by_name("forwarding.probe").unwrap();
        let result = Interpreter::new(Arc::new(module)).execute_function(entry);
        assert_eq!(
            result.unwrap().as_i64(), 1,
            "bootstrap={bootstrap}: exactly one resource owner"
        );
    }
}

#[test]
fn a_generic_function_forwards_its_witness_to_a_borrowed_receiver() {
    run(r#"
fn forward<T>(sink: &Sink<T>, value: T) { sink.consume(value); }
fn probe() -> Int {
    let mut count = Count { value: 0 };
    {
        let sink: Sink<Ticket> = Sink { marker: 0 };
        let value = Ticket { count: &mut count };
        forward<Ticket>(&sink, value);
    }
    count.value
}
"#);
}

#[test]
fn a_field_receiver_maps_the_callers_second_parameter_to_the_callees_first() {
    run(r#"
type Relay<A, B> is { left: &Sink<A>, right: &Sink<B> };
implement<A, B> Relay<A, B> {
    fn consume_right(&self, value: B) { self.right.consume(value); }
}

fn probe() -> Int {
    let mut count = Count { value: 0 };
    {
        let left: Sink<Int> = Sink { marker: 0 };
        let right: Sink<Ticket> = Sink { marker: 1 };
        let relay: Relay<Int, Ticket> = Relay { left: &left, right: &right };
        let value = Ticket { count: &mut count };
        relay.consume_right(value);
    }
    count.value
}
"#);
}

#[test]
fn an_unknown_argument_cannot_borrow_an_unrelated_callers_witness() {
    let source = [TYPES, r#"
fn forward<T>(sink: &Sink<UnboundSlot>, value: T) { sink.consume(value); }
fn probe() -> Int {
    let mut count = Count { value: 0 };
    let sink: Sink<UnboundSlot> = Sink { marker: 0 };
    let value = Ticket { count: &mut count };
    forward<Ticket>(&sink, value);
    count.value
}
"#].join("\n");
    let ast = Parser::new(&source).parse_module().unwrap();
    for bootstrap in [false, true] {
        let error = lower(&ast, bootstrap).expect_err("missing type owner must be rejected at its declaration");
        assert!(matches!(error.kind, CodegenErrorKind::InvalidSourceSignature(ref message)
            if message.contains("unresolved nominal type 'UnboundSlot'")), "{error}");
        let span = error.span.expect("source declaration diagnostic");
        assert_eq!(&source[span.start as usize..span.end as usize], "UnboundSlot");
    }
}

#[test]
fn an_unbound_method_slot_is_refused_even_with_a_concrete_caller_slot() {
    let source = r#"
        type Reader is {};
        implement Reader { fn inspect<T, U>(&self, value: T) -> Int { U.size } }
        fn relay<A, B>(a: A, b: B) -> Int {
            let reader = Reader {};
            reader.inspect(-8)
        }
        fn probe() -> Int { relay(true, 7_i16) }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    for bootstrap in [false, true] {
        let bytes = serialize_module(&lower(&ast, bootstrap).unwrap()).unwrap();
        let module = deserialize_module(&bytes).unwrap();
        let id = module.find_function_by_name("forwarding.probe").unwrap();
        let error = Interpreter::new(Arc::new(module)).execute_function(id).unwrap_err();
        assert!(error.to_string().contains("unresolved declared type layout"), "{error}");
    }
}

#[test]
fn method_level_parameters_keep_their_declared_ids() {
    run(r#"
type Relay<A> is { first: A };
implement<A> Relay<A> {
    fn forward<B>(&self, sink: &Sink<B>, value: B) { sink.consume(value); }
}
fn probe() -> Int {
    let mut count = Count { value: 0 };
    {
        let relay: Relay<Int> = Relay { first: 0 };
        let sink: Sink<Ticket> = Sink { marker: 0 };
        let value = Ticket { count: &mut count };
        relay.forward<Ticket>(&sink, value);
    }
    count.value
}
"#);
}

#[test]
fn a_borrowed_method_argument_remains_owned_by_its_caller() {
    run(r#"
type Reader is {};
implement Reader {
    fn inspect(&self, value: &Ticket) -> Int { value.count.value }
}
fn probe() -> Int {
    let mut count = Count { value: 0 };
    {
        let reader = Reader {};
        let value = Ticket { count: &mut count };
        assert_eq(reader.inspect(&value), 0);
        assert_eq(count.value + 0, 0);
    }
    count.value
}
"#);
}

#[test]
fn inferred_local_factory_forwards_its_return_type_as_an_ownership_witness() {
    run(r#"
type Source<T> is { label: Int };
implement<T> Source<T> {
    fn acquire(factory: fn() -> T) {
        let value: T = factory();
        let owned: T = value;
        drop(owned);
    }
}

fn probe() -> Int {
    let mut count = Count { value: 0 };
    let counter = &mut count;
    let factory = || -> Ticket { Ticket { count: counter } };
    let create = factory;
    Source.acquire(create);
    count.value
}
"#);
}

#[test]
fn concrete_generic_clone_keeps_its_method_witnesses() {
    run(r#"
type Handle<T> is { marker: Int };
implement<T> Handle<T> {
    fn clone(&self) -> Handle<T> {
        assert_eq(T.size, 8);
        Handle { marker: self.marker }
    }
}
fn probe() -> Int {
    let original: Handle<Int> = Handle { marker: 1 };
    let copied = original.clone();
    copied.marker
}
"#);
}

#[test]
fn dynamic_calls_keep_the_same_witnesses_as_direct_generic_methods() {
    use verum_vbc::{Instruction, Reg, RegRange};
    let source = r#"
        type Handle<T> is { marker: Int };
        implement<T> Handle<T> {
            fn clone(&self) -> Handle<T> {
                assert_eq(T.size, 8);
                Handle { marker: self.marker }
            }
        }
        fn probe() -> Int {
            let original: Handle<Int> = Handle { marker: 73 };
            let copied = original.clone();
            copied.marker
        }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    for bootstrap in [false, true] {
        let mut module = lower(&ast, bootstrap).unwrap();
        let entry = module.find_function_by_name("forwarding.probe").unwrap();
        let descriptor = module.get_function(entry).unwrap();
        let start = descriptor.bytecode_offset as usize;
        let end = start + descriptor.bytecode_length as usize;
        let original = verum_vbc::bytecode::decode_instructions(&module.bytecode[start..end]).unwrap();
        let mut body = verum_common::List::new();
        let mut replaced = 0;
        for instruction in original {
            if let Instruction::CallG { dst, func_id, type_args, args } = instruction {
                let callee = module.get_function(verum_vbc::FunctionId(func_id)).unwrap();
                assert_eq!(module.get_string(callee.params[0].name), Some("self"));
                assert_eq!(args.count, 1);
                body.push(Instruction::SetCallWitness { type_args });
                body.push(Instruction::CallM {
                    dst,
                    receiver: args.start,
                    method_id: callee.name.0,
                    args: RegRange { start: Reg(args.start.0 + 1), count: 0 },
                });
                replaced += 1;
            } else {
                body.push(instruction);
            }
        }
        assert_eq!(replaced, 1, "exercise both supported call encodings");
        let start = module.bytecode.len() as u32;
        let length = verum_vbc::bytecode::encode_instructions(&body, &mut module.bytecode) as u32;
        let descriptor = module.functions.iter_mut().find(|f| f.id == entry).unwrap();
        descriptor.bytecode_offset = start;
        descriptor.bytecode_length = length;
        descriptor.instructions = Some(body.into());
        descriptor.value_uses = None;
        let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        verum_vbc::validate::validate_module(&module).unwrap();
        let result = Interpreter::new(Arc::new(module)).execute_function(entry).unwrap();
        assert_eq!(result.try_as_i64(), Some(73));
    }
}
