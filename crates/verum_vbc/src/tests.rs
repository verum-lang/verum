//! Comprehensive tests for verum_vbc crate.

use crate::deserialize::deserialize_module;
use crate::encoding::*;
use crate::format::VbcFlags;
use crate::instruction::{Opcode, Reg, RegRange};
use crate::module::{CallingConvention, Constant, ConstId, FunctionDescriptor, FunctionId, OptimizationHints, ParamDescriptor, VbcModule};
use crate::serialize::serialize_module;
use crate::types::{
    CbgrTier, ContextRef, FieldDescriptor, Mutability, PropertySet, ProtocolId,
    TypeDescriptor, TypeId, TypeKind, TypeParamDescriptor, TypeParamId, TypeRef, VariantDescriptor,
    VariantKind, Variance, Visibility,
};
use crate::validate::validate_module;
use crate::value::Value;

// ============================================================================
// Round-Trip Serialization Tests
// ============================================================================

#[test]
fn test_roundtrip_empty_module() {
    let module = VbcModule::new("empty_module".to_string());
    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    assert_eq!(module.name, loaded.name);
    assert_eq!(module.types.len(), loaded.types.len());
    assert_eq!(module.functions.len(), loaded.functions.len());
    assert_eq!(module.constants.len(), loaded.constants.len());
}

#[test]
fn test_roundtrip_with_all_constant_types() {
    let mut module = VbcModule::new("constants".to_string());

    // Add all constant types
    let str_id = module.intern_string("test string");

    module.add_constant(Constant::Int(i64::MIN));
    module.add_constant(Constant::Int(0));
    module.add_constant(Constant::Int(i64::MAX));
    module.add_constant(Constant::Float(f64::NEG_INFINITY));
    module.add_constant(Constant::Float(0.0));
    module.add_constant(Constant::Float(f64::INFINITY));
    module.add_constant(Constant::String(str_id));
    module.add_constant(Constant::Type(TypeRef::Concrete(TypeId::INT)));
    module.add_constant(Constant::Function(FunctionId(0)));
    module.add_constant(Constant::Protocol(ProtocolId(0)));
    module.add_constant(Constant::Array(vec![ConstId(0), ConstId(1)]));
    module.add_constant(Constant::Bytes(vec![0, 1, 2, 3, 255]));

    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    assert_eq!(module.constants.len(), loaded.constants.len());

    // Verify specific constants
    assert_eq!(loaded.constants[0], Constant::Int(i64::MIN));
    assert_eq!(loaded.constants[2], Constant::Int(i64::MAX));
}

#[test]
fn test_roundtrip_complex_type_refs() {
    let mut module = VbcModule::new("complex_types".to_string());

    // Add various TypeRefs as constants
    let refs = vec![
        TypeRef::Concrete(TypeId::UNIT),
        TypeRef::Concrete(TypeId::INT),
        TypeRef::Generic(TypeParamId(0)),
        TypeRef::Instantiated {
            base: TypeId(100),
            args: vec![
                TypeRef::Concrete(TypeId::INT),
                TypeRef::Concrete(TypeId::TEXT),
            ],
        },
        TypeRef::Function {
            params: vec![TypeRef::Concrete(TypeId::INT)],
            return_type: Box::new(TypeRef::Concrete(TypeId::BOOL)),
            contexts: smallvec::smallvec![ContextRef(1)],
        },
        TypeRef::Reference {
            inner: Box::new(TypeRef::Concrete(TypeId::TEXT)),
            mutability: Mutability::Mutable,
            tier: CbgrTier::Tier1,
        },
        TypeRef::Tuple(vec![
            TypeRef::Concrete(TypeId::INT),
            TypeRef::Concrete(TypeId::FLOAT),
        ]),
        TypeRef::Array {
            element: Box::new(TypeRef::Concrete(TypeId::U8)),
            length: 256,
        },
        TypeRef::Slice(Box::new(TypeRef::Concrete(TypeId::I32))),
    ];

    for type_ref in refs.iter() {
        module.add_constant(Constant::Type(type_ref.clone()));
    }

    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    assert_eq!(module.constants.len(), loaded.constants.len());

    // Verify each TypeRef was preserved
    for (i, expected) in refs.iter().enumerate() {
        if let Constant::Type(actual) = &loaded.constants[i] {
            assert_eq!(expected, actual, "TypeRef mismatch at index {}", i);
        } else {
            panic!("Expected Constant::Type at index {}", i);
        }
    }
}

#[test]
fn test_roundtrip_type_descriptor() {
    let mut module = VbcModule::new("types".to_string());

    let name = module.intern_string("Point");
    let x_name = module.intern_string("x");
    let y_name = module.intern_string("y");

    let desc = TypeDescriptor {
        id: TypeId(16),
        name,
        kind: TypeKind::Record,
        type_params: smallvec::smallvec![],
        fields: smallvec::smallvec![
            FieldDescriptor {
                name: x_name,
                type_ref: TypeRef::Concrete(TypeId::FLOAT),
                offset: 0,
                visibility: Visibility::Public,
            },
            FieldDescriptor {
                name: y_name,
                type_ref: TypeRef::Concrete(TypeId::FLOAT),
                offset: 8,
                visibility: Visibility::Public,
            },
        ],
        variants: smallvec::smallvec![],
        size: 16,
        alignment: 8,
        drop_fn: None,
        clone_fn: Some(100),
        protocols: smallvec::smallvec![],
        visibility: Visibility::Public,
    };

    module.types.push(desc);
    module.header.type_table_count = 1;

    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    assert_eq!(loaded.types.len(), 1);
    assert_eq!(loaded.types[0].fields.len(), 2);
    assert_eq!(loaded.types[0].size, 16);
    assert_eq!(loaded.types[0].clone_fn, Some(100));
}

#[test]
fn test_roundtrip_sum_type() {
    let mut module = VbcModule::new("sum_types".to_string());

    let name = module.intern_string("Option");
    let none_name = module.intern_string("None");
    let some_name = module.intern_string("Some");
    let t_name = module.intern_string("T");

    let desc = TypeDescriptor {
        id: TypeId(16),
        name,
        kind: TypeKind::Sum,
        type_params: smallvec::smallvec![TypeParamDescriptor {
            name: t_name,
            id: TypeParamId(0),
            bounds: smallvec::smallvec![],
            default: None,
            variance: Variance::Covariant,
        }],
        fields: smallvec::smallvec![],
        variants: smallvec::smallvec![
            VariantDescriptor {
                name: none_name,
                tag: 0,
                payload: None,
                kind: VariantKind::Unit,
                arity: 0,
                fields: smallvec::smallvec![],
            },
            VariantDescriptor {
                name: some_name,
                tag: 1,
                payload: Some(TypeRef::Generic(TypeParamId(0))),
                kind: VariantKind::Tuple,
                arity: 1,
                fields: smallvec::smallvec![],
            },
        ],
        size: 0, // Generic, computed at instantiation
        alignment: 8,
        drop_fn: None,
        clone_fn: None,
        protocols: smallvec::smallvec![],
        visibility: Visibility::Public,
    };

    module.types.push(desc);
    module.header.type_table_count = 1;

    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    assert_eq!(loaded.types[0].variants.len(), 2);
    assert_eq!(loaded.types[0].type_params.len(), 1);
    assert!(loaded.types[0].variants[1].payload.is_some());
}

#[test]
fn test_roundtrip_function_descriptor() {
    let mut module = VbcModule::new("functions".to_string());

    let name = module.intern_string("add");
    let a_name = module.intern_string("a");
    let b_name = module.intern_string("b");

    let desc = FunctionDescriptor {
        id: FunctionId(0),
        name,
        parent_type: None,
        type_params: smallvec::smallvec![],
        params: smallvec::smallvec![
            ParamDescriptor {
                name: a_name,
                type_ref: TypeRef::Concrete(TypeId::INT),
                is_mut: false,
                default: None,
            },
            ParamDescriptor {
                name: b_name,
                type_ref: TypeRef::Concrete(TypeId::INT),
                is_mut: false,
                default: None,
            },
        ],
        return_type: TypeRef::Concrete(TypeId::INT),
        contexts: smallvec::smallvec![],
        properties: PropertySet::PURE,
        bytecode_offset: 0,
        bytecode_length: 10,
        locals_count: 0,
        register_count: 3,
        max_stack: 0,
        is_inline_candidate: true,
        is_generic: false,
        visibility: Visibility::Public,
        is_generator: false,
        yield_type: None,
        suspend_point_count: 0,
        calling_convention: CallingConvention::C,
        optimization_hints: OptimizationHints::default(),
        instructions: None,
        func_id_base: 0,
        debug_variables: Vec::new(),
        is_test: false,
    };

    // Add some dummy bytecode
    module.bytecode = vec![0x10, 0x00, 0x01, 0x02, 0x59, 0x00, 0, 0, 0, 0];
    module.functions.push(desc);
    module.header.function_table_count = 1;
    module.header.bytecode_size = 10;

    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    assert_eq!(loaded.functions.len(), 1);
    assert_eq!(loaded.functions[0].params.len(), 2);
    assert!(loaded.functions[0].properties.contains(PropertySet::PURE));
    assert!(loaded.functions[0].is_inline_candidate);
}

#[test]
fn test_roundtrip_with_specializations() {
    use crate::module::SpecializationEntry;

    let mut module = VbcModule::new("specialized".to_string());

    module.specializations.push(SpecializationEntry {
        generic_fn: FunctionId(0),
        type_args: vec![TypeRef::Concrete(TypeId::INT)],
        hash: 0x123456789ABCDEF0,
        bytecode_offset: 100,
        bytecode_length: 50,
        register_count: 5,
    });

    module.header.specialization_table_count = 1;

    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    assert_eq!(loaded.specializations.len(), 1);
    assert_eq!(loaded.specializations[0].hash, 0x123456789ABCDEF0);
    assert_eq!(loaded.specializations[0].type_args.len(), 1);
}

#[test]
fn test_roundtrip_with_source_map() {
    use crate::module::{SourceMap, SourceMapEntry};

    let mut module = VbcModule::new("with_debug".to_string());

    let file = module.intern_string("main.vr");

    module.source_map = Some(SourceMap {
        files: vec![file],
        entries: vec![
            SourceMapEntry {
                bytecode_offset: 0,
                file_idx: 0,
                line: 1,
                column: 1,
            },
            SourceMapEntry {
                bytecode_offset: 10,
                file_idx: 0,
                line: 2,
                column: 5,
            },
        ],
    });

    module.header.flags |= VbcFlags::DEBUG_INFO;

    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    assert!(loaded.source_map.is_some());
    let sm = loaded.source_map.unwrap();
    assert_eq!(sm.files.len(), 1);
    assert_eq!(sm.entries.len(), 2);
    assert_eq!(sm.entries[1].line, 2);
}

// ============================================================================
// Value Tests
// ============================================================================

#[test]
fn test_value_numeric_range() {
    // Test boundary values for inline 48-bit signed integers
    // MAX_SMALL_INT = (1 << 47) - 1 = 140737488355327
    // MIN_SMALL_INT = -(1 << 47) = -140737488355328
    for &val in &[-140737488355328i64, -1, 0, 1, 140737488355327] {
        let v = Value::from_i64(val);
        assert_eq!(v.as_i64(), val);
    }
}

#[test]
fn test_value_special_floats() {
    let special = [
        f64::NEG_INFINITY,
        f64::MIN,
        -0.0,
        0.0,
        f64::MIN_POSITIVE,
        f64::MAX,
        f64::INFINITY,
    ];

    for &f in &special {
        let v = Value::from_f64(f);
        assert_eq!(v.as_f64().to_bits(), f.to_bits());
    }
}

// ============================================================================
// Encoding Tests
// ============================================================================

#[test]
fn test_varint_boundary_values() {
    let values = [
        0u64,
        0x7F,                   // 1 byte max
        0x80,                   // 2 bytes min
        0x3FFF,                 // 2 bytes max
        0x4000,                 // 3 bytes min
        0x1FFFFF,               // 3 bytes max
        u64::MAX >> 1,          // Large value
    ];

    for &val in &values {
        let mut buf = Vec::new();
        encode_varint(val, &mut buf);

        let mut offset = 0;
        let decoded = decode_varint(&buf, &mut offset).unwrap();

        assert_eq!(decoded, val);
        assert_eq!(offset, buf.len());
    }
}

#[test]
fn test_signed_varint_boundary_values() {
    let values = [
        i64::MIN >> 1,
        -1i64,
        0i64,
        1i64,
        i64::MAX >> 1,
    ];

    for &val in &values {
        let mut buf = Vec::new();
        encode_signed_varint(val, &mut buf);

        let mut offset = 0;
        let decoded = decode_signed_varint(&buf, &mut offset).unwrap();

        assert_eq!(decoded, val);
    }
}

// ============================================================================
// Validation Tests
// ============================================================================

#[test]
fn test_validate_valid_module() {
    let mut module = VbcModule::new("valid".to_string());

    // Add valid content
    let name = module.intern_string("test_fn");
    module.functions.push(FunctionDescriptor {
        name,
        return_type: TypeRef::Concrete(TypeId::UNIT),
        ..Default::default()
    });
    module.header.function_table_count = 1;

    assert!(validate_module(&module).is_ok());
}

#[test]
fn test_validate_invalid_type_reference() {
    let mut module = VbcModule::new("invalid".to_string());

    // Add constant with invalid type reference
    module.add_constant(Constant::Type(TypeRef::Concrete(TypeId(99999))));

    let result = validate_module(&module);
    // Should detect invalid type reference
    assert!(result.is_err());
}

// ============================================================================
// Instruction Tests
// ============================================================================

#[test]
fn test_opcode_all_values_valid() {
    // Ensure all 256 byte values map to valid opcodes
    for byte in 0..=255u8 {
        let op = Opcode::from_byte(byte);
        assert_eq!(op.to_byte(), byte);
    }
}

#[test]
fn test_opcode_categories() {
    // Branches
    assert!(Opcode::Jmp.is_branch());
    assert!(Opcode::JmpIf.is_branch());
    assert!(Opcode::Switch.is_branch());
    assert!(!Opcode::Call.is_branch());

    // Returns
    assert!(Opcode::Ret.is_return());
    assert!(Opcode::RetV.is_return());
    assert!(!Opcode::TailCall.is_return());

    // Calls
    assert!(Opcode::Call.is_call());
    assert!(Opcode::CallG.is_call());
    assert!(Opcode::CallV.is_call());
    assert!(!Opcode::Jmp.is_call());

    // Tensors
    assert!(Opcode::TensorNew.is_tensor());
    assert!(Opcode::TensorMatmul.is_tensor());
    assert!(Opcode::TensorExtended.is_tensor());
    assert!(!Opcode::AddI.is_tensor());

    // GPU
    assert!(Opcode::GpuExtended.is_gpu());
    assert!(Opcode::GpuSync.is_gpu());
    assert!(!Opcode::TensorMatmul.is_gpu());
}

#[test]
fn test_reg_range_iteration() {
    let range = RegRange::new(Reg(10), 5);
    let regs: Vec<Reg> = range.iter().collect();

    assert_eq!(regs.len(), 5);
    assert_eq!(regs[0], Reg(10));
    assert_eq!(regs[4], Reg(14));
}

// ============================================================================
// Header Tests
// ============================================================================

#[test]
fn test_header_flags() {
    let all_flags = VbcFlags::HAS_GENERICS
        | VbcFlags::HAS_PRECOMPILED_SPECS
        | VbcFlags::NEEDS_CBGR
        | VbcFlags::HAS_ASYNC
        | VbcFlags::HAS_CONTEXTS
        | VbcFlags::HAS_REFINEMENTS
        | VbcFlags::IS_STDLIB
        | VbcFlags::DEBUG_INFO
        | VbcFlags::COMPRESSED
        | VbcFlags::HAS_TENSORS
        | VbcFlags::HAS_AUTODIFF
        | VbcFlags::HAS_GPU;

    // Serialize and deserialize module with all flags
    let mut module = VbcModule::new("all_flags".to_string());
    module.header.flags = all_flags;

    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    assert_eq!(loaded.header.flags, all_flags);
}

// ============================================================================
// Performance Sanity Tests
// ============================================================================

#[test]
fn test_serialize_many_strings() {
    let mut module = VbcModule::new("many_strings".to_string());

    // Add 1000 strings
    for i in 0..1000 {
        module.intern_string(&format!("string_{}", i));
    }

    let start = std::time::Instant::now();
    let bytes = serialize_module(&module).unwrap();
    let serialize_time = start.elapsed();

    let start = std::time::Instant::now();
    let _ = deserialize_module(&bytes).unwrap();
    let deserialize_time = start.elapsed();

    // Should complete in reasonable time (< 100ms each)
    assert!(
        serialize_time.as_millis() < 100,
        "Serialize took too long: {:?}",
        serialize_time
    );
    assert!(
        deserialize_time.as_millis() < 100,
        "Deserialize took too long: {:?}",
        deserialize_time
    );
}

#[test]
fn test_serialize_many_functions() {
    let mut module = VbcModule::new("many_functions".to_string());

    // Add 1000 functions
    for i in 0..1000 {
        let name = module.intern_string(&format!("fn_{}", i));
        module.functions.push(FunctionDescriptor {
            id: FunctionId(i as u32),
            name,
            ..Default::default()
        });
    }
    module.header.function_table_count = 1000;

    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    assert_eq!(loaded.functions.len(), 1000);
}

// ============================================================================
// Exhaustive Edge Case Tests
// ============================================================================

#[test]
fn test_roundtrip_all_type_kinds() {
    let mut module = VbcModule::new("type_kinds".to_string());

    let kinds = [
        TypeKind::Unit,
        TypeKind::Primitive,
        TypeKind::Record,
        TypeKind::Sum,
        TypeKind::Protocol,
        TypeKind::Newtype,
        TypeKind::Tuple,
        TypeKind::Array,
        TypeKind::Tensor,
    ];

    for (i, kind) in kinds.iter().enumerate() {
        let name = module.intern_string(&format!("Type{}", i));
        module.types.push(TypeDescriptor {
            id: TypeId(16 + i as u32),
            name,
            kind: *kind,
            type_params: smallvec::smallvec![],
            fields: smallvec::smallvec![],
            variants: smallvec::smallvec![],
            size: 8,
            alignment: 8,
            drop_fn: None,
            clone_fn: None,
            protocols: smallvec::smallvec![],
            visibility: Visibility::Public,
        });
    }
    module.header.type_table_count = kinds.len() as u32;

    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    assert_eq!(loaded.types.len(), kinds.len());
    for (i, kind) in kinds.iter().enumerate() {
        assert_eq!(loaded.types[i].kind, *kind, "Kind mismatch at index {}", i);
    }
}

#[test]
fn test_roundtrip_all_visibility_levels() {
    let mut module = VbcModule::new("visibility".to_string());

    let visibilities = [
        Visibility::Public,
        Visibility::Private,
        Visibility::Cog,
    ];

    for (i, vis) in visibilities.iter().enumerate() {
        let name = module.intern_string(&format!("Type{}", i));
        module.types.push(TypeDescriptor {
            id: TypeId(16 + i as u32),
            name,
            kind: TypeKind::Unit,
            type_params: smallvec::smallvec![],
            fields: smallvec::smallvec![],
            variants: smallvec::smallvec![],
            size: 0,
            alignment: 1,
            drop_fn: None,
            clone_fn: None,
            protocols: smallvec::smallvec![],
            visibility: *vis,
        });
    }
    module.header.type_table_count = visibilities.len() as u32;

    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    for (i, vis) in visibilities.iter().enumerate() {
        assert_eq!(loaded.types[i].visibility, *vis);
    }
}

#[test]
fn test_roundtrip_all_cbgr_tiers() {
    let mut module = VbcModule::new("cbgr_tiers".to_string());

    let type_refs = [
        TypeRef::Reference {
            inner: Box::new(TypeRef::Concrete(TypeId::INT)),
            mutability: Mutability::Immutable,
            tier: CbgrTier::Tier0,
        },
        TypeRef::Reference {
            inner: Box::new(TypeRef::Concrete(TypeId::INT)),
            mutability: Mutability::Mutable,
            tier: CbgrTier::Tier1,
        },
        TypeRef::Reference {
            inner: Box::new(TypeRef::Concrete(TypeId::INT)),
            mutability: Mutability::Immutable,
            tier: CbgrTier::Tier2,
        },
    ];

    for tr in &type_refs {
        module.add_constant(Constant::Type(tr.clone()));
    }

    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    for (i, expected) in type_refs.iter().enumerate() {
        if let Constant::Type(actual) = &loaded.constants[i] {
            assert_eq!(expected, actual, "CBGR tier mismatch at {}", i);
        }
    }
}

#[test]
fn test_roundtrip_deeply_nested_types() {
    let mut module = VbcModule::new("nested".to_string());

    // Create deeply nested type: &mut &&Tuple<Array<Slice<Int>, 5>, Bool>
    let nested = TypeRef::Reference {
        inner: Box::new(TypeRef::Reference {
            inner: Box::new(TypeRef::Reference {
                inner: Box::new(TypeRef::Tuple(vec![
                    TypeRef::Array {
                        element: Box::new(TypeRef::Slice(Box::new(TypeRef::Concrete(TypeId::INT)))),
                        length: 5,
                    },
                    TypeRef::Concrete(TypeId::BOOL),
                ])),
                mutability: Mutability::Immutable,
                tier: CbgrTier::Tier0,
            }),
            mutability: Mutability::Immutable,
            tier: CbgrTier::Tier1,
        }),
        mutability: Mutability::Mutable,
        tier: CbgrTier::Tier0,
    };

    module.add_constant(Constant::Type(nested.clone()));

    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    if let Constant::Type(actual) = &loaded.constants[0] {
        assert_eq!(&nested, actual);
    } else {
        panic!("Expected Constant::Type");
    }
}

#[test]
fn test_roundtrip_property_sets() {
    let mut module = VbcModule::new("properties".to_string());

    let properties = [
        PropertySet::empty(),
        PropertySet::PURE,
        PropertySet::IO,
        PropertySet::ASYNC,
        PropertySet::FALLIBLE,
        PropertySet::MUTATES,
        PropertySet::IO | PropertySet::ASYNC | PropertySet::FALLIBLE,
        PropertySet::all(),
    ];

    for (i, props) in properties.iter().enumerate() {
        let name = module.intern_string(&format!("fn_{}", i));
        module.functions.push(FunctionDescriptor {
            id: FunctionId(i as u32),
            name,
            properties: *props,
            return_type: TypeRef::Concrete(TypeId::UNIT),
            ..Default::default()
        });
    }
    module.header.function_table_count = properties.len() as u32;

    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    for (i, props) in properties.iter().enumerate() {
        assert_eq!(loaded.functions[i].properties, *props, "Property mismatch at {}", i);
    }
}

#[test]
fn test_roundtrip_empty_and_full_variants() {
    let mut module = VbcModule::new("variants".to_string());

    let name = module.intern_string("Result");
    let ok_name = module.intern_string("Ok");
    let err_name = module.intern_string("Err");
    let val_name = module.intern_string("value");

    module.types.push(TypeDescriptor {
        id: TypeId(16),
        name,
        kind: TypeKind::Sum,
        type_params: smallvec::smallvec![],
        fields: smallvec::smallvec![],
        variants: smallvec::smallvec![
            VariantDescriptor {
                name: ok_name,
                tag: 0,
                payload: Some(TypeRef::Concrete(TypeId::INT)),
                kind: VariantKind::Tuple,
                arity: 1,
                fields: smallvec::smallvec![],
            },
            VariantDescriptor {
                name: err_name,
                tag: 1,
                payload: None,
                kind: VariantKind::Record,
                arity: 1,
                fields: smallvec::smallvec![FieldDescriptor {
                    name: val_name,
                    type_ref: TypeRef::Concrete(TypeId::TEXT),
                    offset: 0,
                    visibility: Visibility::Public,
                }],
            },
        ],
        size: 24,
        alignment: 8,
        drop_fn: None,
        clone_fn: None,
        protocols: smallvec::smallvec![],
        visibility: Visibility::Public,
    });
    module.header.type_table_count = 1;

    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    assert_eq!(loaded.types[0].variants.len(), 2);
    assert_eq!(loaded.types[0].variants[0].kind, VariantKind::Tuple);
    assert_eq!(loaded.types[0].variants[1].kind, VariantKind::Record);
    assert_eq!(loaded.types[0].variants[1].fields.len(), 1);
}

#[test]
fn test_validate_string_table_bounds() {
    let mut module = VbcModule::new("strings".to_string());

    // Add valid string references
    let s1 = module.intern_string("hello");
    let s2 = module.intern_string("world");

    // Verify lookup works
    assert_eq!(module.get_string(s1), Some("hello"));
    assert_eq!(module.get_string(s2), Some("world"));

    // Verify non-existent returns None
    use crate::types::StringId;
    assert_eq!(module.get_string(StringId(99999)), None);
}

#[test]
fn test_string_deduplication() {
    let mut module = VbcModule::new("dedup".to_string());

    let s1 = module.intern_string("duplicate");
    let s2 = module.intern_string("duplicate");
    let s3 = module.intern_string("unique");

    assert_eq!(s1, s2, "Same string should get same ID");
    assert_ne!(s1, s3, "Different strings should get different IDs");
}

#[test]
fn test_constant_id_ordering() {
    let mut module = VbcModule::new("constants".to_string());

    let c0 = module.add_constant(Constant::Int(0));
    let c1 = module.add_constant(Constant::Int(1));
    let c2 = module.add_constant(Constant::Int(2));

    assert_eq!(c0, ConstId(0));
    assert_eq!(c1, ConstId(1));
    assert_eq!(c2, ConstId(2));
}

#[test]
fn test_function_id_assignment() {
    let mut module = VbcModule::new("funcs".to_string());

    let name1 = module.intern_string("fn1");
    let name2 = module.intern_string("fn2");

    let f1 = FunctionDescriptor {
        id: FunctionId(0),
        name: name1,
        return_type: TypeRef::Concrete(TypeId::UNIT),
        ..Default::default()
    };

    let f2 = FunctionDescriptor {
        id: FunctionId(1),
        name: name2,
        return_type: TypeRef::Concrete(TypeId::UNIT),
        ..Default::default()
    };

    module.functions.push(f1);
    module.functions.push(f2);
    module.header.function_table_count = 2;

    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    assert_eq!(loaded.functions[0].id, FunctionId(0));
    assert_eq!(loaded.functions[1].id, FunctionId(1));
}

#[test]
fn test_bytecode_bounds_checking() {
    use crate::bytecode::encode_instruction;
    use crate::instruction::Instruction;

    let mut module = VbcModule::new("bytecode".to_string());

    // Build a real, decodable bytecode body: three `Mov r0, r0` + a
    // `Ret r0` terminator.  `vec![0; 10]` (the original test's
    // approach) was opaque bytes that the per-instruction validator
    // cannot decode — for the bounds-checking invariant the test
    // intends to pin, the body must be well-formed.
    let mut bc = Vec::new();
    encode_instruction(&Instruction::Mov { dst: Reg(0), src: Reg(0) }, &mut bc);
    encode_instruction(&Instruction::Mov { dst: Reg(0), src: Reg(0) }, &mut bc);
    encode_instruction(&Instruction::Mov { dst: Reg(0), src: Reg(0) }, &mut bc);
    encode_instruction(&Instruction::Ret { value: Reg(0) }, &mut bc);
    let real_len = bc.len() as u32;

    let name = module.intern_string("main");
    module.functions.push(FunctionDescriptor {
        id: FunctionId(0),
        name,
        bytecode_offset: 0,
        bytecode_length: real_len,
        return_type: TypeRef::Concrete(TypeId::UNIT),
        register_count: 4,
        ..Default::default()
    });

    module.bytecode = bc.clone();
    module.header.function_table_count = 1;
    module.header.bytecode_size = real_len;

    // Well-formed bytecode + matching length: must pass validation.
    assert!(validate_module(&module).is_ok());

    // Truncate the bytecode buffer below the function descriptor's
    // declared length: the bounds-checking invariant must reject
    // (function references past end-of-buffer).
    module.bytecode = bc[..real_len as usize - 1].to_vec();
    module.header.bytecode_size = real_len - 1;
    assert!(validate_module(&module).is_err());
}

#[test]
fn test_module_with_max_register_count() {
    let mut module = VbcModule::new("many_registers".to_string());

    let name = module.intern_string("big_fn");
    module.functions.push(FunctionDescriptor {
        id: FunctionId(0),
        name,
        register_count: 16384, // Max reasonable count
        return_type: TypeRef::Concrete(TypeId::UNIT),
        ..Default::default()
    });
    module.header.function_table_count = 1;

    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    assert_eq!(loaded.functions[0].register_count, 16384);
}

#[test]
fn test_generic_type_roundtrip() {
    let mut module = VbcModule::new("generic".to_string());

    let generic = TypeRef::Generic(TypeParamId(0));

    module.add_constant(Constant::Type(generic.clone()));

    let bytes = serialize_module(&module).unwrap();
    let loaded = deserialize_module(&bytes).unwrap();

    if let Constant::Type(actual) = &loaded.constants[0] {
        assert_eq!(&generic, actual);
    }
}

// ============================================================================
// CBGR Integration Tests - Phase 3.9
// ============================================================================

/// Test DereferenceCodegen tier mapping.
#[test]
fn test_cbgr_deref_strategy_for_tier() {
    use crate::cbgr::{CbgrTier, DereferenceCodegen};

    // Tier 0: Runtime validation (inline CBGR check)
    let tier0 = DereferenceCodegen::for_tier(CbgrTier::Tier0);
    assert!(matches!(tier0, DereferenceCodegen::InlineCbgrCheck { .. }));

    // Tier 1: Direct access (compiler proven)
    let tier1 = DereferenceCodegen::for_tier(CbgrTier::Tier1);
    assert!(matches!(tier1, DereferenceCodegen::DirectAccess));

    // Tier 2: Unchecked access (unsafe)
    let tier2 = DereferenceCodegen::for_tier(CbgrTier::Tier2);
    assert!(matches!(tier2, DereferenceCodegen::UncheckedAccess));
}

/// Test DereferenceCodegen instruction counts.
#[test]
fn test_cbgr_deref_instruction_count() {
    use crate::cbgr::DereferenceCodegen;

    // Inline CBGR check: 7 instructions
    let inline = DereferenceCodegen::pending_cbgr();
    assert_eq!(inline.instruction_count(), 7);

    // Direct access: 1 instruction
    let direct = DereferenceCodegen::DirectAccess;
    assert_eq!(direct.instruction_count(), 1);

    // Unchecked access: 1 instruction
    let unchecked = DereferenceCodegen::UncheckedAccess;
    assert_eq!(unchecked.instruction_count(), 1);
}

/// Test DereferenceCodegen tier extraction.
#[test]
fn test_cbgr_deref_tier() {
    use crate::cbgr::{CbgrTier, DereferenceCodegen};

    let inline = DereferenceCodegen::pending_cbgr();
    assert_eq!(inline.tier(), CbgrTier::Tier0);

    let direct = DereferenceCodegen::DirectAccess;
    assert_eq!(direct.tier(), CbgrTier::Tier1);

    let unchecked = DereferenceCodegen::UncheckedAccess;
    assert_eq!(unchecked.tier(), CbgrTier::Tier2);
}

/// Test DereferenceCodegen with concrete values.
#[test]
fn test_cbgr_deref_with_values() {
    use crate::cbgr::DereferenceCodegen;

    let strategy = DereferenceCodegen::pending_cbgr();
    assert!(strategy.is_pending());

    let updated = strategy.with_values(42, 5);
    assert!(!updated.is_pending());

    if let DereferenceCodegen::InlineCbgrCheck {
        expected_generation,
        expected_epoch,
        ..
    } = updated
    {
        assert_eq!(expected_generation, 42);
        assert_eq!(expected_epoch, 5);
    } else {
        panic!("Expected InlineCbgrCheck");
    }
}

/// Test RequiredCapability bit masks.
#[test]
fn test_cbgr_required_capability_bits() {
    use crate::cbgr::RequiredCapability;

    assert_eq!(RequiredCapability::Read.bit_mask(), 0x01);
    assert_eq!(RequiredCapability::Write.bit_mask(), 0x02);
    assert_eq!(RequiredCapability::Execute.bit_mask(), 0x04);
}

/// Test CbgrDereferenceStrategy constructors.
#[test]
fn test_cbgr_dereference_strategy() {
    use crate::cbgr::{CbgrDereferenceStrategy, CbgrTier};

    // Test managed read strategy
    let managed = CbgrDereferenceStrategy::managed_read(1, 0);
    assert_eq!(managed.tier, CbgrTier::Tier0);

    // Test checked read strategy
    let checked = CbgrDereferenceStrategy::checked_read();
    assert_eq!(checked.tier, CbgrTier::Tier1);

    // Test unsafe access strategy
    let unsafe_access = CbgrDereferenceStrategy::unsafe_access();
    assert_eq!(unsafe_access.tier, CbgrTier::Tier2);
}

/// Test CbgrDereferenceStrategy overhead estimation.
#[test]
fn test_cbgr_dereference_overhead() {
    use crate::cbgr::CbgrDereferenceStrategy;

    let managed = CbgrDereferenceStrategy::managed_read(1, 0);
    assert_eq!(managed.estimated_overhead_ns(), 16); // 15ns CBGR + 1ns capability

    let checked = CbgrDereferenceStrategy::checked_read();
    assert_eq!(checked.estimated_overhead_ns(), 0);

    let unsafe_access = CbgrDereferenceStrategy::unsafe_access();
    assert_eq!(unsafe_access.estimated_overhead_ns(), 0);
}

/// Test CBGR tier analysis result integration.
#[test]
fn test_cbgr_tier_analysis_integration() {
    use verum_cbgr::analysis::RefId;
    use verum_cbgr::tier_analysis::TierAnalysisResult;

    // Create mock analysis result
    let mut result = TierAnalysisResult::empty();
    result
        .decisions
        .insert(RefId(1), verum_cbgr::tier_types::ReferenceTier::tier1());
    result.decisions.insert(
        RefId(2),
        verum_cbgr::tier_types::ReferenceTier::tier0(verum_cbgr::tier_types::Tier0Reason::Escapes),
    );

    // Verify decisions are recorded
    assert!(!result.decisions.is_empty());
    assert_eq!(result.decisions.len(), 2);
}

/// Test CbgrCodegenStats tracking.
#[test]
fn test_cbgr_codegen_stats() {
    use crate::cbgr::{CbgrCodegenStats, CbgrTier};

    let mut stats = CbgrCodegenStats::new();
    assert_eq!(stats.total_derefs(), 0);

    // Record derefs
    stats.record_deref(CbgrTier::Tier0);
    stats.record_deref(CbgrTier::Tier0);
    stats.record_deref(CbgrTier::Tier1);
    stats.record_deref(CbgrTier::Tier2);

    assert_eq!(stats.tier0_derefs, 2);
    assert_eq!(stats.tier1_derefs, 1);
    assert_eq!(stats.tier2_derefs, 1);
    assert_eq!(stats.total_derefs(), 4);
}

/// Test CbgrCodegenStats optimization rate.
#[test]
fn test_cbgr_optimization_rate() {
    use crate::cbgr::{CbgrCodegenStats, CbgrTier};

    let mut stats = CbgrCodegenStats::new();

    // Empty = 0% rate
    assert_eq!(stats.optimization_rate(), 0.0);

    // All Tier0 = 0% rate
    stats.record_deref(CbgrTier::Tier0);
    stats.record_deref(CbgrTier::Tier0);
    assert_eq!(stats.optimization_rate(), 0.0);

    // 2 Tier1 out of 4 = 50% rate
    stats.record_deref(CbgrTier::Tier1);
    stats.record_deref(CbgrTier::Tier1);
    assert_eq!(stats.optimization_rate(), 0.5);
}
