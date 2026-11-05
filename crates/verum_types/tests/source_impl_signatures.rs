//! Source implementation methods must honor their instantiated declaration.
use verum_fast_parser::Parser;
use verum_types::TypeChecker;
fn errors(source: &str) -> Vec<String> {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    errors_in(&mut checker, source)
}
fn errors_in(checker: &mut TypeChecker, source: &str) -> Vec<String> {
    let ast = Parser::new(source).parse_module().unwrap();
    checker.set_current_module_path("core.owned");
    let mut errors: Vec<_> = checker
        .prepare_module_declarations(&ast)
        .iter()
        .map(|e| format!("{e:?}"))
        .collect();
    for item in &ast.items {
        if let Err(e) = checker.check_item(item) {
            errors.push(format!("{e:?}"));
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(|e| format!("{e:?}")),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|e| format!("{e:?}")),
    );
    errors
}
fn implementation(name: &str, result: &str, actual: &str) -> String {
    format!(
        "module core.owned; public type Family<A> is protocol {{ type {name}; fn make(a:A)->{result}; }}; public type Concrete is {{ value:() }}; implement Family<Int> for Concrete {{ type {name}=Int; fn make(a:Int)->{actual} {{ a }} }}"
    )
}
#[test]
fn bare_name_does_not_mean_associated_projection() {
    let errors = errors(&implementation("Q", "Q", "Int"));
    assert!(
        errors
            .iter()
            .any(|e| e.contains("TypeNotFound") && e.contains("\"Q\"")),
        "{errors:?}"
    );
}
#[test]
fn owned_projection_allows_actual_assignment() {
    let errors = errors(&implementation("Q", "Self.Q", "Int"));
    assert!(errors.is_empty(), "{errors:?}");
}
#[test]
fn renamed_owned_projection_allows_actual_assignment() {
    let errors = errors(&implementation("Carrier", "Self.Carrier", "Int"));
    assert!(errors.is_empty(), "{errors:?}");
}
#[test]
fn wrong_associated_assignment_rejected() {
    let source = implementation("Carrier", "Self.Carrier", "Int")
        .replace("type Carrier=Int", "type Carrier=Bool");
    let errors = errors(&source);
    assert!(errors.iter().any(|e| e.contains("E427")), "{errors:?}");
}

fn accepts(source: &str) {
    let e = errors(source);
    assert!(e.is_empty(), "{e:?}");
}
fn rejects(source: &str) {
    let e = errors(source);
    assert!(e.iter().any(|x| x.contains("E427")), "{e:?}");
}
#[test]
fn nested_result_carrier() {
    accepts(
        "module core.owned; type F is protocol {type C; fn get()->(Self.C,Int);}; type Model is {x:()}; implement F for Model {type C=Bool;fn get()->(Bool,Int){(true,7)}}",
    );
}
#[test]
fn nested_wrong_result_carrier() {
    rejects(
        "module core.owned; type F is protocol {type C; fn get()->(Self.C,Int);}; type Model is {x:()}; implement F for Model {type C=Bool;fn get()->(Int,Int){(7,7)}}",
    );
}
#[test]
fn argument_carrier() {
    accepts(
        "module core.owned; type F is protocol {type C; fn pass(x:Self.C)->Self.C;}; type Model is {x:()}; implement F for Model {type C=Bool;fn pass(x:Bool)->Bool{x}}",
    );
}
#[test]
fn wrong_argument_carrier() {
    rejects(
        "module core.owned; type F is protocol {type C; fn pass(x:Self.C)->Self.C;}; type Model is {x:()}; implement F for Model {type C=Bool;fn pass(x:Int)->Bool{true}}",
    );
}
#[test]
fn ordinary_alias_carrier() {
    accepts(
        "module core.owned; type Flag is Bool; type F is protocol {type C; fn pass(x:Self.C)->Self.C;}; type Model is {x:()}; implement F for Model {type C=Flag;fn pass(x:Bool)->Bool{x}}",
    );
}
#[test]
fn owner_parameter_carrier() {
    accepts(
        "module core.owned; type F<A> is protocol {type C; fn pass(x:A)->Self.C;}; type Model<T> is {x:T}; implement<T> F<T> for Model<T> {type C=T;fn pass(x:T)->T{x}}",
    );
}
#[test]
fn renamed_local_method_binder() {
    accepts(
        "module core.owned; type F is protocol {type C; fn pair<U>(x:U)->(Self.C,U);}; type Model is {x:()}; implement F for Model {type C=Bool;fn pair<V>(x:V)->(Bool,V){(true,x)}}",
    );
}
#[test]
fn local_method_binder_cannot_specialize() {
    rejects(
        "module core.owned; type F is protocol {type C; fn pair<U>(x:U)->(Self.C,U);}; type Model is {x:()}; implement F for Model {type C=Bool;fn pair<V>(x:V)->(Bool,Int){(true,7)}}",
    );
}
#[test]
fn reference_carrier() {
    accepts(
        "module core.owned; type F is protocol {type C; fn pass(x:&Self.C)->&Self.C;}; type Model is {x:()}; implement F for Model {type C=Bool;fn pass(x:&Bool)->&Bool{x}}",
    );
}
#[test]
fn reference_mutability_mismatch() {
    rejects(
        "module core.owned; type F is protocol {type C; fn pass(x:&Self.C)->&Self.C;}; type Model is {x:()}; implement F for Model {type C=Bool;fn pass(x:&mut Bool)->&mut Bool{x}}",
    );
}
#[test]
fn never_is_not_an_equal_result_carrier() {
    rejects(
        "module core.owned; type F is protocol {type C; fn get()->Self.C;}; type Model is {x:()}; implement F for Model {type C=!;fn get()->Int{7}}",
    );
}
#[test]
fn foreign_receiver_projection_is_preserved() {
    accepts(
        "module core.owned; type Other is protocol {type C;}; type F<A:Other> is protocol {type C; fn pass(x:A.C)->A.C;}; type Model<T> is {x:T}; implement<T:Other> F<T> for Model<T> {type C=Int;fn pass(x:T.C)->T.C{x}}",
    );
}
#[test]
fn foreign_receiver_projection_is_not_own_assignment() {
    rejects(
        "module core.owned; type Other is protocol {type C;}; type F<A:Other> is protocol {type C; fn pass(x:A.C)->A.C;}; type Model<T> is {x:T}; implement<T:Other> F<T> for Model<T> {type C=Int;fn pass(x:Int)->Int{x}}",
    );
}

#[test]
fn same_receiver_other_protocol_projection_is_not_own_assignment() {
    rejects(
        "module core.owned; type Other is protocol {type C;}; type F<A:Other> is protocol {type C; fn pass(x:A.C)->A.C;}; implement<T:Other> F<T> for T {type C=Int;fn pass(x:Int)->Int{x}}",
    );
}

#[test]
fn associated_family_result() {
    accepts(
        "module core.owned; type F is protocol {type C<T>; fn pass<U>(x:Self.C<U>)->Self.C<U>;}; type Model is {x:()}; implement F for Model {type C<T> =(T,Bool);fn pass<V>(x:(V,Bool))->(V,Bool){x}}",
    );
}
#[test]
fn wrong_associated_family_result() {
    rejects(
        "module core.owned; type F is protocol {type C<T>; fn pass<U>(x:Self.C<U>)->Self.C<U>;}; type Model is {x:()}; implement F for Model {type C<T> =(T,Bool);fn pass<V>(x:(V,Int))->(V,Int){x}}",
    );
}
#[test]
fn method_protocol_bounds_can_rename_local_binder() {
    accepts(
        "module core.owned; type Bound is protocol {}; type F is protocol {type C; fn pair<U:Bound>(x:U)->(Self.C,U);}; type Model is {x:()}; implement F for Model {type C=Bool;fn pair<V:Bound>(x:V)->(Bool,V){(true,x)}}",
    );
}
#[test]
fn method_cannot_add_local_bound() {
    rejects(
        "module core.owned; type Bound is protocol {}; type F is protocol {type C; fn pair<U>(x:U)->(Self.C,U);}; type Model is {x:()}; implement F for Model {type C=Bool;fn pair<V:Bound>(x:V)->(Bool,V){(true,x)}}",
    );
}
#[test]
fn owner_parameter_cannot_specialize() {
    rejects(
        "module core.owned; type F<A> is protocol {type C; fn pass(x:A)->Self.C;}; type Model<T> is {x:T}; implement<T> F<T> for Model<T> {type C=Bool;fn pass(x:Int)->Bool{true}}",
    );
}
#[test]
fn distinct_nominal_carriers_with_identical_fields() {
    rejects(
        "module core.owned; type Left is {value:Bool}; type Right is {value:Bool}; type F is protocol {type C; fn get()->Self.C;}; type Model is {x:()}; implement F for Model {type C=Left;fn get()->Right{Right{value:true}}}",
    );
}
#[test]
fn own_receiver_can_be_spelled_concretely() {
    accepts(
        "module core.owned; type F is protocol {fn clone(&self)->Self;}; type Model is {value:Int}; implement F for Model {fn clone(&self)->Model{Model{value:self.value}}}",
    );
}
#[test]
fn receiver_mutation_cannot_be_strengthened() {
    rejects(
        "module core.owned; type F is protocol {fn get(&self)->Int;}; type Model is {value:Int}; implement F for Model {fn get(&mut self)->Int{self.value}}",
    );
}

#[test]
fn method_can_drop_local_bound() {
    accepts(
        "module core.owned; type Bound is protocol {}; type F is protocol {fn pair<U:Bound>(x:U)->U;}; type Model is {x:()}; implement F for Model {fn pair<V>(x:V)->V{x}}",
    );
}
#[test]
fn default_associated_carrier() {
    accepts(
        "module core.owned; type F is protocol {type C = Bool; fn pass(x:Self.C)->Self.C;}; type Model is {x:()}; implement F for Model {fn pass(x:Bool)->Bool{x}}",
    );
}
#[test]
fn wrong_default_associated_carrier() {
    rejects(
        "module core.owned; type F is protocol {type C = Bool; fn pass(x:Self.C)->Self.C;}; type Model is {x:()}; implement F for Model {fn pass(x:Int)->Int{x}}",
    );
}

fn imported_checker() -> TypeChecker {
    use verum_types::core_metadata::*;
    let ast=Parser::new("public type Family<A> is protocol { type Carrier; fn pair<U>(a:A,u:U)->(Self.Carrier,U); };").parse_module().unwrap();
    let verum_ast::ItemKind::Type(declaration) = &ast.items[0].kind else {
        panic!("declaration")
    };
    let descriptor = TypeDescriptor {
        source_declaration: verum_ast::source_signature::source_type_declaration(declaration)
            .unwrap(),
        name: "Family".into(),
        module_path: "provider".into(),
        origin_module_path: None,
        generic_params: vec![GenericParam {
            name: "A".into(),
            bounds: Default::default(),
            default: None,
            type_bounds: Default::default(),
            pid: Some(0),
        }]
        .into(),
        kind: TypeDescriptorKind::Protocol {
            super_protocols: Default::default(),
            associated_types: Default::default(),
            required_methods: Default::default(),
            default_methods: Default::default(),
        },
        size: None,
        alignment: None,
        methods: Default::default(),
        implements: Default::default(),
        decl_span: None,
        is_public: true,
        is_transparent_wrapper: false,
        resource_discipline: verum_common::ResourceDiscipline::Unrestricted,
    };
    let mut metadata = CoreMetadata {
        version: CORE_METADATA_VERSION,
        ..Default::default()
    };
    metadata
        .type_declaration_order
        .push("provider.Family".into());
    metadata.types.insert("provider.Family".into(), descriptor);
    let bytes = encode_metadata(&metadata).unwrap();
    let restored = decode_metadata(&bytes).unwrap();
    TypeChecker::new_with_core_eager(std::sync::Arc::new(restored))
}
#[test]
fn metadata_replayed_signature_keeps_owned_carrier_and_local_scope() {
    let mut checker = imported_checker();
    let e = errors_in(
        &mut checker,
        "module core.owned; type Model is {x:()}; implement provider.Family<Int> for Model { type Carrier=Bool; fn pair<V>(a:Int,u:V)->(Bool,V){(true,u)} }",
    );
    assert!(e.is_empty(), "{e:?}");
}
#[test]
fn metadata_replayed_signature_refuses_wrong_carrier() {
    let mut checker = imported_checker();
    let e = errors_in(
        &mut checker,
        "module core.owned; type Model is {x:()}; implement provider.Family<Int> for Model { type Carrier=Bool; fn pair<V>(a:Int,u:V)->(Int,V){(a,u)} }",
    );
    assert!(e.iter().any(|e| e.contains("E427")), "{e:?}");
}
#[test]
fn metadata_replayed_local_cannot_capture_owner() {
    let mut checker = imported_checker();
    let e = errors_in(
        &mut checker,
        "module core.owned; type Model is {x:()}; implement provider.Family<Int> for Model { type Carrier=Bool; fn pair<V>(a:Int,u:V)->(Bool,Int){(true,a)} }",
    );
    assert!(e.iter().any(|e| e.contains("E427")), "{e:?}");
}

#[test]
fn named_substitution_preserves_free_variables_under_quantifiers() {
    use verum_common::{List, Map};
    use verum_types::{Type, ty::TypeVar};
    let bound = TypeVar::fresh();
    let name = Type::Generic {
        name: "Owner".into(),
        args: List::new(),
    };
    let original = Type::Forall {
        vars: List::from_iter([bound]),
        body: Box::new(Type::Tuple(List::from_iter([name, Type::Var(bound)]))),
    };
    let mut subst = Map::new();
    subst.insert("Owner".into(), Type::Var(bound));
    let changed = original.try_substitute_params(&subst).unwrap();
    let Type::Forall { vars, body } = changed else {
        panic!("forall")
    };
    assert_ne!(vars[0], bound);
    assert_eq!(
        *body,
        Type::Tuple(List::from_iter([Type::Var(bound), Type::Var(vars[0])]))
    );
}
#[test]
fn named_substitution_refuses_depth_failure_without_partial_type() {
    use verum_common::{List, Map};
    use verum_types::Type;
    let mut nested = Type::Generic {
        name: "Owner".into(),
        args: List::new(),
    };
    for _ in 0..105 {
        nested = Type::Tuple(List::from_iter([nested]));
    }
    let mut subst = Map::new();
    subst.insert("Owner".into(), Type::Bool);
    assert!(
        nested
            .try_substitute_params(&subst)
            .unwrap_err()
            .contains("depth budget")
    );
}

#[test]
fn inherited_associated_method_uses_established_parent_assignment() {
    accepts(
        "module core.owned; type Base is protocol {type C;fn get()->Self.C;};type F is protocol extends Base {};type Model is {x:()};implement Base for Model {type C=Bool;fn get()->Bool{true}} implement F for Model {fn get()->Bool{true}}",
    );
}
#[test]
fn inherited_associated_method_cannot_replace_parent_assignment() {
    rejects(
        "module core.owned; type Base is protocol {type C;fn get()->Self.C;};type F is protocol extends Base {};type Model is {x:()};implement Base for Model {type C=Bool;fn get()->Bool{true}} implement F for Model {fn get()->Int{1}}",
    );
}
#[test]
fn inherited_projection_does_not_use_child_shadow() {
    rejects(
        "module core.owned; type Base is protocol {type C;fn get()->Self.C;};type F is protocol extends Base {type C;};type Model is {x:()};implement Base for Model {type C=Bool;fn get()->Bool{true}} implement F for Model {type C=Int;fn get()->Self.C{1}}",
    );
}
#[test]
fn inherited_projection_can_use_parent_carrier_despite_child_shadow() {
    accepts(
        "module core.owned; type Base is protocol {type C;fn get()->Self.C;};type F is protocol extends Base {type C;};type Model is {x:()};implement Base for Model {type C=Bool;fn get()->Bool{true}} implement F for Model {type C=Int;fn get()->Bool{true}}",
    );
}
#[test]
fn inherited_generic_argument_keeps_foreign_projection_origin() {
    rejects(
        "module core.owned;type Other is protocol {type C;};type Base<A:Other> is protocol {type C;fn get(x:A.C)->A.C;};type F is protocol extends Base<Self> {};type Model is {x:()};implement Other for Model {type C=Bool;}implement Base<Model> for Model {type C=Int;fn get(x:Model.C)->Model.C{x}}implement F for Model {fn get(x:Int)->Int{x}}",
    );
}

#[test]
fn rejected_signature_is_not_published_as_protocol_evidence() {
    use verum_ast::{Ident, Path, Span};
    use verum_common::List;
    use verum_types::Type;
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let e = errors_in(
        &mut checker,
        "module core.owned;type F is protocol {type C;fn get()->Self.C;};type Model is {x:()};implement F for Model {type C=Bool;fn get()->Int{1}}",
    );
    assert!(e.iter().any(|e| e.contains("E427")), "{e:?}");
    let receiver = Type::Named {
        path: Path::single(Ident::new("core.owned.Model", Span::dummy())),
        args: List::new(),
    };
    assert!(
        !checker
            .protocol_checker
            .read()
            .implements_by_name(&receiver, "core.owned.F")
    );
}

mod independent_inherited_and_quantified_controls {
    use verum_fast_parser::Parser;
    use verum_types::TypeChecker;
    fn errors(source: &str) -> Vec<String> {
        let ast = Parser::new(source).parse_module().unwrap();
        let mut c = TypeChecker::new();
        c.register_primitives();
        c.set_current_module_path("core.review");
        let mut e: Vec<_> = c
            .prepare_module_declarations(&ast)
            .iter()
            .map(|x| format!("{x:?}"))
            .collect();
        for i in &ast.items {
            if let Err(x) = c.check_item(i) {
                e.push(format!("{x:?}"));
            }
        }
        e.extend(c.take_deferred_errors().iter().map(|x| format!("{x:?}")));
        e.extend(
            c.diagnostics()
                .iter()
                .filter(|x| x.is_error())
                .map(|x| format!("{x:?}")),
        );
        e
    }
    fn reject(s: &str) {
        let e = errors(s);
        assert!(
            e.iter().any(|x| x.contains("E427")),
            "wrong carrier accepted or rejected elsewhere: {e:?}"
        )
    }
    fn accept(s: &str) {
        let e = errors(s);
        assert!(e.is_empty(), "{e:?}")
    }
    #[test]
    fn nested_function_wrong_result_rejected() {
        reject(
            "module core.review; type F is protocol {type C;fn take(f:fn(Int)->Self.C)->Int;}; type Model is {x:()}; implement F for Model {type C=Bool;fn take(f:fn(Int)->Int)->Int{1}}",
        );
    }
    #[test]
    fn nested_function_wrong_parameter_rejected() {
        reject(
            "module core.review; type F is protocol {type C;fn take(f:fn(Self.C)->Int)->Int;}; type Model is {x:()}; implement F for Model {type C=Bool;fn take(f:fn(Int)->Int)->Int{1}}",
        );
    }
    #[test]
    fn rank2_wrong_result_rejected() {
        reject(
            "module core.review; type F is protocol {type C;fn take(f:fn<R>(R)->Self.C)->Int;}; type Model is {x:()}; implement F for Model {type C=Bool;fn take(f:fn<S>(S)->Int)->Int{1}}",
        );
    }
    #[test]
    fn rank2_wrong_parameter_rejected() {
        reject(
            "module core.review; type F is protocol {fn take(f:fn<R>(R)->R)->Int;}; type Model is {x:()}; implement F for Model {fn take(f:fn<S>(Int)->S)->Int{1}}",
        );
    }
    #[test]
    fn rank2_valid_renaming() {
        accept(
            "module core.review; type F is protocol {fn take(f:fn<R>(R)->R)->Int;}; type Model is {x:()}; implement F for Model {fn take(f:fn<S>(S)->S)->Int{1}}",
        );
    }
    #[test]
    fn rank2_valid_owned_projection() {
        accept(
            "module core.review; type F is protocol {type C;fn take(f:fn<R>(R)->Self.C)->Int;}; type Model is {x:()}; implement F for Model {type C=Bool;fn take(f:fn<S>(S)->Bool)->Int{1}}",
        );
    }
    #[test]
    fn owner_bound_already_available() {
        accept(
            "module core.review; type Bound is protocol {};type F<A> is protocol {fn take(x:A)->Int;}; type Model<T> is {x:T}; implement<T:Bound> F<T> for Model<T> {fn take(x:T)->Int where T:Bound {1}}",
        );
    }
    #[test]
    fn foreign_same_receiver_projection_rejected() {
        reject(
            "module core.review; type Other is protocol {type C;};type F<A:Other> is protocol {type C;fn take(x:A.C)->Self.C;}; implement<T:Other> F<T> for T {type C=Int;fn take(x:Int)->Int{x}}",
        );
    }
    #[test]
    fn inherited_existing_base_bad_override_rejected() {
        reject(
            "module core.review;type Base is protocol {fn get()->Bool;};type F is protocol extends Base {};type Model is {x:()};implement Base for Model {fn get()->Bool{true}} implement F for Model {fn get()->Int{1}}",
        );
    }
    #[test]
    fn inherited_without_base_bad_override_rejected() {
        reject(
            "module core.review;type Base is protocol {fn get()->Bool;};type F is protocol extends Base {};type Model is {x:()};implement F for Model {fn get()->Int{1}}",
        );
    }
    #[test]
    fn nested_quantifier_cannot_capture_method_binder() {
        reject(
            "module core.review;type F is protocol {fn take<T>(x:fn<R>(R)->T)->Int;};type Model is {x:()};implement F for Model {fn take<U>(x:fn<S>(S)->S)->Int{1}}",
        );
    }
    #[test]
    fn callable_bound_cannot_strengthen_parameter() {
        reject(
            "module core.review;type F is protocol {fn take<T:fn(Int)->Bool>(x:T)->Int;};type Model is {x:()};implement F for Model {fn take<U:fn(Bool)->Bool>(x:U)->Int{1}}",
        );
    }
    #[test]
    fn bound_argument_foreign_owner_cannot_equal_self() {
        reject(
            "module core.review;type Bound<A> is protocol {};type F<A> is protocol {fn take<T:Bound<A>>(x:T)->Int;};type Model is {x:()};implement F<Int> for Model {fn take<U:Bound<Self>>(x:U)->Int{1}}",
        );
    }
    #[test]
    fn inherited_bad_override_cannot_supply_a_generic_witness() {
        reject(
            "module core.review;type Base is protocol {fn get(&self)->Bool;};type F is protocol extends Base {};type Model is {x:()};implement F for Model {fn get(&self)->Int{1}} fn call<T:F>(x:&T)->Bool{x.get()} fn witness()->Bool{let m=Model{x:()};call(&m)}",
        );
    }
    #[test]
    fn inherited_contradictory_bases_cannot_select_one_signature() {
        reject(
            "module core.review;type Left is protocol {fn get()->Bool;};type Right is protocol {fn get()->Int;};type F is protocol extends Left+Right {};type Model is {x:()};implement F for Model {fn get()->Bool{true}}",
        );
    }
    #[test]
    fn metadata_replayed_inherited_mismatch_rejected() {
        use verum_types::core_metadata::*;
        let mut metadata = CoreMetadata {
            version: CORE_METADATA_VERSION,
            ..Default::default()
        };
        for (name, source) in [
            ("Base", "public type Base is protocol {fn get()->Bool;};"),
            ("F", "public type F is protocol extends provider.Base {};"),
        ] {
            let ast = Parser::new(source).parse_module().unwrap();
            let verum_ast::ItemKind::Type(declaration) = &ast.items[0].kind else {
                panic!("declaration")
            };
            let d = TypeDescriptor {
                source_declaration: verum_ast::source_signature::source_type_declaration(
                    declaration,
                )
                .unwrap(),
                name: name.into(),
                module_path: "provider".into(),
                origin_module_path: None,
                generic_params: Default::default(),
                kind: TypeDescriptorKind::Protocol {
                    super_protocols: Default::default(),
                    associated_types: Default::default(),
                    required_methods: Default::default(),
                    default_methods: Default::default(),
                },
                size: None,
                alignment: None,
                methods: Default::default(),
                implements: Default::default(),
                decl_span: None,
                is_public: true,
                is_transparent_wrapper: false,
                resource_discipline: verum_common::ResourceDiscipline::Unrestricted,
            };
            let key = format!("provider.{name}");
            metadata.type_declaration_order.push(key.clone().into());
            metadata.types.insert(key.into(), d);
        }
        let wire = encode_metadata(&metadata).unwrap();
        let decoded = decode_metadata(&wire).unwrap();
        let mut c = TypeChecker::new_with_core_eager(std::sync::Arc::new(decoded));
        c.set_current_module_path("core.review");
        let source = "module core.review;type Model is {x:()};implement provider.F for Model {fn get()->Int{1}}";
        let ast = Parser::new(source).parse_module().unwrap();
        let mut e: Vec<_> = c
            .prepare_module_declarations(&ast)
            .iter()
            .map(|x| format!("{x:?}"))
            .collect();
        for item in &ast.items {
            if let Err(x) = c.check_item(item) {
                e.push(format!("{x:?}"));
            }
        }
        e.extend(c.take_deferred_errors().iter().map(|x| format!("{x:?}")));
        e.extend(
            c.diagnostics()
                .iter()
                .filter(|x| x.is_error())
                .map(|x| format!("{x:?}")),
        );
        assert!(
            e.iter().any(|x| x.contains("E427")),
            "metadata inherited wrong carrier accepted or refused elsewhere: {e:?}"
        );
    }
}
