//! T2860: body premises are obligations at every selected call.
use std::sync::Arc;
use verum_ast::{ItemKind, Module};
use verum_common::{List, ResourceDiscipline, Text};
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, core_metadata::*};

const OWNER: &str = "equalities.provider";
const TYPES: &str = r#"
    public type Outcome<T> is Idle | Selected(T);
    public type Family is protocol { type Item; };
"#;

fn parse(source: &str) -> Module {
    Parser::new(source).parse_module().expect("source grammar")
}

fn check(checker: &mut TypeChecker, module: &Module) -> List<Text> {
    let mut errors = checker.prepare_module_declarations(module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    errors
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|error| format!("{error:?}").into())
        .collect()
}

fn source_errors(declaration: &str, caller: &str) -> List<Text> {
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path(OWNER);
    check(
        &mut checker,
        &parse(&format!("{TYPES} {declaration} {caller}")),
    )
}

fn metadata(declaration: &str) -> CoreMetadata {
    let mut metadata = CoreMetadata {
        version: CORE_METADATA_VERSION,
        ..Default::default()
    };
    for item in parse(TYPES).items {
        let ItemKind::Type(declaration) = item.kind else {
            panic!("type declaration")
        };
        let kind = match &declaration.body {
            verum_ast::decl::TypeDeclBody::Protocol(_) => TypeDescriptorKind::Protocol {
                super_protocols: List::new(),
                associated_types: List::new(),
                required_methods: List::new(),
                default_methods: List::new(),
            },
            verum_ast::decl::TypeDeclBody::Variant(variants) => TypeDescriptorKind::Variant {
                cases: variants.iter().map(|variant| VariantCase {
                    name: variant.name.name.clone(),
                    payload: variant.data.as_ref().map(|payload| match payload {
                        verum_ast::decl::VariantData::Tuple(types) => VariantPayload::Tuple(
                            types.iter().map(verum_ast::pretty::format_descriptor_type).collect(),
                        ),
                        _ => panic!("fixture uses ordinary tuple cases"),
                    }),
                }).collect(),
            },
            _ => panic!("fixture uses a protocol and an ordinary sum"),
        };
        // Ordinary sums use the complete descriptor telescope and cases; the
        // source selector truthfully omits a redundant source AST for them.
        let generic_params = declaration.generics.iter().enumerate().map(|(index, parameter)| {
            let verum_ast::ty::GenericParamKind::Type { name, bounds, default } = &parameter.kind else {
                panic!("fixture uses ordinary type parameters");
            };
            assert!(bounds.is_empty() && default.is_none());
            GenericParam {
                name: name.name.clone(), bounds: List::new(), default: None,
                type_bounds: List::new(), pid: Some(u16::try_from(index).unwrap()),
            }
        }).collect();
        let key: Text = format!("{OWNER}.{}", declaration.name.name).into();
        metadata.type_declaration_order.push(key.clone());
        metadata.types.insert(
            key,
            TypeDescriptor {
                name: declaration.name.name.clone(),
                module_path: OWNER.into(),
                origin_module_path: None,
                source_declaration: verum_ast::source_signature::source_type_declaration(
                    &declaration,
                )
                .unwrap(),
                generic_params,
                kind,
                size: None,
                alignment: None,
                methods: List::new(),
                implements: List::new(),
                decl_span: None,
                is_public: true,
                is_transparent_wrapper: false,
                resource_discipline: ResourceDiscipline::Unrestricted,
            },
        );
    }
    let module = parse(declaration);
    let ItemKind::Function(function) = &module.items[0].kind else {
        panic!("function declaration")
    };
    // Use the real producer decision. A hand-installed AST would conceal an
    // archive route that never carries the equality in the first place.
    let signature = verum_ast::source_signature::source_signature(function, false)
        .unwrap()
        .expect("where equality requires its source signature carrier");
    metadata.functions.insert(
        format!("{OWNER}.choose").into(),
        FunctionDescriptor {
            name: "choose".into(),
            module_path: OWNER.into(),
            origin_module_path: None,
            source_signature: Some(signature),
            generic_params: List::new(),
            params: List::from_single(ParamDescriptor {
                name: "item".into(),
                ty: "__opaque_src".into(),
                declared_ty: "__opaque_src".into(),
                has_default: false,
                default_literal: None,
            }),
            return_type: "__opaque_src".into(),
            contexts: List::new(),
            is_async: false,
            is_unsafe: false,
            intrinsic_id: None,
            parent_type: None,
            impl_generic_names: List::new(),
            is_const: false,
            decl_span: None,
            is_public: true,
            explicit_type_param_ids: None,
        },
    );
    decode_metadata(&encode_metadata(&metadata).unwrap()).unwrap()
}

fn metadata_errors(declaration: &str, caller: &str, eager: bool) -> List<Text> {
    let metadata = Arc::new(metadata(declaration));
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        TypeChecker::new_with_core(metadata)
    };
    checker.register_primitives();
    checker.set_current_module_path("equalities.client");
    let caller = caller
        .replace("choose", &format!("{OWNER}.choose"))
        .replace("Outcome", &format!("{OWNER}.Outcome"))
        .replace("Family", &format!("{OWNER}.Family"));
    check(&mut checker, &parse(&caller))
}

fn rejects(errors: &[Text]) {
    assert!(
        errors.iter().any(|error| error.contains("Mismatch")
            || error.contains("E405")
            || error.contains("equality")),
        "a constant body cannot discharge a false call equality: {errors:#?}"
    );
}

const EXACT: &str = "public fn choose<T>(item:T)->Int where T = Outcome<Int> { 0 }";
const PROJECTED: &str = "public fn choose<F:Family>(item:F)->Int where F.Item = Int { 0 }";

#[test]
fn source_exact_equality_checks_the_argument_even_with_a_constant_body() {
    let errors = source_errors(EXACT, "fn run()->Int { choose(Outcome.Selected(37)) }");
    assert!(errors.is_empty(), "{errors:#?}");
    rejects(&source_errors(EXACT, "fn run()->Int { choose(true) }"));
}

#[test]
fn explicit_type_equality_is_an_independent_parser_control() {
    let declaration = EXACT.replace("where T", "where type T");
    let errors = source_errors(
        &declaration,
        "fn run()->Int { choose(Outcome.Selected(37)) }",
    );
    assert!(errors.is_empty(), "{errors:#?}");
    rejects(&source_errors(
        &declaration,
        "fn run()->Int { choose(true) }",
    ));
}

#[test]
fn source_projected_equality_requires_the_actual_callers_member() {
    let errors = source_errors(
        PROJECTED,
        "fn run<F:Family<Item=Int>>(item:F)->Int { choose(item) }",
    );
    assert!(errors.is_empty(), "{errors:#?}");
    rejects(&source_errors(
        PROJECTED,
        "fn run<F:Family<Item=Bool>>(item:F)->Int { choose(item) }",
    ));
}

#[test]
fn another_binder_or_an_unconstrained_projection_cannot_prove_equality() {
    for caller in [
        "fn run<F:Family>(item:F)->Int { choose(item) }",
        "fn run<F:Family, G:Family<Item=Int>>(item:F,other:G)->Int { choose(item) }",
    ] {
        rejects(&source_errors(PROJECTED, caller));
    }
}

#[test]
fn exact_equality_is_not_a_directional_numeric_conversion() {
    let declaration = "public fn choose<T>(item:T)->Int where type T = Int { 0 }";
    let errors = source_errors(declaration, "fn run(item:Int)->Int { choose(item) }");
    assert!(errors.is_empty(), "{errors:#?}");
    rejects(&source_errors(
        declaration,
        "fn run(item:Int32)->Int { choose<Int32>(item) }",
    ));
}

#[test]
fn metadata_retains_and_checks_exact_equality() {
    for eager in [false, true] {
        let errors = metadata_errors(
            EXACT,
            "fn run(item:Outcome<Int>)->Int { choose(item) }",
            eager,
        );
        assert!(errors.is_empty(), "eager={eager}: {errors:#?}");
        rejects(&metadata_errors(
            EXACT,
            "fn run()->Int { choose(true) }",
            eager,
        ));
    }
}

#[test]
fn metadata_retains_projected_equality_and_its_owner() {
    for eager in [false, true] {
        let errors = metadata_errors(
            PROJECTED,
            "fn run<F:Family<Item=Int>>(item:F)->Int { choose(item) }",
            eager,
        );
        assert!(errors.is_empty(), "eager={eager}: {errors:#?}");
        rejects(&metadata_errors(
            PROJECTED,
            "fn run<F:Family<Item=Bool>>(item:F)->Int { choose(item) }",
            eager,
        ));
    }
}

#[test]
fn a_generic_caller_uses_only_its_own_equations() {
    let declaration = "public fn choose<T>(item:T)->Int where T=Int { 0 }";
    for caller in [
        "fn relay<T>(item:T)->Int where T=Int { choose(item) }",
        "fn relay<S>(item:S)->Int where S=Int { choose(item) }",
        "fn relay<T,U>(item:T)->Int where T=U,U=Int { choose(item) }",
    ] {
        let errors = source_errors(declaration, caller);
        assert!(errors.is_empty(), "{caller}: {errors:#?}");
        for eager in [false, true] {
            let errors = metadata_errors(declaration, caller, eager);
            assert!(errors.is_empty(), "eager={eager}, {caller}: {errors:#?}");
        }
    }
    rejects(&source_errors(
        declaration,
        "fn relay<T>(item:T)->Int { choose(item) }",
    ));
    rejects(&source_errors(
        declaration,
        "fn bounded<T>(item:T)->Int where T=Int { choose(item) }
         fn unbounded<T>(item:T)->Int { choose(item) }",
    ));
    for eager in [false, true] {
        rejects(&metadata_errors(
            declaration,
            "fn relay<T,U>(item:T,other:U)->Int where U=Int { choose(item) }",
            eager,
        ));
    }
}

#[test]
fn named_function_values_retain_their_instantiation_equations() {
    let errors = source_errors(
        EXACT,
        "fn run(item:Outcome<Int>)->Int { let selected=choose; selected(item) }",
    );
    assert!(errors.is_empty(), "{errors:#?}");
    rejects(&source_errors(
        EXACT,
        "fn run()->Int { let selected=choose; selected(true) }",
    ));
}

#[test]
fn transitive_and_composite_equations_check_every_explicit_argument() {
    let declaration = "public fn choose<T,U>(item:T)->Int where T=U,U=Int { 0 }";
    let errors = source_errors(declaration, "fn run()->Int { choose<Int,Int>(37) }");
    assert!(errors.is_empty(), "{errors:#?}");
    rejects(&source_errors(
        declaration,
        "fn run()->Int { choose<Int,Bool>(37) }",
    ));
    let declaration = "public fn choose<T>(item:T)->Int where [T]=[Int] { 0 }";
    let errors = source_errors(declaration, "fn run()->Int { choose<Int>(37) }");
    assert!(errors.is_empty(), "{errors:#?}");
    rejects(&source_errors(
        declaration,
        "fn run()->Int { choose<Bool>(true) }",
    ));
}

#[test]
fn inherited_protocol_methods_keep_projected_equalities() {
    let declaration = r#"
        type Source is protocol {
            type Item;
            fn choose<T>(&self,item:T)->Int where Self.Item=T { 0 }
        };
        type Number is ();
        implement Source for Number { type Item=Int; }
    "#;
    let errors = source_errors(
        declaration,
        "fn run(value:&Number)->Int { value.choose(37) }",
    );
    assert!(errors.is_empty(), "{errors:#?}");
    rejects(&source_errors(
        declaration,
        "fn run(value:&Number)->Int { value.choose(true) }",
    ));
}

#[test]
fn inherent_methods_keep_equations_through_receiver_selection() {
    let declaration = r#"
        type Holder is ();
        implement Holder {
            fn choose<T>(&self,item:T)->Int where T=Int { 0 }
        }
    "#;
    let errors = source_errors(
        declaration,
        "fn run(value:&Holder)->Int { value.choose(37) }",
    );
    assert!(errors.is_empty(), "{errors:#?}");
    rejects(&source_errors(
        declaration,
        "fn run(value:&Holder)->Int { value.choose(true) }",
    ));
}

#[test]
fn an_override_cannot_add_an_unpromised_equation() {
    let declaration = r#"
        type Source is protocol { fn choose<T>(&self,item:T)->Int; };
        type Number is ();
        implement Source for Number { fn choose<T>(&self,item:T)->Int where T=Int { 0 } }
    "#;
    let errors = source_errors(declaration, "");
    assert!(
        errors
            .iter()
            .any(|error| error.contains("strengthens") && error.contains("equalit")),
        "{errors:#?}"
    );
}

#[test]
fn stale_sidecars_cannot_publish_an_erased_equality_as_current() {
    let current = metadata(EXACT);
    let key = Text::from(format!("{OWNER}.choose"));
    let restored = decode_metadata(&encode_metadata(&current).unwrap()).unwrap();
    assert_eq!(
        restored
            .functions
            .get(&key)
            .expect("exact provider function")
            .source_signature,
        current
            .functions
            .get(&key)
            .expect("exact provider function")
            .source_signature
    );
    assert!(
        restored
            .functions
            .get(&key)
            .expect("exact provider function")
            .source_signature
            .as_ref()
            .unwrap()
            .generic_where_clause
            .is_some()
    );

    let mut stale = current;
    stale.version = CORE_METADATA_VERSION - 1;
    stale.functions.get_mut(&key).unwrap().source_signature = None;
    let mut bytes: List<u8> = CORE_METADATA_MAGIC.iter().copied().collect();
    bytes.extend_from_slice(&stale.version.to_le_bytes());
    bytes.extend(bincode::serialize(&stale).unwrap());
    let error = decode_metadata(&bytes).unwrap_err();
    assert!(error.contains("unsupported metadata schema"), "{error}");
    assert!(
        encode_metadata(&stale)
            .unwrap_err()
            .contains("schema mismatch")
    );
}

#[test]
fn nominal_equality_uses_full_declaration_identity_across_carriers() {
    use verum_types::Type;
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path(OWNER);
    assert!(check(&mut checker, &parse(TYPES)).is_empty());
    let named = Type::Named {
        path: verum_ast::Path::single(verum_ast::Ident::new(
            "equalities.provider.Outcome",
            verum_ast::Span::dummy(),
        )),
        args: List::from_single(Type::Int),
    };
    let application = |owner: &str, argument: Type| Type::Generic {
        name: owner.into(),
        args: List::from_single(argument),
    };
    let generic = application("equalities.provider.Outcome", Type::Int);
    assert!(checker.types_equivalent(&named, &generic));
    assert!(checker.types_equivalent(&generic, &named));
    assert!(!checker.types_equivalent(&named, &application("equalities.other.Outcome", Type::Int)));
    assert!(!checker.types_equivalent(
        &named,
        &application("equalities.provider.Outcome", Type::Bool)
    ));
    let left_nested = application("equalities.provider.Outcome", generic.clone());
    let right_nested = application(
        "equalities.provider.Outcome",
        application("equalities.other.Outcome", Type::Int),
    );
    assert!(!checker.types_equivalent(&left_nested, &right_nested));
    let variable = Type::Var(verum_types::ty::TypeVar::fresh());
    let other_variable = Type::Var(verum_types::ty::TypeVar::fresh());
    assert!(!checker.types_equivalent(&variable, &other_variable));
    for unresolved in [Type::Unknown, variable] {
        assert!(!checker.types_equivalent(&unresolved, &Type::Int));
        assert!(!checker.types_equivalent(&unresolved, &named));
    }
    let representation_source = parse("fn repr(value:Outcome<Int>) {}");
    let ItemKind::Function(function) = &representation_source.items[0].kind else {
        panic!("function declaration");
    };
    let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &function.params[0].kind else {
        panic!("declared parameter");
    };
    let representation = checker.ast_to_type(ty).unwrap();
    assert!(
        matches!(representation, Type::Variant(_)),
        "{representation:?}"
    );
    assert!(!checker.types_equivalent(&named, &representation));
    let borrowed = Type::Reference {
        inner: Box::new(named.clone()),
        mutable: false,
    };
    assert!(checker.types_equivalent(
        &borrowed,
        &Type::Reference {
            inner: Box::new(generic.clone()),
            mutable: false
        }
    ));
    for other in [
        Type::Reference {
            inner: Box::new(generic.clone()),
            mutable: true,
        },
        Type::CheckedReference {
            inner: Box::new(generic.clone()),
            mutable: false,
        },
        Type::UnsafeReference {
            inner: Box::new(generic),
            mutable: false,
        },
    ] {
        assert!(!checker.types_equivalent(&borrowed, &other));
    }
}

#[test]
fn exact_function_equalities_check_source_and_metadata_calls() {
    let declaration = "public fn choose<F>(item:F)->Int where F=fn(Int)->Int { 0 }";
    for caller in ["fn run(item:fn(Int)->Int)->Int { choose(item) }"] {
        let errors = source_errors(declaration, caller);
        assert!(errors.is_empty(), "{errors:#?}");
        for eager in [false, true] {
            let errors = metadata_errors(declaration, caller, eager);
            assert!(errors.is_empty(), "eager={eager}: {errors:#?}");
        }
    }
    for caller in [
        "fn run(item:fn(Bool)->Int)->Int { choose(item) }",
        "fn run(item:fn(Int)->Bool)->Int { choose(item) }",
        "fn run(item:async fn(Int)->Int)->Int { choose(item) }",
    ] {
        rejects(&source_errors(declaration, caller));
        for eager in [false, true] {
            rejects(&metadata_errors(declaration, caller, eager));
        }
    }
}

#[test]
fn callable_bounds_remain_directional_but_exact_callable_equations_do_not() {
    let bound = "public fn choose<F:async fn(Int)->Int>(item:F)->Int { 0 }";
    let caller = "fn run(item:fn(Int)->Int)->Int { choose(item) }";
    let errors = source_errors(bound, caller);
    assert!(errors.is_empty(), "{errors:#?}");
    let exact = "public fn choose<F>(item:F)->Int where F=async fn(Int)->Int { 0 }";
    // Fix F explicitly: argument conversion to an async-permitting contract
    // must not be confused with proof that the two declared types are equal.
    let exact_caller = "fn run(item:fn(Int)->Int)->Int { choose<fn(Int)->Int>(item) }";
    rejects(&source_errors(exact, exact_caller));
    for eager in [false, true] {
        rejects(&metadata_errors(exact, exact_caller, eager));
    }
}

#[test]
fn projected_function_equations_keep_the_selected_family_owner() {
    let declaration = "public fn choose<F:Family>(item:F)->Int where F.Item=fn(Int)->Int { 0 }";
    let valid = "fn run<F:Family<Item=fn(Int)->Int>>(item:F)->Int { choose(item) }";
    let invalid = "fn run<F:Family<Item=fn(Int)->Bool>>(item:F)->Int { choose(item) }";
    let errors = source_errors(declaration, valid);
    assert!(errors.is_empty(), "{errors:#?}");
    rejects(&source_errors(declaration, invalid));
    for eager in [false, true] {
        let errors = metadata_errors(declaration, valid, eager);
        assert!(errors.is_empty(), "eager={eager}: {errors:#?}");
        rejects(&metadata_errors(declaration, invalid, eager));
    }
}

#[test]
fn function_type_equality_compares_requirements_without_inventing_body_properties() {
    use verum_types::{Type, computational_properties::PropertySet};
    let checker = TypeChecker::with_minimal_context();
    let declared = Type::function_with_async(List::from_single(Type::Int), Type::Int, false);
    let mut inferred = declared.clone();
    let Type::Function { properties, .. } = &mut inferred else {
        unreachable!()
    };
    *properties = Some(PropertySet::pure());
    assert!(checker.types_equivalent(&declared, &inferred));
    let asynchronous = Type::function_with_async(List::from_single(Type::Int), Type::Int, true);
    assert!(!checker.types_equivalent(&declared, &asynchronous));
    assert!(!checker.types_equivalent(&asynchronous, &declared));
}
