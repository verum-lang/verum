//! T1553: lifecycle roots survive monomorphization and function-ID compaction.
use std::sync::Arc;

use verum_vbc::{
    bytecode::encode_instructions_with_fixup,
    instruction::{BinaryIntOp, Instruction as I, Reg},
    interpreter::Interpreter,
    module::{FunctionDescriptor, FunctionId, VbcModule},
    mono::{MergeError, ModuleMerger, MonomorphizationResolver},
    types::{TypeId, TypeRef},
};

fn add_function(module: &mut VbcModule, id: u32, name: &str, body: Vec<I>) {
    let offset = module.bytecode.len() as u32;
    let length = encode_instructions_with_fixup(&body, &mut module.bytecode) as u32;
    let mut descriptor = FunctionDescriptor::new(module.intern_string(name));
    descriptor.id = FunctionId(id);
    descriptor.bytecode_offset = offset;
    descriptor.bytecode_length = length;
    descriptor.register_count = 5;
    descriptor.return_type = TypeRef::Concrete(TypeId::INT);
    descriptor.instructions = Some(body);
    module.functions.push(descriptor);
}

fn append_digit(digit: i64) -> Vec<I> {
    vec![
        I::LoadI {
            dst: Reg(0),
            value: 0,
        },
        I::TlsGet {
            dst: Reg(1),
            slot: Reg(0),
        },
        I::LoadI {
            dst: Reg(2),
            value: 10,
        },
        I::BinaryI {
            op: BinaryIntOp::Mul,
            dst: Reg(1),
            a: Reg(1),
            b: Reg(2),
        },
        I::LoadI {
            dst: Reg(2),
            value: digit,
        },
        I::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(1),
            a: Reg(1),
            b: Reg(2),
        },
        I::TlsSet {
            slot: Reg(0),
            val: Reg(1),
        },
        I::Ret { value: Reg(1) },
    ]
}

fn lifecycle_module() -> VbcModule {
    let mut module = VbcModule::new("lifecycle".into());
    add_function(
        &mut module,
        200,
        "probe",
        vec![
            I::LoadI {
                dst: Reg(0),
                value: 0,
            },
            I::TlsGet {
                dst: Reg(1),
                slot: Reg(0),
            },
            I::Ret { value: Reg(1) },
        ],
    );
    add_function(
        &mut module,
        91,
        "__tls_init_library_seed",
        vec![
            I::LoadI {
                dst: Reg(0),
                value: 0,
            },
            I::LoadI {
                dst: Reg(1),
                value: 1,
            },
            I::TlsSet {
                slot: Reg(0),
                val: Reg(1),
            },
            I::Ret { value: Reg(1) },
        ],
    );
    let library = module.intern_string("library.lifecycle");
    module.functions[1].origin_module = Some(library);
    add_function(&mut module, 7, "__tls_init_user_append", append_digit(2));
    add_function(
        &mut module,
        900,
        "__tls_init_library_append",
        append_digit(3),
    );
    module.functions[3].origin_module = Some(library);
    add_function(&mut module, 77, "library_finalizer", append_digit(4));
    module.functions[4].origin_module = Some(library);
    add_function(&mut module, 8, "user_finalizer", append_digit(5));
    // Preserve declaration order AND priorities. The interpreter sorts
    // priorities stably; lowering owns its existing execution-order policy.
    module.global_ctors = vec![
        (FunctionId(900), 20),
        (FunctionId(91), 10),
        (FunctionId(7), 10),
    ];
    module.global_dtors = vec![(FunctionId(8), 65535), (FunctionId(77), 7)];
    module
}

fn merge(module: VbcModule) -> Result<VbcModule, MergeError> {
    ModuleMerger::new(module, None, vec![], MonomorphizationResolver::new())
        .merge()
        .map(|(module, _)| module)
}

#[test]
fn user_and_library_rosters_keep_order_priority_and_exact_remapped_functions() {
    let module = merge(lifecycle_module()).unwrap();
    assert_eq!(
        module.global_ctors,
        vec![
            (FunctionId(3), 20),
            (FunctionId(1), 10),
            (FunctionId(2), 10)
        ]
    );
    assert_eq!(
        module.global_dtors,
        vec![(FunctionId(5), 65535), (FunctionId(4), 7)]
    );
    for (id, name) in [
        (3, "__tls_init_library_append"),
        (1, "__tls_init_library_seed"),
        (2, "__tls_init_user_append"),
        (5, "user_finalizer"),
        (4, "library_finalizer"),
    ] {
        assert_eq!(
            module.get_string(module.get_function(FunctionId(id)).unwrap().name),
            Some(name)
        );
    }
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let loaded = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    assert_eq!(loaded.global_ctors, module.global_ctors);
    assert_eq!(loaded.global_dtors, module.global_dtors);
}

#[test]
fn post_mono_static_initialization_executes_the_declared_bodies_in_priority_order() {
    let module = merge(lifecycle_module()).unwrap();
    let mut interpreter = Interpreter::new(Arc::new(module));
    interpreter.run_global_ctors().unwrap();
    assert_eq!(
        interpreter
            .execute_function(FunctionId(0))
            .unwrap()
            .as_i64(),
        123
    );
}

#[test]
fn absent_ctor_is_an_identity_error_not_a_new_output_id() {
    let mut module = lifecycle_module();
    // There is no source FunctionId(3), but compaction creates that output ID.
    module.global_ctors.push((FunctionId(3), 0));
    assert!(matches!(
        merge(module),
        Err(MergeError::FunctionNotFound {
            function_id: FunctionId(3),
            ..
        })
    ));
}

#[test]
fn absent_dtor_is_an_identity_error() {
    let mut module = lifecycle_module();
    module.global_dtors.push((FunctionId(2), 0));
    assert!(matches!(
        merge(module),
        Err(MergeError::FunctionNotFound {
            function_id: FunctionId(2),
            ..
        })
    ));
}

#[test]
fn module_without_lifecycle_roots_does_not_invent_any() {
    let mut module = lifecycle_module();
    module.global_ctors.clear();
    module.global_dtors.clear();
    let module = merge(module).unwrap();
    assert!(module.global_ctors.is_empty());
    assert!(module.global_dtors.is_empty());
}

#[cfg(feature = "codegen")]
#[test]
fn source_static_survives_a_real_generic_instantiation() {
    use verum_vbc::{
        codegen::{CodegenConfig, VbcCodegen},
        mono::{InstantiationGraph, discover_call_instantiations, monomorphize_minimal},
    };
    let ast = verum_fast_parser::Parser::new(
        r#"
module source_lifecycle;
static mut SEED: Int = 37;
fn identity<T>(value: T) -> T { value }
fn probe() -> Int { identity<Int>(SEED) }
"#,
    )
    .parse_module()
    .unwrap();
    let source = VbcCodegen::with_config(CodegenConfig::new("source_lifecycle"))
        .compile_module(&ast)
        .unwrap();
    assert_eq!(source.global_ctors.len(), 1);
    let mut graph = InstantiationGraph::new();
    for function in &source.functions {
        if let Some(body) = &function.instructions {
            discover_call_instantiations(&source, body, function.func_id_base, &mut graph).unwrap();
        }
    }
    assert!(!graph.is_empty(), "test must enter the real mono merger");
    let module = monomorphize_minimal(source, &graph).unwrap().module;
    assert_eq!(module.global_ctors.len(), 1);
    let entry = module
        .functions
        .iter()
        .find(|function| {
            module
                .get_string(function.name)
                .is_some_and(|name| name == "probe" || name == "source_lifecycle.probe")
        })
        .unwrap()
        .id;
    let mut interpreter = Interpreter::new(Arc::new(module));
    interpreter.run_global_ctors().unwrap();
    assert_eq!(interpreter.execute_function(entry).unwrap().as_i64(), 37);
}
