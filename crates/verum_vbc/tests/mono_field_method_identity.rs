#![cfg(feature = "codegen")]

use verum_vbc::{
    bytecode::{decode_instructions, encode_instructions_with_fixup},
    codegen::{CodegenConfig, VbcCodegen},
    instruction::Instruction as I,
    module::{FunctionId, VbcModule},
    mono::{InstantiationGraph, discover_call_instantiations, monomorphize_minimal},
    types::{TypeId, TypeRef},
};

fn source(nested: bool, borrowed: bool) -> VbcModule {
    let field = if nested { "Boxed<Item>" } else { "Item" };
    let value = if nested {
        "self.value.inner"
    } else {
        "self.value"
    };
    let input = if nested { "Boxed { inner: 7 }" } else { "7" };
    let receiver = if borrowed { "&self" } else { "self" };
    let text = format!(
        r#"
module field_calls;
type Answer is {{ value: Int }};
implement Answer {{ fn from_value(value: Int) -> Answer {{ Answer {{ value }} }} }}
type Boxed<T> is {{ inner: T }};
type Factory<Item> is {{ value: {field} }};
implement<Item> Factory<Item> {{ fn build<Output>({receiver}) -> Output {{ Output.from_value({value}) }} }}
fn probe() -> Int {{ let factory: Factory<Int> = Factory {{ value: {input} }}; factory.build<Answer>().value }}
"#
    );
    let ast = verum_fast_parser::Parser::new(&text)
        .parse_module()
        .unwrap();
    let mut module = VbcCodegen::with_config(CodegenConfig::new("field_calls"))
        .compile_module(&ast)
        .unwrap();
    module.resolve_protocol_dispatch();
    module
}
fn body(module: &VbcModule, id: FunctionId) -> Vec<I> {
    let f = module.get_function(id).unwrap();
    decode_instructions(
        &module.bytecode
            [f.bytecode_offset as usize..(f.bytecode_offset + f.bytecode_length) as usize],
    )
    .unwrap()
}
fn function(module: &VbcModule, suffix: &str) -> FunctionId {
    module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n.ends_with(suffix))
        })
        .unwrap()
        .id
}
fn specialize(module: VbcModule) -> VbcModule {
    let mut graph = InstantiationGraph::new();
    for f in &module.functions {
        discover_call_instantiations(&module, &body(&module, f.id), 0, &mut graph).unwrap();
    }
    assert_eq!(graph.len(), 1);
    monomorphize_minimal(module, &graph).unwrap().module
}
fn specialized_body(module: &VbcModule) -> Vec<I> {
    let id = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n.contains("Factory.build$mono$"))
        })
        .unwrap()
        .id;
    body(module, id)
}
fn positive(nested: bool, borrowed: bool) {
    let source = source(nested, borrowed);
    let target = function(&source, "Answer.from_value");
    let entry = function(&source, "probe");
    let result = specialize(source);
    let ops = specialized_body(&result);
    assert!(
        ops.iter()
            .any(|op| matches!(op,I::Call {func_id,..} if *func_id==target.0)),
        "typed field must retain exact static target: {ops:?}"
    );
    assert!(
        !ops.iter().any(|op| matches!(op, I::CallM { .. })),
        "no unresolved from_value: {ops:?}"
    );
    let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(result))
        .execute_function(entry)
        .unwrap();
    assert_eq!(value.as_i64(), 7);
}
#[test]
fn source_field_argument_keeps_the_concrete_type_method_target() {
    positive(false, false);
}
#[test]
fn nested_generic_fields_substitute_each_declaring_owner() {
    positive(true, false);
}
#[test]
fn borrowed_receiver_field_uses_its_declared_pointee() {
    positive(false, true);
}

fn mutate_body(module: &mut VbcModule, change: impl FnOnce(&mut Vec<I>)) {
    let id = function(module, "Factory.build");
    let mut ops = body(module, id);
    change(&mut ops);
    let offset = module.bytecode.len() as u32;
    let length = encode_instructions_with_fixup(&ops, &mut module.bytecode) as u32;
    let f = module.get_function_mut(id).unwrap();
    f.bytecode_offset = offset;
    f.bytecode_length = length;
    f.instructions = Some(ops);
}
fn remains_dynamic(module: VbcModule) {
    let result = specialize(module);
    assert!(
        specialized_body(&result)
            .iter()
            .any(|op| matches!(op, I::CallM { .. })),
        "unproved field must not select a target"
    );
}
#[test]
fn unknown_field_index_does_not_reuse_a_prior_argument_fact() {
    let mut module = source(false, false);
    mutate_body(&mut module, |ops| {
        let index = ops
            .iter()
            .position(|op| matches!(op, I::GetF { .. }))
            .unwrap();
        let I::GetF { dst, .. } = ops[index] else {
            unreachable!()
        };
        if let I::GetF { field_idx, .. } = &mut ops[index] {
            *field_idx = 900;
        }
        ops.insert(index, I::LoadI { dst, value: 7 });
    });
    remains_dynamic(module);
}
#[test]
fn field_write_to_type_token_register_invalidates_the_token() {
    let mut module = source(false, false);
    mutate_body(&mut module, |ops| {
        let token = ops
            .iter()
            .find_map(|op| {
                if let I::LoadT { dst, .. } = op {
                    Some(*dst)
                } else {
                    None
                }
            })
            .unwrap();
        let field = ops
            .iter()
            .find_map(|op| {
                if let I::GetF { obj, field_idx, .. } = op {
                    Some((*obj, *field_idx))
                } else {
                    None
                }
            })
            .unwrap();
        let call = ops
            .iter()
            .position(|op| matches!(op, I::CallM { .. }))
            .unwrap();
        ops.insert(
            call,
            I::GetF {
                dst: token,
                obj: field.0,
                field_idx: field.1,
            },
        );
    });
    remains_dynamic(module);
}
#[test]
fn incompatible_declared_field_type_does_not_borrow_another_owner() {
    let mut module = source(false, false);
    let owner = module
        .types
        .iter_mut()
        .find(|t| module.strings.get(t.name) == Some("Factory"))
        .unwrap();
    owner.fields[0].type_ref = TypeRef::Concrete(TypeId::TEXT);
    remains_dynamic(module);
}
#[test]
fn missing_owner_arguments_do_not_bind_fields_from_method_generic_ids() {
    let mut module = source(false, false);
    let id = function(&module, "Factory.build");
    let f = module.get_function_mut(id).unwrap();
    let TypeRef::Instantiated { base, .. } = &f.params[0].type_ref else {
        panic!("owner")
    };
    f.params[0].type_ref = TypeRef::Concrete(*base);
    remains_dynamic(module);
}
