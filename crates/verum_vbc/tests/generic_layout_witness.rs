//! Generic layout queries preserve their declared parameter through execution.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::VbcCodegen;
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::VbcModule;

fn compile(source: &str) -> VbcModule {
    let module = Parser::new(source).parse_module().expect("source grammar");
    VbcCodegen::new()
        .compile_module(&module)
        .expect("source VBC")
}

fn run(module: VbcModule, name: &str) -> i64 {
    let id = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name) == Some(name))
        .unwrap_or_else(|| {
            panic!(
                "missing {name}: {:?}",
                module
                    .functions
                    .iter()
                    .map(|f| module.get_string(f.name))
                    .collect::<Vec<_>>()
            )
        })
        .id;
    Interpreter::new(Arc::new(module))
        .execute_function(id)
        .expect("execute layout query")
        .as_i64()
}

#[test]
fn generic_byte_size_matches_direct_declaration() {
    let source = "fn size<T>() -> Int { T.size } fn probe() -> Int { size<Byte>() }";
    assert_eq!(run(compile(source), "probe"), 1);
}

#[test]
fn generic_int_size_preserves_value() {
    let source = "fn size<T>() -> Int { T.size } fn probe() -> Int { size<Int>() }";
    assert_eq!(run(compile(source), "probe"), 8);
}

#[test]
fn nested_call_passes_caller_layout_witness() {
    let source = "fn size<T>() -> Int { T.size } fn outer<U>() -> Int { size<U>() } fn probe() -> Int { outer<Byte>() }";
    assert_eq!(run(compile(source), "probe"), 1);
}

#[test]
fn alignment_and_stride_preserve_narrow_instantiation() {
    let source = "fn stride<T>() -> Int { T.stride } fn alignment<T>() -> Int { T.alignment } fn probe() -> Int { stride<Byte>() + alignment<Byte>() }";
    assert_eq!(run(compile(source), "probe"), 2);
}

#[test]
fn ordinary_record_queries_agree_with_direct_properties() {
    let source = "type Triple is { x: Int, y: Int, z: Int }; fn size<T>() -> Int { T.size } fn direct() -> Int { Triple.size } fn probe() -> Int { size<Triple>() }";
    let module = compile(source);
    assert_eq!(run(module.clone(), "probe"), run(module, "direct"));
}

#[test]
fn repr_c_queries_agree_with_direct_properties() {
    let source = "@repr(C) type Pair is { x: Byte, y: Int32 }; fn size<T>() -> Int { T.size } fn direct() -> Int { Pair.size } fn probe() -> Int { size<Pair>() }";
    let module = compile(source);
    assert_eq!(run(module.clone(), "probe"), run(module, "direct"));
}

#[test]
fn generic_parameter_shadows_same_spelled_nominal() {
    let source = "type T is { x: Int, y: Int, z: Int }; fn size<T>() -> Int { T.size } fn probe() -> Int { size<Byte>() }";
    assert_eq!(run(compile(source), "probe"), 1);
}

#[test]
fn method_parameter_shadows_owner_parameter() {
    let source = "type Owner<T> is { value: T }; implement<T> Owner<T> { fn measure<T>(self) -> Int { T.size } } fn probe() -> Int { let owner = Owner { value: 7 }; owner.measure<Byte>() }";
    assert_eq!(run(compile(source), "probe"), 1);
}

fn roundtrip(module: &VbcModule) -> VbcModule {
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(module).unwrap(),
    )
    .unwrap()
}

#[test]
fn nominal_byte_does_not_acquire_primitive_layout_by_spelling() {
    let module = compile(
        "type Byte is { first: Int, second: Int }; fn size<T>() -> Int { T.size } fn direct() -> Int { Byte.size } fn probe() -> Int { size<Byte>() }",
    );
    assert_eq!(run(module.clone(), "direct"), 16);
    assert_eq!(run(roundtrip(&module), "probe"), 16);
}

#[test]
fn record_and_c_layout_facts_survive_wire_without_changing_slot_extent() {
    let module = roundtrip(&compile(
        "type Triple is { x: Int, y: Int, z: Int }; @repr(C) type Pair is { x: Byte, y: Int32 };",
    ));
    for (name, declared, slots) in [("Triple", 24, 24), ("Pair", 8, 16)] {
        let descriptor = module
            .types
            .iter()
            .find(|ty| module.get_string(ty.name) == Some(name))
            .unwrap();
        assert_eq!(descriptor.declared_layout.unwrap().size, declared);
        assert_eq!(descriptor.size, slots);
    }
}

#[test]
fn missing_nominal_layout_is_unknown_even_when_object_extent_is_known() {
    let mut module = compile(
        "type Cell is { value: Int }; fn size<T>() -> Int { T.size } fn probe() -> Int { size<Cell>() }",
    );
    let descriptor = module
        .types
        .iter_mut()
        .find(|ty| ty.declared_layout.is_some())
        .unwrap();
    let id = descriptor.id;
    descriptor.declared_layout = None;
    assert_eq!(
        verum_vbc::type_layout::query(
            &module,
            &verum_vbc::types::TypeRef::Concrete(id),
            verum_vbc::instruction::LayoutProperty::Size
        ),
        None
    );
    let id = module.find_function_by_name("probe").unwrap();
    assert!(
        Interpreter::new(Arc::new(module))
            .execute_function(id)
            .is_err()
    );
}

#[test]
fn concrete_array_and_reference_witnesses_use_shared_layout_constants() {
    for (ty, expected) in [
        ("[Byte; 5]", 5),
        ("&Byte", 16),
        ("&checked Byte", 16),
        ("&unsafe Byte", 8),
        ("&[Byte]", 32),
    ] {
        let source = format!(
            "type Shape is {ty}; fn size<T>() -> Int {{ T.size }} fn probe() -> Int {{ size<Shape>() }}"
        );
        assert_eq!(run(roundtrip(&compile(&source)), "probe"), expected, "{ty}");
    }
}

#[test]
fn imported_calls_keep_each_declaring_nominal_layout_in_both_orders() {
    use verum_common::Map;
    use verum_vbc::module::FunctionId;
    let alpha = roundtrip(&compile(
        "module alpha; type Payload is { a: Int, b: Int, c: Int }; fn alpha_size<T>() -> Int { T.size } fn alpha_probe() -> Int { alpha_size<Payload>() }",
    ));
    let beta = roundtrip(&compile(
        "module beta; type Payload is { a: Int }; fn beta_size<T>() -> Int { T.size } fn beta_probe() -> Int { beta_size<Payload>() }",
    ));
    for modules in [[&alpha, &beta], [&beta, &alpha]] {
        let mut codegen = VbcCodegen::new();
        for module in modules {
            codegen.import_archive_module_types(module);
        }
        for (index, module) in modules.into_iter().enumerate() {
            let mapping: Map<_, _> = module
                .functions
                .iter()
                .enumerate()
                .map(|(i, function)| {
                    (
                        function.id.0,
                        FunctionId(7000 + 100 * index as u32 + i as u32),
                    )
                })
                .collect();
            assert!(codegen.merge_archive_function_bodies(module, &mapping.into()) > 0);
        }
        let module = roundtrip(&codegen.finalize_module_from_state().unwrap());
        assert_eq!(run(module.clone(), "alpha.alpha_probe"), 24);
        assert_eq!(run(module, "beta.beta_probe"), 8);
    }
}

#[test]
fn typed_query_roundtrip_keeps_nested_identity_and_following_instructions() {
    use verum_vbc::{
        bytecode::{decode_instructions, encode_instruction},
        instruction::{Instruction, LayoutProperty, Reg},
        types::{TypeId, TypeParamId, TypeRef},
    };
    let query = Instruction::TypeLayout {
        dst: Reg(301),
        property: LayoutProperty::Stride,
        type_ref: TypeRef::Array {
            element: Box::new(TypeRef::Instantiated {
                base: TypeId(7001),
                args: vec![TypeRef::Generic(TypeParamId(0x8000))],
            }),
            length: 17,
        },
    };
    let mut bytes = Vec::new();
    encode_instruction(&query, &mut bytes);
    encode_instruction(&Instruction::Ret { value: Reg(301) }, &mut bytes);
    assert_eq!(
        decode_instructions(&bytes).unwrap(),
        [query, Instruction::Ret { value: Reg(301) }]
    );
    bytes[4] = 7; // wide destination follows two-byte extended header.
    assert!(decode_instructions(&bytes).is_err());
}

#[test]
fn queries_do_not_hide_later_calls_from_mono_or_reachability() {
    use verum_vbc::mono::{InstantiationGraph, discover_call_instantiations, monomorphize_minimal};
    let mut module = roundtrip(&compile(
        "fn inner<T>() -> Int { T.size } fn outer<T>() -> Int { let n = T.size; n + inner<Byte>() } fn probe() -> Int { outer<Int>() } fn unreachable() -> Int { 99 }",
    ));
    let probe = module.find_function_by_name("probe").unwrap();
    // Reachability consumes the decoded cache supplied by the compilation pipeline.
    for function in &mut module.functions {
        function.instructions = Some(
            verum_vbc::bytecode::decode_instructions(
                &module.bytecode[function.bytecode_offset as usize
                    ..(function.bytecode_offset + function.bytecode_length) as usize],
            )
            .unwrap(),
        );
    }
    let roots = verum_vbc::reachability::analyze_with_roots(&module, &[probe.0]);
    for name in ["inner", "outer", "probe"] {
        assert!(
            roots
                .reachable_ids
                .contains(&module.find_function_by_name(name).unwrap().0),
            "{name}"
        );
    }
    assert!(
        !roots
            .reachable_ids
            .contains(&module.find_function_by_name("unreachable").unwrap().0)
    );
    let mut graph = InstantiationGraph::new();
    for function in &mut module.functions {
        function.is_generic = !function.type_params.is_empty();
    }
    for function in &module.functions {
        let instructions = verum_vbc::bytecode::decode_instructions(
            &module.bytecode[function.bytecode_offset as usize
                ..(function.bytecode_offset + function.bytecode_length) as usize],
        )
        .unwrap();
        discover_call_instantiations(&module, &instructions, function.func_id_base, &mut graph)
            .unwrap();
    }
    let mono = monomorphize_minimal(module, &graph).unwrap().module;
    assert_eq!(run(mono, "probe"), 9);
}

#[test]
fn parenthesized_reference_queries_share_the_declaration_authority() {
    for (source, expected) in [
        ("fn probe() -> Int { (&checked Byte).size }", 16),
        ("fn probe() -> Int { (&[Byte]).size }", 32),
        (
            "fn size<T>() -> Int { (&T).size } fn probe() -> Int { size<Byte>() }",
            16,
        ),
        (
            "fn size<T>() -> Int { (&T).size } type Shape is [Byte]; fn probe() -> Int { size<Shape>() }",
            32,
        ),
    ] {
        assert_eq!(run(compile(source), "probe"), expected, "{source}");
    }
}

#[test]
fn ordinary_value_field_named_size_is_not_a_layout_query() {
    assert_eq!(
        run(
            compile(
                "type Cell is { size: Int }; fn probe() -> Int { let cell = Cell { size: 37 }; cell.size }"
            ),
            "probe"
        ),
        37
    );
}

#[test]
fn reference_alias_arguments_retain_their_layout_in_nested_frames() {
    for (ty, expected) in [
        ("&Byte", 16),
        ("&checked Byte", 16),
        ("&unsafe Byte", 8),
        ("&[Byte]", 32),
    ] {
        let source = format!(
            "type Shape is {ty}; fn size<T>() -> Int {{ T.size }} fn outer<U>() -> Int {{ size<U>() }} fn probe() -> Int {{ outer<Shape>() }}"
        );
        assert_eq!(run(roundtrip(&compile(&source)), "probe"), expected, "{ty}");
    }
}

#[test]
fn layout_queries_refuse_extent_overflow_and_invalid_alignment() {
    use verum_vbc::{
        instruction::LayoutProperty,
        types::{TypeId, TypeRef},
    };
    let mut module = compile("type Cell is { value: Int };");
    let array = TypeRef::Array {
        element: Box::new(TypeRef::Concrete(TypeId::BYTE)),
        length: u64::MAX,
    };
    assert_eq!(
        verum_vbc::type_layout::query(&module, &array, LayoutProperty::Size),
        None
    );
    let descriptor = module
        .types
        .iter_mut()
        .find(|ty| ty.declared_layout.is_some())
        .unwrap();
    descriptor.declared_layout.as_mut().unwrap().alignment = 3;
    let ty = TypeRef::Concrete(descriptor.id);
    assert_eq!(
        verum_vbc::type_layout::query(&module, &ty, LayoutProperty::Stride),
        None
    );
}

#[test]
fn lexical_value_bindings_shadow_type_properties_independent_of_case() {
    for source in [
        "type Cell is { size: Int }; fn probe() -> Int { let Cell = Cell { size: 37 }; Cell.size }",
        "type Cell is { size: Int }; fn get(Cell: Cell) -> Int { Cell.size } fn probe() -> Int { get(Cell { size: 37 }) }",
        "type Cell is { size: Int }; fn get(Cell: &Cell) -> Int { Cell.size } fn probe() -> Int { let item = Cell { size: 37 }; get(&item) }",
        "type Cell is { size: Int }; fn probe() -> Int { let Cell = Cell { size: 37 }; (Cell).size }",
        "type Cell is { size: Int }; fn probe() -> Int { let Cell = Cell { size: 37 }; (&Cell).size }",
        "type Cell is { size: Int }; fn probe() -> Int { let Cell = Cell { size: 37 }; let result = { let Cell = Cell { size: 11 }; Cell.size }; Cell.size + result - 11 }",
    ] {
        let module = compile(source);
        assert_eq!(run(roundtrip(&module), "probe"), 37, "{source}");
    }
}

#[test]
fn declaration_properties_remain_available_to_constant_layouts() {
    for (source, expected) in [
        ("type Cell is { first: Int, second: Int }; type Shape is [Byte; Cell.size]; fn probe() -> Int { Shape.size }", 16),
        ("type Cell is { first: Int, second: Int }; const CAP: Int = Cell.size; type Shape is [Byte; CAP]; fn probe() -> Int { Shape.size }", 16),
        ("type Shape is [Byte; Int.size]; fn probe() -> Int { Shape.size }", 8),
    ] {
        assert_eq!(run(roundtrip(&compile(source)), "probe"), expected, "{source}");
    }
}

#[test]
fn primitive_spelling_value_binding_keeps_its_fields() {
    for property in ["size", "name", "bits", "id", "is_signed"] {
        let source = format!(
            "type Cell is {{ {property}: Int }}; fn probe()->Int {{ let Int=Cell{{{property}:37}}; Int.{property} }}"
        );
        assert_eq!(run(roundtrip(&compile(&source)), "probe"), 37, "{property}");
    }
}

#[test]
fn declaration_properties_share_the_existing_nonlayout_route() {
    for (source, expected) in [
        ("fn probe()->Int { (Int).bits }", 64),
        ("fn probe()->Int { if Int.id==Int64.id {1} else {0} }", 1),
        ("fn probe()->Int { if (Int).name==\"Int\" {1} else {0} }", 1),
        ("fn probe()->Int { if (Int).is_signed {1} else {0} }", 1),
    ] {
        assert_eq!(
            run(roundtrip(&compile(source)), "probe"),
            expected,
            "{source}"
        );
    }
}

#[test]
fn qualified_property_receivers_preserve_declarations_and_value_roots() {
    for source in [
        "module alpha { type Cell is {a:Int,b:Int}; } module beta { type Cell is {a:Int}; } fn probe()->Int {alpha.Cell.size+beta.Cell.size}",
        "module beta { type Cell is {a:Int}; } module alpha { type Cell is {a:Int,b:Int}; } fn probe()->Int {alpha.Cell.size+beta.Cell.size}",
    ] {
        assert_eq!(run(roundtrip(&compile(source)), "probe"), 24, "{source}");
    }
    assert_eq!(
        run(
            compile(
                "type Cell is {size:Int}; type Holder is {Cell:Cell}; fn probe()->Int {let alpha=Holder{Cell:Cell{size:37}}; alpha.Cell.size}"
            ),
            "probe"
        ),
        37
    );
}

#[test]
fn array_property_count_cycles_keep_the_constant_dependency_guard() {
    for source in ["const N: Int = ([Byte; N]).size; fn probe() { let bytes: [Byte; N] = [0; N]; }"]
    {
        let ast = Parser::new(source).parse_module().expect("source grammar");
        let error = VbcCodegen::new()
            .compile_module(&ast)
            .expect_err("cyclic count must be rejected");
        assert!(error.to_string().contains("cyclic constant"), "{error}");
    }
}

#[test]
fn reference_name_uses_the_existing_explicit_property_semantics() {
    use verum_ast::decl::FunctionBody;
    use verum_ast::expr::ExprKind;
    use verum_ast::ty::{Type, TypeKind};
    use verum_ast::{ItemKind, TypeProperty};
    use verum_common::{Heap, Maybe};
    let mut ast = Parser::new("fn probe()->Text {(&Int).name}")
        .parse_module()
        .expect("source grammar");
    let actual = VbcCodegen::new()
        .compile_module(&ast)
        .expect("field property");
    let ItemKind::Function(function) = &mut ast.items[0].kind else {
        panic!("function")
    };
    let Maybe::Some(FunctionBody::Block(block)) = &mut function.body else {
        panic!("block")
    };
    let Maybe::Some(expression) = &mut block.expr else {
        panic!("tail")
    };
    let ExprKind::Field { expr: base, .. } = &expression.kind else {
        panic!("field")
    };
    let ExprKind::Paren(reference) = &base.kind else {
        panic!("paren")
    };
    let ExprKind::Unary { expr: inner, .. } = &reference.kind else {
        panic!("reference")
    };
    let ExprKind::Path(path) = &inner.kind else {
        panic!("type path")
    };
    let ty = Type::new(
        TypeKind::Reference {
            inner: Heap::new(Type::new(TypeKind::Path(path.clone()), inner.span)),
            mutable: false,
        },
        reference.span,
    );
    expression.kind = ExprKind::TypeProperty {
        ty,
        property: TypeProperty::Name,
    };
    let explicit = VbcCodegen::new()
        .compile_module(&ast)
        .expect("explicit property");
    assert_eq!(actual.bytecode, explicit.bytecode);
    assert!(actual.strings.iter().eq(explicit.strings.iter()));
}

#[test]
fn imported_layout_constant_keeps_its_declaring_type_and_ignores_caller_values() {
    use verum_vbc::codegen::{CodegenConfig, ItemFailurePolicy};
    for reverse in [false, true] {
        for local_shadow in [false, true] {
            for caller_first in [false, true] {
                let alpha = Parser::new("module alpha; public type Cell is {a:Int,b:Int}; public const SIZE:Int=Cell.size;")
                .parse_module().expect("alpha");
                let beta = Parser::new("module beta; public type Cell is {a:Int};")
                    .parse_module()
                    .expect("beta");
                let source = format!(
                    "module caller; fn probe()->Int {{ {} let bytes:[Byte;alpha.SIZE]=[0;alpha.SIZE]; bytes.len() }}",
                    if local_shadow { "let Cell=7;" } else { "" }
                );
                let caller = Parser::new(&source).parse_module().expect("caller");
                let modules = if reverse {
                    vec![&beta, &alpha, &caller]
                } else {
                    vec![&alpha, &beta, &caller]
                };
                let mut codegen = VbcCodegen::with_config(CodegenConfig::new("main"));
                codegen
                    .collect_unit_declarations(&modules)
                    .expect("collect declarations");
                let mut body_order = modules.clone();
                if caller_first {
                    body_order.rotate_right(1);
                }
                codegen
                    .compile_unit_items(&body_order, ItemFailurePolicy::Strict)
                    .expect("compile source");
                let module = roundtrip(&codegen.finalize_module().expect("finalize"));
                let size = module
                    .functions
                    .iter()
                    .find(|function| module.get_string(function.name) == Some("alpha.SIZE"))
                    .expect("exact alpha constant");
                assert_eq!(
                    size.intrinsic_name.and_then(|id| module.get_string(id)),
                    Some("__const_val_16"),
                    "reverse={reverse}, local_shadow={local_shadow}, caller_first={caller_first}"
                );
                let entry = module
                    .find_function_by_name("caller.probe")
                    .expect("caller probe");
                let value = Interpreter::new(Arc::new(module))
                    .execute_function(entry)
                    .expect("execute array count");
                assert_eq!(
                    value.as_i64(),
                    16,
                    "reverse={reverse}, local_shadow={local_shadow}, caller_first={caller_first}"
                );
            }
        }
    }
}

#[test]
fn generic_associated_property_keeps_its_projection_through_wire() {
    use verum_vbc::instruction::{Instruction, LayoutProperty};
    use verum_vbc::types::{CbgrTier, Mutability, TypeParamId, TypeRef};
    for (operand, wrapped) in [("T.Item", false), ("(&unsafe T.Item)", true)] {
        let source = format!(
            "type Source is protocol {{type Item;}}; fn measure<T:Source>()->Int {{{operand}.size}}"
        );
        let module = roundtrip(&compile(&source));
        let function = module
            .functions
            .iter()
            .find(|f| module.get_string(f.name) == Some("measure"))
            .expect("measure");
        let projection = TypeRef::AssociatedProjection {
            base: Box::new(TypeRef::Generic(TypeParamId(0))),
            assoc: "Item".into(),
        };
        let expected = if wrapped {
            TypeRef::Reference {
                inner: Box::new(projection),
                tier: CbgrTier::Tier2,
                mutability: Mutability::Immutable,
            }
        } else {
            projection
        };
        let start = function.bytecode_offset as usize;
        let instructions = verum_vbc::bytecode::decode_instructions(
            &module.bytecode[start..start + function.bytecode_length as usize],
        )
        .expect("decode");
        assert!(instructions.iter().any(|instruction|matches!(instruction,Instruction::TypeLayout{type_ref,property:LayoutProperty::Size,..} if type_ref==&expected)),"{operand}: {instructions:?}");
    }
}

#[test]
fn concrete_self_associated_property_uses_its_declared_binding() {
    for operand in ["Self.Item", "(&unsafe Self.Item)"] {
        let source = format!(
            "type Source is protocol {{type Item;}}; type Cell is {{value:Int}}; implement Source for Cell {{type Item=Int; fn measure(&self)->Int {{{operand}.size}} }} fn probe()->Int {{Cell{{value:7}}.measure()}}"
        );
        assert_eq!(run(roundtrip(&compile(&source)), "probe"), 8, "{operand}");
    }
}

#[test]
fn value_root_does_not_become_a_generic_projection() {
    let source = "type Payload is {size:Int}; type Cell is {Item:Payload}; fn measure<T>()->Int {let T=Cell{Item:Payload{size:37}}; T.Item.size} fn probe()->Int {measure<Byte>()}";
    assert_eq!(run(roundtrip(&compile(source)), "probe"), 37);
}

#[test]
fn module_alias_layout_property_preserves_exact_declared_owner() {
    for source in [
        "module alpha {public type Cell is {size:Bool};} mount alpha as short; fn probe()->Int {short.Cell.size}",
        "mount alpha as short; module alpha {public type Cell is {size:Bool};} fn probe()->Int {short.Cell.size}",
    ] {
        assert_eq!(run(roundtrip(&compile(source)), "probe"), 8, "{source}");
    }
}

#[test]
fn module_alias_layout_property_preserves_value_root() {
    let source = "module alpha {public type Cell is {size:Bool};} mount alpha as short; type Value is {size:Int}; type Holder is {Cell:Value}; fn probe()->Int {let short=Holder{Cell:Value{size:37}}; short.Cell.size}";
    assert_eq!(run(roundtrip(&compile(source)), "probe"), 37);
}

#[test]
fn module_alias_publication_does_not_export_nested_or_file_owned_leaves() {
    for source in [
        "module alpha {module child {public type Cell is {a:Int};}} module beta {module child {public type Cell is {a:Int,b:Int};}} mount child as short;",
        "module beta {module child {public type Cell is {a:Int,b:Int};}} module alpha {module child {public type Cell is {a:Int};}} mount child as short;",
        "module alpha; module child {public type Cell is {a:Int};} mount child as short;",
    ] {
        let ast = Parser::new(source).parse_module().expect("source grammar");
        let mut codegen = VbcCodegen::new();
        codegen.compile_module(&ast).expect("compile declarations");
        assert!(
            !codegen.ctx_mut().module_aliases.contains_key("short"),
            "unqualified child exported from {source}"
        );
    }
}
