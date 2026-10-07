//! T1602: declaration signatures precede ABI erasure and authorize no cleanup.
#![cfg(feature = "codegen")]
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    module::{FunctionDescriptor, FunctionId, VbcModule},
    serialize::serialize_module,
    types::{CbgrTier, Mutability, TypeId, TypeParamId, TypeRef as T},
};
fn compile(source: &str) -> VbcModule {
    VbcCodegen::new()
        .compile_module(&Parser::new(source).parse_module().unwrap())
        .unwrap()
}
fn roundtrip(m: &VbcModule) -> VbcModule {
    deserialize_module(&serialize_module(m).unwrap()).unwrap()
}
fn function<'a>(m: &'a VbcModule, name: &str) -> &'a FunctionDescriptor {
    m.functions
        .iter()
        .find(|f| m.get_string(f.name) == Some(name))
        .unwrap_or_else(|| panic!("{name}"))
}
fn nominal(m: &VbcModule, name: &str) -> TypeId {
    m.types
        .iter()
        .find(|t| m.get_string(t.name) == Some(name))
        .unwrap()
        .id
}
fn reference(inner: T, tier: CbgrTier, mutable: bool) -> T {
    T::Reference {
        inner: Box::new(inner),
        tier,
        mutability: if mutable {
            Mutability::Mutable
        } else {
            Mutability::Immutable
        },
    }
}

#[test]
fn source_managed_checked_and_unsafe_record_borrows_are_not_the_abi_carriers() {
    let m = compile(
        "type Token is { value: Int }; fn receive(a: Token, b: &Token, c: &checked mut Token, d: &unsafe Token) -> Int { 7 }",
    );
    for m in [m.clone(), roundtrip(&m)] {
        let f = function(&m, "receive");
        let token = T::Concrete(nominal(&m, "Token"));
        assert_eq!(
            f.semantic_parameter_types().unwrap(),
            &[
                Some(token.clone()),
                Some(reference(token.clone(), CbgrTier::Tier0, false)),
                Some(reference(token.clone(), CbgrTier::Tier1, true)),
                Some(reference(token.clone(), CbgrTier::Tier2, false))
            ]
        );
        assert_eq!(
            f.params[1].type_ref, token,
            "managed record ABI remains erased"
        );
        assert_eq!(
            f.params[3].type_ref,
            T::Concrete(TypeId::PTR),
            "unsafe ABI remains a word"
        );
    }
}

#[test]
fn unsupported_positions_remain_unknown_without_shifting_following_formals() {
    let m = compile(
        "fn inspect<N>(a: fn<R>(R) -> R, b: [Int; N], c: Missing, d: &Int) -> Int { 7 } fn empty() -> Int { 0 }",
    );
    let expected = [
        None,
        None,
        None,
        Some(reference(T::Concrete(TypeId::INT), CbgrTier::Tier0, false)),
    ];
    for m in [m.clone(), roundtrip(&m)] {
        assert_eq!(
            function(&m, "inspect").semantic_parameter_types(),
            Some(expected.as_slice())
        );
        assert_eq!(
            function(&m, "empty").semantic_parameter_types(),
            Some([].as_slice())
        );
    }
}

#[test]
fn exact_method_shadow_ids_and_self_owner_survive_wire() {
    let m = compile(
        "type Container<Item> is { value: Item }; implement<Item> Container<Item> { fn take<Item>(&checked mut self, value: &Item) -> Int { 1 } }",
    );
    for m in [m.clone(), roundtrip(&m)] {
        let f = function(&m, "Container.take");
        assert_eq!(
            f.semantic_parameter_types().unwrap(),
            &[
                Some(reference(
                    T::Instantiated {
                        base: nominal(&m, "Container"),
                        args: vec![T::Generic(TypeParamId(0))]
                    },
                    CbgrTier::Tier1,
                    true
                )),
                Some(reference(
                    T::Generic(TypeParamId(0x8000)),
                    CbgrTier::Tier0,
                    false
                )),
            ]
        );
    }
}

#[test]
fn inline_source_siblings_and_overloads_own_their_formal_lists_in_both_orders() {
    let a = "module alpha { public type Token is { value: Int }; public fn same(value: &Token) -> Int { 1 } public fn same(a: Int, b: Int) -> Int { 2 } }";
    let b = "module beta { public type Token is { text: Text }; public fn same(value: &Token) -> Int { 3 } }";
    for source in [format!("{a} {b}"), format!("{b} {a}")] {
        let m = roundtrip(&compile(&source));
        for owner in ["alpha", "beta"] {
            let f = m
                .functions
                .iter()
                .find(|f| {
                    m.get_string(f.name) == Some(&format!("{owner}.same")) && f.params.len() == 1
                })
                .unwrap();
            let ty = m
                .types
                .iter()
                .find(|t| {
                    t.origin_module.and_then(|id| m.get_string(id)) == Some(owner)
                        && m.get_string(t.name) == Some("Token")
                })
                .unwrap();
            assert_eq!(
                f.semantic_parameter_types().unwrap(),
                &[Some(reference(T::Concrete(ty.id), CbgrTier::Tier0, false))]
            );
        }
    }
}

#[test]
fn source_context_formals_reset_with_function_identity() {
    let mut cg = VbcCodegen::new();
    let m = cg
        .compile_module(
            &Parser::new("fn first(value: &Int) -> Int { 1 }")
                .parse_module()
                .unwrap(),
        )
        .unwrap();
    assert!(function(&m, "first").semantic_params.is_some());
    let id = cg.ctx_mut().functions["first"].id;
    assert!(cg.ctx_mut().semantic_fn_params.contains_key(&id));
    cg.ctx_mut().reset();
    assert!(cg.ctx_mut().semantic_fn_params.is_empty());
}

#[test]
fn archive_remap_preserves_semantic_owner_and_unknown_legacy_even_with_id_collisions() {
    let alpha = roundtrip(&compile(
        "module alpha; type Token is { value: Int }; fn receive(value: &Token) -> Int { 1 }",
    ));
    let beta = roundtrip(&compile(
        "module beta; type Token is { text: Text }; fn receive(value: &checked mut Token) -> Int { 2 }",
    ));
    assert_eq!(nominal(&alpha, "Token"), nominal(&beta, "Token"));
    for reverse in [false, true] {
        let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        let sources = if reverse {
            [&beta, &alpha]
        } else {
            [&alpha, &beta]
        };
        for (index, source) in sources.into_iter().enumerate() {
            cg.import_archive_module_types(source);
            let f = source
                .functions
                .iter()
                .find(|f| f.params.len() == 1 && f.semantic_params.is_some())
                .unwrap();
            let remap = [(f.id.0, FunctionId(1000 + index as u32))]
                .into_iter()
                .collect();
            cg.merge_archive_function_bodies(source, &remap);
            let slots = cg.ctx_mut().semantic_fn_params[&FunctionId(1000 + index as u32)].clone();
            let inner = match slots[0].as_ref().unwrap() {
                T::Reference { inner, .. } => inner.as_ref().clone(),
                _ => panic!(),
            };
            assert_ne!(inner, T::Concrete(TypeId::PTR));
            // Re-import is idempotent and source-owned, not a second local remap.
            cg.merge_archive_function_bodies(source, &remap);
            assert_eq!(
                cg.ctx_mut().semantic_fn_params[&FunctionId(1000 + index as u32)],
                slots
            );
        }
        let local = cg.finalize_module_from_state().unwrap();
        let a = function(&local, "alpha.receive")
            .semantic_parameter_types()
            .unwrap();
        let b = function(&local, "beta.receive")
            .semantic_parameter_types()
            .unwrap();
        assert_ne!(a, b);
        let mut legacy = alpha.clone();
        let f = legacy
            .functions
            .iter_mut()
            .find(|f| f.semantic_params.is_some())
            .unwrap();
        let id = f.id;
        f.semantic_params = None;
        let remap = [(id.0, FunctionId(1000))].into_iter().collect();
        cg.merge_archive_function_bodies(&legacy, &remap);
        assert!(
            !cg.ctx_mut()
                .semantic_fn_params
                .contains_key(&FunctionId(1000)),
            "absence never recovers from ABI or stale cache"
        );
    }
}

#[test]
fn wrong_arity_is_not_a_semantic_signature() {
    let mut m = compile("fn receive(value: &Int) -> Int { 1 }");
    let id = function(&m, "receive").id;
    m.functions[id.0 as usize].semantic_params = Some(List::new());
    assert!(
        m.functions[id.0 as usize]
            .semantic_parameter_types()
            .is_none()
    );
    assert!(serialize_module(&m).is_err());
}

#[test]
fn bootstrap_collects_nominal_dependencies_erased_from_the_abi_parameter() {
    let ast = Parser::new(
        "module alpha; type Token is { value: Int }; fn receive(p: &unsafe Token) -> Int { 1 }",
    )
    .parse_module()
    .unwrap();
    let mut producer = VbcCodegen::with_config(CodegenConfig::new("alpha"));
    let m = roundtrip(&producer.compile_module(&ast).unwrap());
    let registry = producer.export_functions();
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    cg.import_functions(&registry);
    let consumer = Parser::new("mount alpha.receive; fn local() -> Int { 0 }")
        .parse_module()
        .unwrap();
    assert_eq!(
        cg.import_bootstrap_nominal_dependencies(&[&consumer], &[&m])
            .unwrap(),
        1
    );
    let id = cg.ctx_mut().functions["alpha.receive"].id;
    let actual = cg.ctx_mut().semantic_fn_params[&id].clone();
    cg.collect_unit_declarations(&[&consumer]).unwrap();
    let result = cg.compile_function_bodies(&consumer).unwrap();
    assert_eq!(
        actual.as_slice(),
        &[Some(reference(
            T::Concrete(nominal(&result, "Token")),
            CbgrTier::Tier2,
            false
        ))]
    );
}

#[test]
fn missing_archive_type_map_never_retains_a_foreign_numeric_identity() {
    let source = roundtrip(&compile(
        "type Token is { value: Int }; fn receive(p: &Token) -> Int { 1 }",
    ));
    let f = function(&source, "receive");
    let mut cg = VbcCodegen::new();
    // Occupy the same numeric type ID with an unrelated source declaration.
    cg.collect_unit_declarations(&[&Parser::new("type Stranger is { text: Text };")
        .parse_module()
        .unwrap()])
        .unwrap();
    let remap = [(f.id.0, FunctionId(7000))].into_iter().collect();
    cg.merge_archive_function_bodies(&source, &remap);
    assert_eq!(
        cg.ctx_mut().semantic_fn_params[&FunctionId(7000)].as_slice(),
        &[None]
    );
}

#[test]
fn actual_monomorphization_substitutes_exact_method_shadow_ids() {
    use verum_vbc::mono::{InstantiationGraph, SourceLocation, monomorphize_minimal};
    let mut m = compile(
        "type Container<Item> is { value: Item }; implement<Item> Container<Item> { fn take<Item>(&self, value: &Item) -> Int { 1 } }",
    );
    for f in &mut m.functions {
        f.is_generic = !f.type_params.is_empty();
    }
    let id = function(&m, "Container.take").id;
    let mut graph = InstantiationGraph::new();
    graph.record_instantiation(
        id,
        vec![T::Concrete(TypeId::INT), T::Concrete(TypeId::BOOL)],
        SourceLocation::default(),
    );
    let result = monomorphize_minimal(m, &graph).unwrap();
    assert_eq!(result.metrics.new_specializations, 1);
    let m = roundtrip(&result.module);
    let specialized = m
        .functions
        .iter()
        .find(|f| {
            f.type_params.is_empty() && f.semantic_params.as_ref().is_some_and(|p| p.len() == 2)
        })
        .unwrap();
    assert_eq!(
        specialized.semantic_parameter_types().unwrap()[1],
        Some(reference(T::Concrete(TypeId::BOOL), CbgrTier::Tier0, false))
    );
    let Some(T::Reference { inner, .. }) = &specialized.semantic_parameter_types().unwrap()[0]
    else {
        panic!()
    };
    assert_eq!(
        inner.as_ref(),
        &T::Instantiated {
            base: nominal(&m, "Container"),
            args: vec![T::Concrete(TypeId::INT)]
        }
    );
}

#[test]
fn linker_remaps_semantic_types_with_their_function_and_type_owners() {
    use verum_vbc::linker::VbcLinker;
    let mut linker = VbcLinker::new("aarch64-apple-darwin");
    for owner in ["alpha", "beta"] {
        let field = if owner == "alpha" { "Int" } else { "Text" };
        let source = format!(
            "module {owner}; type Token is {{ value: {field} }}; fn receive(p: &Token) -> Int {{ 1 }}"
        );
        let m = VbcCodegen::with_config(CodegenConfig::new(owner))
            .compile_module(&Parser::new(&source).parse_module().unwrap())
            .unwrap();
        linker.add_user_module(roundtrip(&m)).unwrap();
    }
    let m = roundtrip(&linker.finalize());
    let alpha = function(&m, "alpha.receive")
        .semantic_parameter_types()
        .unwrap();
    let beta = function(&m, "beta.receive")
        .semantic_parameter_types()
        .unwrap();
    assert_ne!(alpha, beta, "same source IDs must separate after linking");
    for (owner, params) in [("alpha", alpha), ("beta", beta)] {
        let Some(T::Reference { inner, .. }) = &params[0] else {
            panic!()
        };
        let T::Concrete(id) = inner.as_ref() else {
            panic!()
        };
        let ty = m.types.iter().find(|ty| ty.id == *id).unwrap();
        assert_eq!(
            ty.fields[0].type_ref,
            T::Concrete(if owner == "alpha" {
                TypeId::INT
            } else {
                TypeId::TEXT
            })
        );
    }
}

#[test]
fn higher_kinded_application_is_not_recovered_as_its_bare_generic_head() {
    let m = compile("fn receive<F<_>>(value: F<Int>, callback: fn(&Int) -> Int) -> Int { 1 }");
    assert_eq!(
        function(&m, "receive").semantic_parameter_types().unwrap(),
        &[
            None,
            Some(T::Function {
                params: vec![reference(T::Concrete(TypeId::INT), CbgrTier::Tier0, false)],
                return_type: Box::new(T::Concrete(TypeId::INT)),
                contexts: Default::default()
            }),
        ]
    );
}

#[test]
fn formal_signature_change_invalidates_existing_value_use_observations() {
    let mut m = compile("fn relay(value: &Int) -> &Int { let result = value; result }");
    let id = function(&m, "relay").id;
    assert!(m.value_use_receipts(id).is_some());
    m.functions[id.0 as usize].semantic_params.as_mut().unwrap()[0] =
        Some(T::Concrete(TypeId::INT));
    assert!(
        m.value_use_receipts(id).is_none(),
        "unchanged ABI/body cannot reseal a different declaration"
    );
}

#[test]
fn an_unmounted_sibling_type_is_not_a_declaration_formal() {
    let sibling = "module alpha { public type Token is { value: Int }; }";
    let consumer = "module beta { public fn receive(value: &Token) -> Int { 1 } }";
    for source in [
        format!("{sibling} {consumer}"),
        format!("{consumer} {sibling}"),
    ] {
        let m = compile(&source);
        assert_eq!(
            function(&m, "beta.receive").semantic_parameter_types(),
            Some([None].as_slice())
        );
    }
}

#[test]
fn concrete_impl_receiver_uses_its_source_arguments_in_both_producers() {
    let source = "type Container<T> is { value: T }; implement Container<Int> { fn read(&self, other: &Self) -> Int { 1 } } fn later(value: &Int) -> Int { 2 }";
    for policy in [
        None,
        Some(ItemFailurePolicy::Strict),
        Some(ItemFailurePolicy::StubAndContinue),
    ] {
        let ast = Parser::new(source).parse_module().unwrap();
        let mut cg = VbcCodegen::new();
        let m = if let Some(policy) = policy {
            cg.collect_unit_declarations(&[&ast]).unwrap();
            cg.compile_unit_items(&[&ast], policy).unwrap();
            cg.finalize_module_from_state().unwrap()
        } else {
            cg.compile_module(&ast).unwrap()
        };
        for m in [m.clone(), roundtrip(&m)] {
            let expected = reference(
                T::Instantiated {
                    base: nominal(&m, "Container"),
                    args: vec![T::Concrete(TypeId::INT)],
                },
                CbgrTier::Tier0,
                false,
            );
            assert_eq!(
                function(&m, "Container.read")
                    .semantic_parameter_types()
                    .unwrap(),
                &[Some(expected), None],
                "regular Self remains unknown until its resolver owns the exact target"
            );
            assert_eq!(
                function(&m, "later").semantic_parameter_types().unwrap(),
                &[Some(reference(
                    T::Concrete(TypeId::INT),
                    CbgrTier::Tier0,
                    false
                ))]
            );
        }
    }
}

#[test]
fn an_explicit_type_mount_and_qualified_path_preserve_the_named_owner() {
    let sibling = "module alpha { public type Token is { value: Int }; }";
    let consumer = "module beta { mount alpha.Token; public fn receive(a: &Token, b: &alpha.Token) -> Int { 1 } }";
    for source in [
        format!("{sibling} {consumer}"),
        format!("{consumer} {sibling}"),
    ] {
        let m = roundtrip(&compile(&source));
        let expected = Some(reference(
            T::Concrete(nominal(&m, "Token")),
            CbgrTier::Tier0,
            false,
        ));
        assert_eq!(
            function(&m, "beta.receive").semantic_parameter_types(),
            Some([expected.clone(), expected].as_slice())
        );
    }
}
