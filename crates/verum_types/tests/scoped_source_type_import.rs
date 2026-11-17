//! T2832: a permitted scoped import must retain its declaration authority.
use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::VerumParser;
use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
use verum_types::TypeChecker;

const PROVIDER: &str = "realm.component.model";
const RECORD: &str = "public(in realm.component) type Packet is { public(in realm.component) value:Int };";

fn registry(owner: &str, source: &str, providers: &[(&str, &str)]) -> ModuleRegistry {
    let mut registry = ModuleRegistry::new();
    for (owner, body) in providers.iter().copied().chain(std::iter::once((owner, source))) {
        let id = registry.allocate_id();
        let file = FileId::new(id.as_u32());
        let source = format!("module {owner}; {body}");
        let ast = VerumParser::new().parse_module_str(&source, file).expect("source grammar");
        let path = ModulePath::from_str(owner);
        let exports = extract_exports_from_module(&ast, id, &path).expect("source exports");
        let mut info = ModuleInfo::new(id, path, ast, file, source.into());
        info.exports = exports;
        registry.register(info);
    }
    verum_modules::resolve_specific_reexport_kinds(&mut registry).unwrap();
    verum_modules::resolve_glob_reexports(&mut registry).unwrap();
    registry
}

fn errors(owner: &str, source: &str, providers: &[(&str, &str)]) -> List<Text> {
    let registry = registry(owner, source, providers);
    let module = registry.get_by_path(owner).unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_module_registry_direct(registry.clone());
    checker.set_current_module_path(owner);
    let mut errors: List<Text> = checker.prepare_module_declarations(&module.ast)
        .into_iter().map(|error| format!("{error:?}").into()).collect();
    if errors.is_empty() {
        checker.with_module_declarations(&module.ast, |checker| {
            for (index, item) in module.ast.items.iter().enumerate() {
                if Some(index) == module.ast.file_header_index() { continue; }
                if let Err(error) = checker.check_item(item) {
                    errors.push(format!("{error:?}").into());
                    break;
                }
            }
        });
    }
    errors.extend(checker.take_deferred_errors().into_iter().map(|error| Text::from(format!("{error:?}"))));
    errors.extend(checker.diagnostic_sources().iter().map(|error| Text::from(format!("{error:?}"))));
    // Field checking can accumulate an E400 and continue, without returning a
    // TypeError. Such diagnostics are still refusals at the compiler boundary.
    errors.extend(checker.diagnostics().iter().filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| Text::from(format!("{diagnostic:?}"))));
    errors
}

fn allowed(owner: &str, source: &str, providers: &[(&str, &str)]) {
    let errors = errors(owner, source, providers);
    assert!(errors.is_empty(), "{owner}: {source}: {errors:#?}");
}

fn denied(owner: &str, source: &str, providers: &[(&str, &str)], name: &str) {
    let errors = errors(owner, source, providers);
    assert!(errors.iter().any(|error| error.contains("VisibilityError") && error.contains(name)),
        "{owner}: {source}: expected access denial for {name}: {errors:#?}");
    assert!(!errors.iter().any(|error| error.contains("unknown declaration visibility")),
        "a missing declaration cannot stand in for an access decision: {errors:#?}");
}

fn operations(name: &str) -> Text {
    format!("fn make()->{name} {{ {name} {{ value:73 }} }}
        fn read(packet:&{name})->Int {{ packet.value }}
        fn write(packet:&mut {name}) {{ packet.value = 11; }}").into()
}

#[test]
fn direct_scoped_import_preserves_record_authority() {
    for owner in ["realm.component", "realm.component.client", "realm.component.model.child"] {
        allowed(owner, &format!("mount {PROVIDER}.Packet; {}", operations("Packet")), &[(PROVIDER, RECORD)]);
    }
}

#[test]
fn renamed_and_reexported_scoped_imports_preserve_authority() {
    let providers = [
        (PROVIDER, RECORD),
        ("realm.component.api", "public(in super) mount realm.component.model.Packet as Ticket;"),
        ("realm.component.facade", "public(in realm.component) mount realm.component.api.Ticket as Token;"),
    ];
    for (mount, name) in [
        ("mount realm.component.model.Packet as Token;", "Token"),
        ("mount realm.component.api.Ticket;", "Ticket"),
        ("mount realm.component.facade.Token;", "Token"),
    ] {
        allowed("realm.component.client", &format!("{mount} {}", operations(name)), &providers);
    }
}

#[test]
fn relative_and_parent_visibility_use_the_declaring_scope() {
    for (visibility, permitted, forbidden) in [
        ("public(in super)", "realm.component.child", "realm.component_extra"),
        ("public(super)", "realm.component", "realm.component.child"),
        ("public(cog)", "realm.other", "foreign.component"),
    ] {
        let provider = format!("{visibility} type Packet is {{ public value:Int }};");
        let source = format!("mount {PROVIDER}.Packet; {}", operations("Packet"));
        allowed(permitted, &source, &[(PROVIDER, &provider)]);
        denied(forbidden, &source, &[(PROVIDER, &provider)], "Packet");
    }
}

#[test]
fn outside_and_prefix_lookalikes_cannot_import_scoped_types() {
    for owner in ["realm.client", "realm.component_extra", "foreign.component"] {
        for body in [Text::from("fn probe() {}"), operations("Packet")] {
            denied(owner, &format!("mount {PROVIDER}.Packet; {body}"), &[(PROVIDER, RECORD)], "Packet");
        }
    }
}

#[test]
fn public_reexports_do_not_widen_the_source_type_scope() {
    let providers = [
        (PROVIDER, RECORD),
        ("realm.component.api", "public mount realm.component.model.Packet as Ticket;"),
        ("realm.component.facade", "public mount realm.component.api.Ticket as Token;"),
    ];
    for (path, name) in [("realm.component.api.Ticket", "Ticket"), ("realm.component.facade.Token", "Token")] {
        allowed("realm.component.client", &format!("mount {path}; {}", operations(name)), &providers);
        // Even an unused mount must report the underlying source refusal.
        denied("realm.outside", &format!("mount {path}; fn probe() {{}}"), &providers, "Packet");
    }
}

#[test]
fn scoped_type_access_does_not_grant_private_field_access() {
    let record = "public(in realm.component) type Packet is { secret:Int, public value:Int };";
    let imports = format!("mount {PROVIDER}.Packet;");
    allowed("realm.component.client", &format!("{imports} fn read(packet:&Packet)->Int {{ packet.value }}"), &[(PROVIDER, record)]);
    for body in [
        "fn read(packet:&Packet)->Int { packet.secret }",
        "fn write(packet:&mut Packet) { packet.secret = 7; }",
        "fn make()->Packet { Packet { secret:7, value:73 } }",
    ] {
        denied("realm.component.client", &format!("{imports} {body}"), &[(PROVIDER, record)], "secret");
    }
}

#[test]
fn permitted_scoped_record_keeps_its_field_types() {
    let record = "public(in realm.component) type Packet<T> is { public(in realm.component) value:T };";
    let providers = [
        (PROVIDER, record),
        ("unrelated", "public type Packet<T> is { public value:Bool };"),
    ];
    let imports = "mount realm.component.model.Packet; mount unrelated.Packet as Other;";
    allowed("realm.component.client", &format!("{imports} fn make()->Packet<Int> {{ Packet {{ value:73 }} }}"), &providers);
    let errors = errors("realm.component.client", &format!("{imports} fn make()->Packet<Int> {{ Packet {{ value:true }} }}"), &providers);
    assert!(errors.iter().any(|error| error.contains("E400") && error.contains("Type mismatch")), "{errors:#?}");
}

#[test]
fn public_archive_resolution_has_no_lexical_privileges() {
    let registry = registry("realm.component.client", "", &[
        (PROVIDER, RECORD),
        ("realm.component.api", "public mount realm.component.model.Packet;"),
        ("open", "public type Packet is { public value:Int };"),
    ]);
    let mut checker = TypeChecker::with_minimal_context();
    checker.set_current_module_path("realm.component.model");
    for owner in [PROVIDER, "realm.component.api"] {
        assert!(checker.resolve_public_source_type(owner, "Packet", &registry).is_none(), "{owner}");
    }
    let (_, owner) = checker.resolve_public_source_type("open", "Packet", &registry).unwrap();
    assert_eq!(owner.as_str(), "open");
}

#[test]
fn glob_order_cannot_replace_a_named_bindings_access_check() {
    for glob_owner in [PROVIDER, "unrelated"] {
        let glob = format!("public mount {glob_owner}.*;");
        let named = format!("public mount {PROVIDER}.Packet;");
        for facade in [format!("{glob} {named}"), format!("{named} {glob}")] {
            let providers = [
                (PROVIDER, RECORD),
                ("unrelated", "public type Packet is { public value:Bool };"),
                ("realm.component.api", facade.as_str()),
            ];
            allowed("realm.component.client",
                &format!("mount realm.component.api.Packet; {}", operations("Packet")), &providers);
            denied("realm.outside", "mount realm.component.api.Packet; fn probe() {}", &providers, "Packet");
        }
    }
}

#[test]
fn public_named_routes_survive_private_lexical_duplicates_in_both_orders() {
    for (private, public, name) in [
        ("mount open.{Packet};", "public mount open.Packet;", "Packet"),
        ("mount open.{Packet as Ticket};", "public mount open.Packet as Ticket;", "Ticket"),
    ] {
        for facade in [format!("{private} {public}"), format!("{public} {private}")] {
            let providers = [
                ("open", "public type Packet is { public value:Int };"),
                ("realm.component.api", facade.as_str()),
            ];
            allowed("outside.client",
                &format!("mount realm.component.api.{name}; {}", operations(name)), &providers);

            let registry = registry("outside.client", "", &providers);
            let checker = TypeChecker::with_minimal_context();
            let (declaration, owner) = checker.resolve_public_source_type(
                "realm.component.api", name, &registry).expect("exact public source declaration");
            assert_eq!(owner.as_str(), "open");
            assert_eq!(declaration.name.name.as_str(), "Packet");
        }
    }
}

#[test]
fn public_named_route_retains_its_source_beside_a_private_homonym() {
    let private = "mount unrelated.Packet as Ticket;";
    let public = "public mount open.Packet as Ticket;";
    for facade in [format!("{private} {public}"), format!("{public} {private}")] {
        let providers = [
            ("open", "public type Packet is { public value:Int };"),
            ("unrelated", "public type Packet is { public value:Bool };"),
            ("realm.component.api", facade.as_str()),
        ];
        let imports = "mount realm.component.api.Ticket;";
        allowed("outside.client", &format!("{imports} {}", operations("Ticket")), &providers);
        let errors = errors("outside.client",
            &format!("{imports} fn make()->Ticket {{ Ticket {{ value:true }} }}"), &providers);
        assert!(errors.iter().any(|error| error.contains("E400") && error.contains("Type mismatch")),
            "public source field authority was replaced: {errors:#?}");
    }
}

#[test]
fn public_route_cannot_widen_a_hidden_canonical_source() {
    for record in [RECORD, "type Packet is { public value:Int };"] {
        let private = "mount unrelated.Packet as Ticket;";
        let public = "public mount realm.component.model.Packet as Ticket;";
        for facade in [format!("{private} {public}"), format!("{public} {private}")] {
            let providers = [
                (PROVIDER, record),
                ("unrelated", "public type Packet is { public value:Bool };"),
                ("realm.component.api", facade.as_str()),
            ];
            let errors = errors("outside.client", "mount realm.component.api.Ticket; fn probe() {}", &providers);
            assert!(errors.iter().any(|error| error.contains("VisibilityError")
                && error.contains("Packet") && error.contains(PROVIDER)),
                "the actual canonical source must refuse access: {errors:#?}");
        }
    }
}

#[test]
fn private_named_binding_still_precedes_an_unrelated_public_glob() {
    let private = "mount realm.component.model.Packet;";
    let glob = "public mount unrelated.*;";
    for facade in [format!("{private} {glob}"), format!("{glob} {private}")] {
        let providers = [
            (PROVIDER, RECORD),
            ("unrelated", "public type Packet is { public value:Bool };"),
            ("realm.component.api", facade.as_str()),
        ];
        denied("outside.client", "mount realm.component.api.Packet; fn probe() {}", &providers, "Packet");
    }
}
