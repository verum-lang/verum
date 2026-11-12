//! Imported spellings select exact declaration owners without publishing homonyms.
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Text};
use verum_fast_parser::VerumParser;
use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
use verum_types::TypeChecker;

fn check(source: &str, providers: &[(&str, &str)]) -> List<Text> {
    let mut registry = ModuleRegistry::new();
    for (owner, body) in providers.iter().chain(std::iter::once(&("cog", source))) {
        let id = registry.allocate_id();
        let file = FileId::new(id.as_u32());
        let ast = VerumParser::new()
            .parse_module_str(body, file)
            .expect("source grammar");
        let path = ModulePath::from_str(owner);
        let exports = extract_exports_from_module(&ast, id, &path).expect("source exports");
        let mut info = ModuleInfo::new(id, path, ast, file, (*body).into());
        info.exports = exports;
        registry.register(info);
    }
    let module = registry.get_by_path("cog").unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_module_registry_direct(registry.clone());
    checker.set_current_module_path("cog");
    for item in &module.ast.items {
        if let ItemKind::Module(declaration) = &item.kind {
            checker.pre_register_module_public(declaration, "cog");
        }
    }
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&module.ast)
        .into_iter()
        .map(|error| format!("{error:?}").into())
        .collect();
    for item in &module.ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors
}

const RECORD_PROVIDERS: &[(&str, &str)] = &[
    (
        "providers.early",
        "public type Token is { public value: Int }; implement Token { public fn get(&self)->Int {self.value} }",
    ),
    (
        "providers.late",
        "public type Token is { public value: Bool }; implement Token { public fn get(&self)->Bool {self.value} }",
    ),
];
const MOUNTS: &[&str] = &[
    "mount providers.early.*; mount providers.late.Token as Second;",
    "mount providers.late.Token as Second; mount providers.early.*;",
    "mount providers.{early.*, late.Token as Second};",
    "mount providers.{late.Token as Second, early.*};",
    "mount providers.{early.*, late.{Token as Second}};",
    "mount providers.{late.{Token as Second}, early.*};",
];

#[test]
fn renamed_record_import_preserves_glob_type_in_both_orders() {
    for mount in MOUNTS {
        let source = format!(
            "{mount} fn first(value: Token)->Int {{value.value}} fn second(value: Second)->Bool {{value.value}}"
        );
        let errors = check(&source, RECORD_PROVIDERS);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
}

#[test]
fn renamed_record_receivers_use_their_declared_implementations() {
    for mount in MOUNTS {
        let source = format!(
            "{mount} fn first(value: Token)->Int {{value.get()}} fn second(value: Second)->Bool {{value.get()}}"
        );
        let errors = check(&source, RECORD_PROVIDERS);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
}

#[test]
fn renamed_record_cannot_accept_a_same_named_foreign_value() {
    let identical_shapes = [
        (
            "providers.early",
            "public type Token is { public value: Int };",
        ),
        (
            "providers.late",
            "public type Token is { public value: Int };",
        ),
    ];
    for providers in [RECORD_PROVIDERS, identical_shapes.as_slice()] {
        for mount in MOUNTS {
            let source = format!("{mount} fn wrong(value: Token)->Second {{value}}");
            let errors = check(&source, providers);
            assert!(
                errors.iter().any(|error| error.contains("Mismatch")),
                "{source}: {errors:?}"
            );
        }
    }
}

#[test]
fn renamed_variant_does_not_replace_the_root_declaration() {
    for mount in [
        "mount other.State as Foreign;",
        "mount other.{State as Foreign};",
    ] {
        for source in [
            format!(
                "public type State is | Cold | Warm; {mount} fn foreign()->Foreign {{Foreign.Hot}} fn probe()->State {{cog.State.Cold}}"
            ),
            format!(
                "{mount} public type State is | Cold | Warm; fn foreign()->Foreign {{Foreign.Hot}} fn probe()->State {{cog.State.Cold}}"
            ),
        ] {
            let errors = check(
                &source,
                &[("other", "public type State is | Hot | Cold(Int);")],
            );
            assert!(errors.is_empty(), "{source}: {errors:?}");
        }
    }
}

#[test]
fn renamed_variant_cannot_supply_a_missing_root_constructor() {
    let errors = check(
        "public type State is | Cold | Warm; mount other.{State as Foreign}; fn probe()->State {cog.State.Hot}",
        &[("other", "public type State is | Hot | Cold(Int);")],
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("UnboundVariable") || error.contains("TypeNotFound")),
        "{errors:?}"
    );
}

#[test]
fn renamed_newtype_constructors_keep_owner_and_argument_contracts() {
    let providers = [
        ("providers.early", "public type Token is (Int);"),
        ("providers.late", "public type Token is (Bool);"),
    ];
    for mount in MOUNTS {
        for body in [
            "fn first()->Token {Token(7)} fn second()->Second {Second(true)}",
            "fn first()->Token {let make=Token; make(7)} fn second()->Second {let make=Second; make(true)}",
        ] {
            let source = format!("{mount} {body}");
            let errors = check(&source, &providers);
            assert!(errors.is_empty(), "{source}: {errors:?}");
        }
        let source = format!("{mount} fn wrong()->Second {{Second(7)}}");
        let errors = check(&source, &providers);
        assert!(
            errors.iter().any(|error| error.contains("Mismatch")),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn a_renamed_import_does_not_mount_the_original_type_spelling() {
    for source in [
        "mount providers.late.Token as Second; fn probe(value: Token)->Second {value}",
        "mount providers.late.{Token as Second}; fn probe(value: Token)->Second {value}",
    ] {
        let errors = check(source, RECORD_PROVIDERS);
        assert!(
            errors.iter().any(|error| error.contains("TypeNotFound")),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn undeclared_nominal_types_have_no_spelling_based_exemption() {
    for name in [
        "Token",
        "DateTime",
        "Resource",
        "Heap",
        "Box",
        "HashMap",
        "Arc",
        "Parser",
        "T",
        "Z",
        "UnlistedType",
    ] {
        let source = format!("fn identity(value: {name})->{name} {{value}}");
        let errors = check(&source, &[]);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("TypeNotFound") && error.contains(name)),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn declared_names_and_real_primitives_remain_available() {
    for source in [
        "fn identity<T>(value:T)->T {value}",
        "fn primitives(value:Int, wide:UInt64, flag:Bool, text:Text)->Int {value}",
        "type Token is {value:Int}; fn identity(value:Token)->Token {value}",
        "mount provider.Token; fn identity(value:Token)->Token {value}",
        "mount provider.Token as Alias; fn identity(value:Alias)->Alias {value}",
    ] {
        let errors = check(
            source,
            &[("provider", "public type Token is {public value:Int};")],
        );
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
}

#[test]
fn lazy_sum_load_retains_every_qualified_constructor_form() {
    let providers = [("provider", "public type Packet<T> is Idle | One(T) | Pair(Int, Bool) | Record { value: Int };")];
    for (mount, spelling) in [
        ("mount provider.Packet;", "Packet"),
        ("mount provider.Packet as Message;", "Message"),
        ("mount provider.{Packet as Message};", "Message"),
        ("mount provider.*;", "Packet"),
    ] {
        let source = format!(r#"
            {mount}
            fn idle()->{spelling}<Int> {{ provider.Packet.Idle }}
            fn one()->{spelling}<Int> {{ provider.Packet.One(7) }}
            fn pair()->{spelling}<Int> {{ provider.Packet.Pair(7, true) }}
            fn record()->{spelling}<Int> {{ provider.Packet.Record {{ value: 7 }} }}
            fn indirect()->{spelling}<Bool> {{ let make = provider.Packet.One; make(true) }}
        "#);
        let errors = check(&source, &providers);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
}
