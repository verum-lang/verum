//! Declaration semantics across the sidecar boundary, including hostile erased fields.
use std::sync::Arc;
use verum_ast::{FileId, ItemKind, Module};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::{Type, TypeChecker, core_metadata::*};

fn parse(source: &str) -> Module {
    FastParser::new().parse_module_str(source, FileId::new(1)).unwrap()
}

fn metadata(owner: &str, source: &str) -> CoreMetadata {
    let ast = parse(source);
    let ItemKind::Type(decl) = &ast.items[0].kind else { panic!("type") };
    let descriptor = TypeDescriptor {
        source_declaration: verum_ast::source_signature::source_type_declaration(decl).unwrap(),
        name: decl.name.name.clone(), module_path: owner.into(), origin_module_path: None,
        // VBC assigns runtime PIDs only to ordinary type parameters.
        generic_params: decl.generics.iter().filter_map(|p| {
            let verum_ast::ty::GenericParamKind::Type { name, .. } = &p.kind else { return None };
            Some(GenericParam { name: name.name.clone(), bounds: List::new(), default: None, type_bounds: List::new(), pid: None })
        }).collect(),
        // Runtime field strings are erased; declaration semantics must survive.
        kind: match &decl.body {
            verum_ast::TypeDeclBody::Record(fields) => TypeDescriptorKind::Record { fields: fields.iter().map(|f| FieldDescriptor {
                name: f.name.name.clone(), ty: "__opaque_src".into(), is_public: true, declared_visibility: Some(verum_ast::Visibility::Public),
            }).collect() },
            verum_ast::TypeDeclBody::Protocol(_) => TypeDescriptorKind::Protocol { super_protocols: List::new(), associated_types: List::new(), required_methods: List::new(), default_methods: List::new() },
            _ => panic!("fixture kind"),
        },
        size: None, alignment: None, methods: List::new(), implements: List::new(),
        decl_span: None, is_public: true, is_transparent_wrapper: false,
        resource_discipline: verum_common::ResourceDiscipline::Unrestricted,
    };
    let mut metadata = CoreMetadata { version: CORE_METADATA_VERSION, ..Default::default() };
    let key: Text = format!("{owner}.{}", descriptor.name).into();
    metadata.type_declaration_order.push(key.clone());
    metadata.types.insert(key, descriptor);
    metadata
}

fn check(metadata: CoreMetadata, source: &str, eager: bool) -> (TypeChecker, List<Text>) {
    let mut checker = if eager { TypeChecker::new_with_core_eager(Arc::new(metadata)) }
        else { TypeChecker::new_with_core(Arc::new(metadata)) };
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    let module = parse(source);
    let mut errors: List<Text> = checker.prepare_module_declarations(&module).iter()
        .map(|e| format!("{e}").into()).collect();
    for item in &module.items {
        if let Err(e) = checker.check_item(item) { errors.push(format!("{e}").into()); }
    }
    errors.extend(checker.take_deferred_errors().iter().map(|e| Text::from(format!("{e}"))));
    errors.extend(checker.diagnostics().iter().filter(|d| d.is_error()).map(|d| Text::from(format!("{d:?}"))));
    (checker, errors)
}

#[test]
fn nominal_fields_retain_universes_and_ordinary_generic_slots() {
    let metadata = metadata("test.provider", "public type Carrier<A> is { public value: Type(1), public extra: A };");
    let bytes = encode_metadata(&metadata).unwrap();
    for eager in [false, true] {
        let (checker, errors) = check(decode_metadata(&bytes).unwrap(),
            "fn field(x: test.provider.Carrier<Int>) -> Type(1) { x.value } fn other(x: test.provider.Carrier<Int>) -> Int { x.extra } fn independent(x: test.provider.Carrier<Bool>) -> Bool { x.extra }", eager);
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
        let Type::Record(fields) = checker.resolved_type("__struct_fields_test.provider.Carrier").unwrap() else { panic!("record template") };
        assert_eq!(fields[&Text::from("value")], Type::universe(verum_types::universe_level::UniverseLevel::Concrete(1)));
        let (_, errors) = check(decode_metadata(&bytes).unwrap(),
            "fn bad(x: test.provider.Carrier<Int>) -> Type(0) { x.value }", eager);
        assert!(!errors.is_empty(), "downward universe cast accepted");
        let (_, errors) = check(decode_metadata(&bytes).unwrap(),
            "fn bad(x: test.provider.Carrier<Int>) -> Bool { x.extra }", eager);
        assert!(!errors.is_empty(), "ordinary generic argument erased");
    }
}

#[test]
fn documented_source_and_imported_declarations_have_the_same_level_checks() {
    let source = "public type Carrier<A> is { public value: Type(1), public extra: A }; fn read(x: &Carrier<Int>) -> Type(1) { x.value } fn payload(x: &Carrier<Int>) -> Int { x.extra }";
    let data = CoreMetadata { version: CORE_METADATA_VERSION, ..Default::default() };
    let (_, errors) = check(data.clone(), source, false);
    assert!(errors.is_empty(), "{errors:?}");
    let (_, errors) = check(data, &source.replace("-> Type(1)", "-> Type(0)"), false);
    assert!(!errors.is_empty(), "source declaration permitted a lowered result");
}

#[test]
fn malformed_nominal_carriers_have_no_erased_fallback() {
    let base = metadata("test.provider", "public type Carrier is { public value: Type(1) };");
    for eager in [false, true] {
        let mut wrong = base.clone();
        wrong.types.values_mut().next().unwrap().source_declaration.as_mut().unwrap().name.name = "Different".into();
        assert!(encode_metadata(&wrong).is_err());
        let (_, errors) = check(wrong, "fn f(x: test.provider.Carrier) -> Type(1) { x.value }", eager);
        assert!(errors.iter().any(|e| e.contains("descriptor")), "{errors:?}");
    }
    let mut bytes = encode_metadata(&base).unwrap();
    bytes[8..12].copy_from_slice(&2u32.to_le_bytes());
    assert!(decode_metadata(&bytes).is_err());
}

#[test]
fn protocol_method_results_retain_the_declared_level() {
    let data = metadata("test.provider", "public type Producer is protocol { fn value(&self) -> Type(1); };");
    let bytes = encode_metadata(&data).unwrap();
    for eager in [false, true] {
        for receiver in ["<A: test.provider.Producer>(x: &A)", "(x: &test.provider.Producer)"] {
            let (_, errors) = check(decode_metadata(&bytes).unwrap(),
                &format!("fn read{receiver} -> Type(1) {{ x.value() }}"), eager);
            assert!(errors.is_empty(), "eager={eager}, {receiver}: {errors:?}");
            let (_, errors) = check(decode_metadata(&bytes).unwrap(),
                &format!("fn bad{receiver} -> Type(0) {{ x.value() }}"), eager);
            assert!(!errors.is_empty(), "eager={eager}, {receiver}: protocol method erased a universe level");
        }
    }
}

#[test]
fn named_protocol_methods_check_the_declared_receiver_access() {
    let data = metadata("provider", "public type Counter is protocol { fn read(&self) -> Type(1); fn increment(&mut self) -> Type(1); };");
    let bytes = encode_metadata(&data).unwrap();
    for eager in [false, true] {
        let (_, errors) = check(decode_metadata(&bytes).unwrap(),
            "fn read(x: &provider.Counter) -> Type(1) { x.read() } fn increment(x: &mut provider.Counter) -> Type(1) { x.increment() }", eager);
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");

        let (_, errors) = check(decode_metadata(&bytes).unwrap(),
            "fn invalid(x: &provider.Counter) -> Type(1) { x.increment() }", eager);
        assert!(errors.iter().any(|error| error.contains("cannot write through a shared reference")),
            "eager={eager}: {errors:?}");
    }
}

#[test]
fn legacy_archived_dyn_methods_count_only_the_declared_value_parameters() {
    let owner = "provider";
    let key: Text = "provider.Counter".into();
    let mut data = metadata(owner, "public type Counter is protocol { fn read(&self, value: Int) -> Int; };");
    let parameter = |name: &str, ty: &str| ParamDescriptor {
        name: name.into(), ty: ty.into(), declared_ty: "".into(), has_default: false, default_literal: None,
    };
    let read = MethodSignature {
        name: "read".into(),
        dispatch: MethodDispatch { receiver: ReceiverKind::SelfRef, is_async: false, has_generics: false, returns_self: false },
        params: [parameter("self", "&Self"), parameter("value", "Int")].into_iter().collect(),
        return_type: "Int".into(), contexts: List::new(),
    };
    // Some descriptors carry the implicit receiver as an ABI slot, others already omit it.
    let mut fallback = read.clone();
    fallback.name = "fallback".into();
    fallback.params = List::from_single(parameter("value", "Int"));
    let descriptor = data.types.get_mut(&key).unwrap();
    // Exercise the legacy wire representation deliberately; fresh producers
    // retain the complete protocol declaration.
    descriptor.source_declaration = None;
    assert!(descriptor.source_declaration.is_none());
    descriptor.kind = TypeDescriptorKind::Protocol {
        super_protocols: List::new(), associated_types: List::new(),
        required_methods: List::from_single(read.clone()), default_methods: List::from_single(fallback.name.clone()),
    };
    data.protocols.insert(key, ProtocolDescriptor {
        name: "Counter".into(), module_path: owner.into(), origin_module_path: None, generic_params: List::new(),
        super_protocols: List::new(), associated_types: List::new(), required_methods: List::from_single(read),
        default_methods: List::from_single(fallback), decl_span: None,
    });
    let bytes = encode_metadata(&data).unwrap();
    for eager in [false, true] {
        for method in ["read", "fallback"] {
            for (arguments, valid) in [("1", true), ("true", false), ("", false), ("1, 2", false)] {
                let source = format!("fn call(x: &(dyn provider.Counter)) -> Int {{ x.{method}({arguments}) }}");
                let (_, errors) = check(decode_metadata(&bytes).unwrap(), &source, eager);
                assert_eq!(errors.is_empty(), valid, "eager={eager}, {source}: {errors:?}");
            }
        }
    }
}

#[test]
fn equal_leaf_names_keep_their_declaring_owner() {
    let mut data = metadata("test.first", "public type Carrier<A> is { public value: Type(1), public extra: A };");
    let other = metadata("test.second", "public type Carrier<A> is { public value: Type(2), public extra: A };");
    data.type_declaration_order.extend(other.type_declaration_order);
    for (key, descriptor) in other.types { data.types.insert(key, descriptor); }
    for eager in [false, true] {
        let (_, errors) = check(data.clone(),
            "type Carrier is { public value: Bool }; fn a(x: test.first.Carrier<Int>) -> Type(1) { x.value } fn b(x: test.second.Carrier<Bool>) -> Type(2) { x.value } fn own(x: Carrier) -> Bool { x.value }", eager);
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
        let (_, errors) = check(data.clone(), "fn bad(x: test.second.Carrier<Int>) -> Type(1) { x.value }", eager);
        assert!(!errors.is_empty());
    }
}

#[test]
fn private_type_bindings_survive_metadata_without_bare_name_capture() {
    let mut data = metadata("api", "public type Carrier<A> is { public level: Type, public payload: Renamed, public extra: A };");
    for owner in ["actual", "unrelated"] {
        let dependency = metadata(owner, "public type Token is { public value: Type(1) };");
        data.type_declaration_order.extend(dependency.type_declaration_order);
        for (key, descriptor) in dependency.types { data.types.insert(key, descriptor); }
    }
    // A global convenience alias deliberately points at the wrong declaration.
    data.types.insert("Renamed".into(), data.types.get(&Text::from("unrelated.Token")).unwrap().clone());
    data.declaration_type_bindings.entry("api".into()).or_default()
        .insert("Renamed".into(), Some("actual.Token".into()));
    let source = "fn payload(x: api.Carrier<Int>) -> actual.Token { x.payload } fn extra(x: api.Carrier<Int>) -> Int { x.extra }";
    let bytes = encode_metadata(&data).unwrap();
    for eager in [false, true] {
        let (_, errors) = check(decode_metadata(&bytes).unwrap(), source, eager);
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
        let (_, errors) = check(decode_metadata(&bytes).unwrap(),
            "fn wrong(x: api.Carrier<Int>) -> unrelated.Token { x.payload }", eager);
        assert!(!errors.is_empty(), "private binding captured unrelated Token");
        for binding in [None, Some(None), Some(Some(Text::from("missing.Token")))] {
            let mut bad = data.clone();
            let bindings = bad.declaration_type_bindings.get_mut(&Text::from("api")).unwrap();
            bindings.remove(&Text::from("Renamed"));
            if let Some(binding) = binding { bindings.insert("Renamed".into(), binding); }
            let (checker, errors) = check(bad,
                "fn wrong(x: api.Carrier<Int>) -> unrelated.Token { x.payload }", eager);
            assert!(!errors.is_empty(), "missing lexical authority borrowed global alias, eager={eager}, alias={:?}, fields={:?}", checker.resolved_type("Renamed"), checker.resolved_type("__struct_fields_api.Carrier"));
        }
    }
}

#[test]
fn imported_generic_defaults_use_the_declaration_scope() {
    let mut data = metadata("api", "public type Carrier<A = Renamed, B = A> is { public level: Type, public payload: A };");
    let declaration = data.types.get(&Text::from("api.Carrier")).unwrap().source_declaration.as_ref().unwrap();
    assert!(matches!(&declaration.generics[0].kind, verum_ast::ty::GenericParamKind::Type { default: Some(_), .. }), "{:?}", declaration.generics);
    let dependency = metadata("actual", "public type Token is { public value: Type(1) };");
    data.type_declaration_order.extend(dependency.type_declaration_order);
    for (key, descriptor) in dependency.types { data.types.insert(key, descriptor); }
    data.declaration_type_bindings.entry("api".into()).or_default()
        .insert("Renamed".into(), Some("actual.Token".into()));
    for eager in [false, true] {
        let (checker, errors) = check(data.clone(), "fn inspect(x: api.Carrier<actual.Token, Int>) -> actual.Token { x.payload }", eager);
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
        let Type::Tuple(defaults) = checker.resolved_type("__type_param_defaults_api.Carrier").unwrap_or_else(|| panic!("eager={eager}, bare={:?}", checker.resolved_type("__type_param_defaults_Carrier")))
            else { panic!("default parameter template") };
        assert_eq!(defaults[0].to_string(), "actual.Token");
        assert_eq!(defaults[1].to_string(), "A");
        let mut missing = data.clone();
        missing.declaration_type_bindings.clear();
        let (_, errors) = check(missing, "fn inspect(x: api.Carrier<actual.Token, Int>) -> actual.Token { x.payload }", eager);
        assert!(!errors.is_empty(), "default with missing lexical binding accepted");
    }
}

fn namespace_carrier_metadata(declaration: &str) -> CoreMetadata {
    let mut data = metadata("core.parent.api", declaration);
    for owner in ["core.parent.core.deep", "core.parent.decoy.deep"] {
        let dependency = metadata(owner,
            "public type Envelope<T> is { public level: Type, public value: T };");
        data.type_declaration_order.extend(dependency.type_declaration_order);
        for (key, descriptor) in dependency.types { data.types.insert(key, descriptor); }
    }
    data.declaration_type_bindings.entry("core.parent.api".into()).or_default()
        .insert("selected".into(), Some("core.parent.core".into()));
    data
}

#[test]
fn archived_qualified_generic_heads_replay_their_namespace_binding() {
    let data = namespace_carrier_metadata(
        "public type Carrier<T> is { public level: Type, public payload: selected.deep.Envelope<T> };");
    let bytes = encode_metadata(&data).unwrap();
    for eager in [false, true] {
        let (_, errors) = check(decode_metadata(&bytes).unwrap(),
            "type Envelope<T> is { public wrong: Bool }; fn payload<T>(x: core.parent.api.Carrier<T>) -> core.parent.core.deep.Envelope<T> { x.payload } fn value(x: core.parent.api.Carrier<Int>) -> Int { x.payload.value }",
            eager);
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
        let (_, errors) = check(decode_metadata(&bytes).unwrap(),
            "fn wrong(x: core.parent.api.Carrier<Int>) -> core.parent.decoy.deep.Envelope<Int> { x.payload }", eager);
        assert!(errors.iter().any(|error| error.to_lowercase().contains("mismatch")),
            "foreign owner, eager={eager}: {errors:?}");
    }
}

#[test]
fn archived_qualified_generic_heads_refuse_missing_and_ambiguous_bindings() {
    let data = namespace_carrier_metadata(
        "public type Carrier<T> is { public level: Type, public payload: selected.deep.Envelope<T> };");
    for eager in [false, true] {
        for binding in [None, Some(None), Some(Some(Text::from("unavailable")))] {
            let mut invalid = data.clone();
            let scope = invalid.declaration_type_bindings.get_mut(&Text::from("core.parent.api")).unwrap();
            scope.remove(&Text::from("selected"));
            if let Some(binding) = binding.clone() { scope.insert("selected".into(), binding); }
            let (_, errors) = check(decode_metadata(&encode_metadata(&invalid).unwrap()).unwrap(),
                "fn value(x: core.parent.api.Carrier<Int>) -> Int { x.payload.value }", eager);
            assert!(errors.iter().any(|error| error.contains("selected.deep.Envelope")),
                "binding={binding:?}, eager={eager}: {errors:?}");
        }
    }
}

#[test]
fn archived_qualified_defaults_retain_the_selected_parameter_registration() {
    let data = namespace_carrier_metadata(
        "public type Carrier<A = selected.deep.Envelope<Int>, B = A> is { public level: Type, public payload: A };");
    let bytes = encode_metadata(&data).unwrap();
    for eager in [false, true] {
        let (checker, errors) = check(decode_metadata(&bytes).unwrap(),
            "fn value(x: core.parent.api.Carrier<core.parent.core.deep.Envelope<Int>, Bool>) -> Int { x.payload.value }", eager);
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
        let Type::Tuple(defaults) = checker.resolved_type("__type_param_defaults_core.parent.api.Carrier")
            .expect("the source declaration retains its ordered defaults") else { panic!("default tuple") };
        assert_eq!(defaults[0].to_string(), "core.parent.core.deep.Envelope<Int>");
        assert_eq!(defaults[1].to_string(), "A");
    }
}

#[test]
fn archived_qualified_bindings_cannot_grant_cross_owner_private_access() {
    let mut data = namespace_carrier_metadata(
        "public type Carrier<T> is { public level: Type, public payload: selected.deep.Envelope<T> };");
    let descriptor = data.types.get_mut(&Text::from("core.parent.core.deep.Envelope")).unwrap();
    descriptor.is_public = false;
    descriptor.source_declaration.as_mut().unwrap().visibility = verum_ast::Visibility::Private;
    let bytes = encode_metadata(&data).unwrap();
    for eager in [false, true] {
        let (_, errors) = check(decode_metadata(&bytes).unwrap(),
            "fn value(x: core.parent.api.Carrier<Int>) -> Int { x.payload.value }", eager);
        assert!(errors.iter().any(|error| error.contains("Private") && error.contains("core.parent.core.deep")),
            "eager={eager}: {errors:?}");
    }
}

#[test]
fn archived_qualified_bindings_keep_owner_local_private_access() {
    let mut data = metadata("core.parent.api",
        "public type Carrier<T> is { public level: Type, public payload: selected.Envelope<T> };");
    let mut dependency = metadata("core.parent.api",
        "type Envelope<T> is { public level: Type, public value: T };");
    dependency.types.values_mut().next().unwrap().is_public = false;
    data.type_declaration_order.extend(dependency.type_declaration_order);
    for (key, descriptor) in dependency.types { data.types.insert(key, descriptor); }
    data.declaration_type_bindings.entry("core.parent.api".into()).or_default()
        .insert("selected".into(), Some("core.parent.api".into()));
    let bytes = encode_metadata(&data).unwrap();
    for eager in [false, true] {
        let (_, errors) = check(decode_metadata(&bytes).unwrap(),
            "fn value(x: core.parent.api.Carrier<Int>) -> Int { x.payload.value }", eager);
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
    }
}

#[test]
fn archived_qualified_self_and_parameter_projections_keep_their_receiver() {
    let data = metadata("provider",
        "public type Reader is protocol { type Item; fn level(&self) -> Type(1); fn take(&self, value: Self.Item) -> Int; };");
    let bytes = encode_metadata(&data).unwrap();
    for eager in [false, true] {
        let source = "fn read<A: provider.Reader>(reader: &A, value: A.Item) -> Int { reader.take(value) }";
        let (_, errors) = check(decode_metadata(&bytes).unwrap(), source, eager);
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
        let (_, errors) = check(decode_metadata(&bytes).unwrap(),
            &source.replace("-> Int", "-> Bool"), eager);
        assert!(errors.iter().any(|error| error.to_lowercase().contains("mismatch")),
            "eager={eager}: {errors:?}");
    }
}

#[test]
fn dependent_record_replay_preserves_the_field_binder_and_meta_index() {
    let mut data = metadata("test.pack", "public type Pack<N: meta Int> is { public carrier: Type, public evidence: Witness<carrier, N> };");
    let witness = metadata("test.pack", "public type Witness<A, N: meta Int> is { public level: Type, public item: A };");
    data.type_declaration_order.extend(witness.type_declaration_order);
    for (key, descriptor) in witness.types { data.types.insert(key, descriptor); }
    let bytes = encode_metadata(&data).unwrap();
    for eager in [false, true] {
        let good = "fn pack(w: test.pack.Witness<Int, 0>) -> test.pack.Pack<0> { test.pack.Pack { carrier: Int, evidence: w } }";
        let (checker, errors) = check(decode_metadata(&bytes).unwrap(), good, eager);
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
        let Some(Type::Record(fields)) = checker.resolved_type("__struct_fields_test.pack.Pack") else {
            panic!("missing qualified record")
        };
        let slots = fields[&Text::from("evidence")].record_field_slots();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].owner.as_str(), "test.pack.Pack");
        assert_eq!(slots[0].ordinal, 0);
        let bad = good.replace("Witness<Int, 0>", "Witness<Int, 1>");
        let (_, errors) = check(decode_metadata(&bytes).unwrap(), &bad, eager);
        assert!(errors.iter().any(|e| e.to_lowercase().contains("mismatch")), "eager={eager}: {errors:?}");
    }
}

#[test]
fn field_shadowing_preserves_the_declared_meta_argument_sort() {
    let data = metadata("test.pack", "public type Shadow<N: meta Int> is { public N: Type, public payload: N };");
    let bytes = encode_metadata(&data).unwrap();
    for eager in [false, true] {
        let (_, errors) = check(decode_metadata(&bytes).unwrap(),
            "fn good(x: &test.pack.Shadow<0>) {}", eager);
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
        let (_, errors) = check(decode_metadata(&bytes).unwrap(),
            "fn bad(x: &test.pack.Shadow<Bool>) {}", eager);
        assert!(!errors.is_empty(), "field shadow erased meta argument sort: eager={eager}");
    }
}

#[test]
fn decoded_protocol_checks_the_order_and_sort_of_erased_meta_slots() {
    let data = metadata("test.provider", "public type Predicate<A, N: meta Int> is protocol { fn index(&self) -> Int; };");
    assert_eq!(data.types.values().next().unwrap().generic_params.len(), 1);
    let bytes = encode_metadata(&data).unwrap();
    for eager in [false, true] {
        for arguments in ["Int, 0", "Bool, -1"] {
            let source = format!("fn user(x: &test.provider.Predicate<{arguments}>) {{}}");
            let (_, errors) = check(decode_metadata(&bytes).unwrap(), &source, eager);
            assert!(errors.is_empty(), "eager={eager}, {arguments}: {errors:?}");
        }
        for arguments in ["Int", "0, Int", "Int, Bool", "Int, true", "Int, 0, 1"] {
            let source = format!("fn user(x: &test.provider.Predicate<{arguments}>) {{}}");
            let (_, errors) = check(decode_metadata(&bytes).unwrap(), &source, eager);
            assert!(!errors.is_empty(), "eager={eager}: accepted Predicate<{arguments}>");
        }
    }
}

#[test]
fn nested_type_binders_shadow_without_capturing_or_leaking_a_field() {
    let good = "public type Boxed is { public carrier: Type, public callback: fn<carrier>(carrier) -> carrier, public payload: carrier };";
    let data = CoreMetadata { version: CORE_METADATA_VERSION, ..Default::default() };
    let (checker, errors) = check(data, good, false);
    assert!(errors.is_empty(), "{errors:?}");
    let Some(Type::Record(fields)) = checker.resolved_type("__struct_fields_consumer.Boxed") else {
        panic!("missing local record")
    };
    assert!(fields[&Text::from("callback")].record_field_slots().is_empty());
    assert_eq!(fields[&Text::from("payload")].record_field_slots().len(), 1);
}

#[test]
fn declaration_replay_refuses_field_dependent_array_indices() {
    let data = metadata("test.pack", "public type Bad is { public carrier: Type, public values: [Int; carrier] };");
    for eager in [false, true] {
        let (_, errors) = check(data.clone(), "fn use_bad(x: test.pack.Bad) {}", eager);
        assert!(errors.iter().any(|e| e.contains("value-level record index")), "eager={eager}: {errors:?}");
    }
}

#[test]
#[ignore = "requires the pinned Lean toolchain; checks actual decoded nominal fields"]
fn lean_replays_observed_metadata_field_levels() {
    let mut witness = String::from(include_str!("../../../verification/external/lean/VerumExternalReplay/UniverseInstantiation.lean"));
    for expected in [0u32, 1, 2, u32::MAX] {
        let data = metadata("test.provider", &format!("public type Carrier<A> is {{ public value: Type({expected}), public extra: A }};"));
        let bytes = encode_metadata(&data).unwrap();
        let (checker, errors) = check(decode_metadata(&bytes).unwrap(), "", true);
        assert!(errors.is_empty(), "{errors:?}");
        let Type::Record(fields) = checker.resolved_type("__struct_fields_test.provider.Carrier").unwrap() else { panic!("record") };
        let Type::Universe { level: verum_types::universe_level::UniverseLevel::Concrete(observed) } = &fields[&Text::from("value")] else { panic!("concrete field level") };
        witness.push_str(&format!("\nnamespace NominalField{expected}\nopen Verum.UniverseInstantiation\ndef observed : Formula := .concrete {observed}\ntheorem checked : resolve [] [] 64 true (.concrete {expected}) = some observed := by decide\ntheorem correspondence (p m : Nat → Nat) : denote p m observed = {expected} := by\n  have model : Model [] p m := by intro id value h; cases h\n  exact resolve_sound model checked\n#print axioms correspondence\nend NominalField{expected}\n"));
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("NominalMetadata.lean");
    let replay = || std::process::Command::new(std::env::var_os("VERUM_LEAN").unwrap_or_else(|| "lean".into()))
        .arg(&path).current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verification/external/lean"))
        .output().expect("pinned Lean");
    std::fs::write(&path, &witness).unwrap();
    let output = replay();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    for forbidden in ["sorryAx", "Lean.ofReduceBool", "warning:", "error:"] {
        assert!(!stdout.contains(forbidden) && !stderr.contains(forbidden), "{stdout}\n{stderr}");
    }
    let tampered = witness.replacen("def observed : Formula := .concrete 1\n", "def observed : Formula := .concrete 0\n", 1);
    assert_ne!(tampered, witness);
    std::fs::write(&path, tampered).unwrap();
    assert!(!replay().status.success(), "independent replay accepted a lowered field level");
}

#[test]
fn nominal_record_array_extents_follow_the_declared_field() {
    let m = metadata(
        "test.provider",
        "public type Carrier is { public value: &[[Int; 2]; 3], public level: Type(1) };",
    );
    for (field, abi, admitted) in [
        ("value", "&[[Int; 2]; 3]", true),
        ("value", "&[[Int; 3]; 2]", false),
        ("missing", "&[[Int; 2]; 3]", false),
    ] {
        let mut fixture = m.clone();
        let d = fixture.types.values_mut().next().unwrap();
        let TypeDescriptorKind::Record { fields } = &mut d.kind else {
            panic!("record")
        };
        fields[0].name = field.into();
        fields[0].ty = abi.into();
        assert_eq!(encode_metadata(&fixture).is_ok(), admitted, "{field} {abi}");
    }
}

#[test]
fn protocol_extent_uses_the_value_parameter_after_the_receiver() {
    use verum_ast::decl::{ProtocolItemKind, TypeDeclBody};
    let instance = metadata("test.provider",
        "public type Producer is protocol { fn measure(&self, x: [Int; 2]) -> Type(1); };");
    let associated = metadata("test.provider",
        "public type Producer is protocol { fn measure(x: [Int; 2]) -> Type(1); };");
    let shared = ReceiverKind::SelfRef;
    let static_method = ReceiverKind::None;
    for (base, receiver, slots, valid) in [
        (&instance, shared, vec![("p0", "[Int; 2]")], true),
        (&instance, shared, vec![("p0", "[Int; 3]")], false),
        (&instance, shared, vec![("self", "&Self"), ("p0", "[Int; 2]")], true),
        (&instance, shared, vec![("self", "&Self"), ("p0", "[Int; 3]")], false),
        (&instance, shared, vec![("p0", "&Self"), ("p1", "[Int; 2]")], false),
        (&instance, shared, vec![("self", "[Int; 2]")], false),
        (&instance, shared, vec![("self", "[Int; 3]"), ("p0", "[Int; 2]")], false),
        (&associated, static_method, vec![("self", "[Int; 2]")], true),
        (&associated, static_method, vec![("self", "[Int; 3]")], false),
        (&associated, shared, vec![("self", "[Int; 2]")], false),
    ] {
        let mut data = base.clone();
        let descriptor = data.types.values_mut().next().unwrap();
        let source = descriptor.source_declaration.as_ref().unwrap();
        let TypeDeclBody::Protocol(body) = &source.body else { panic!("protocol") };
        let ProtocolItemKind::Function { decl, .. } = &body.items[0].kind else { panic!("method") };
        let dispatch = MethodDispatch { receiver, ..MethodDispatch::from_declaration(decl) };
        descriptor.kind = TypeDescriptorKind::Protocol {
            super_protocols: List::new(), associated_types: List::new(), default_methods: List::new(),
            required_methods: List::from_single(MethodSignature {
                name: "measure".into(), dispatch,
                params: slots.iter().map(|&(name, ty)| ParamDescriptor {
                    name: name.into(), ty: ty.into(), declared_ty: "".into(), has_default: false, default_literal: None,
                }).collect(),
                return_type: "__opaque_src".into(), contexts: List::new(),
            }),
        };
        assert_eq!(encode_metadata(&data).is_ok(), valid, "{receiver:?} {slots:?}");
    }
}
