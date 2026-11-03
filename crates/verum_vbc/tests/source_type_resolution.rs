//! Unresolved source names cannot become plausible ABI or ownership types.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenError, VbcCodegen},
};

fn compile(body: &str) -> Result<VbcModule, CodegenError> {
    let source = format!("module declaration_probe; {body}");
    let ast = Parser::new(&source).parse_module().expect("valid grammar");
    VbcCodegen::with_config(CodegenConfig::new("declaration_probe").with_validation())
        .compile_module(&ast)
}

fn refuses(body: &str) {
    let error = compile(body).expect_err("unknown source nominal cannot be emitted");
    let message = error.to_string();
    assert!(message.contains("Missing"), "{message}");
    assert!(message.contains("declaration_probe"), "{message}");
    assert!(
        error.span.is_some(),
        "the diagnostic retains the source location"
    );
}

#[test]
fn record_field_requires_a_declared_nominal() {
    refuses("type Holder is { inner: Missing }; fn main() -> Int { 0 }");
}

#[test]
fn nested_arguments_require_declared_nominals() {
    refuses("type Box<T> is { value: T }; type Holder is { inner: Box<Missing> };");
}

#[test]
fn variant_payload_requires_a_declared_nominal() {
    refuses("type Outcome is Failed(Missing) | Passed(Int);");
}

#[test]
fn alias_target_requires_a_declared_nominal() {
    refuses("type Box<T> is { value: T }; type Alias is Box<Missing>;");
}

#[test]
fn function_parameter_requires_a_declared_nominal() {
    refuses("fn inspect(value: Missing) -> Int { 0 }");
}

#[test]
fn a_bodyless_function_cannot_publish_an_unknown_parameter() {
    refuses("fn inspect(value: Missing) -> Int;");
}

#[test]
fn function_result_requires_a_declared_nominal() {
    refuses("fn missing() -> Missing { 0 }");
}

#[test]
fn interface_metadata_cannot_hide_an_unknown_parameter() {
    let ast = Parser::new("module declaration_probe; fn inspect(value: Missing) -> Int { 0 }")
        .parse_module()
        .unwrap();
    let error = VbcCodegen::with_config(CodegenConfig::new("declaration_probe"))
        .collect_declaration_interface(&[&ast])
        .expect_err("invalid interface cannot be published");
    assert!(error.to_string().contains("Missing"), "{error}");
}

#[test]
fn nested_structural_types_cannot_hide_an_unknown_nominal() {
    refuses("type Holder is { callback: fn((&[Missing], Int)) -> Int };");
}

#[test]
fn generic_function_scope_survives_parameter_lowering() {
    compile("fn preserve<T>(value: T) -> T { value }").expect("declared generic binder");
}

#[test]
fn locally_quantified_function_binders_are_not_missing_types() {
    use verum_vbc::types::{TypeParamId, TypeRef};
    let module =
        compile("type Poly<A> is { apply: fn<R>(R, A) -> (R, A) };").expect("rank-2 local binder");
    let poly = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Poly"))
        .unwrap();
    let expected = TypeRef::Rank2Function {
        type_param_count: 1,
        params: vec![
            TypeRef::Generic(TypeParamId(0)),
            TypeRef::Generic(TypeParamId(1)),
        ],
        return_type: Box::new(TypeRef::Tuple(vec![
            TypeRef::Generic(TypeParamId(0)),
            TypeRef::Generic(TypeParamId(1)),
        ])),
        contexts: Default::default(),
    };
    assert_eq!(poly.fields[0].declaration_type, Some(expected));
}

#[test]
fn nested_quantifiers_preserve_local_outer_and_shadowed_identity() {
    use verum_vbc::types::{TypeParamId, TypeRef};
    let module = compile(
        "type Poly<A> is { apply: fn<R>(fn<S>(S, R, A) -> A) -> R, shadow: fn<A>(A) -> A };",
    )
    .expect("nested and shadowed binders");
    let poly = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Poly"))
        .unwrap();
    let g = |index| TypeRef::Generic(TypeParamId(index));
    let expected = TypeRef::Rank2Function {
        type_param_count: 1,
        params: vec![TypeRef::Rank2Function {
            type_param_count: 1,
            params: vec![g(0), g(1), g(2)],
            return_type: Box::new(g(2)),
            contexts: Default::default(),
        }],
        return_type: Box::new(g(0)),
        contexts: Default::default(),
    };
    assert_eq!(poly.fields[0].declaration_type, Some(expected));
    assert_eq!(
        poly.fields[1].declaration_type,
        Some(TypeRef::Rank2Function {
            type_param_count: 1,
            params: vec![g(0)],
            return_type: Box::new(g(0)),
            contexts: Default::default(),
        })
    );
    let loaded = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    assert_eq!(loaded.get_type(poly.id).unwrap().fields, poly.fields);
}

#[test]
fn quantified_parameters_keep_semantic_metadata() {
    use verum_vbc::types::{TypeParamId, TypeRef};
    let module = compile("fn inspect<A>(callback: fn<R>(R, A) -> A) -> Int { 0 }")
        .expect("quantified source parameter");
    let function = module
        .functions
        .iter()
        .find(|function| module.get_string(function.name) == Some("declaration_probe.inspect"))
        .unwrap();
    assert_eq!(
        function.semantic_parameter_types().unwrap(),
        &[Some(TypeRef::Rank2Function {
            type_param_count: 1,
            params: vec![
                TypeRef::Generic(TypeParamId(0)),
                TypeRef::Generic(TypeParamId(1))
            ],
            return_type: Box::new(TypeRef::Generic(TypeParamId(1))),
            contexts: Default::default(),
        })]
    );
}

#[test]
fn quantified_constraints_cannot_be_silently_erased() {
    let error = compile("type Poly is { apply: fn<R: Missing>(R) -> R };")
        .expect_err("unrepresented constraints cannot become unconstrained quantifiers");
    assert!(
        error.to_string().contains("type-binder representation"),
        "{error}"
    );
    assert!(error.span.is_some());
}

#[test]
fn quantified_shadow_cannot_capture_the_impl_self_type() {
    use verum_vbc::types::{TypeParamId, TypeRef};
    let module = compile("type Holder<A> is { value: A }; implement<A> Holder<A> { fn inspect(callback: fn<A>(Self) -> A) -> Int { 0 } }")
        .expect("Self retains the outer declaration under a local binder");
    let owner = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Holder"))
        .unwrap();
    let function = module
        .functions
        .iter()
        .find(|function| module.get_string(function.name) == Some("Holder.inspect"))
        .unwrap();
    assert_eq!(
        function.params[0].type_ref,
        TypeRef::Rank2Function {
            type_param_count: 1,
            params: vec![TypeRef::Instantiated {
                base: owner.id,
                args: vec![TypeRef::Generic(TypeParamId(1))],
            }],
            return_type: Box::new(TypeRef::Generic(TypeParamId(0))),
            contexts: Default::default(),
        }
    );
}

#[test]
fn a_foreign_same_leaf_does_not_supply_declaration_authority() {
    refuses(
        "module foreign { public type Missing is { value: Int }; } type Holder is { inner: Missing };",
    );
}

#[test]
fn forwards_and_generic_fields_keep_their_declared_identity() {
    use verum_vbc::types::{TypeParamId, TypeRef};
    let module = compile("type Holder is { inner: Box<Later> }; type Box<T> is { value: T }; type Later is { value: Int };")
        .expect("reachable source identities are claimed before field lowering");
    let descriptor = |name| {
        module
            .types
            .iter()
            .find(|ty| module.get_string(ty.name) == Some(name))
            .unwrap()
    };
    let inner = descriptor("Holder").fields[0]
        .declaration_type
        .as_ref()
        .unwrap();
    assert_eq!(
        inner,
        &TypeRef::Instantiated {
            base: descriptor("Box").id,
            args: vec![TypeRef::Concrete(descriptor("Later").id)]
        }
    );
    assert_eq!(
        descriptor("Box").fields[0].declaration_type,
        Some(TypeRef::Generic(TypeParamId(0)))
    );
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
}

#[test]
fn explicit_raw_pointer_representation_is_preserved() {
    use verum_vbc::types::{TypeId, TypeRef};
    let module = compile("type Opaque is { marker: Int }; type Foreign<T> is { pointer: *const T, reference: &unsafe Opaque };")
        .expect("explicit raw pointers retain their physical carrier");
    let descriptor = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Foreign"))
        .unwrap();
    assert!(
        descriptor
            .fields
            .iter()
            .all(|field| field.type_ref == TypeRef::Concrete(TypeId::PTR))
    );
    assert!(matches!(
        descriptor.fields[1].declaration_type,
        Some(TypeRef::Reference { .. })
    ));
}

#[test]
fn owned_declaration_graph_resolves_cycles_without_provisional_types() {
    use verum_vbc::types::TypeRef;
    let alpha = Parser::new("mount beta.B; public type A is { peer: &B };")
        .parse_module()
        .unwrap();
    let beta = Parser::new("mount alpha.A; public type B is { peer: &A };")
        .parse_module()
        .unwrap();
    for sources in [
        vec![("alpha", &alpha), ("beta", &beta)],
        vec![("beta", &beta), ("alpha", &alpha)],
    ] {
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("declaration_graph"));
        let module = codegen
            .collect_owned_declaration_interface(&sources)
            .expect("source graph is complete");
        for module in [
            module.clone(),
            verum_vbc::deserialize::deserialize_module(
                &verum_vbc::serialize::serialize_module(&module).unwrap(),
            )
            .unwrap(),
        ] {
            let descriptor = |name| {
                module
                    .types
                    .iter()
                    .find(|ty| module.get_string(ty.name) == Some(name))
                    .unwrap()
            };
            for (name, owner, other) in [("A", "alpha", "B"), ("B", "beta", "A")] {
                let ty = descriptor(name);
                assert_eq!(
                    ty.origin_module.and_then(|id| module.get_string(id)),
                    Some(owner)
                );
                let Some(TypeRef::Reference { inner, .. }) = &ty.fields[0].declaration_type else {
                    panic!("declaration borrow must remain structural");
                };
                assert_eq!(inner.as_ref(), &TypeRef::Concrete(descriptor(other).id));
            }
            assert!(module.bytecode.is_empty());
        }
    }
}

#[test]
fn declaration_graph_keeps_short_headers_under_their_resolved_parent() {
    let ast = Parser::new("module nested; public type Token is { value: Int };")
        .parse_module()
        .unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("declaration_graph"))
        .collect_owned_declaration_interface(&[("owner", &ast)])
        .unwrap();
    let token = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Token"))
        .unwrap();
    assert_eq!(
        token.origin_module.and_then(|id| module.get_string(id)),
        Some("owner.nested")
    );
}

#[test]
fn declaration_graph_keeps_same_named_impls_owned_by_each_source() {
    let declaration = "type Reader is protocol { fn read(&self) -> Int; }; type Handle is { value: Int }; implement Reader for Handle { fn read(&self) -> Int { self.value } }";
    let alpha = Parser::new(declaration).parse_module().unwrap();
    let beta = Parser::new(declaration).parse_module().unwrap();
    for sources in [
        vec![("alpha", &alpha), ("beta", &beta)],
        vec![("beta", &beta), ("alpha", &alpha)],
    ] {
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("declaration_graph"));
        let module = codegen.collect_owned_declaration_interface(&sources).unwrap();
        let exports = codegen.export_functions();
        let alpha = exports.get("alpha.Handle.read").expect("alpha declaration");
        let beta = exports.get("beta.Handle.read").expect("beta declaration");
        assert_ne!(alpha.id, beta.id);
        for (owner, source) in [("alpha", alpha), ("beta", beta)] {
            let methods: Vec<_> = module.functions.iter().filter(|function|
                function.parent_type.is_some()
                    && function.origin_module.and_then(|id| module.get_string(id)) == Some(owner)
                    && module.get_string(function.name) == Some("Handle.read")
            ).collect();
            assert_eq!(methods.len(), 1, "{owner}: no cross-owner sibling ordinal");
            assert_eq!(methods[0].id, source.id);
        }
    }
}

#[test]
fn inline_module_retains_its_outer_nominal_type_with_the_same_name() {
    let module = compile("type Factory<T> is { value: T }; module Factory { fn preserve<T>(value: Factory<T>) -> Factory<T> { value } }").unwrap();
    for module in [module.clone(), verum_vbc::deserialize::deserialize_module(&verum_vbc::serialize::serialize_module(&module).unwrap()).unwrap()] {
        let factory = module.types.iter().find(|ty| module.get_string(ty.name) == Some("Factory")).unwrap();
        let preserve = module.functions.iter().find(|function| module.get_string(function.name).is_some_and(|name| name.ends_with("preserve"))).unwrap();
        assert_eq!(preserve.semantic_return.as_ref().and_then(|ty| ty.base_type_id()), Some(factory.id));
        assert_eq!(preserve.semantic_parameter_types().unwrap()[0], preserve.semantic_return);
    }
}

#[test]
fn anonymous_entry_inline_module_can_use_its_outer_nominal_type() {
    let ast = Parser::new("type Token is { value: Int }; module nested { type Holder is { token: Token }; }").parse_module().unwrap();
    VbcCodegen::new().compile_module(&ast).expect("actual inline lexical edge, including anonymous source");
}

#[test]
fn inline_scope_inherits_mounts_but_prefers_its_own_declaration() {
    let foreign = Parser::new("module foreign; public type Token is { value: Int };").parse_module().unwrap();
    let local = Parser::new("module local; mount foreign.Token; module inherited { type Holder is { token: Token }; } module shadow { type Token is { flag: Bool }; type Holder is { token: Token }; }").parse_module().unwrap();
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("graph"));
    let module = cg.collect_owned_declaration_interface(&[("local", &local), ("foreign", &foreign)]).unwrap();
    let descriptor = |owner, name| module.types.iter().find(|ty|
        module.get_string(ty.name) == Some(name)
            && ty.origin_module.and_then(|id| module.get_string(id)) == Some(owner)
    ).unwrap();
    for (scope, declaring) in [("local.inherited", "foreign"), ("local.shadow", "local.shadow")] {
        assert_eq!(descriptor(scope, "Holder").fields[0].declaration_type.as_ref().and_then(|ty| ty.base_type_id()), Some(descriptor(declaring, "Token").id));
    }
}

#[test]
fn dotted_file_name_does_not_create_lexical_ancestry() {
    let outer = Parser::new("module outer; public type Token is { value: Int };").parse_module().unwrap();
    let child = Parser::new("module outer.child; fn use_token(token: Token) -> Int { 1 }").parse_module().unwrap();
    let error = VbcCodegen::with_config(CodegenConfig::new("graph"))
        .collect_owned_declaration_interface(&[("outer", &outer), ("outer.child", &child)])
        .expect_err("separate files require a mount, regardless of path prefix");
    assert!(error.to_string().contains("unresolved nominal type 'Token' in declaration scope 'outer.child'"), "{error}");
}

#[test]
fn inline_scope_cannot_capture_a_siblings_type() {
    let error = compile("module left { type Token is { value: Int }; } module right { fn use_token(token: Token) -> Int { 1 } }").unwrap_err();
    assert!(error.to_string().contains("unresolved nominal type 'Token' in declaration scope 'declaration_probe.right'"), "{error}");
}

#[test]
fn unresolved_inline_mount_cannot_fall_back_to_an_outer_declaration() {
    let error = compile("type Token is { value: Int }; module nested { mount missing.Token; fn use_token(token: Token) -> Int { 1 } }").unwrap_err();
    assert!(error.to_string().contains("unresolved nominal type 'Token' in declaration scope 'declaration_probe.nested'"), "{error}");
}
