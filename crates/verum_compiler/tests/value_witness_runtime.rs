//! Source-owned mixed generic witnesses through ordinary bytecode and its wire form.
use verum_common::Shared;
use verum_fast_parser::Parser;
use verum_types::TypeChecker;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

fn checked(source: &str) -> verum_ast::Module {
    checked_with_universes(source, false)
}
fn checked_with_universes(source: &str, universes: bool) -> verum_ast::Module {
    let module = Parser::new(source).parse_module().expect("source grammar");
    let errors = source_errors(&module, universes);
    assert!(errors.is_empty(), "source typing: {errors:?}");
    module
}

fn source_errors(module: &verum_ast::Module, universes: bool) -> Vec<String> {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_universe_poly_enabled(universes);
    checker.set_current_module_path("witness_calls");
    let mut errors: Vec<_> = checker
        .prepare_module_declarations(module)
        .into_iter()
        .map(|e| format!("{e:?}"))
        .collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}"));
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(|e| format!("{e:?}")),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| format!("{d:?}")),
    );
    errors
}
fn execute(definitions: &str, body: &str, expected: i64) {
    execute_with_universes(definitions, body, expected, false);
}
fn execute_with_universes(definitions: &str, body: &str, expected: i64, universes: bool) {
    let source = format!(
        "module witness_calls; type Cell<N: meta Int> is {{ value: () }}; {definitions} fn probe() -> Int {{ {body} }}"
    );
    let ast = checked_with_universes(&source, universes);
    let module = VbcCodegen::with_config(CodegenConfig::new("witness_calls"))
        .compile_module(&ast)
        .expect("checked source compiles");
    let wire =
        deserialize_module(&serialize_module(&module).expect("serialize")).expect("deserialize");
    for module in [module, wire] {
        let entry = module
            .find_function_by_name("witness_calls.probe")
            .expect("actual compiled probe");
        let value = Interpreter::new(Shared::new(module).into_arc())
            .execute_function(entry)
            .expect("actual execution");
        assert_eq!(
            value.try_as_i64(),
            Some(expected),
            "body observes the selected call's value witness"
        );
    }
}
const SIZE: &str = "fn size<N: meta Int>(cell: Cell<N>) -> Int { N }";
#[test]
fn explicit_value() {
    execute(
        SIZE,
        "let cell: Cell<7> = Cell { value: () }; size<7>(cell)",
        7,
    );
}
#[test]
fn inferred_value() {
    execute(
        SIZE,
        "let cell: Cell<7> = Cell { value: () }; size(cell)",
        7,
    );
}
#[test]
fn mixed_explicit_slots() {
    execute(
        "fn mixed<A, N: meta Int, B>(first: A, cell: Cell<N>, last: B) -> Int { N }",
        "let cell: Cell<7> = Cell { value: () }; mixed<Int, 7, Bool>(3, cell, true)",
        7,
    );
}
#[test]
fn mixed_inferred_slots() {
    execute(
        "fn mixed<A, N: meta Int, B>(first: A, cell: Cell<N>, last: B) -> Int { N }",
        "let cell: Cell<7> = Cell { value: () }; mixed(3, cell, true)",
        7,
    );
}
#[test]
fn caller_value_forwarding() {
    execute(
        &format!("{SIZE} fn forward<M: meta Int>(cell: Cell<M>) -> Int {{ size<M>(cell) }}"),
        "let cell: Cell<7> = Cell { value: () }; forward<7>(cell)",
        7,
    );
}
#[test]
fn inferred_caller_value_forwarding() {
    execute(
        &format!("{SIZE} fn forward<M: meta Int>(cell: Cell<M>) -> Int {{ size(cell) }}"),
        "let cell: Cell<7> = Cell { value: () }; forward(cell)",
        7,
    );
}
#[test]
fn separate_call_instances_keep_separate_values() {
    execute(
        SIZE,
        "let a: Cell<7> = Cell { value: () }; let b: Cell<11> = Cell { value: () }; size(a) + size(b)",
        18,
    );
}
#[test]
fn nested_nominal_value_scope() {
    execute(
        "type Frame<N: meta Int> is { cell: Cell<N> }; fn size<N: meta Int>(frame: Frame<N>) -> Int { N }",
        "let cell: Cell<7> = Cell { value: () }; let frame: Frame<7> = Frame { cell: cell }; size(frame)",
        7,
    );
}
#[test]
fn method_owner_value_scope() {
    execute(
        "implement<N: meta Int> Cell<N> { fn count(&self) -> Int { N } }",
        "let cell: Cell<7> = Cell { value: () }; cell.count()",
        7,
    );
}
#[test]
fn method_local_value_shadows_owner() {
    execute(
        "implement<N: meta Int> Cell<N> { fn count<N: meta Int>(&self, other: Cell<N>) -> Int { N } }",
        "let cell: Cell<7> = Cell { value: () }; let other: Cell<11> = Cell { value: () }; cell.count<11>(other)",
        11,
    );
}

fn refused(source: &str, reason: &str) {
    let ast = Parser::new(&format!("module witness_calls; {source}"))
        .parse_module()
        .expect("negative grammar");
    let error = VbcCodegen::with_config(CodegenConfig::new("witness_calls"))
        .compile_module(&ast)
        .expect_err("the code producer itself must refuse invalid witness role");
    assert!(
        format!("{error:?}").contains(reason),
        "missing causal refusal '{reason}': {error:?}"
    );
}
#[test]
fn type_parameter_cannot_supply_a_nominal_value_slot() {
    refused(
        "type Cell<N: meta Int> is { value: () }; fn wrong<T>(cell: Cell<T>) -> Int { 0 }",
        "type parameter cannot occupy a value slot",
    );
}
#[test]
fn value_parameter_cannot_supply_a_nominal_type_slot() {
    refused(
        "type Boxed<T> is { value: () }; fn wrong<N: meta Int>(cell: Boxed<N>) -> Int { 0 }",
        "value parameter cannot occupy a type slot",
    );
}
#[test]
fn explicit_value_cannot_supply_a_callable_type_slot() {
    refused(
        "fn id<T>(value: T) -> T { value } fn probe() -> Int { id<7>(7) }",
        "type parameter received a value argument",
    );
}
#[test]
fn explicit_type_cannot_supply_a_callable_value_slot() {
    refused(
        "type Cell<N: meta Int> is { value: () }; fn size<N: meta Int>(cell: Cell<N>) -> Int { N } fn probe() -> Int { let cell: Cell<7> = Cell { value: () }; size<Int>(cell) }",
        "value parameter received a type argument",
    );
}

#[test]
fn integer_alias_domain_keeps_runtime_witness() {
    execute(
        "type Count = Int; fn size<N: meta Count>(cell: Cell<N>) -> Int { N }",
        "let cell: Cell<7> = Cell { value: () }; size<7>(cell)",
        7,
    );
}
#[test]
fn refined_integer_domain_keeps_runtime_witness() {
    execute(
        "fn size<N: meta Int{>=0}>(cell: Cell<N>) -> Int { N }",
        "let cell: Cell<7> = Cell { value: () }; size<7>(cell)",
        7,
    );
}
#[test]
fn implicit_value_parameter_keeps_its_runtime_slot() {
    execute(
        "fn size<{N: meta Int}>(cell: Cell<N>) -> Int { N }",
        "let cell: Cell<7> = Cell { value: () }; size(cell)",
        7,
    );
}
#[test]
fn erased_lifetime_does_not_shift_explicit_type_role() {
    execute(
        "fn id<'a, T>(value: T) -> T { value }",
        "id<'static, Int>(7)",
        7,
    );
}
#[test]
fn absent_value_witness_is_not_defaulted_to_an_unrelated_caller_slot() {
    refused(
        "fn value<N: meta Int>() -> Int { N } fn caller<M: meta Int>() -> Int { value() }",
        "value parameter requires an exact declaration-owned call witness",
    );
}
#[test]
fn sequence_value_arguments_execute_through_the_typed_source_pipeline() {
    execute(
        "const W: Int = 3; fn get<const S: [Int]>() -> Int { S[0] * 10 + S[1] }
         fn outer<const Row: [Int]>() -> Int { get<Row>() }",
        "outer<[W; 2]>() + get<[4, -1]>()",
        72,
    );
    execute(
        "const ROW: [Int; 2] = [4, 2]; fn pair<const S: [Int; 2]>() -> Int { S[0] * 10 + S[1] }
         fn element<const S: [[Int]]>() -> Int { S[1][0] } fn empty<const S: [Int; 0]>() -> Int { 1 }",
        "pair<ROW>() + element<[[], [7, 9]]>() + empty<[]>()",
        50,
    );
}

#[test]
fn invalid_sequence_arguments_are_refused_before_bytecode() {
    for (domain, value, reason) in [
        ("[Int; 2]", "[1; 1]", "exactly 2"),
        ("[Int]", "[false; 0]", "Bool"),
        ("[[Int]]", "[[1; 8192]; 8192]", "permitted extent"),
        ("[Int]", "[runtime, 3]", "constant"),
    ] {
        let source = format!("module witness_calls; fn read<const S: {domain}>() -> Int {{ 1 }}
            fn probe(runtime: Int) -> Int {{ read<{value}>() }}");
        let module = Parser::new(&source).parse_module().expect("negative grammar");
        let errors = source_errors(&module, false);
        assert!(errors.iter().any(|error| error.contains(reason)), "{value}: {errors:?}");
    }
}

#[test]
fn unsupported_selected_scalar_carrier_is_not_reinterpreted_as_integer() {
    refused(
        "fn value<B: meta Bool>() -> Int { 1 } fn probe() -> Int { value<7>() }",
        "selected value parameter has no supported runtime scalar carrier",
    );
}
#[test]
fn overflowing_signed_witness_is_refused() {
    refused(
        "fn value<N: meta Int>() -> Int { N } fn probe() -> Int { value<9223372036854775808>() }",
        "scalar",
    );
}
#[test]
fn vm_type_load_refuses_missing_generic_binding_after_wire_roundtrip() {
    use verum_vbc::{
        bytecode::encode_instruction,
        instruction::{Instruction, Reg},
        interpreter::InterpreterError,
        types::{TypeParamDescriptor, TypeParamId, TypeRef},
    };
    let ast = checked("module witness_calls; fn probe() -> Int { 7 }");
    let mut module = VbcCodegen::with_config(CodegenConfig::new("witness_calls"))
        .compile_module(&ast)
        .unwrap();
    let entry = module.find_function_by_name("witness_calls.probe").unwrap();
    let mut bytecode = Vec::new();
    encode_instruction(
        &Instruction::LoadT {
            dst: Reg(0),
            type_ref: TypeRef::Generic(TypeParamId(42)),
        },
        &mut bytecode,
    );
    encode_instruction(&Instruction::Ret { value: Reg(0) }, &mut bytecode);
    let descriptor = module
        .functions
        .iter_mut()
        .find(|function| function.id == entry)
        .unwrap();
    descriptor.type_params.push(TypeParamDescriptor {
        id: TypeParamId(42),
        ..Default::default()
    });
    descriptor.bytecode_offset = 0;
    descriptor.bytecode_length = bytecode.len() as u32;
    descriptor.register_count = descriptor.register_count.max(1);
    module.bytecode = bytecode;
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, wire] {
        let error = Interpreter::new(Shared::new(module).into_arc())
            .execute_function(entry)
            .expect_err("unbound witness must not materialize nil");
        assert!(
            matches!(error, InterpreterError::InvalidBytecode { message, .. } if message.contains("unresolved generic witness slot 42"))
        );
    }
}

#[test]
fn concrete_implementation_does_not_invent_a_nominal_owner_binder() {
    execute(
        "implement Cell<7> { fn count(&self) -> Int { 7 } }",
        "let cell: Cell<7> = Cell { value: () }; cell.count()",
        7,
    );
}

#[test]
fn nominal_lifetimes_preserve_borrowed_type_argument() {
    execute(
        "type Holder<'a,T> is {item:&'a mut T}; implement<'a,T> Holder<'a,T>{fn get(&self)->&T{self.item}} fn wrap<'a,T>(item:&'a mut T)->Holder<'a,T>{Holder{item:item}}",
        "let mut value=7; let holder=wrap(&mut value); *holder.get()",
        7,
    );
}

#[test]
fn universe_value_type_slots_execute_without_a_level_runtime_witness() {
    execute_with_universes(
        "fn mixed<universe u,N:meta Int,T>(cell:Cell<N>,value:T)->Int{N}",
        "let cell:Cell<7> =Cell{value:()}; mixed<2,7,Int>(cell,5)",
        7,
        true,
    );
}
#[test]
fn local_level_cannot_claim_the_same_spelled_owner_type_slot() {
    execute_with_universes(
        "type Holder<N> is {value:N}; implement<N> Holder<N>{fn size<universe N,M:meta Int>(&self,cell:Cell<M>)->Int{M}}",
        "let holder:Holder<Int> =Holder{value:5}; let cell:Cell<7> =Cell{value:()}; holder.size<2,7>(cell)",
        7,
        true,
    );
}
#[test]
fn local_level_cannot_claim_the_same_spelled_owner_value_slot() {
    execute_with_universes(
        "type Store<N:meta Int> is {value:()}; implement<N:meta Int> Store<N>{fn size<universe N,M:meta Int>(&self,cell:Cell<M>)->Int{M}}",
        "let store:Store<5> =Store{value:()}; let cell:Cell<7> =Cell{value:()}; store.size<2,7>(cell)",
        7,
        true,
    );
}
