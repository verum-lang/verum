//! T1807: deferred static initialization keeps declaration-owned generic witnesses.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_common::{List, ResourceDiscipline, Text};
use verum_fast_parser::Parser;
use verum_vbc::{
    Instruction, VbcModule,
    bytecode::decode_instructions,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
    types::{CbgrTier, TypeRef},
};
const TYPES: &str = r#"
type Choice<T> is Empty | Value(T);
type Holder<T> is { item: T };
implement<T> Holder<T> {
    fn new(item: T) -> Holder<T> { Holder { item: item } }
}
"#;
fn compile(sources: &[Text], bootstrap: bool) -> VbcModule {
    let modules: List<_> = sources.iter().map(|s| Parser::new(s).parse_module().unwrap()).collect();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("static_generic"));
    let mut module = if bootstrap {
        let modules: List<_> = modules.iter().collect();
        codegen.collect_unit_declarations(&modules).unwrap();
        codegen.compile_unit_items(&modules, ItemFailurePolicy::Strict).unwrap();
        codegen.finalize_module_from_state().unwrap()
    } else { codegen.compile_module(&modules[0]).unwrap() };
    for f in &mut module.functions { f.value_uses = None; }
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap()).unwrap()
}
fn witness(module: &VbcModule, initializer: &str) -> TypeRef {
    let function = module.functions.iter().find(|f|
        module.strings.get(f.name) == Some(initializer)).unwrap();
    let body = decode_instructions(&module.bytecode[function.bytecode_offset as usize..
        (function.bytecode_offset + function.bytecode_length) as usize]).unwrap();
    body.iter().find_map(|op| match op {
        Instruction::CallG { func_id, type_args, .. } if module.get_function(verum_vbc::module::FunctionId(*func_id))
            .and_then(|f| module.strings.get(f.name)).is_some_and(|n| n.ends_with("Holder.new")) => {
            assert_eq!(type_args.len(),1); Some(type_args[0].clone())
        }, _ => None,
    }).unwrap_or_else(|| panic!("static constructor {initializer} must carry its generic witness: {body:?}"))
}
fn run(module: VbcModule) {
    let id = module.find_function_by_name("static_generic.probe").unwrap();
    let mut interpreter = Interpreter::new(Arc::new(module));
    interpreter.run_global_ctors().unwrap();
    assert_eq!(interpreter.execute_function(id).unwrap().as_i64(),42);
}
#[test]
fn static_initializer_supplies_nested_generic_constructor_witnesses() {
    for element in ["Int", "()"] {
        for bootstrap in [false, true] {
            let source = format!(r#"{TYPES}
static ROOT: Holder<Choice<{element}>> = Holder.new(Choice.Empty);
fn probe() -> Int {{ match ROOT.item {{ Choice.Empty => 42, Choice.Value(_) => 0 }} }}
"#);
            let module = compile(&[source.into()], bootstrap);
            let parameter = witness(&module, "__tls_init_ROOT");
            assert_eq!(module.resource_discipline(&parameter), ResourceDiscipline::Unrestricted);
            run(module);
        }
    }
}
#[test]
fn nested_reference_witnesses_preserve_their_borrow_tiers() {
    for (reference, tier) in [("&Socket", CbgrTier::Tier0), ("&checked Socket", CbgrTier::Tier1), ("&unsafe Socket", CbgrTier::Tier2)] {
        for bootstrap in [false, true] {
            let source = format!(r#"{TYPES}
type affine Socket is {{ fd: Int }};
static ROOT: Holder<Choice<{reference}>> = Holder.new(Choice.Empty);
fn probe() -> Int {{ 42 }}
"#);
            let module = compile(&[source.into()], bootstrap);
            let parameter = witness(&module, "__tls_init_ROOT");
            let TypeRef::Instantiated { args, .. } = &parameter else { panic!("{parameter:?}") };
            assert!(matches!(&args[0], TypeRef::Reference { tier: actual, .. } if *actual == tier), "{parameter:?}");
            assert_eq!(module.resource_discipline(&parameter), ResourceDiscipline::Unrestricted);
            run(module);
        }
    }
}
#[test]
fn deferred_initializers_restore_each_declarations_nominal_owner() {
    let common: Text = format!("module common; {TYPES}").into();
    let first: Text = r#"module first;
type affine Payload is { fd: Int };
static FIRST: static_generic.common.Holder<static_generic.common.Choice<Payload>> = static_generic.common.Holder.new(static_generic.common.Choice.Empty);
"#.into();
    let second: Text = r#"module second;
type Payload is { value: Int };
static SECOND: static_generic.common.Holder<static_generic.common.Choice<Payload>> = static_generic.common.Holder.new(static_generic.common.Choice.Empty);
"#.into();
    for sources in [verum_common::list![common.clone(),first.clone(),second.clone()], verum_common::list![second,first,common]] {
        let module = compile(&sources, true);
        let first = witness(&module,"__tls_init_FIRST");
        let descriptors: List<_> = module.types.iter().filter(|ty| module.strings.get(ty.name).is_some_and(|name| name.contains("Payload") || name.contains("Choice") || name.contains("Holder")))
            .map(|ty| (ty.id,module.strings.get(ty.name),ty.resource_discipline,ty.type_params.clone(),ty.variants.clone())).collect();
        assert_eq!(module.resource_discipline(&first), ResourceDiscipline::Affine, "witness={first:?}; descriptors={descriptors:?}");
        assert_eq!(module.resource_discipline(&witness(&module,"__tls_init_SECOND")), ResourceDiscipline::Unrestricted);
        Interpreter::new(Arc::new(module)).run_global_ctors().unwrap();
    }
}
