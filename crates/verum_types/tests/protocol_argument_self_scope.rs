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
fn bad(source: &str) {
    assert!(
        !errors(source).is_empty(),
        "accepted unproved source: {source}"
    );
}
#[test]
fn declaration_bound_preserves_current_protocol_header() {
    good(
        "type Factory<T> is protocol {}; type Sequence is protocol {type Item; fn collect<C:Factory<Self.Item>>(self,value:C)->C;};",
    );
}
#[test]
fn body_bound_preserves_current_protocol_header() {
    good(
        "type Factory<T> is protocol {}; type Sequence is protocol {type Item; fn collect<C:Factory<Self.Item>>(self,value:C)->C{value}};",
    );
}
#[test]
fn declared_default_self_preserves_projection_owner() {
    good(
        "type Combine<Rhs=Self> is protocol{type Output;fn combine(self,rhs:Rhs)->Self.Output;}; type Sequence is protocol{type Item;fn reduce(self,value:Self.Item)->Self.Item where Self.Item:Combine<Output=Self.Item>;};",
    );
}
#[test]
fn body_default_self_preserves_projection_owner() {
    good(
        "type Combine<Rhs=Self> is protocol{type Output;fn combine(self,rhs:Rhs)->Self.Output;}; type Sequence is protocol{type Item;fn reduce(self,value:Self.Item)->Self.Item where Self.Item:Combine<Output=Self.Item>{value}};",
    );
}
#[test]
fn foreign_protocol_cannot_supply_current_self_member() {
    bad(
        "type Factory<T> is protocol{};type Foreign is protocol{type Item;};type Sequence is protocol{fn collect<C:Factory<Self.Item>>(self,value:C)->C{value}};",
    );
}
#[test]
fn wrong_member_cannot_be_admitted_by_header() {
    bad(
        "type Factory<T> is protocol{};type Sequence is protocol{type Other;fn collect<C:Factory<Self.Item>>(self,value:C)->C{value}};",
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
fn imported_protocol_self_argument_uses_declaring_header() {
    let e = imported(
        "mount fixture.lib.Sequence;",
        "public type Factory<T> is protocol {}; public type Sequence is protocol {type Item; fn collect<C:Factory<Self.Item>>(self,value:C)->C;};",
        None,
    );
    assert!(e.is_empty(), "{e:#?}");
}
#[test]
fn imported_protocol_default_body_self_argument_uses_declaring_header() {
    let e = imported(
        "mount fixture.lib.Sequence;",
        "public type Factory<T> is protocol {}; public type Sequence is protocol {type Item; fn collect<C:Factory<Self.Item>>(self,value:C)->C{value}};",
        None,
    );
    assert!(e.is_empty(), "{e:#?}");
}
#[test]
fn imported_default_argument_preserves_callers_self_projection() {
    let e = imported(
        "mount fixture.lib.Sequence;",
        "public type Combine<Rhs=Self> is protocol{type Output;fn combine(self,rhs:Rhs)->Self.Output;}; public type Sequence is protocol{type Item;fn reduce(self,value:Self.Item)->Self.Item where Self.Item:Combine<Output=Self.Item>{value}};",
        None,
    );
    assert!(e.is_empty(), "{e:#?}");
}
#[test]
fn imported_associated_member_order_preserves_authority() {
    let e = imported(
        "mount fixture.lib.Sequence;",
        "public type Factory<T> is protocol {}; public type Sequence is protocol {fn collect<C:Factory<Self.Item>>(self,value:C)->C; type Item;};",
        None,
    );
    assert!(e.is_empty(), "{e:#?}");
}
#[test]
fn imported_method_parameter_shadows_owner_without_changing_self() {
    let e = imported(
        "mount fixture.lib.Sequence;",
        "public type Factory<T> is protocol {}; public type Sequence<T> is protocol {type Item;fn collect<T:Factory<Self.Item>>(self,value:T)->T;};",
        None,
    );
    assert!(e.is_empty(), "{e:#?}");
}
#[test]
fn imported_wrong_header_is_rejected() {
    let e = imported(
        "mount fixture.lib.Sequence;",
        "public type Factory<T> is protocol {}; public type Sequence is protocol {type Other;fn collect<C:Factory<Self.Item>>(self,value:C)->C;};",
        None,
    );
    assert!(!e.is_empty(), "unproved associated member admitted");
}
#[test]
fn foreign_same_leaf_header_cannot_authorize_imported_self() {
    let e = imported(
        "mount fixture.foreign.Sequence as ForeignSequence; mount fixture.lib.Sequence;",
        "public type Factory<T> is protocol {}; public type Sequence is protocol {type Other;fn collect<C:Factory<Self.Item>>(self,value:C)->C;};",
        Some("public type Sequence is protocol {type Item;};"),
    );
    assert!(!e.is_empty(), "foreign header authorized local Self.Item");
}
#[test]
fn failed_module_prepass_never_publishes_incomplete_protocol() {
    let source = "public type Factory<T> is protocol{}; public type Broken is protocol{fn collect<C:Factory<Self.Item>>(self,value:C)->C;};";
    let ast = FastParser::new()
        .parse_module_str(source, FileId::new(11))
        .unwrap();
    let mut c = TypeChecker::new();
    c.register_primitives();
    c.set_current_cog("fixture");
    c.set_current_module_path("fixture.main");
    c.register_module_protocols(&ast, "fixture.lib");
    assert!(
        c.protocol_checker
            .read()
            .get_protocol_definition("fixture.lib.Broken")
            .is_none(),
        "failed method published incomplete protocol"
    );
    assert!(!c.diagnostic_sources().is_empty());
    let before = c.diagnostic_sources().len();
    c.register_module_protocols(&ast, "fixture.lib");
    assert!(
        c.diagnostic_sources().len() > before,
        "failed scan retained a success marker"
    );
}
