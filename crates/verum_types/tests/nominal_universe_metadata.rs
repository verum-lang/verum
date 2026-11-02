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
        generic_params: decl.generics.iter().map(|p| {
            let verum_ast::ty::GenericParamKind::Type { name, .. } = &p.kind else { panic!("type parameter") };
            GenericParam { name: name.name.clone(), bounds: List::new(), default: None, type_bounds: List::new(), pid: None }
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
