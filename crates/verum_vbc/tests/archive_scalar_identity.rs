//! T1546: scalar implementation carriers do not introduce nominal scalar IDs.
#![cfg(feature = "codegen")]

use std::collections::HashMap;
use verum_vbc::bytecode::encode_instructions_with_fixup;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::{Instruction, Reg};
use verum_vbc::module::{FunctionDescriptor, FunctionId, ParamDescriptor, VbcModule};
use verum_vbc::types::{ProtocolId, ProtocolImpl, TypeDescriptor, TypeId, TypeKind, TypeRef};

fn carrier(owner: &str, name: &str, id: TypeId) -> VbcModule {
    let mut module = VbcModule::new(owner.to_owned());
    let origin = module.strings.intern(owner);
    let protocol = TypeId(1500);
    let protocol_name = module.strings.intern(&format!("{owner}Protocol"));
    module.types.push(TypeDescriptor {
        id: protocol,
        name: protocol_name,
        kind: TypeKind::Protocol,
        origin_module: Some(origin),
        ..Default::default()
    });
    let name = module.strings.intern(name);
    let carried_arg = module.strings.intern(&format!("{owner}Argument"));
    module.types.push(TypeDescriptor {
        id,
        name,
        kind: TypeKind::Primitive,
        origin_module: Some(origin),
        protocols: [ProtocolImpl {
            protocol: ProtocolId(protocol.0),
            methods: vec![],
            associated_types: vec![],
            protocol_args_text: vec![carried_arg],
            type_param_fn_bounds: vec![],
        }]
        .into_iter()
        .collect(),
        ..Default::default()
    });
    let instructions = vec![Instruction::Ret { value: Reg(0) }];
    let len = encode_instructions_with_fixup(&instructions, &mut module.bytecode);
    module.functions.push(FunctionDescriptor {
        id: FunctionId(0),
        name: module.strings.intern(&format!("{owner}.identity")),
        params: [ParamDescriptor {
            name: module.strings.intern("value"),
            type_ref: TypeRef::Concrete(id),
            ..Default::default()
        }]
        .into_iter()
        .collect(),
        return_type: TypeRef::Concrete(id),
        register_count: 1,
        bytecode_length: len as u32,
        instructions: Some(instructions),
        ..Default::default()
    });
    module
}

fn import(modules: &[&VbcModule]) -> VbcModule {
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    for i in 0..100 {
        cg.ctx_mut().intern_string_raw(&format!("unrelated_{i}"));
    }
    for source in modules {
        cg.import_archive_module_types(source);
    }
    for (i, source) in modules.iter().enumerate() {
        cg.merge_archive_function_bodies(
            source,
            &HashMap::from([(0, FunctionId(7000 + i as u32))]),
        );
    }
    let result = cg.finalize_module_from_state().unwrap();
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&result).unwrap(),
    )
    .unwrap()
}

#[test]
fn primitive_method_signatures_keep_canonical_ids_in_both_import_orders() {
    for name in [
        "Bool", "Int", "Float", "Text", "UInt8", "Byte", "USize", "ISize", "Char",
    ] {
        let id = TypeId::from_well_known_scalar_name(name).unwrap();
        let alpha = carrier("alpha", name, id);
        let beta = carrier("beta", name, id);
        for sources in [[&alpha, &beta], [&beta, &alpha]] {
            let module = import(&sources);
            assert_eq!(module.functions.len(), 2);
            for function in &module.functions {
                assert_eq!(
                    function.return_type,
                    TypeRef::Concrete(id),
                    "{name}: {:?}",
                    module.strings.get(function.name)
                );
                assert_eq!(function.params[0].type_ref, TypeRef::Concrete(id), "{name}");
            }
        }
    }
}

#[test]
fn scalar_protocol_attachments_survive_combining_and_repeated_imports() {
    let alpha = carrier("alpha", "Bool", TypeId::BOOL);
    let beta = carrier("beta", "Bool", TypeId::BOOL);
    for sources in [[&alpha, &beta, &alpha], [&beta, &alpha, &beta]] {
        let module = import(&sources);
        let scalars = module
            .types
            .iter()
            .filter(|ty| ty.id == TypeId::BOOL)
            .collect::<Vec<_>>();
        assert_eq!(scalars.len(), 1);
        let mut args = scalars[0]
            .protocols
            .iter()
            .map(|pi| {
                let proto = module
                    .types
                    .iter()
                    .find(|ty| ty.id.0 == pi.protocol.0)
                    .unwrap();
                (
                    module.strings.get(proto.name).unwrap(),
                    module.strings.get(pi.protocol_args_text[0]).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        args.sort();
        assert_eq!(
            args,
            [
                ("alphaProtocol", "alphaArgument"),
                ("betaProtocol", "betaArgument")
            ]
        );
    }
}

#[test]
fn scalar_alias_carriers_share_identity_and_preserve_both_implementations() {
    let byte = carrier("alpha", "Byte", TypeId::BYTE);
    let u8 = carrier("beta", "UInt8", TypeId::U8);
    for sources in [[&byte, &u8], [&u8, &byte]] {
        let module = import(&sources);
        let mut aliases = module
            .types
            .iter()
            .filter(|ty| ty.id == TypeId::U8)
            .map(|ty| {
                assert_eq!(ty.protocols.len(), 1);
                (
                    module.strings.get(ty.name).unwrap(),
                    module
                        .strings
                        .get(ty.protocols[0].protocol_args_text[0])
                        .unwrap(),
                )
            })
            .collect::<Vec<_>>();
        aliases.sort();
        assert_eq!(
            aliases,
            [("Byte", "alphaArgument"), ("UInt8", "betaArgument")]
        );
        for function in &module.functions {
            assert_eq!(function.return_type, TypeRef::Concrete(TypeId::U8));
        }
    }
}

#[test]
fn nominal_same_named_type_never_claims_scalar_identity() {
    let scalar = carrier("alpha", "Bool", TypeId::BOOL);
    let mut nominal = carrier("beta", "Bool", TypeId(1600));
    nominal.types[1].kind = TypeKind::Record;
    for sources in [[&scalar, &nominal], [&nominal, &scalar]] {
        let module = import(&sources);
        let a = module
            .functions
            .iter()
            .find(|f| module.strings.get(f.name) == Some("alpha.identity"))
            .unwrap();
        let b = module
            .functions
            .iter()
            .find(|f| module.strings.get(f.name) == Some("beta.identity"))
            .unwrap();
        assert_eq!(a.return_type, TypeRef::Concrete(TypeId::BOOL));
        assert_ne!(b.return_type, a.return_type);
    }
}

#[test]
fn a_primitive_name_without_its_declared_builtin_id_is_not_a_scalar_carrier() {
    let scalar = carrier("alpha", "Bool", TypeId::BOOL);
    let foreign = carrier("beta", "Bool", TypeId(1600));
    let module = import(&[&scalar, &foreign]);
    let a = module
        .functions
        .iter()
        .find(|f| module.strings.get(f.name) == Some("alpha.identity"))
        .unwrap();
    let b = module
        .functions
        .iter()
        .find(|f| module.strings.get(f.name) == Some("beta.identity"))
        .unwrap();
    assert_eq!(a.return_type, TypeRef::Concrete(TypeId::BOOL));
    assert_ne!(b.return_type, a.return_type);
}

#[test]
fn eager_registration_preserves_scalar_alias_ids_and_names() {
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    for name in ["Byte", "UInt8", "USize", "ISize"] {
        let id = TypeId::from_well_known_scalar_name(name).unwrap();
        let descriptor = TypeDescriptor {
            id,
            name: verum_vbc::types::StringId(cg.ctx_mut().intern_string_raw(name)),
            kind: TypeKind::Primitive,
            ..Default::default()
        };
        cg.register_archive_type_qualified(descriptor, name.to_owned(), Some("alpha"), None);
    }
    let module = cg.finalize_module_from_state().unwrap();
    for name in ["Byte", "UInt8", "USize", "ISize"] {
        let descriptor = module
            .types
            .iter()
            .find(|ty| module.strings.get(ty.name) == Some(name))
            .unwrap();
        assert_eq!(
            descriptor.id,
            TypeId::from_well_known_scalar_name(name).unwrap()
        );
    }
}
