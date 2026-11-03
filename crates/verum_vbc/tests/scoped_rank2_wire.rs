//! Scope is part of a type's meaning: old ambiguous carriers cannot be re-labelled.
use verum_vbc::{
    Instruction, Reg, VbcModule,
    archive::ArchiveBuilder,
    bytecode::{decode_instructions, encode_instruction},
    deserialize::deserialize_module,
    error::VbcError,
    module::FunctionDescriptor,
    serialize::serialize_module,
    types::{ContextRef, TypeId, TypeParamId, TypeRef},
};

fn generic(id: u16) -> TypeRef {
    TypeRef::Generic(TypeParamId(id))
}
fn scoped() -> TypeRef {
    TypeRef::Rank2Function {
        type_param_count: 1,
        params: vec![generic(0), generic(1)],
        return_type: Box::new(TypeRef::Rank2Function {
            type_param_count: 2,
            params: vec![generic(0), generic(1), generic(2), generic(3)],
            return_type: Box::new(generic(2)),
            contexts: Default::default(),
        }),
        contexts: [ContextRef(7)].into_iter().collect(),
    }
}
fn module(ty: TypeRef) -> VbcModule {
    let mut module = VbcModule::new("scoped-wire".into());
    let mut function = FunctionDescriptor::new(module.intern_string("apply"));
    function.return_type = ty;
    module.add_function(function);
    module
}
fn instruction(ty: TypeRef) -> Vec<u8> {
    let mut bytes = Vec::new();
    encode_instruction(
        &Instruction::LoadT {
            dst: Reg(0),
            type_ref: ty,
        },
        &mut bytes,
    );
    bytes
}

#[test]
fn nested_scopes_round_trip_through_module_instruction_and_archive() {
    let ty = scoped();
    let module = module(ty.clone());
    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();
    assert_eq!(loaded.header.version_minor, verum_vbc::VERSION_MINOR);
    assert!(loaded.header.version_minor >= 35);
    assert_eq!(loaded.functions[0].return_type, ty);
    let code = instruction(ty.clone());
    assert_eq!(code[2], 0x0C);
    assert!(
        matches!(&decode_instructions(&code).unwrap()[0], Instruction::LoadT { type_ref, .. } if type_ref == &ty)
    );
    let mut builder = ArchiveBuilder::new();
    builder.add_module("scoped", &module, &[]).unwrap();
    assert_eq!(
        builder.finish().load_module("scoped").unwrap().functions[0].return_type,
        ty
    );
}

#[test]
fn legacy_module_rank2_is_rejected_even_inside_an_archive_or_new_header() {
    let ty = scoped();
    let bytes = serialize_module(&module(ty)).unwrap();
    // Fixed module type encoding: scope tag, u16 binder count, parameter count,
    // first Generic(u16) parameter. Locate the one outer declaration, not a byte
    // that happens to equal the tag in another section.
    let prefix = [0x0C, 1, 0, 2, 0x02, 0, 0];
    let offsets: Vec<_> = bytes
        .windows(prefix.len())
        .enumerate()
        .filter_map(|(i, window)| (window == prefix).then_some(i))
        .collect();
    assert_eq!(offsets.len(), 1);
    let mut noncanonical = bytes.clone();
    noncanonical[offsets[0]] = 0x1C;
    assert!(matches!(
        deserialize_module(&noncanonical),
        Err(VbcError::InvalidTypeRefTag(0x1C))
    ));
    for minor in [34u16, 35] {
        let mut legacy = bytes.clone();
        legacy[6..8].copy_from_slice(&minor.to_le_bytes());
        legacy[offsets[0]] = 0x09;
        assert!(matches!(
            deserialize_module(&legacy),
            Err(VbcError::InvalidTypeRefTag(0x09))
        ));
        let mut builder = ArchiveBuilder::new();
        builder.add_module_data("legacy", legacy, &[]).unwrap();
        assert!(matches!(
            builder.finish().load_module("legacy"),
            Err(VbcError::InvalidTypeRefTag(0x09))
        ));
    }
}

#[test]
fn source_use_seals_preserve_scoped_types_and_distinguish_local_from_outer_ids() {
    use verum_vbc::value_use::ValueUsePlan;
    let mut module = module(scoped());
    let descriptor = &module.functions[0];
    let seal =
        ValueUsePlan::signature_hash(descriptor, &[]).expect("scoped signature remains sealable");
    let decoded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    assert_eq!(
        ValueUsePlan::signature_hash(&decoded.functions[0], &[]),
        Some(seal)
    );
    let TypeRef::Rank2Function { params, .. } = &mut module.functions[0].return_type else {
        unreachable!()
    };
    params.swap(0, 1);
    assert_ne!(
        ValueUsePlan::signature_hash(&module.functions[0], &[]).unwrap(),
        seal
    );
}

#[test]
fn legacy_signature_seals_are_invalidated_even_for_non_rank2_types() {
    use verum_common::List;
    use verum_vbc::value_use::{ValueUsePlan, ValueUseReceipt};
    let descriptor = module(TypeRef::Concrete(TypeId::INT)).functions.remove(0);
    let params: List<_> = descriptor
        .params
        .iter()
        .map(|p| (&p.type_ref, p.is_mut))
        .collect();
    let type_ids: List<_> = descriptor.type_params.iter().map(|p| p.id).collect();
    let uses: Vec<ValueUseReceipt> = Vec::new();
    let legacy = bincode::serialize(&(
        params,
        type_ids,
        &descriptor.return_type,
        &descriptor.yield_type,
        &descriptor.semantic_params,
        &descriptor.semantic_return,
        descriptor.register_count,
        uses.as_slice(),
    ))
    .unwrap();
    let old = *blake3::hash(&legacy).as_bytes();
    let current = ValueUsePlan::signature_hash(&descriptor, &uses).unwrap();
    assert_ne!(current, old);
    let mut plan = ValueUsePlan {
        body_hash: [0; 32],
        signature_hash: old,
        uses: List::new(),
    };
    assert!(!plan.matches_signature(&descriptor));
    plan.signature_hash = current;
    assert!(plan.matches_signature(&descriptor));
}

#[test]
fn legacy_instruction_rank2_and_overflowed_scope_ids_are_rejected() {
    let mut legacy = instruction(scoped());
    legacy[2] = 0x08;
    assert!(matches!(
        decode_instructions(&legacy),
        Err(VbcError::InvalidTypeRef {
            discriminant: 0x08,
            ..
        })
    ));
    for tag in [0x01, 0x0C] {
        let mut bad = instruction(generic(0));
        bad.truncate(2);
        bad.extend([tag, 0x80, 0x80, 0x04]); // 65536 cannot become local/outer ID zero.
        assert!(
            matches!(decode_instructions(&bad), Err(VbcError::InvalidTypeRef { discriminant, .. }) if discriminant == tag)
        );
    }
    let code = instruction(generic(u16::MAX));
    assert!(
        matches!(&decode_instructions(&code).unwrap()[0], Instruction::LoadT { type_ref, .. } if type_ref == &generic(u16::MAX))
    );
}

#[test]
fn json_requires_explicit_scoped_identity_including_nested_descriptors() {
    let ty = scoped();
    let json = serde_json::to_string(&ty).unwrap();
    assert!(json.contains("ScopedRank2Function"));
    assert_eq!(serde_json::from_str::<TypeRef>(&json).unwrap(), ty);
    assert!(
        serde_json::from_str::<TypeRef>(&json.replace("ScopedRank2Function", "Rank2Function"))
            .is_err()
    );
    let descriptor = module(ty).functions.remove(0);
    let json = serde_json::to_string(&descriptor).unwrap();
    assert!(serde_json::from_str::<FunctionDescriptor>(&json).is_ok());
    assert!(
        serde_json::from_str::<FunctionDescriptor>(
            &json.replace("ScopedRank2Function", "Rank2Function")
        )
        .is_err()
    );
}

#[test]
fn unversioned_binary_serde_cannot_bypass_the_rank2_migration() {
    assert!(bincode::serialize(&scoped()).is_err());
    // Legacy bincode enum ordinal 4, one binder, no value parameters,
    // Concrete(Int) return, no contexts. Renaming a serde variant alone leaves
    // this binary shape unchanged and would silently accept the old meaning.
    let mut legacy = Vec::new();
    legacy.extend(4u32.to_le_bytes());
    legacy.extend(1u16.to_le_bytes());
    legacy.extend(0u64.to_le_bytes());
    legacy.extend(0u32.to_le_bytes());
    legacy.extend(TypeId::INT.0.to_le_bytes());
    legacy.extend(0u64.to_le_bytes());
    assert!(bincode::deserialize::<TypeRef>(&legacy).is_err());
}

#[test]
fn non_rank2_legacy_carriers_keep_their_meaning_and_wire_tags() {
    for (ty, opcode_tag) in [
        (TypeRef::Concrete(TypeId::INT), 0x00),
        (generic(u16::MAX), 0x01),
        (TypeRef::Slice(Box::new(generic(4))), 0x07),
        (TypeRef::ConstValue(-7), 0x0A),
    ] {
        let code = instruction(ty.clone());
        assert_eq!(code[2], opcode_tag);
        assert!(
            matches!(&decode_instructions(&code).unwrap()[0], Instruction::LoadT { type_ref, .. } if type_ref == &ty)
        );
        let mut bytes = serialize_module(&module(ty.clone())).unwrap();
        bytes[6..8].copy_from_slice(&34u16.to_le_bytes());
        assert_eq!(
            deserialize_module(&bytes).unwrap().functions[0].return_type,
            ty
        );
        let encoded = bincode::serialize(&ty).unwrap();
        assert_eq!(bincode::deserialize::<TypeRef>(&encoded).unwrap(), ty);
        let json = serde_json::to_string(&ty).unwrap();
        assert_eq!(serde_json::from_str::<TypeRef>(&json).unwrap(), ty);
    }
}
