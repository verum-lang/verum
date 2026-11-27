//! Generic method calls forward the caller's declaration-owned parameter IDs.
#![cfg(feature = "codegen")]
#[path = "fixtures/canonical_drop.rs"]
mod canonical_drop;

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, CodegenErrorKind, CodegenResult, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

const TYPES: &str = r#"
mount core.base.protocols.Drop;
type Count is { value: Int };
type affine Ticket is { count: &mut Count };
implement Drop for Ticket { fn drop(&mut self) { self.count.value = self.count.value + 1; } }
type Sink<T> is { marker: Int };
implement<T> Sink<T> {
    fn consume(&self, value: T) { let owned: T = value; drop(owned); }
}
"#;

#[test]
fn specialized_instance_edges_require_an_adjacent_witness_and_one_exact_source_declaration() {
    use verum_vbc::{
        Instruction, Reg, RegRange,
        mono::{InstantiationGraph, discover_call_instantiations},
        types::{TypeId, TypeRef},
    };
    let ast = Parser::new(TYPES).parse_module().unwrap();
    let original = lower(&ast, true).unwrap();
    let mut module = deserialize_module(&serialize_module(&original).unwrap()).unwrap();
    let id = module.find_function_by_name("forwarding.Sink.consume").unwrap();
    let declaration = module.get_function(id).unwrap().clone();
    let actual = [TypeRef::Concrete(TypeId::INT)];
    let witness = Instruction::SetCallWitness { type_args: actual.to_vec() };
    let call = |name, count| Instruction::CallM {
        dst: Reg(2), receiver: Reg(0), method_id: name,
        args: RegRange { start: Reg(1), count },
    };
    let mut graph = InstantiationGraph::new();
    let qualified = declaration.name.0;
    let suffix = module.intern_string("Sink.consume").0;
    for instructions in [
        vec![witness.clone(), call(suffix, 1)],
        vec![witness.clone(), call(qualified, 0)],
        vec![witness.clone(), Instruction::Nop, call(qualified, 1)],
    ] {
        discover_call_instantiations(&module, &instructions, 0, &mut graph).unwrap();
        assert!(graph.is_empty());
    }
    let instructions = [witness, call(qualified, 1)];
    discover_call_instantiations(&module, &instructions, 0, &mut graph).unwrap();
    assert!(graph.contains(id, &actual));
    module.add_function(declaration);
    let mut ambiguous = InstantiationGraph::new();
    let error = discover_call_instantiations(&module, &instructions, 0, &mut ambiguous).unwrap_err();
    assert!(error.to_string().contains("competing instance declarations"));
    assert!(ambiguous.is_empty());
}

fn lower(ast: &verum_ast::Module, bootstrap: bool) -> CodegenResult<verum_vbc::module::VbcModule> {
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("forwarding"));
    canonical_drop::initialize(&mut codegen);
    if bootstrap {
        codegen.collect_unit_declarations(&[ast])?;
        codegen.compile_unit_items(&[ast], ItemFailurePolicy::Strict)?;
        codegen.finalize_module_from_state()
    } else {
        codegen.compile_additional_module(ast)
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
fn static_factories_retain_the_implementation_receiver_across_binder_reordering() {
    let source = r#"
        type Pair<A, B> is { marker: Int };
        implement<A, B> Pair<A, B> {
            fn fresh() -> Pair<A, B> { Pair { marker: 0 } }
            fn from_values(left: A, right: B) -> Pair<A, B> { Pair { marker: 1 } }
            fn left(&self) -> UInt64 { A.id }
            fn right(&self) -> UInt64 { B.id }
        }
        implement<Y, X> Pair<X, Y> {
            fn check<Extra>(&self, extra: Extra) -> Bool {
                let a = Self.fresh();
                let b = Pair.fresh();
                let reversed = Pair.from_values(true, 37);
                a.left() == X.id && a.right() == Y.id && b.left() == X.id && b.right() == Y.id
                    && reversed.left() == Bool.id && reversed.right() == Int.id
            }
        }
        fn probe() -> Int {
            let p: Pair<Int, Bool> = Pair { marker: 0 };
            if p.check<Text>("third") { 1 } else { 0 }
        }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    for bootstrap in [false, true] {
        let module = lower(&ast, bootstrap).unwrap();
        let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        for module in [module, wire] {
            let entry = module.find_function_by_name("forwarding.probe").unwrap();
            let mut vm = Interpreter::try_new_validated(Arc::new(module)).unwrap();
            assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(1));
        }
        let conflicting = source.replace("Pair.from_values(true, 37)", "Self.from_values(true, 37)");
        let invalid = Parser::new(&conflicting).parse_module().unwrap();
        assert!(lower(&invalid, bootstrap).is_err(), "Self must keep its selected parameters");
    }
}

#[test]
fn a_dispatched_source_method_receives_its_owned_parameter() {
    run(r#"
type Map<K, V> is { marker: Int };
implement<K, V> Map<K, V> {
    fn consume(&self, value: V) {}
    fn forward(&self, value: V) { self.consume(value); }
}
fn probe() -> Int {
    let mut count = Count { value: 0 };
    let table: Map<Int, Ticket> = Map { marker: 0 };
    table.forward(Ticket { count: &mut count });
    count.value
}
"#);
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
fn returned_local_constrains_its_selected_generic_factory() {
    let source = r#"
        type Stamp<T> is { owner: UInt64 };
        implement<T> Stamp<T> {
            fn blank() -> Stamp<T> { Stamp { owner: T.id } }
            fn accepts(&self, sample: T) -> Bool { self.owner == T.id }
        }
        fn build<T>(sample: T) -> Stamp<T> {
            let result = Stamp.blank();
            if !result.accepts(sample) { panic("factory lost its witness"); }
            result
        }
        fn early<T>(sample: T) -> Stamp<T> {
            if true {
                let result = Stamp.blank();
                if !result.accepts(sample) { panic("return lost its witness"); }
                return result;
            }
            build(sample)
        }
        fn shadow<T>(sample: T) -> Stamp<T> {
            let result: Stamp<Bool> = Stamp.blank();
            if result.owner != Bool.id { panic("wrong shadowed witness"); }
            let result = Stamp.blank();
            if !result.accepts(sample) { panic("wrong shadowing witness"); }
            result
        }
        fn probe() -> Int {
            let a = build(true);
            let b = early(42);
            let c = shadow(13);
            if a.owner == Bool.id && b.owner == Int.id && c.owner == Int.id { 1 } else { 0 }
        }
    "#;
    run(source);
    let mismatch = source.replace("result.accepts(sample)", "result.accepts(true)");
    let ast = Parser::new(&format!("{TYPES}\n{mismatch}")).parse_module().unwrap();
    for bootstrap in [false, true] {
        assert!(lower(&ast, bootstrap).is_err(), "a rigid caller parameter cannot be specialized to Bool");
    }
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
