//! T1536: user code consumes imported signatures in its own TypeId namespace.
#![cfg(feature = "codegen")]

use verum_common::{List, Map};
use verum_fast_parser::Parser;
use verum_vbc::codegen::context::FunctionInfo;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::module::{FunctionId, VbcModule};
use verum_vbc::types::{TypeId, TypeRef};

fn producer() -> VbcModule {
    let source = r#"
module core.intrinsics.control;
type Location is { file: Text, line: Int, column: Int };
type IntrinsicPanicInfo is { message: Text, location: Maybe<Location> };
@intrinsic("catch_unwind")
fn fence<T>(f: fn() -> T) -> Result<T, IntrinsicPanicInfo> { @intrinsic("catch_unwind", f) }
"#;
    let ast = Parser::new(source).parse_module().expect("producer syntax");
    let module = VbcCodegen::with_config(CodegenConfig::new("core.intrinsics.control"))
        .compile_module(&ast)
        .expect("producer compiles");
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize producer"),
    )
    .expect("load producer")
}

fn load(source: &VbcModule, caller: &verum_ast::Module) -> VbcCodegen {
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    cg.register_builtin_variants();
    cg.collect_unit_declarations(&[caller])
        .expect("local declarations");
    // The actual user loader also imports canonical core.base sums. Build
    // those declarations from source rather than injecting constructor tags.
    for (owner, declaration) in [
        ("core.base.result", "type Result<T, E> is Ok(T) | Err(E);"),
        ("core.base.maybe", "type Maybe<T> is None | Some(T);"),
    ] {
        let ast = Parser::new(&format!("module {owner}; {declaration}"))
            .parse_module()
            .unwrap();
        let sum = VbcCodegen::with_config(CodegenConfig::new(owner))
            .compile_module(&ast)
            .unwrap();
        cg.import_archive_module_types(&sum);
    }
    let local_before = cg
        .ctx_mut()
        .lookup_function("consumer.local_identity")
        .expect("local function")
        .return_type
        .clone();
    install(&mut cg, source, 7000);
    assert_eq!(
        cg.ctx_mut()
            .lookup_function("consumer.local_identity")
            .unwrap()
            .return_type,
        local_before,
        "local caller facts do not belong to the imported namespace"
    );
    cg
}

fn install(cg: &mut VbcCodegen, source: &VbcModule, first_id: u32) {
    let mut remap = Map::new();
    for (index, function) in source.functions.iter().enumerate() {
        let id = FunctionId(first_id + index as u32);
        // Mirror register_module_filtered: source descriptor supplies all
        // structural signature facts before ordinary module type import.
        let name = source.strings.get(function.name).expect("name");
        let info = FunctionInfo {
            id,
            param_count: function.params.len(),
            type_param_ids: function.type_params.iter().map(|p| p.id).collect(),
            explicit_type_param_ids: function.explicit_type_param_ids.clone(),
            return_type: Some(function.return_type.clone()),
            return_type_name: function
                .return_type_name
                .and_then(|id| source.strings.get(id))
                .map(str::to_owned),
            intrinsic_name: function
                .intrinsic_name
                .and_then(|id| source.strings.get(id))
                .map(str::to_owned),
            ..Default::default()
        };
        cg.ctx_mut()
            .register_function(name.to_owned(), info.clone());
        cg.ctx_mut()
            .register_function(format!("{name}_alias"), info);
        cg.ctx_mut().archive_fn_param_types.insert(
            id.0,
            function.params.iter().map(|p| p.type_ref.clone()).collect(),
        );
        remap.insert(function.id.0, id);
    }
    cg.import_archive_module_types(source);
    cg.merge_archive_function_bodies(source, &remap.into());
}

const CALLER: &str = r#"
module consumer;
type Occupied0 is { first: Int };
type Occupied1 is { second: Int };
type Occupied2 is { third: Int };
fn local_identity(value: Occupied0) -> Occupied0 { value }
fn probe() -> Bool {
    let result = core.intrinsics.control.fence(|| { panic("imported panic"); 0 });
    match result {
        Ok(_) => false,
        Err(info) => {
            let absent = match info.location { None => true, Some(_) => false };
            info.message == "imported panic" && absent
        }
    }
}
"#;

#[test]
fn imported_intrinsic_info_and_descriptor_have_the_same_local_error_identity() {
    let source = producer();
    let caller = Parser::new(CALLER).parse_module().expect("caller syntax");
    let mut cg = load(&source, &caller);
    let carried = cg
        .ctx_mut()
        .lookup_function("core.intrinsics.control.fence")
        .expect("imported function")
        .return_type
        .clone()
        .expect("return");
    let module = cg.finalize_module_from_state().expect("consumer module");
    let declared = module
        .functions
        .iter()
        .find(|f| module.strings.get(f.name) == Some("core.intrinsics.control.fence"))
        .expect("merged descriptor");
    let error = module
        .types
        .iter()
        .find(|t| module.strings.get(t.name) == Some("IntrinsicPanicInfo"))
        .expect("local panic type");
    assert!(matches!(&declared.return_type,
        TypeRef::Instantiated { base, args } if *base == TypeId::RESULT && args[1] == TypeRef::Concrete(error.id)));
    assert_eq!(
        carried, declared.return_type,
        "FunctionInfo cannot retain an archive-local error ID"
    );
}

#[test]
fn user_catch_body_compiles_after_ordinary_archive_import() {
    let source = producer();
    let caller = Parser::new(CALLER).parse_module().expect("caller syntax");
    let module = load(&source, &caller)
        .compile_function_bodies(&caller)
        .expect("consumer compiles with the imported error descriptor");
    let id = module
        .functions
        .iter()
        .find(|f| module.strings.get(f.name) == Some("consumer.probe"))
        .unwrap()
        .id;
    let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(id)
        .expect("imported catch executes");
    assert_eq!(value.as_bool(), true, "actual value: {value:?}");
}

fn nominal_producer(owner: &str, foreign: bool) -> VbcModule {
    let fields = if foreign {
        "other: Int, message: Int, location: Int"
    } else {
        "message: Text, location: Maybe<Location>"
    };
    let source = format!(
        r#"
module {owner};
type Location is {{ file: Text, line: Int, column: Int }};
type IntrinsicPanicInfo is {{ {fields} }};
type Packet is Empty | Payload(IntrinsicPanicInfo) | Named {{ payload: Maybe<Location> }};
fn relay(value: IntrinsicPanicInfo) -> IntrinsicPanicInfo {{ value }}
fn wrap(value: Maybe<IntrinsicPanicInfo>) -> Maybe<IntrinsicPanicInfo> {{ value }}
"#
    );
    let ast = Parser::new(&source).parse_module().expect("nominal source");
    let module = VbcCodegen::with_config(CodegenConfig::new(owner))
        .compile_module(&ast)
        .expect("nominal producer compiles");
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap()
}

fn nominal<'a>(
    module: &'a VbcModule,
    owner: &str,
    leaf: &str,
) -> &'a verum_vbc::types::TypeDescriptor {
    module
        .types
        .iter()
        .find(|ty| {
            module.strings.get(ty.name) == Some(leaf)
                && ty.origin_module.and_then(|id| module.strings.get(id)) == Some(owner)
        })
        .expect("exact nominal owner")
}

fn check_nominal_imports(check_functions: bool) {
    let alpha = nominal_producer("alpha", false);
    let beta = nominal_producer("beta", true);
    assert_eq!(
        nominal(&alpha, "alpha", "IntrinsicPanicInfo").id,
        nominal(&beta, "beta", "IntrinsicPanicInfo").id,
        "the negative control must collide in the source namespaces"
    );
    for order in [[&alpha, &beta], [&beta, &alpha]] {
        let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        for (index, module) in order.iter().enumerate() {
            install(&mut cg, module, 8000 + index as u32 * 100);
        }
        // Repeat the same producer and mapped FunctionIds: remapping must
        // derive from source facts, never remap already-local numeric IDs.
        install(&mut cg, order[0], 8000);
        let infos: List<_> = ["alpha.relay", "alpha.wrap", "beta.relay", "beta.wrap"]
            .into_iter()
            .map(|name| {
                let info = cg.ctx_mut().lookup_function(name).unwrap().clone();
                let alias = cg
                    .ctx_mut()
                    .lookup_function(&format!("{name}_alias"))
                    .unwrap();
                assert_eq!(alias.id, info.id);
                assert_eq!(
                    alias.return_type, info.return_type,
                    "all mapped aliases must be translated"
                );
                let params = cg.ctx_mut().archive_fn_param_types[&info.id.0].clone();
                (name, info.return_type.unwrap(), params)
            })
            .collect();
        let module = cg.finalize_module_from_state().unwrap();
        if check_functions {
            for (name, result, params) in infos {
                let function = module
                    .functions
                    .iter()
                    .find(|f| module.strings.get(f.name) == Some(name))
                    .unwrap();
                assert_eq!(result, function.return_type, "{name}: return namespace");
                let declared: List<_> =
                    function.params.iter().map(|p| p.type_ref.clone()).collect();
                assert_eq!(
                    params.as_slice(),
                    declared.as_slice(),
                    "{name}: parameter namespace"
                );
            }
        }
        for owner in ["alpha", "beta"] {
            let error = nominal(&module, owner, "IntrinsicPanicInfo");
            let location = nominal(&module, owner, "Location");
            if owner == "alpha" {
                assert!(
                    matches!(&error.fields[1].type_ref,
                    TypeRef::Instantiated { base, args } if *base == TypeId::MAYBE && args[0] == TypeRef::Concrete(location.id)),
                    "nested field must reference {owner}.Location: {:?}",
                    error.fields[1].type_ref
                );
            }
            let packet = nominal(&module, owner, "Packet");
            let payload = packet
                .variants
                .iter()
                .find(|v| module.strings.get(v.name) == Some("Payload"))
                .unwrap();
            assert_eq!(
                payload.fields[0].type_ref,
                TypeRef::Concrete(error.id),
                "tuple variant owner {owner}"
            );
            let named = packet
                .variants
                .iter()
                .find(|v| module.strings.get(v.name) == Some("Named"))
                .unwrap();
            assert!(
                matches!(&named.fields[0].type_ref,
                TypeRef::Instantiated { base, args } if *base == TypeId::MAYBE && args[0] == TypeRef::Concrete(location.id)),
                "record variant owner {owner}: {:?}",
                named.fields[0].type_ref
            );
        }
        assert_ne!(
            nominal(&module, "alpha", "IntrinsicPanicInfo").id,
            nominal(&module, "beta", "IntrinsicPanicInfo").id
        );
    }
}

#[test]
fn source_owned_results_and_parameters_survive_both_import_orders() {
    check_nominal_imports(true);
}

#[test]
fn nested_fields_and_variants_survive_both_import_orders() {
    check_nominal_imports(false);
}

#[test]
fn bundled_protocol_signatures_keep_exact_source_owners() {
    let modules: List<_> = ["alpha.child", "beta.child"].into_iter().map(|owner| {
        let source = format!("module {owner}; type Visitor is protocol {{ fn visit(&self) -> Int; }}; fn relay(value: Visitor) -> Visitor {{ value }}");
        let ast = Parser::new(&source).parse_module().unwrap();
        let mut module = VbcCodegen::with_config(CodegenConfig::new(owner)).compile_module(&ast).unwrap();
        // Bootstrap bundles source units, retaining their declaration origins.
        module.name = owner.split('.').next().unwrap().to_owned();
        verum_vbc::deserialize::deserialize_module(&verum_vbc::serialize::serialize_module(&module).unwrap()).unwrap()
    }).collect();
    for order in [[0, 1], [1, 0]] {
        let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        for (slot, index) in order.into_iter().enumerate() {
            install(&mut cg, &modules[index], 9000 + slot as u32 * 100);
        }
        install(&mut cg, &modules[order[0]], 9000);
        let signatures: List<_> = ["alpha.child.relay", "beta.child.relay"]
            .into_iter()
            .map(|name| {
                (
                    name,
                    cg.ctx_mut()
                        .lookup_function(name)
                        .unwrap()
                        .return_type
                        .clone()
                        .unwrap(),
                )
            })
            .collect();
        let module = cg.finalize_module_from_state().unwrap();
        let alpha = nominal(&module, "alpha.child", "Visitor");
        let beta = nominal(&module, "beta.child", "Visitor");
        assert_ne!(
            alpha.id, beta.id,
            "foreign same-leaf protocols remain distinct"
        );
        for ((name, result), ty) in signatures.into_iter().zip([alpha, beta]) {
            assert_eq!(result, TypeRef::Concrete(ty.id), "{name}");
            let function = module
                .functions
                .iter()
                .find(|f| module.strings.get(f.name) == Some(name))
                .unwrap();
            assert_eq!(
                function.params[0].type_ref,
                TypeRef::Concrete(ty.id),
                "{name}"
            );
        }
    }
}

#[test]
fn omitted_field_spelling_uses_remapped_descriptor_for_nested_projection() {
    let ast = Parser::new(
        "module alpha; type Inner is { other: Int, needle: Int }; type Outer is { inner: Inner };",
    )
    .parse_module()
    .unwrap();
    let mut source = VbcCodegen::with_config(CodegenConfig::new("alpha"))
        .compile_module(&ast)
        .unwrap();
    for ty in &mut source.types {
        if source.strings.get(ty.name) == Some("Outer") {
            ty.fields[0].type_name = verum_vbc::types::StringId::EMPTY;
        }
    }
    let source = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&source).unwrap(),
    )
    .unwrap();
    let ast = Parser::new("module consumer; type Inner is { needle: Int, other: Int }; fn probe(value: &alpha.Outer) -> Int { value.inner.needle }").parse_module().unwrap();
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    cg.collect_unit_declarations(&[&ast]).unwrap();
    install(&mut cg, &source, 10000);
    let module = cg.compile_function_bodies(&ast).unwrap();
    let probe = module
        .functions
        .iter()
        .find(|f| module.strings.get(f.name) == Some("consumer.probe"))
        .unwrap();
    let mut fields = List::new();
    let mut pc = probe.bytecode_offset as usize;
    let end = pc + probe.bytecode_length as usize;
    while pc < end {
        let op = verum_vbc::bytecode::decode_instruction(&module.bytecode, &mut pc).unwrap();
        assert!(!matches!(
            op,
            verum_vbc::instruction::Instruction::GetFieldNamed { .. }
        ));
        if let verum_vbc::instruction::Instruction::GetF { field_idx, .. } = op {
            fields.push(field_idx);
        }
    }
    assert_eq!(
        fields.as_slice(),
        &[0, 1],
        "nested projection follows alpha.Inner, never the coincident foreign numeric ID"
    );
}
