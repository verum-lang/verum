use verum_ast::FileId;
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;
fn errors(source: &str) -> Vec<String> {
    let ast = FastParser::new()
        .parse_module_str(source, FileId::new(7))
        .unwrap();
    let mut c = TypeChecker::new();
    c.register_primitives();
    c.set_current_cog("fixture");
    c.set_current_module_path("fixture.owner");
    let mut errors = c
        .prepare_module_declarations(&ast)
        .into_iter()
        .map(|e| format!("{e:?}"))
        .collect::<Vec<_>>();
    for item in &ast.items {
        if let Err(e) = c.check_item(item) {
            errors.push(format!("{e:?}"));
        }
    }
    errors.extend(c.take_deferred_errors().iter().map(|e| format!("{e:?}")));
    errors.extend(
        c.diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| format!("{d:?}")),
    );
    errors
}
fn good(source: &str) {
    let e = errors(source);
    assert!(e.is_empty(), "{source}\n{e:#?}");
}
fn unsafe_dyn(source: &str) {
    let e = errors(source);
    assert!(
        e.iter().any(|e| e.contains("not object-safe")),
        "wrong refusal: {e:#?}"
    );
}
fn bad(source: &str) {
    assert!(
        !errors(source).is_empty(),
        "accepted unproved source: {source}"
    );
}
fn imported(source: &str, provider: &str, extra: Option<&str>) -> Vec<String> {
    use verum_modules::{
        ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
    };
    let mut c = TypeChecker::new();
    c.register_primitives();
    c.set_current_cog("fixture");
    c.set_current_module_path("fixture.main");
    let mut registry = ModuleRegistry::new();
    let mut sources = vec![("fixture.lib", provider), ("fixture.main", source)];
    if let Some(extra) = extra {
        sources.push(("fixture.foreign", extra));
    }
    for (i, (owner, source)) in sources.iter().enumerate() {
        let ast = FastParser::new()
            .parse_module_str(source, FileId::new(i as u32))
            .unwrap();
        let id = ModuleId::new(i as u32);
        let path = ModulePath::from_str(owner);
        let mut info =
            ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, (*source).into());
        info.exports = extract_exports_from_module(&ast, id, &path).unwrap();
        registry.register(info);
    }
    c.set_module_registry_direct(registry.clone());
    let ast = FastParser::new()
        .parse_module_str(source, FileId::new(1))
        .unwrap();
    let mut e = Vec::new();
    for item in &ast.items {
        if let verum_ast::ItemKind::Mount(m) = &item.kind {
            if let Err(x) = c.process_import(m, "fixture.main", &registry) {
                e.push(x.to_string());
            }
        }
    }
    e.extend(c.take_deferred_errors().iter().map(ToString::to_string));
    e.extend(c.diagnostic_sources().iter().map(ToString::to_string));
    e
}

#[test]
fn recursive_dyn_source_header_is_completed() {
    good("type Recursive is protocol { fn next(&self)->&dyn Recursive; };");
}
#[test]
fn recursive_dyn_import_header_is_completed() {
    let e = imported(
        "mount fixture.lib.Recursive;",
        "public type Recursive is protocol { fn next(&self)->&dyn Recursive; };",
        None,
    );
    assert!(e.is_empty(), "{e:#?}");
}
#[test]
fn recursive_dyn_late_generic_method_is_rejected() {
    unsafe_dyn(
        "type Recursive is protocol {fn next(&self)->&dyn Recursive; fn transform<T>(&self,value:T)->T;};",
    );
}
#[test]
fn recursive_dyn_early_generic_method_is_rejected() {
    unsafe_dyn(
        "type Recursive is protocol {fn transform<T>(&self,value:T)->T; fn next(&self)->&dyn Recursive;};",
    );
}
#[test]
fn recursive_dyn_static_method_is_rejected() {
    unsafe_dyn("type Recursive is protocol {fn next(&self)->&dyn Recursive; fn create()->Int;};");
}
#[test]
fn recursive_dyn_self_result_is_rejected() {
    unsafe_dyn(
        "type Recursive is protocol {fn next(&self)->&dyn Recursive; fn copy(&self)->Self;};",
    );
}
#[test]
fn foreign_same_leaf_record_cannot_supply_pending_protocol() {
    let e = imported(
        "mount fixture.lib.Recursive;",
        "mount fixture.foreign.Recursive as Foreign; public type Recursive is protocol { fn next(&self)->&dyn Foreign; };",
        Some("public type Recursive is {value:Int};"),
    );
    assert!(!e.is_empty(), "foreign record admitted as dyn protocol");
}
#[test]
fn missing_dyn_bound_is_rejected() {
    bad("type Recursive is protocol {fn next(&self)->&dyn Missing;};");
}
#[test]
fn mutually_recursive_imported_dyn_headers_are_completed() {
    let e = imported(
        "mount fixture.lib.First;",
        "public type First is protocol {fn next(&self)->&dyn Second;}; public type Second is protocol {fn next(&self)->&dyn First;};",
        None,
    );
    assert!(e.is_empty(), "{e:#?}");
}
#[test]
fn mutually_recursive_unsafe_import_is_rejected() {
    let e = imported(
        "mount fixture.lib.First;",
        "public type First is protocol {fn next(&self)->&dyn Second;fn generic<T>(&self,x:T)->T;}; public type Second is protocol {fn next(&self)->&dyn First;};",
        None,
    );
    assert!(
        e.iter().any(|e| e.contains("not object-safe")),
        "wrong refusal: {e:#?}"
    );
}

#[test]
fn recursive_by_value_receiver_is_rejected() {
    unsafe_dyn("type Recursive is protocol {fn next(&self)->&dyn Recursive;fn consume(self);};");
}
#[test]
fn foreign_same_leaf_unsafe_protocol_cannot_borrow_current_header() {
    let e = imported(
        "mount fixture.lib.Recursive;",
        "mount fixture.foreign.Recursive as Foreign; public type Recursive is protocol {fn next(&self)->&dyn Foreign;};",
        Some("public type Recursive is protocol {fn create()->Int;};"),
    );
    assert!(
        e.iter().any(|e| e.contains("not object-safe")),
        "wrong refusal: {e:#?}"
    );
}
#[test]
fn failed_recursive_group_does_not_publish_or_cache_either_protocol() {
    use verum_modules::{
        ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
    };
    let source = "public type First is protocol {fn next(&self)->&dyn Second;fn generic<T>(&self,x:T)->T;}; public type Second is protocol {fn next(&self)->&dyn First;};";
    let ast = FastParser::new()
        .parse_module_str(source, FileId::new(9))
        .unwrap();
    let path = ModulePath::from_str("fixture.lib");
    let id = ModuleId::new(9);
    let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, source.into());
    info.exports = extract_exports_from_module(&ast, id, &path).unwrap();
    let mut registry = ModuleRegistry::new();
    registry.register(info);
    let mut c = TypeChecker::new();
    c.register_primitives();
    c.set_current_cog("fixture");
    c.set_module_registry_direct(registry);
    for _ in 0..2 {
        let before = c.diagnostic_sources().len();
        c.register_module_protocols(&ast, "fixture.lib");
        assert!(
            c.diagnostic_sources().len() > before,
            "failed group cached as complete"
        );
        for name in ["fixture.lib.First", "fixture.lib.Second"] {
            assert!(
                c.protocol_checker
                    .read()
                    .get_protocol_definition(name)
                    .is_none(),
                "unsafe recursive group published {name}"
            );
        }
    }
}

#[test]
fn imported_recursive_protocol_rename_keeps_defining_owner() {
    let e = imported(
        "mount fixture.lib.Recursive as Renamed;",
        "public type Recursive is protocol {fn next(&self)->&dyn Recursive;};",
        None,
    );
    assert!(e.is_empty(), "{e:#?}");
}
fn replay(source: &str) -> TypeChecker {
    use std::sync::Arc;
    use verum_common::List;
    use verum_types::core_metadata::*;
    let ast = FastParser::new()
        .parse_module_str(source, FileId::new(12))
        .unwrap();
    let mut metadata = CoreMetadata {
        version: CORE_METADATA_VERSION,
        ..Default::default()
    };
    for item in &ast.items {
        let verum_ast::ItemKind::Type(declaration) = &item.kind else {
            panic!("type declaration")
        };
        let name = declaration.name.name.clone();
        let descriptor = TypeDescriptor {
            source_declaration: verum_ast::source_signature::source_type_declaration(declaration)
                .unwrap(),
            name: name.clone(),
            module_path: "owner".into(),
            origin_module_path: None,
            generic_params: List::new(),
            kind: TypeDescriptorKind::Protocol {
                super_protocols: List::new(),
                associated_types: List::new(),
                required_methods: List::new(),
                default_methods: List::new(),
            },
            size: None,
            alignment: None,
            methods: List::new(),
            implements: List::new(),
            decl_span: None,
            is_public: true,
            is_transparent_wrapper: false,
            resource_discipline: verum_common::ResourceDiscipline::Unrestricted,
        };
        let key: verum_common::Text = format!("owner.{name}").into();
        metadata.type_declaration_order.push(key.clone());
        metadata.types.insert(key, descriptor);
    }
    let decoded = decode_metadata(&encode_metadata(&metadata).unwrap()).unwrap();
    TypeChecker::new_with_core_eager(Arc::new(decoded))
}
#[test]
fn recursive_source_sidecar_replays_complete_dyn_header() {
    let mut c = replay("type Recursive is protocol {fn next(&self)->&dyn Recursive;};");
    assert!(c.take_deferred_errors().is_empty());
    assert!(c.diagnostic_sources().is_empty());
    let registry = c.protocol_checker.read();
    let protocol = registry
        .get_protocol_definition("owner.Recursive")
        .expect("complete source protocol");
    assert!(registry.check_object_safety(&protocol.name).is_ok());
    let verum_types::Type::Function { return_type, .. } =
        &protocol.methods.get(&"next".into()).unwrap().ty
    else {
        panic!("function")
    };
    let verum_types::Type::Reference { inner, .. } = return_type.as_ref() else {
        panic!("reference {return_type:?}")
    };
    let verum_types::Type::DynProtocol { bounds, .. } = inner.as_ref() else {
        panic!("dyn result")
    };
    assert_eq!(
        bounds.as_slice(),
        &[verum_common::Text::from("owner.Recursive")]
    );
}
#[test]
fn unsafe_recursive_source_sidecar_never_publishes() {
    let mut c = replay(
        "type Recursive is protocol {fn next(&self)->&dyn Recursive;fn generic<T>(&self,x:T)->T;};",
    );
    let e = c
        .take_deferred_errors()
        .iter()
        .map(ToString::to_string)
        .chain(c.diagnostic_sources().iter().map(ToString::to_string))
        .collect::<Vec<_>>();
    assert!(
        e.iter().any(|e| e.contains("not object-safe")),
        "wrong refusal: {e:#?}"
    );
    assert!(
        c.protocol_checker
            .read()
            .get_protocol_definition("owner.Recursive")
            .is_none()
    );
}

#[test]
fn foreign_safe_protocol_does_not_impose_dyn_on_current_header() {
    let e = imported(
        "mount fixture.lib.Recursive;",
        "mount fixture.foreign.Recursive as Foreign; public type Recursive is protocol {fn next(&self)->&dyn Foreign;fn create()->Int;};",
        Some("public type Recursive is protocol {fn read(&self)->Int;};"),
    );
    assert!(
        e.is_empty(),
        "wrong owner acquired object-safety obligation: {e:#?}"
    );
}

#[test]
fn packaging_a_protocol_requires_implementation_evidence() {
    let declarations = "
        type Sink is protocol { fn put(&mut self, n: Int); };
        type Other is protocol { fn tag(&self) -> Int; };
        type Log is { last: Int };
        type Wrong is { last: Int };
        implement Sink for Log { fn put(&mut self, n: Int) { self.last = n; } }
    ";
    for protocol in ["dyn Sink", "Sink"] {
        let signature = format!("{declarations} fn write(s: &mut {protocol}) {{ s.put(37); }}");
        good(&format!("{signature} fn route(s: &mut Log) {{ write(s); }}"));
        good(&format!("{signature} fn route<T: Sink>(s: &mut T) {{ write(s); }}"));
        bad(&format!("{signature} fn route(s: &mut Wrong) {{ write(s); }}"));
        bad(&format!("{signature} fn route<T>(s: &mut T) {{ write(s); }}"));
        bad(&format!("{signature} fn immutable(s: &Log) {{ write(s); }}"));
    }
    good(&format!("{declarations} fn erase(s: &Log) -> &dyn Sink {{ s as &dyn Sink }}"));
    bad(&format!("{declarations} fn erase(s: &Wrong) -> &dyn Sink {{ s as &dyn Sink }}"));
    bad(&format!("{declarations} fn erase(s: &dyn Sink) -> &Log {{ s as &Log }}"));
    bad(&format!("{declarations} fn both(s: &Log) -> &dyn Sink + Other {{ s }}"));
}

#[test]
fn erasure_preserves_declared_associated_equalities() {
    let declarations = "
        type Source is protocol { type Item; fn read(&self) -> Int; };
        type Log is { last: Int };
        implement Source for Log { type Item = Int; fn read(&self) -> Int { self.last } }
    ";
    good(&format!("{declarations} fn get(x: &Log) -> &dyn Source<Item = Int> {{ x }}"));
    bad(&format!("{declarations} fn get(x: &Log) -> &dyn Source<Item = Bool> {{ x }}"));
    bad(&format!("{declarations} fn get(x: &Log) -> &dyn Source<Missing = Int> {{ x }}"));
}
